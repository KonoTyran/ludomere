use super::api::{RemoteObject, Storage};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

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

pub fn export_cloud_saves(
    game: &crate::domain::InstalledGame,
    destination: &Path,
) -> Result<PathBuf> {
    let cloud = super::authenticated_storage(game)?;
    let objects = cloud.list()?;
    export_objects(game.product_id, destination, &objects, &cloud)
}

pub(super) fn export_objects(
    product_id: i64,
    destination: &Path,
    objects: &[RemoteObject],
    cloud: &dyn Storage,
) -> Result<PathBuf> {
    let stamp = chrono::Utc::now().timestamp_millis();
    let name = format!("ludomere-cloud-export-{product_id}-{stamp}");
    fs::create_dir_all(destination)?;
    let staging = destination.join(format!(".{name}.partial"));
    let completed = destination.join(name);
    if staging.exists() || completed.exists() {
        bail!("cloud-save export destination already exists");
    }
    fs::create_dir(&staging)?;
    let result = write_export(product_id, &staging, objects, cloud);
    if let Err(error) = result {
        fs::remove_dir_all(&staging).ok();
        return Err(error);
    }
    fs::rename(&staging, &completed)?;
    Ok(completed)
}

fn write_export(
    product_id: i64,
    destination: &Path,
    objects: &[RemoteObject],
    cloud: &dyn Storage,
) -> Result<()> {
    let mut paths = HashSet::new();
    let mut entries = Vec::with_capacity(objects.len());
    for object in objects {
        let relative = safe_export_path(&object.namespace, &object.path)?;
        if !paths.insert(relative.to_string_lossy().to_lowercase()) {
            bail!("cloud-save export contains duplicate paths");
        }
        let bytes = cloud.download(&object.namespace, &object.path)?;
        let path = destination.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &bytes)?;
        entries.push(CloudExportEntry {
            namespace: object.namespace.clone(),
            path: object.path.clone(),
            remote_size: object.size,
            exported_size: bytes.len() as u64,
            modified_at: object.modified_at,
            remote_revision: object.etag.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        });
    }
    let manifest = CloudExportManifest {
        format_version: 1,
        product_id,
        exported_at: chrono::Utc::now().timestamp(),
        files: entries,
    };
    fs::write(
        destination.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    verify_export(destination, &manifest)
}

pub(super) fn verify_export(destination: &Path, manifest: &CloudExportManifest) -> Result<()> {
    let saved: CloudExportManifest = serde_json::from_slice(
        &fs::read(destination.join("manifest.json")).context("reading export manifest")?,
    )?;
    if &saved != manifest {
        bail!("cloud-save export manifest verification failed");
    }
    for entry in &manifest.files {
        let path = destination.join(safe_export_path(&entry.namespace, &entry.path)?);
        let bytes = fs::read(path)?;
        if bytes.len() as u64 != entry.exported_size
            || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
        {
            bail!("cloud-save export file verification failed");
        }
    }
    Ok(())
}

fn safe_export_path(namespace: &str, path: &str) -> Result<PathBuf> {
    let mut relative = PathBuf::new();
    for value in [namespace, path] {
        let path = Path::new(value);
        if path.is_absolute() {
            bail!("cloud-save service returned an absolute path");
        }
        for component in path.components() {
            let Component::Normal(component) = component else {
                bail!("cloud-save service returned an unsafe path");
            };
            let component = component
                .to_str()
                .context("cloud-save path is not valid UTF-8")?;
            if invalid_portable_component(component) {
                bail!("cloud-save path is invalid on supported platforms");
            }
            relative.push(component);
        }
    }
    if relative.as_os_str().is_empty() {
        bail!("cloud-save service returned an empty path");
    }
    Ok(relative)
}

fn invalid_portable_component(component: &str) -> bool {
    component.is_empty()
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|character| character.is_control() || r#"<>:"\|?*"#.contains(character))
        || matches!(
            component
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct MemoryCloud(Mutex<HashMap<String, Vec<u8>>>);

    impl Storage for MemoryCloud {
        fn list(&self) -> Result<Vec<RemoteObject>> {
            Ok(Vec::new())
        }

        fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .get(&format!("{namespace}/{path}"))
                .cloned()
                .context("missing test object")
        }

        fn upload(
            &self,
            _namespace: &str,
            _path: &str,
            _data: &[u8],
            _modified_at: i64,
        ) -> Result<RemoteObject> {
            unreachable!()
        }

        fn delete(&self, _namespace: &str, _path: &str, _etag: Option<&str>) -> Result<()> {
            unreachable!()
        }
    }

    #[test]
    fn export_preserves_paths_and_writes_verified_manifest() {
        let root =
            std::env::temp_dir().join(format!("ludomere-cloud-export-test-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let cloud = MemoryCloud::default();
        cloud
            .0
            .lock()
            .unwrap()
            .insert("main/profile/save.dat".into(), b"save".to_vec());
        let object = RemoteObject {
            namespace: "main".into(),
            path: "profile/save.dat".into(),
            size: 3,
            modified_at: 10,
            etag: "revision".into(),
        };
        let export = export_objects(42, &root, &[object], &cloud).unwrap();
        assert_eq!(
            fs::read(export.join("main/profile/save.dat")).unwrap(),
            b"save"
        );
        let manifest: CloudExportManifest =
            serde_json::from_slice(&fs::read(export.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.product_id, 42);
        assert_eq!(manifest.files[0].remote_size, 3);
        assert_eq!(manifest.files[0].exported_size, 4);
        assert_eq!(manifest.files[0].remote_revision, "revision");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_rejects_traversal_and_case_collisions() {
        assert!(safe_export_path("main", "../save.dat").is_err());
        assert!(safe_export_path("main", "CON").is_err());
        let cloud = MemoryCloud::default();
        cloud
            .0
            .lock()
            .unwrap()
            .insert("main/Save.dat".into(), Vec::new());
        let objects = ["Save.dat", "save.dat"].map(|path| RemoteObject {
            namespace: "main".into(),
            path: path.into(),
            size: 0,
            modified_at: 0,
            etag: String::new(),
        });
        let root = std::env::temp_dir().join(format!(
            "ludomere-cloud-export-collision-{}",
            std::process::id()
        ));
        fs::remove_dir_all(&root).ok();
        assert!(export_objects(42, &root, &objects, &cloud).is_err());
        fs::remove_dir_all(root).ok();
    }
}
