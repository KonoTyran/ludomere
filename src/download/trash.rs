//! Private home-Trash copies keep cross-filesystem sources intact until the queue commits.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) struct PreparedTrash {
    files: File,
    info: File,
    name: CString,
    metadata_name: CString,
    pub(super) committed: bool,
    published: bool,
    info_created: bool,
    verified: Option<File>,
    modified: (i64, i64),
    changed: (i64, i64, u64),
    info_contents: String,
}

impl PreparedTrash {
    pub(super) fn verify_present(&self) -> Result<()> {
        let file = self
            .verified
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("The Trash copy is incomplete"))?;
        let metadata = file.metadata()?;
        let mut entry = std::mem::MaybeUninit::<libc::stat>::uninit();
        ensure!(
            unsafe {
                libc::fstatat(
                    self.files.as_raw_fd(),
                    self.name.as_ptr(),
                    entry.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } == 0,
            "The Trash copy was removed; the original was retained"
        );
        let entry = unsafe { entry.assume_init() };
        ensure!(
            metadata.nlink() == 1
                && entry.st_ino == metadata.ino()
                && entry.st_dev == metadata.dev()
                && (metadata.mtime(), metadata.mtime_nsec()) == self.modified
                && (metadata.ctime(), metadata.ctime_nsec(), metadata.len()) == self.changed,
            "The Trash copy changed; the original was retained"
        );
        let fd = unsafe {
            libc::openat(
                self.info.as_raw_fd(),
                self.metadata_name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        ensure!(
            fd >= 0,
            "The Trash recovery information was removed; the original was retained"
        );
        let info = unsafe { File::from_raw_fd(fd) };
        ensure!(
            info.metadata()?.is_file(),
            "The Trash recovery information changed; the original was retained"
        );
        let mut contents = String::new();
        info.take(self.info_contents.len() as u64 + 1)
            .read_to_string(&mut contents)?;
        ensure!(
            contents == self.info_contents,
            "The Trash recovery information changed; the original was retained"
        );
        Ok(())
    }
}

impl Drop for PreparedTrash {
    fn drop(&mut self) {
        if !self.committed {
            // Only our O_EXCL names in our pinned private directories are ever cleaned up.
            unsafe {
                if self.published {
                    libc::unlinkat(self.files.as_raw_fd(), self.name.as_ptr(), 0);
                }
                if self.info_created {
                    libc::unlinkat(self.info.as_raw_fd(), self.metadata_name.as_ptr(), 0);
                }
            }
        }
    }
}

pub(super) fn prepare(
    source: &mut File,
    original: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<PreparedTrash> {
    let root = private_directory(
        &dirs::data_dir()
            .ok_or_else(|| anyhow::anyhow!("The desktop data directory is unavailable"))?
            .join("Trash"),
    )?;
    let files = private_child(&root, "files")?;
    let info = private_child(&root, "info")?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = CString::new(format!(
        "ludomere-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))?;
    let metadata_name = CString::new(format!("{}.trashinfo", name.to_str()?))?;
    let temporary_name = CString::new(format!("{}.part", name.to_str()?))?;
    let mut prepared = PreparedTrash {
        files,
        info,
        name,
        metadata_name,
        committed: false,
        published: false,
        info_created: false,
        verified: None,
        modified: (0, 0),
        changed: (0, 0, 0),
        info_contents: String::new(),
    };
    let metadata = source.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "Only regular managed files can be moved to Trash"
    );
    let mut info_file = create_file(&prepared.info, &prepared.metadata_name)?;
    prepared.info_created = true;
    let escaped = original
        .as_os_str()
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    prepared.info_contents = format!(
        "[Trash Info]\nPath={escaped}\nDeletionDate={}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
    );
    info_file.write_all(prepared.info_contents.as_bytes())?;
    info_file.sync_all()?;
    let mut temporary_created = false;
    let result = (|| -> Result<()> {
        let mut output = create_file(&root, &temporary_name)?;
        temporary_created = true;
        source.rewind()?;
        let mut input_hash = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            ensure!(!cancelled(), "Installer cleanup was cancelled");
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(count as u64)
                .ok_or_else(|| anyhow::anyhow!("Installer size overflow"))?;
            ensure!(
                copied <= metadata.len(),
                "The installer changed while preparing Trash"
            );
            output.write_all(&buffer[..count])?;
            input_hash.update(&buffer[..count]);
        }
        ensure!(
            copied == metadata.len(),
            "The installer changed while preparing Trash"
        );
        output.sync_all()?;
        output.rewind()?;
        let mut output_hash = Sha256::new();
        loop {
            ensure!(!cancelled(), "Installer cleanup was cancelled");
            let count = output.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output_hash.update(&buffer[..count]);
        }
        ensure!(
            input_hash.finalize() == output_hash.finalize(),
            "Trash copy verification failed; the original was retained"
        );
        output.set_permissions(std::fs::Permissions::from_mode(metadata.mode() & 0o777))?;
        output.set_times(std::fs::FileTimes::new().set_modified(metadata.modified()?))?;
        output.sync_all()?;
        let now = source.metadata()?;
        ensure!(
            now.len() == metadata.len()
                && now.mtime() == metadata.mtime()
                && now.mtime_nsec() == metadata.mtime_nsec(),
            "The installer changed while preparing Trash"
        );
        ensure!(
            unsafe {
                libc::renameat2(
                    root.as_raw_fd(),
                    temporary_name.as_ptr(),
                    prepared.files.as_raw_fd(),
                    prepared.name.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } == 0,
            "Could not publish the verified Trash copy: {}",
            std::io::Error::last_os_error()
        );
        prepared.published = true;
        temporary_created = false;
        let metadata = output.metadata()?;
        prepared.modified = (metadata.mtime(), metadata.mtime_nsec());
        prepared.changed = (metadata.ctime(), metadata.ctime_nsec(), metadata.len());
        prepared.verified = Some(output);
        prepared.files.sync_all()?;
        prepared.info.sync_all()?;
        Ok(())
    })();
    if result.is_err() && temporary_created {
        unsafe {
            libc::unlinkat(root.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
    }
    result?;
    Ok(prepared)
}

fn create_file(parent: &File, name: &CString) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    ensure!(
        fd >= 0,
        "Could not create private Trash data: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn private_directory(path: &Path) -> Result<File> {
    ensure!(
        path.is_absolute(),
        "The desktop Trash path must be absolute"
    );
    let mut directory = File::open("/")?;
    for part in path
        .components()
        .filter(|part| !matches!(part, Component::RootDir))
    {
        ensure!(
            matches!(part, Component::Normal(_)),
            "Unsafe desktop Trash path"
        );
        let name = CString::new(part.as_os_str().as_bytes())?;
        unsafe {
            libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700);
        }
        directory = open_directory(&directory, &name)?;
    }
    validate_private(&directory)?;
    Ok(directory)
}

fn private_child(parent: &File, name: &str) -> Result<File> {
    let name = CString::new(name)?;
    unsafe {
        libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700);
    }
    let directory = open_directory(parent, &name)?;
    validate_private(&directory)?;
    Ok(directory)
}

fn open_directory(parent: &File, name: &CString) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(
        fd >= 0,
        "Could not open a safe Trash directory: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn validate_private(directory: &File) -> Result<()> {
    let metadata = directory.metadata()?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "Trash directories must be private and owned by this user"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn private_trash_copy_is_verified_recoverable_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("inert # installer.bin");
        std::fs::write(&original, b"inert old installer").unwrap();
        let mut source = File::open(&original).unwrap();
        let prepared = prepare(&mut source, &original, || false).unwrap();
        let name = prepared.name.to_str().unwrap().to_owned();
        let home = dirs::data_dir().unwrap().join("Trash");
        assert_eq!(
            std::fs::read(home.join("files").join(&name)).unwrap(),
            b"inert old installer"
        );
        let info =
            std::fs::read_to_string(home.join("info").join(format!("{name}.trashinfo"))).unwrap();
        assert!(info.contains("%20%23%20"));
        assert!(original.exists());
        let second = prepare(&mut source, &original, || false).unwrap();
        assert_ne!(prepared.name, second.name);
        drop(prepared);
        assert!(!home.join("files").join(&name).exists());
        assert!(!home.join("info").join(format!("{name}.trashinfo")).exists());
        assert_eq!(std::fs::read(&original).unwrap(), b"inert old installer");
    }

    #[test]
    fn cancelled_copy_and_unsafe_directory_preserve_the_original() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("installer");
        std::fs::write(&original, vec![b'x'; 128 * 1024]).unwrap();
        let trash = dirs::data_dir().unwrap().join("Trash");
        let mut source = File::open(&original).unwrap();
        let before = trash
            .join("files")
            .read_dir()
            .map(|files| files.count())
            .unwrap_or_default();
        let reads = std::cell::Cell::new(0);
        assert!(
            prepare(&mut source, &original, || {
                reads.set(reads.get() + 1);
                reads.get() > 1
            })
            .is_err()
        );
        assert_eq!(trash.join("files").read_dir().unwrap().count(), before);
        assert_eq!(std::fs::read(&original).unwrap().len(), 128 * 1024);
        let unsafe_root = root.path().join("unsafe");
        symlink(root.path(), &unsafe_root).unwrap();
        assert!(private_directory(&unsafe_root).is_err());
    }

    #[test]
    fn copy_to_trash_supports_a_distinct_source_filesystem() {
        // /dev/shm supplies an inert local cross-device fixture, never a user payload.
        let source_root = tempfile::Builder::new()
            .prefix("ludomere-trash-fixture-")
            .tempdir_in("/dev/shm")
            .unwrap();
        let original = source_root.path().join("installer");
        std::fs::write(&original, b"cross-volume fixture").unwrap();
        let mut file = File::open(&original).unwrap();
        let prepared = prepare(&mut file, &original, || false).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 20);
        assert!(original.exists());
        let data = dirs::data_dir()
            .unwrap()
            .join("Trash/files")
            .join(prepared.name.to_str().unwrap());
        assert_eq!(std::fs::read(data).unwrap(), b"cross-volume fixture");
    }
}
