//! Explicit cloud exports and recovery-first remote deletion. Worker-only operations.
use super::api::{RemoteObject, Storage};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub struct ManagementSession {
    pub game: crate::domain::InstalledGame,
    pub account_id: String,
    pub account_session: u64,
    pub cancelled: Arc<AtomicBool>,
}

impl ManagementSession {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire)
            || crate::online::account_session() != self.account_session
        {
            bail!("cloud-save operation cancelled because its account session ended");
        }
        if crate::auth::load_saved_token()?.is_none_or(|token| token.user_id != self.account_id) {
            bail!("the GOG account changed; reopen cloud-save management");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudExportEntry {
    pub namespace: String,
    pub path: String,
    pub remote_size: u64,
    pub exported_size: u64,
    pub modified_at: i64,
    pub remote_revision: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudExportManifest {
    pub format_version: u32,
    pub product_id: i64,
    pub exported_at: i64,
    pub files: Vec<CloudExportEntry>,
}

#[derive(Debug)]
pub struct CloudDeletionReport {
    pub deleted: usize,
    pub requested: usize,
    pub recovery_snapshot: PathBuf,
    pub error: Option<String>,
}

pub fn inventory(session: &ManagementSession) -> Result<Vec<RemoteObject>> {
    let _activity = crate::profile_reset::begin_activity("cloud inventory")?;
    let _operation = super::begin_operation(session.game.product_id)?;
    session.check()?;
    let cloud = super::authenticated_storage(&session.game, &session.account_id)?;
    let objects = cloud.list().map_err(|_| {
        anyhow::anyhow!("could not load remote saves; check your connection and retry")
    })?;
    session.check()?;
    validate_objects(&objects)?;
    Ok(objects)
}

pub fn export(session: &ManagementSession, destination: &Path) -> Result<PathBuf> {
    let _activity = crate::profile_reset::begin_activity("cloud export")?;
    let _operation = super::begin_operation(session.game.product_id)?;
    session.check()?;
    let cloud = super::authenticated_storage(&session.game, &session.account_id)?;
    let objects = cloud
        .list()
        .map_err(|_| anyhow::anyhow!("could not load remote saves for export"))?;
    export_objects(
        session.game.product_id,
        destination,
        &objects,
        &cloud,
        &|| session.check(),
    )
}

pub fn delete(
    session: &ManagementSession,
    selected: &[RemoteObject],
) -> Result<CloudDeletionReport> {
    let _activity = crate::profile_reset::begin_activity("cloud deletion")?;
    let _operation = super::begin_operation(session.game.product_id)?;
    let check = || {
        session.check()?;
        if crate::installation::is_game_running(session.game.product_id) {
            bail!("close the game before deleting remote saves");
        }
        Ok(())
    };
    check()?;
    let cloud = super::authenticated_storage(&session.game, &session.account_id)?;
    let store = crate::state::StateStore::open()?;
    let locations = store.cloud_save_record(session.game.product_id)?.locations;
    delete_objects(
        &store,
        session.game.product_id,
        &locations,
        selected,
        &crate::identity::data_root()
            .join("cloud-save-deletion-recovery")
            .join(session.game.product_id.to_string()),
        &cloud,
        &check,
    )
}

fn delete_objects(
    store: &crate::state::StateStore,
    product_id: i64,
    locations: &[crate::domain::CloudSaveLocation],
    selected: &[RemoteObject],
    recovery_root: &Path,
    cloud: &dyn Storage,
    check: &dyn Fn() -> Result<()>,
) -> Result<CloudDeletionReport> {
    if selected.is_empty() {
        bail!("select at least one remote save");
    }
    let account_id = cloud
        .account_id()
        .context("cloud deletion requires an identified GOG account")?;
    validate_objects(selected)?;
    check()?;
    current_selection(cloud, selected)?;
    let recovery_snapshot = export_objects(product_id, recovery_root, selected, cloud, check)?;
    let tombstones = selected
        .iter()
        .map(|object| {
            Ok(crate::state::CloudSaveTombstone {
                namespace: object.namespace.clone(),
                path: object.path.clone(),
                remote_etag: object.etag.clone(),
                local_etag: local_etag(locations, object)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    check()?;
    // Intent is durable before the first irreversible request. A crash or lost DELETE response
    // therefore cannot allow a normal sync to reupload an unchanged local copy.
    store.record_cloud_save_tombstones(account_id, product_id, &tombstones)?;
    let mut report = CloudDeletionReport {
        deleted: 0,
        requested: selected.len(),
        recovery_snapshot,
        error: None,
    };
    for object in selected {
        let result = (|| {
            check()?;
            current_selection(cloud, std::slice::from_ref(object))?;
            check()?;
            cloud.delete_revision(object)
        })();
        match result {
            Ok(()) => report.deleted += 1,
            Err(error) => {
                report.error = Some(error.to_string());
                break;
            }
        }
    }
    Ok(report)
}

fn current_selection(cloud: &dyn Storage, selected: &[RemoteObject]) -> Result<()> {
    let current = cloud.list().map_err(|_| {
        anyhow::anyhow!("could not recheck remote revisions; no further deletion was attempted")
    })?;
    validate_objects(&current)?;
    let current = current
        .iter()
        .map(|object| ((object.namespace.as_str(), object.path.as_str()), object))
        .collect::<HashMap<_, _>>();
    for selected in selected {
        if current
            .get(&(selected.namespace.as_str(), selected.path.as_str()))
            .is_none_or(|current| **current != *selected)
        {
            bail!("remote saves changed; refresh the inventory before retrying");
        }
    }
    Ok(())
}

fn export_objects(
    product_id: i64,
    destination: &Path,
    objects: &[RemoteObject],
    cloud: &dyn Storage,
    check: &dyn Fn() -> Result<()>,
) -> Result<PathBuf> {
    validate_objects(objects)?;
    check()?;
    let destination_handle = absolute_directory(destination, true)?;
    let staging = tempfile::Builder::new()
        .prefix(".ludomere-cloud-partial-")
        .tempdir_in(format!("/proc/self/fd/{}", destination_handle.as_raw_fd()))?;
    let staging_name = staging.path().file_name().unwrap();
    let staging_handle = directory_at(&destination_handle, Path::new(staging_name), false)?;
    let staging_path = PathBuf::from(format!("/proc/self/fd/{}", staging_handle.as_raw_fd()));
    let mut manifest = CloudExportManifest {
        format_version: 1,
        product_id,
        exported_at: chrono::Utc::now().timestamp(),
        files: Vec::new(),
    };
    let mut total = 0_u64;
    for object in objects {
        check()?;
        let bytes = cloud.download_revision(object)?;
        total = total.saturating_add(bytes.len() as u64);
        if bytes.len() > 256 * 1024 * 1024 || total > 4 * 1024 * 1024 * 1024 {
            bail!("cloud export exceeds the safety limit");
        }
        let relative = safe_export_path(&object.namespace, &object.path)?;
        let parent = directory_at(&staging_handle, relative.parent().unwrap(), true)?;
        let path = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
            .join(relative.file_name().unwrap());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        parent.sync_all()?;
        manifest.files.push(CloudExportEntry {
            namespace: object.namespace.clone(),
            path: object.path.clone(),
            remote_size: object.size,
            exported_size: bytes.len() as u64,
            modified_at: object.modified_at,
            remote_revision: object.etag.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        });
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(staging_path.join("manifest.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.sync_all()?;
    verify_export(&staging_path, &manifest)?;
    check()?;
    current_selection(cloud, objects)?;
    check()?;
    verify_directory_identity(destination, &destination_handle)?;
    let suffix = staging_name
        .to_string_lossy()
        .replace(".ludomere-cloud-partial-", "");
    let name = format!(
        "ludomere-cloud-export-{product_id}-{}-{suffix}",
        manifest.exported_at
    );
    let completed = destination.join(&name);
    staging_handle.sync_all()?;
    let from = std::ffi::CString::new(staging_name.as_encoded_bytes())?;
    let to = std::ffi::CString::new(name)?;
    // Both names remain relative to the originally selected directory, even if an ancestor moves.
    if unsafe {
        libc::renameat2(
            destination_handle.as_raw_fd(),
            from.as_ptr(),
            destination_handle.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    destination_handle.sync_all()?;
    verify_directory_identity(destination, &destination_handle)?;
    Ok(completed)
}

fn verify_directory_identity(path: &Path, expected: &fs::File) -> Result<()> {
    let actual = absolute_directory(path, false)?.metadata()?;
    let expected = expected.metadata()?;
    if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        bail!("export directory changed during the operation; nothing was deleted");
    }
    Ok(())
}

fn verify_export(destination: &Path, manifest: &CloudExportManifest) -> Result<()> {
    let saved: CloudExportManifest =
        serde_json::from_slice(&fs::read(destination.join("manifest.json"))?)?;
    if &saved != manifest {
        bail!("cloud export manifest verification failed");
    }
    for entry in &manifest.files {
        let bytes = fs::read(destination.join(safe_export_path(&entry.namespace, &entry.path)?))?;
        if bytes.len() as u64 != entry.exported_size
            || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
        {
            bail!("cloud export checksum verification failed");
        }
    }
    Ok(())
}

fn validate_objects(objects: &[RemoteObject]) -> Result<()> {
    if objects.len() > 10_000 {
        bail!("cloud inventory exceeds the safety limit");
    }
    let mut paths = HashSet::new();
    for object in objects {
        let path = safe_export_path(&object.namespace, &object.path)?
            .to_string_lossy()
            .to_lowercase();
        if !paths.insert(path) {
            bail!("cloud inventory contains duplicate or case-colliding paths");
        }
        super::api::conditional_revision(&object.etag)?;
    }
    Ok(())
}

fn safe_export_path(namespace: &str, path: &str) -> Result<PathBuf> {
    if namespace.is_empty() || namespace.contains('/') || path.is_empty() {
        bail!("cloud save has an empty or invalid namespace/path");
    }
    for value in [namespace, path] {
        if value.starts_with('/')
            || value.split('/').any(|part| {
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || part.ends_with(['.', ' '])
                    || part
                        .chars()
                        .any(|c| c.is_control() || r#"<>:"\|?*"#.contains(c))
            })
        {
            bail!("cloud save has an unsafe path");
        }
    }
    Ok(PathBuf::from(namespace).join(path))
}

fn absolute_directory(path: &Path, create: bool) -> Result<fs::File> {
    if !path.is_absolute() {
        bail!("choose an absolute export directory");
    }
    directory_at(&fs::File::open("/")?, path.strip_prefix("/")?, create)
}

fn directory_at(base: &fs::File, relative: &Path, create: bool) -> Result<fs::File> {
    let mut directory = base.try_clone()?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("unsafe cloud-save directory")
        };
        let name = std::ffi::CString::new(name.as_encoded_bytes())?;
        let open = || unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let mut fd = open();
        if fd < 0
            && create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            directory.sync_all()?;
            fd = open();
        }
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(directory)
}

fn local_etag(
    locations: &[crate::domain::CloudSaveLocation],
    object: &RemoteObject,
) -> Result<Option<String>> {
    let Some(location) = locations
        .iter()
        .find(|location| location.remote_namespace == object.namespace)
    else {
        return Ok(None);
    };
    safe_export_path(&object.namespace, &object.path)?;
    let path = location.path.join(&object.path);
    let parent = match absolute_directory(path.parent().context("local save has no parent")?, false)
    {
        Ok(parent) => parent,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let path = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
        .join(path.file_name().context("local save has no filename")?);
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > 256 * 1024 * 1024 {
        bail!("local cloud save is not a bounded regular file");
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 * 1024 {
        bail!("local cloud save exceeds the safety limit");
    }
    Ok(Some(format!("{:x}", md5::compute(bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MemoryCloud {
        objects: Mutex<HashMap<String, (RemoteObject, Vec<u8>)>>,
        database: PathBuf,
        fail_delete: bool,
        lose_reply: bool,
        mutate_download: bool,
        swap_directory: Option<(PathBuf, PathBuf, PathBuf)>,
        account: &'static str,
        cancel_on_list: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }

    impl Storage for MemoryCloud {
        fn account_id(&self) -> Option<&str> {
            Some(self.account)
        }
        fn list(&self) -> Result<Vec<RemoteObject>> {
            if let Some(cancelled) = &self.cancel_on_list {
                // Model cancellation/account invalidation arriving during the last network list.
                cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
            Ok(self
                .objects
                .lock()
                .unwrap()
                .values()
                .map(|(object, _)| object.clone())
                .collect())
        }
        fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(&format!("{namespace}/{path}"))
                .context("missing fixture object")?
                .1
                .clone())
        }
        fn download_revision(&self, selected: &RemoteObject) -> Result<Vec<u8>> {
            if let Some((old, moved, outside)) = &self.swap_directory {
                fs::rename(old, moved)?;
                std::os::unix::fs::symlink(outside, old)?;
            }
            let mut objects = self.objects.lock().unwrap();
            let (current, bytes) = objects
                .get_mut(&format!("{}/{}", selected.namespace, selected.path))
                .context("missing fixture object")?;
            if current != selected {
                bail!("revision changed");
            }
            if self.mutate_download {
                current.etag = "changed".into();
            }
            Ok(bytes.clone())
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
            self.objects.lock().unwrap().insert(
                format!("{namespace}/{path}"),
                (object.clone(), data.to_vec()),
            );
            Ok(object)
        }
        fn delete_revision(&self, selected: &RemoteObject) -> Result<()> {
            let store = crate::state::StateStore::open_at(&self.database)?;
            assert!(
                store
                    .cloud_save_tombstones(self.account, 42)?
                    .iter()
                    .any(|entry| entry.namespace == selected.namespace
                        && entry.path == selected.path)
            );
            if self.fail_delete && selected.path == "second.dat" {
                bail!("fixture deletion rejected");
            }
            let mut objects = self.objects.lock().unwrap();
            let key = format!("{}/{}", selected.namespace, selected.path);
            if objects
                .get(&key)
                .is_none_or(|(current, _)| current != selected)
            {
                bail!("revision changed");
            }
            objects.remove(&key);
            if self.lose_reply {
                bail!("fixture response lost");
            }
            Ok(())
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        crate::state::StateStore,
        MemoryCloud,
        crate::domain::CloudSaveLocation,
    ) {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.sqlite3");
        let store = crate::state::StateStore::open_at(&database).unwrap();
        let object = RemoteObject {
            namespace: "main".into(),
            path: "profile/save.dat".into(),
            size: 6,
            modified_at: 10,
            etag: "revision".into(),
        };
        let cloud = MemoryCloud {
            objects: Mutex::new(HashMap::from([(
                "main/profile/save.dat".into(),
                (object, b"remote".to_vec()),
            )])),
            database,
            fail_delete: false,
            lose_reply: false,
            mutate_download: false,
            swap_directory: None,
            account: "fixture-account",
            cancel_on_list: None,
        };
        let location = crate::domain::CloudSaveLocation {
            name: "main".into(),
            remote_namespace: "main".into(),
            path: root.path().join("local"),
            user_override: false,
        };
        fs::create_dir_all(location.path.join("profile")).unwrap();
        fs::write(location.path.join("profile/save.dat"), b"local").unwrap();
        (root, store, cloud, location)
    }

    #[test]
    fn export_preserves_paths_and_checksums_and_refuses_changed_revisions() {
        let (root, _, mut cloud, _) = fixture();
        let objects = cloud.list().unwrap();
        let output = export_objects(42, &root.path().join("exports"), &objects, &cloud, &|| {
            Ok(())
        })
        .unwrap();
        assert_eq!(
            fs::read(output.join("main/profile/save.dat")).unwrap(),
            b"remote"
        );
        let manifest: CloudExportManifest =
            serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.files[0].remote_revision, "revision");
        verify_export(&output, &manifest).unwrap();
        fs::write(output.join("main/profile/save.dat"), b"damaged").unwrap();
        assert!(verify_export(&output, &manifest).is_err());
        cloud.mutate_download = true;
        let other = root.path().join("changed");
        assert!(export_objects(42, &other, &objects, &cloud, &|| Ok(())).is_err());
        assert_eq!(fs::read_dir(other).unwrap().count(), 0);
    }

    #[test]
    fn final_revision_list_cancellation_does_not_publish_export() {
        let (root, _, mut cloud, _) = fixture();
        let objects = cloud.list().unwrap();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        cloud.cancel_on_list = Some(cancelled.clone());
        let destination = root.path().join("exports");
        let result = export_objects(42, &destination, &objects, &cloud, &|| {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                bail!("account changed or cancelled");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
        assert_eq!(cloud.objects.lock().unwrap().len(), 1);
    }

    #[test]
    fn durable_intent_precedes_delete_and_suppresses_only_unchanged_same_account_local_saves() {
        let (root, store, mut cloud, location) = fixture();
        let selected = cloud.list().unwrap();
        let report = delete_objects(
            &store,
            42,
            std::slice::from_ref(&location),
            &selected,
            &root.path().join("recovery"),
            &cloud,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(report.deleted, 1);
        assert!(report.error.is_none());
        assert_eq!(
            fs::read(report.recovery_snapshot.join("main/profile/save.dat")).unwrap(),
            b"remote"
        );
        let result = crate::cloud_saves::sync::synchronize(
            &store,
            42,
            std::slice::from_ref(&location),
            crate::domain::CloudSyncMode::Normal,
            &cloud,
        )
        .unwrap();
        assert_eq!(result.uploaded, 0);
        cloud.account = "other-account";
        assert_eq!(
            crate::cloud_saves::sync::synchronize(
                &store,
                42,
                std::slice::from_ref(&location),
                crate::domain::CloudSyncMode::Normal,
                &cloud
            )
            .unwrap()
            .uploaded,
            1
        );
        cloud.account = "fixture-account";
        cloud.objects.lock().unwrap().clear();
        fs::write(location.path.join("profile/save.dat"), b"changed-local").unwrap();
        assert_eq!(
            crate::cloud_saves::sync::synchronize(
                &store,
                42,
                &[location],
                crate::domain::CloudSyncMode::Normal,
                &cloud
            )
            .unwrap()
            .uploaded,
            1
        );
        assert!(
            store
                .cloud_save_tombstones("fixture-account", 42)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn lost_delete_response_still_leaves_recovery_and_upload_suppression() {
        let (root, store, mut cloud, location) = fixture();
        cloud.lose_reply = true;
        let report = delete_objects(
            &store,
            42,
            std::slice::from_ref(&location),
            &cloud.list().unwrap(),
            &root.path().join("recovery"),
            &cloud,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(report.deleted, 0);
        assert!(report.error.is_some());
        assert!(report.recovery_snapshot.join("manifest.json").is_file());
        assert_eq!(
            crate::cloud_saves::sync::synchronize(
                &store,
                42,
                std::slice::from_ref(&location),
                crate::domain::CloudSyncMode::Normal,
                &cloud
            )
            .unwrap()
            .uploaded,
            0
        );
        assert_eq!(
            crate::cloud_saves::sync::synchronize(
                &store,
                42,
                &[location],
                crate::domain::CloudSyncMode::ForceUpload,
                &cloud
            )
            .unwrap()
            .uploaded,
            1
        );
    }

    #[test]
    fn failed_intent_write_never_deletes_remote_data() {
        let (root, store, cloud, location) = fixture();
        rusqlite::Connection::open(&cloud.database).unwrap().execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON cloud_save_tombstones BEGIN SELECT RAISE(ABORT, 'fixture database failure'); END;").unwrap();
        assert!(
            delete_objects(
                &store,
                42,
                &[location],
                &cloud.list().unwrap(),
                &root.path().join("recovery"),
                &cloud,
                &|| Ok(())
            )
            .is_err()
        );
        assert_eq!(cloud.list().unwrap().len(), 1);
    }

    #[test]
    fn partial_failure_reports_only_confirmed_deletions_and_keeps_all_recovery_files() {
        let (root, store, mut cloud, location) = fixture();
        let second = RemoteObject {
            namespace: "main".into(),
            path: "second.dat".into(),
            size: 6,
            modified_at: 10,
            etag: "second-revision".into(),
        };
        cloud
            .objects
            .lock()
            .unwrap()
            .insert("main/second.dat".into(), (second, b"second".to_vec()));
        cloud.fail_delete = true;
        let mut selected = cloud.list().unwrap();
        selected.sort_by(|a, b| a.path.cmp(&b.path));
        let report = delete_objects(
            &store,
            42,
            &[location],
            &selected,
            &root.path().join("recovery"),
            &cloud,
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(report.deleted, 1);
        assert_eq!(report.requested, 2);
        assert!(report.error.is_some());
        assert_eq!(cloud.list().unwrap()[0].path, "second.dat");
        assert!(
            report
                .recovery_snapshot
                .join("main/profile/save.dat")
                .is_file()
        );
        assert!(report.recovery_snapshot.join("main/second.dat").is_file());
    }

    #[test]
    fn export_rejects_unsafe_paths_collisions_and_symlink_destinations() {
        let (root, _, cloud, _) = fixture();
        for (namespace, path) in [
            ("", "save"),
            ("main", "../save"),
            ("main", "/save"),
            ("main", "a\\b"),
            ("main", "a//b"),
            ("../main", "save"),
        ] {
            assert!(safe_export_path(namespace, path).is_err());
        }
        let mut objects = cloud.list().unwrap();
        let mut collision = objects[0].clone();
        collision.path = collision.path.to_uppercase();
        objects.push(collision);
        assert!(validate_objects(&objects).is_err());
        std::os::unix::fs::symlink(root.path().join("local"), root.path().join("link")).unwrap();
        assert!(
            export_objects(
                42,
                &root.path().join("link/exports"),
                &cloud.list().unwrap(),
                &cloud,
                &|| Ok(())
            )
            .is_err()
        );
        assert!(!root.path().join("local/exports").exists());
    }

    #[test]
    fn parent_replacement_cannot_redirect_export_or_allow_remote_deletion() {
        let (root, store, mut cloud, location) = fixture();
        let destination = root.path().join("recovery");
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        cloud.swap_directory = Some((
            destination.clone(),
            root.path().join("moved"),
            outside.clone(),
        ));
        assert!(
            delete_objects(
                &store,
                42,
                &[location],
                &cloud.list().unwrap(),
                &destination,
                &cloud,
                &|| Ok(())
            )
            .is_err()
        );
        assert_eq!(cloud.list().unwrap().len(), 1);
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path().join("moved")).unwrap().count(), 0);
    }

    #[test]
    fn cancellation_and_local_symlinks_prevent_deletion() {
        let (root, store, cloud, location) = fixture();
        assert!(
            delete_objects(
                &store,
                42,
                std::slice::from_ref(&location),
                &cloud.list().unwrap(),
                &root.path().join("recovery"),
                &cloud,
                &|| bail!("cancelled")
            )
            .is_err()
        );
        fs::remove_file(location.path.join("profile/save.dat")).unwrap();
        std::os::unix::fs::symlink(
            root.path().join("state.sqlite3"),
            location.path.join("profile/save.dat"),
        )
        .unwrap();
        assert!(
            delete_objects(
                &store,
                42,
                &[location],
                &cloud.list().unwrap(),
                &root.path().join("recovery"),
                &cloud,
                &|| Ok(())
            )
            .is_err()
        );
        assert_eq!(cloud.list().unwrap().len(), 1);
    }
}
