//! A receipt records already-published files until their SQLite registration commits.
use crate::domain::RemoteArtifact;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const NAME: &std::ffi::CStr = c"completion.json";
const MAX_BYTES: u64 = 1024 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    format: u8,
    job: String,
    artifacts_sha256: String,
    destination: PathBuf,
    files: Vec<PublishedFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedFile {
    name: String,
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

pub(super) struct Completion {
    directory: File,
    receipt: Receipt,
    identity: (u64, u64),
}

impl Completion {
    pub fn load(
        staging: &Path,
        artifacts: &[RemoteArtifact],
        destination: &Path,
    ) -> Result<Option<Self>> {
        let (_, _, directory) = super::cleanup::open_file(staging)?;
        ensure!(
            directory.metadata()?.is_dir(),
            "Invalid completion receipt directory"
        );
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                NAME.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error.into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.len() <= MAX_BYTES,
            "Unsafe completion receipt"
        );
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Completion receipt exceeds the size limit"
        );
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        ensure!(
            receipt.format == 1
                && receipt.job == super::job_id(&artifacts.iter().collect::<Vec<_>>())
                && receipt.artifacts_sha256
                    == format!("{:x}", Sha256::digest(serde_json::to_vec(artifacts)?))
                && receipt.destination == destination
                && receipt.files.len() == artifacts.len(),
            "Completion receipt does not match this download"
        );
        let result = Self {
            directory,
            receipt,
            identity: (metadata.dev(), metadata.ino()),
        };
        result.validate()?;
        Ok(Some(result))
    }

    pub fn record(
        staging: &Path,
        artifacts: &[RemoteArtifact],
        destination: &Path,
        files: &[PathBuf],
    ) -> Result<Self> {
        ensure!(
            files.len() == artifacts.len() && !files.is_empty(),
            "Incomplete downloaded file list"
        );
        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            ensure!(
                path.parent() == Some(destination),
                "Downloaded file is outside its destination"
            );
            let (_, _, file) = super::cleanup::open_file(path)?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && metadata.nlink() == 1,
                "Downloaded payload is not a regular private file"
            );
            entries.push(PublishedFile {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("Invalid downloaded filename")?
                    .to_owned(),
                device: metadata.dev(),
                inode: metadata.ino(),
                size: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            });
        }
        let receipt = Receipt {
            format: 1,
            job: super::job_id(&artifacts.iter().collect::<Vec<_>>()),
            artifacts_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(artifacts)?)),
            destination: destination.to_owned(),
            files: entries,
        };
        let bytes = serde_json::to_vec(&receipt)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Completion receipt exceeds the size limit"
        );
        let (_, _, directory) = super::cleanup::open_file(staging)?;
        ensure!(
            directory.metadata()?.is_dir(),
            "Invalid completion receipt directory"
        );
        let temporary = CString::new(format!(
            ".completion-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ))?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            if unsafe {
                libc::renameat2(
                    directory.as_raw_fd(),
                    temporary.as_ptr(),
                    directory.as_raw_fd(),
                    NAME.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            directory.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            unsafe {
                libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0);
            }
        }
        result?;
        Self::load(staging, artifacts, destination)?.context("Completion receipt disappeared")
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.receipt
            .files
            .iter()
            .map(|file| self.receipt.destination.join(&file.name))
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        let mut names = HashSet::new();
        for file in &self.receipt.files {
            let name = Path::new(&file.name);
            ensure!(
                name.components().count() == 1
                    && matches!(name.components().next(), Some(Component::Normal(_)))
                    && names.insert(&file.name),
                "Invalid or duplicate completion filename"
            );
            let (_, _, payload) = super::cleanup::open_file(&self.receipt.destination.join(name))?;
            let metadata = payload.metadata()?;
            ensure!(
                metadata.is_file()
                    && metadata.nlink() == 1
                    && metadata.dev() == file.device
                    && metadata.ino() == file.inode
                    && metadata.len() == file.size
                    && (metadata.mtime(), metadata.mtime_nsec()) == file.modified
                    && (metadata.ctime(), metadata.ctime_nsec()) == file.changed,
                "Downloaded file changed before completion could be registered"
            );
            // Even numeric GOG catalog sizes can be rounded. The receipt binds
            // the exact file published after the HTTP response completed.
        }
        Ok(())
    }

    pub fn remove(self) -> Result<()> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                NAME.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let stat = unsafe { stat.assume_init() };
        ensure!(
            (stat.st_dev, stat.st_ino) == self.identity,
            "Completion receipt changed before cleanup"
        );
        if unsafe { libc::unlinkat(self.directory.as_raw_fd(), NAME.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        self.directory.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    fn fixture() -> (
        tempfile::TempDir,
        Vec<RemoteArtifact>,
        PathBuf,
        PathBuf,
        Vec<PathBuf>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let artifacts = vec![serde_json::from_value(serde_json::json!({"product_id":7,"kind":"installer","name":"Fixture","size_bytes":4,"download_path":"/not-requested"})).unwrap()];
        let destination = root.path().join("game/installer");
        let staging = root.path().join(".ludomere-staging/job");
        fs::create_dir_all(&destination).unwrap();
        fs::create_dir_all(&staging).unwrap();
        let files = vec![destination.join("server-name.bin")];
        fs::write(&files[0], b"data").unwrap();
        (root, artifacts, destination, staging, files)
    }

    #[test]
    fn legacy_display_size_receipt_recovers_without_changing_published_files() {
        let (_root, mut artifacts, destination, staging, files) = fixture();
        artifacts[0].size_label = Some("0.1 kB".into());
        artifacts[0].size_bytes = Some(100);
        fs::write(&files[0], [b'x'; 104]).unwrap();
        let result = Completion::record(&staging, &artifacts, &destination, &files);
        let receipt_bytes = fs::read(staging.join("completion.json")).unwrap();
        // Older versions left this same receipt after comparing a rounded catalog
        // display size against the exact published length.
        assert_eq!(
            serde_json::from_slice::<Receipt>(&receipt_bytes)
                .unwrap()
                .files[0]
                .size,
            104
        );
        result.unwrap();
        let loaded = Completion::load(&staging, &artifacts, &destination)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.files(), files);
        assert_eq!(
            fs::read(staging.join("completion.json")).unwrap(),
            receipt_bytes
        );
        assert_eq!(fs::read(&files[0]).unwrap(), [b'x'; 104]);
        fs::write(&files[0], [b'y'; 104]).unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
        fs::remove_file(&files[0]).unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
    }

    #[test]
    fn numeric_catalog_sizes_do_not_override_observed_receipt_identity() {
        for identity in 0..4 {
            let (_root, mut artifacts, destination, staging, files) = fixture();
            artifacts[0].size_bytes = Some(381_681_664);
            File::options()
                .write(true)
                .open(&files[0])
                .unwrap()
                .set_len(382_662_456)
                .unwrap();
            if identity > 0 {
                artifacts[0].size_label = Some("0.1 kB".into());
            }
            match identity {
                1 => artifacts[0].provider_group_id = Some("official-group".into()),
                2 => artifacts[0].provider_file_id = Some("official-file".into()),
                3 => {
                    artifacts[0].provider_category =
                        Some(crate::domain::DownloadCategory::Installer)
                }
                _ => {}
            }
            Completion::record(&staging, &artifacts, &destination, &files).unwrap();
            assert_eq!(
                Completion::load(&staging, &artifacts, &destination)
                    .unwrap()
                    .unwrap()
                    .files(),
                files
            );
            assert_eq!(fs::metadata(&files[0]).unwrap().len(), 382_662_456);
        }
    }

    #[test]
    fn receipt_roundtrip_binds_artifacts_destination_and_changed_payload() {
        let (_root, artifacts, destination, staging, files) = fixture();
        let receipt = Completion::record(&staging, &artifacts, &destination, &files).unwrap();
        assert_eq!(receipt.files(), files);
        assert_eq!(
            fs::metadata(staging.join("completion.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut changed = artifacts.clone();
        changed[0].version = Some("different".into());
        assert!(Completion::load(&staging, &changed, &destination).is_err());
        assert!(Completion::load(&staging, &artifacts, &destination.join("other")).is_err());
        let modified = fs::metadata(&files[0]).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        fs::write(&files[0], b"edit").unwrap();
        File::options()
            .write(true)
            .open(&files[0])
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
        assert!(staging.join("completion.json").is_file());
        assert_eq!(fs::read(&files[0]).unwrap(), b"edit");
    }

    #[test]
    fn malformed_oversized_symlink_and_traversal_receipts_fail_closed() {
        let (root, artifacts, destination, staging, files) = fixture();
        Completion::record(&staging, &artifacts, &destination, &files).unwrap();
        let path = staging.join("completion.json");
        let bytes = fs::read(&path).unwrap();
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["files"][0]["name"] = "../outside".into();
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
        fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
        fs::remove_file(&path).unwrap();
        let external = root.path().join("outside-receipt");
        fs::write(&external, &bytes).unwrap();
        symlink(&external, &path).unwrap();
        assert!(Completion::load(&staging, &artifacts, &destination).is_err());
        assert_eq!(fs::read(&external).unwrap(), bytes);
        assert_eq!(fs::read(&files[0]).unwrap(), b"data");
    }

    #[test]
    fn existing_receipt_is_never_replaced_and_payload_links_are_rejected() {
        let (root, artifacts, destination, staging, files) = fixture();
        let receipt = Completion::record(&staging, &artifacts, &destination, &files).unwrap();
        let before = fs::read(staging.join("completion.json")).unwrap();
        assert!(Completion::record(&staging, &artifacts, &destination, &files).is_err());
        assert_eq!(fs::read(staging.join("completion.json")).unwrap(), before);
        fs::hard_link(&files[0], root.path().join("linked")).unwrap();
        assert!(receipt.validate().is_err());
        assert_eq!(fs::read(&files[0]).unwrap(), b"data");
    }
}
