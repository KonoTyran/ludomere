use super::api::{RemoteObject, Storage};
use crate::{
    domain::{
        CloudSaveConflict, CloudSaveFileMetadata, CloudSaveLocation, CloudSyncMode, CloudSyncResult,
    },
    state::StateStore,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 10_000;
const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;
const BACKUP_GENERATIONS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Baseline {
    local_etag: Option<String>,
    remote_etag: Option<String>,
    metadata: CloudSaveFileMetadata,
}

#[derive(Debug, Clone)]
struct LocalFile {
    metadata: CloudSaveFileMetadata,
    path: PathBuf,
}

pub fn synchronize(
    store: &StateStore,
    product_id: i64,
    locations: &[CloudSaveLocation],
    mode: CloudSyncMode,
    cloud: &dyn Storage,
) -> Result<CloudSyncResult> {
    store.set_cloud_save_status(product_id, crate::domain::CloudSaveStatus::Syncing, None)?;
    let result = run(store, product_id, locations, mode, cloud);
    if let Err(error) = &result {
        store.set_cloud_save_status(
            product_id,
            crate::domain::CloudSaveStatus::Error,
            Some(&format!("{error:#}")),
        )?;
    }
    result
}

fn run(
    store: &StateStore,
    product_id: i64,
    locations: &[CloudSaveLocation],
    mode: CloudSyncMode,
    cloud: &dyn Storage,
) -> Result<CloudSyncResult> {
    let local = scan(locations)?;
    let remote = remote_map(cloud.list()?)?;
    let baseline: BTreeMap<String, Baseline> = store
        .cloud_save_baseline(product_id)?
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default();
    let mut keys = local
        .keys()
        .chain(remote.keys())
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    let mut result = CloudSyncResult::default();
    let mut next = baseline.clone();

    for key in keys {
        let local_file = local.get(&key);
        let remote_file = remote.get(&key);
        let previous = baseline.get(&key);
        let local_changed =
            local_file.map(|f| Some(&f.metadata.etag)) != previous.map(|b| b.local_etag.as_ref());
        let remote_changed =
            remote_file.map(|f| Some(&f.etag)) != previous.map(|b| b.remote_etag.as_ref());
        let conflict =
            local_file.is_some() && remote_file.is_some() && local_changed && remote_changed;
        if conflict && mode == CloudSyncMode::Normal {
            result.conflicts.push(conflict_for(local_file, remote_file));
            continue;
        }
        match (local_file, remote_file, mode) {
            (Some(local), _, CloudSyncMode::ForceUpload)
            | (Some(local), None, _)
            | (Some(local), Some(_), CloudSyncMode::Normal)
                if local_changed =>
            {
                let bytes = read_file(&local.path)?;
                let uploaded = cloud.upload(
                    &local.metadata.location,
                    &remote_path(&local.metadata),
                    &bytes,
                    local.metadata.modified_at,
                )?;
                next.insert(
                    key,
                    Baseline {
                        local_etag: Some(local.metadata.etag.clone()),
                        remote_etag: Some(uploaded.etag),
                        metadata: local.metadata.clone(),
                    },
                );
                result.uploaded += 1;
            }
            (Some(local), Some(remote), CloudSyncMode::ForceDownload)
            | (Some(local), Some(remote), CloudSyncMode::Normal)
                if remote_changed =>
            {
                backup(product_id, local)?;
                write_download(cloud, remote, &local.path)?;
                let refreshed = local_metadata(
                    &local.metadata.location,
                    &local.metadata.relative_path,
                    &local.path,
                )?;
                next.insert(
                    key,
                    Baseline {
                        local_etag: Some(refreshed.etag.clone()),
                        remote_etag: Some(remote.etag.clone()),
                        metadata: refreshed,
                    },
                );
                result.downloaded += 1;
            }
            (None, Some(remote), _) => {
                let location = locations
                    .iter()
                    .find(|location| location.remote_namespace == remote.namespace)
                    .context("cloud save has an unknown namespace")?;
                let relative = safe_relative(&remote.path)?;
                let destination = location.path.join(&relative);
                write_download(cloud, remote, &destination)?;
                let metadata = local_metadata(&location.name, &relative, &destination)?;
                next.insert(
                    key,
                    Baseline {
                        local_etag: Some(metadata.etag.clone()),
                        remote_etag: Some(remote.etag.clone()),
                        metadata,
                    },
                );
                result.downloaded += 1;
            }
            (None, None, _) => {}
            (Some(local), None, _) => {
                // Local deletion never propagates: retain the remote baseline if one existed.
                if let Some(previous) = previous {
                    next.insert(key, previous.clone());
                } else {
                    next.remove(&key);
                }
                let _ = local;
            }
            (Some(local), Some(remote), _) => {
                next.insert(
                    key,
                    Baseline {
                        local_etag: Some(local.metadata.etag.clone()),
                        remote_etag: Some(remote.etag.clone()),
                        metadata: local.metadata.clone(),
                    },
                );
            }
        }
    }
    if !result.conflicts.is_empty() {
        store.record_cloud_save_conflicts(product_id, &result.conflicts)?;
        return Ok(result);
    }
    store.complete_cloud_save_sync(product_id, &serde_json::to_string(&next)?)?;
    Ok(result)
}

fn scan(locations: &[CloudSaveLocation]) -> Result<BTreeMap<String, LocalFile>> {
    let mut files = BTreeMap::new();
    for location in locations {
        if !location.path.exists() {
            continue;
        }
        let root = fs::canonicalize(&location.path)?;
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    bail!("cloud-save directories may not contain symbolic links");
                }
                if file_type.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if files.len() >= MAX_FILES {
                    bail!("cloud-save locations contain too many files");
                }
                let path = fs::canonicalize(entry.path())?;
                if !path.starts_with(&root) {
                    bail!("cloud-save file escapes its configured location");
                }
                let relative = path.strip_prefix(&root)?.to_owned();
                let metadata = local_metadata(&location.remote_namespace, &relative, &path)?;
                files.insert(
                    key(&location.remote_namespace, &relative),
                    LocalFile { metadata, path },
                );
            }
        }
    }
    Ok(files)
}

fn local_metadata(location: &str, relative: &Path, path: &Path) -> Result<CloudSaveFileMetadata> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_FILE_SIZE {
        bail!("cloud-save file exceeds the safety limit");
    }
    let mut file = fs::File::open(path)?;
    let mut digest = md5::Context::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.consume(&buffer[..read]);
    }
    Ok(CloudSaveFileMetadata {
        location: location.into(),
        relative_path: relative.into(),
        size: metadata.len(),
        modified_at: metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64,
        etag: format!("{:x}", digest.compute()),
    })
}

fn remote_map(objects: Vec<RemoteObject>) -> Result<HashMap<String, RemoteObject>> {
    let mut map = HashMap::new();
    for object in objects {
        let relative = safe_relative(&object.path)?;
        map.insert(key(&object.namespace, &relative), object);
    }
    Ok(map)
}

fn key(namespace: &str, path: &Path) -> String {
    format!("{namespace}/{}", path.to_string_lossy().replace('\\', "/"))
}
fn remote_path(metadata: &CloudSaveFileMetadata) -> String {
    metadata.relative_path.to_string_lossy().replace('\\', "/")
}
fn safe_relative(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        bail!("cloud-save service returned an unsafe path");
    }
    Ok(path)
}
fn read_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_FILE_SIZE {
        bail!("cloud-save file exceeds the safety limit");
    }
    fs::read(path).map_err(Into::into)
}
fn write_download(cloud: &dyn Storage, remote: &RemoteObject, destination: &Path) -> Result<()> {
    let bytes = cloud.download(&remote.namespace, &remote.path)?;
    if bytes.len() > MAX_FILE_SIZE as usize {
        bail!("cloud-save file exceeds the safety limit");
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("ludomere-cloud-part");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, destination)?;
    Ok(())
}
fn conflict_for(local: Option<&LocalFile>, remote: Option<&RemoteObject>) -> CloudSaveConflict {
    let remote_metadata = remote.map(|file| CloudSaveFileMetadata {
        location: file.namespace.clone(),
        relative_path: PathBuf::from(&file.path),
        size: file.size,
        modified_at: file.modified_at,
        etag: file.etag.clone(),
    });
    CloudSaveConflict {
        location: local
            .map(|f| f.metadata.location.clone())
            .or_else(|| remote.map(|f| f.namespace.clone()))
            .unwrap_or_default(),
        relative_path: local
            .map(|f| f.metadata.relative_path.clone())
            .or_else(|| remote.map(|f| PathBuf::from(&f.path)))
            .unwrap_or_default(),
        local: local.map(|f| f.metadata.clone()),
        remote: remote_metadata,
    }
}
fn backup(product_id: i64, file: &LocalFile) -> Result<()> {
    let root = crate::identity::data_root()
        .join("cloud-save-backups")
        .join(product_id.to_string());
    fs::create_dir_all(&root)?;
    let generation = root.join(chrono::Utc::now().timestamp_millis().to_string());
    let destination = generation
        .join(&file.metadata.location)
        .join(&file.metadata.relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&file.path, destination)?;
    let mut generations = fs::read_dir(&root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    generations.sort_by_key(|entry| entry.file_name());
    let remove = generations.len().saturating_sub(BACKUP_GENERATIONS);
    for old in generations.into_iter().take(remove) {
        fs::remove_dir_all(old.path())?;
    }
    Ok(())
}

pub fn backup_directory(product_id: i64) -> PathBuf {
    crate::identity::data_root()
        .join("cloud-save-backups")
        .join(product_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryCloud(Mutex<HashMap<String, (Vec<u8>, RemoteObject)>>);
    impl Storage for MemoryCloud {
        fn list(&self) -> Result<Vec<RemoteObject>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .values()
                .map(|(_, object)| object.clone())
                .collect())
        }
        fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&format!("{namespace}/{path}"))
                .context("missing object")?
                .0
                .clone())
        }
        fn upload(
            &self,
            namespace: &str,
            path: &str,
            data: &[u8],
            modified_at: i64,
        ) -> Result<RemoteObject> {
            let object = RemoteObject {
                namespace: namespace.into(),
                path: path.into(),
                size: data.len() as u64,
                modified_at,
                etag: format!("{:x}", md5::compute(data)),
            };
            self.0.lock().unwrap().insert(
                format!("{namespace}/{path}"),
                (data.to_vec(), object.clone()),
            );
            Ok(object)
        }

        fn delete(&self, namespace: &str, path: &str, _etag: Option<&str>) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .remove(&format!("{namespace}/{path}"));
            Ok(())
        }
    }

    fn fixture(name: &str) -> (PathBuf, StateStore, CloudSaveLocation) {
        let root =
            std::env::temp_dir().join(format!("ludomere-cloud-sync-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("saves")).unwrap();
        let store = StateStore::open_at(&root.join("state.sqlite3")).unwrap();
        let location = CloudSaveLocation {
            name: "main".into(),
            path: root.join("saves"),
            remote_namespace: "main".into(),
            user_override: false,
        };
        (root, store, location)
    }

    #[test]
    fn initial_local_file_uploads() {
        let (root, store, location) = fixture("upload");
        fs::write(location.path.join("save.dat"), b"local").unwrap();
        let cloud = MemoryCloud::default();
        let result = synchronize(
            &store,
            1,
            &[location],
            crate::domain::CloudSyncMode::Normal,
            &cloud,
        )
        .unwrap();
        assert_eq!(result.uploaded, 1);
        assert!(cloud.0.lock().unwrap().contains_key("main/save.dat"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn simultaneous_changes_conflict_without_overwrite() {
        let (root, store, location) = fixture("conflict");
        fs::write(location.path.join("save.dat"), b"first").unwrap();
        let cloud = MemoryCloud::default();
        synchronize(
            &store,
            2,
            std::slice::from_ref(&location),
            crate::domain::CloudSyncMode::Normal,
            &cloud,
        )
        .unwrap();
        fs::write(location.path.join("save.dat"), b"local changed").unwrap();
        let object = RemoteObject {
            namespace: "main".into(),
            path: "save.dat".into(),
            size: 13,
            modified_at: 2,
            etag: "remote-changed".into(),
        };
        cloud
            .0
            .lock()
            .unwrap()
            .insert("main/save.dat".into(), (b"cloud changed".to_vec(), object));
        let result = synchronize(
            &store,
            2,
            &[location],
            crate::domain::CloudSyncMode::Normal,
            &cloud,
        )
        .unwrap();
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(
            fs::read(root.join("saves/save.dat")).unwrap(),
            b"local changed"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
