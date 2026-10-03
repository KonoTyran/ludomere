//! Private per-launch output and bounded, sanitized presentation reads.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const TAIL_BYTES: u64 = 256 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct RuntimeLog {
    pub name: String,
    pub path: PathBuf,
    pub modified: i64,
}

pub struct RuntimeLogTail {
    pub text: String,
    pub truncated: bool,
    pub bytes: u64,
}

fn directory(root: &Path, create: bool) -> Result<File> {
    ensure!(root.is_absolute(), "Runtime log folder must be absolute");
    let mut directory = File::open("/")?;
    for part in root.components() {
        let Component::Normal(name) = part else {
            ensure!(part == Component::RootDir, "Invalid runtime log folder");
            continue;
        };
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(name.as_bytes())?;
        if create && unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
        }
        // Each component is opened relative to the previous verified directory.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(
            fd >= 0,
            "Could not open runtime log folder: {}",
            std::io::Error::last_os_error()
        );
        directory = unsafe { File::from_raw_fd(fd) };
    }
    Ok(directory)
}

fn open_file(directory: &File, name: &str, flags: i32) -> Result<File> {
    let name = std::ffi::CString::new(name)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "Runtime log must be a regular file without additional links"
    );
    Ok(file)
}

fn matches_product(product_id: i64, name: &str) -> bool {
    name == format!("{product_id}.log")
        || name
            .strip_prefix(&format!("{product_id}-"))
            .and_then(|name| name.strip_suffix(".log"))
            .is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'-')
            })
}

pub(super) fn create(product_id: i64) -> Result<(PathBuf, File)> {
    create_at(&crate::identity::runtime_logs(), product_id)
}

fn create_at(root: &Path, product_id: i64) -> Result<(PathBuf, File)> {
    ensure!(product_id > 0, "Invalid game identity");
    let directory = directory(root, true)?;
    let name = format!(
        "{product_id}-{}-{}-{}.log",
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .context("Invalid launch time")?,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let file = open_file(
        &directory,
        &name,
        libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT | libc::O_EXCL,
    )?;
    Ok((root.join(name), file))
}

pub fn list_runtime_logs(product_id: i64) -> Result<Vec<RuntimeLog>> {
    list_at(&crate::identity::runtime_logs(), product_id)
}

fn list_at(root: &Path, product_id: i64) -> Result<Vec<RuntimeLog>> {
    ensure!(product_id > 0, "Invalid game identity");
    if !root.try_exists()? {
        return Ok(Vec::new());
    }
    let directory = directory(root, false)?;
    let mut logs = Vec::new();
    for (index, entry) in
        fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?.enumerate()
    {
        ensure!(
            index < 100_000,
            "Runtime log history is too large to list; open the log folder"
        );
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !matches_product(product_id, &name) {
            continue;
        }
        let metadata = match fs::symlink_metadata(
            Path::new(&format!("/proc/self/fd/{}", directory.as_raw_fd())).join(&name),
        ) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.nlink() != 1 {
            continue;
        }
        let file = open_file(&directory, &name, libc::O_RDONLY)
            .context("Could not read a saved runtime log; check its permissions and retry")?;
        logs.push(RuntimeLog {
            modified: file.metadata()?.mtime(),
            path: root.join(&name),
            name,
        });
    }
    logs.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.name.cmp(&a.name))
    });
    logs.truncate(100);
    Ok(logs)
}

pub fn read_runtime_log(product_id: i64, name: &str) -> Result<RuntimeLogTail> {
    read_at(&crate::identity::runtime_logs(), product_id, name)
}

fn read_at(root: &Path, product_id: i64, name: &str) -> Result<RuntimeLogTail> {
    ensure!(
        product_id > 0 && matches_product(product_id, name),
        "Invalid runtime log identity"
    );
    read_tail(
        open_file(&directory(root, false)?, name, libc::O_RDONLY)?,
        TAIL_BYTES,
    )
}

pub(super) fn installation_tail(path: &Path) -> Result<String> {
    let root = path.parent().context("Installation log has no folder")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid installation log name")?;
    Ok(read_tail(
        open_file(&directory(root, false)?, name, libc::O_RDONLY)?,
        8192,
    )?
    .text)
}

fn read_tail(mut file: File, maximum: u64) -> Result<RuntimeLogTail> {
    let bytes = file.metadata()?.len();
    let truncated = bytes > maximum;
    file.seek(SeekFrom::Start(bytes.saturating_sub(maximum)))?;
    let mut buffer = Vec::new();
    file.take(maximum).read_to_end(&mut buffer)?;
    // A partial first line could omit the label identifying a secret value.
    let start = if truncated {
        buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1)
    } else {
        0
    };
    let mut text = sanitize(&String::from_utf8_lossy(&buffer[start..]));
    let text_truncated = text.len() > maximum as usize;
    if text_truncated {
        let mut end = maximum as usize;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Ok(RuntimeLogTail {
        text,
        truncated: truncated || text_truncated,
        bytes,
    })
}

pub(super) fn diagnostic(file: &mut File, text: &str) {
    let _ = writeln!(file, "[Ludomere] {}", sanitize(text));
}

pub(crate) fn sanitize(text: &str) -> String {
    let mut safe = String::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| {
                [
                    "access_token",
                    "refresh_token",
                    "client_secret",
                    "authorization",
                    "password",
                    "cookie",
                    "bearer",
                    "token",
                    "secret",
                ]
                .contains(&word)
            })
        {
            safe.push_str("[credential-bearing line redacted]\n");
            continue;
        }
        for part in line.split_inclusive(char::is_whitespace) {
            if part.to_ascii_lowercase().contains("https://")
                || part.to_ascii_lowercase().contains("http://")
            {
                safe.push_str("[URL redacted]");
                safe.extend(
                    part.chars()
                        .rev()
                        .take_while(|ch| ch.is_whitespace())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev(),
                );
            } else {
                safe.extend(part.chars().filter(|ch| !ch.is_control() || *ch == '\t'));
            }
        }
        safe.push('\n');
    }
    safe
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn launches_are_private_distinct_and_keep_legacy_and_both_output_streams() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("7.log"), "legacy").unwrap();
        let (first, mut output) = create_at(root.path(), 7).unwrap();
        diagnostic(&mut output, "Launch requested: product 7 (native)");
        let status = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "printf 'stdout fixture\\n'; printf 'stderr fixture\\n' >&2",
            ])
            .stdout(output.try_clone().unwrap())
            .stderr(output.try_clone().unwrap())
            .status()
            .unwrap();
        assert!(status.success());
        let (second, _) = create_at(root.path(), 7).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(root.path().join("7.log")).unwrap(),
            "legacy"
        );
        let text = read_at(root.path(), 7, first.file_name().unwrap().to_str().unwrap())
            .unwrap()
            .text;
        assert!(text.contains("stdout fixture") && text.contains("stderr fixture"));
        assert_eq!(list_at(root.path(), 7).unwrap().len(), 3);
    }

    #[test]
    fn tail_bounds_redacts_and_rejects_foreign_paths_links_and_fifos() {
        let root = tempfile::tempdir().unwrap();
        let (path, mut file) = create_at(root.path(), 7).unwrap();
        file.write_all(&vec![b'x'; TAIL_BYTES as usize + 30])
            .unwrap();
        file.write_all(
            b"\naccess_token=secret\nopen (https://host/path?token=secret)\nnormal output\n",
        )
        .unwrap();
        let tail = read_at(root.path(), 7, path.file_name().unwrap().to_str().unwrap()).unwrap();
        assert!(tail.truncated && tail.text.len() < TAIL_BYTES as usize);
        assert!(!tail.text.contains("secret") && tail.text.contains("normal output"));
        assert!(read_at(root.path(), 8, path.file_name().unwrap().to_str().unwrap()).is_err());
        assert!(read_at(root.path(), 7, "../7.log").is_err());
        symlink(&path, root.path().join("7.log")).unwrap();
        assert!(read_at(root.path(), 7, "7.log").is_err());
        fs::remove_file(root.path().join("7.log")).unwrap();
        let fifo = std::ffi::CString::new(root.path().join("7.log").as_os_str().as_encoded_bytes())
            .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(read_at(root.path(), 7, "7.log").is_err());
        let linked = root.path().join("linked");
        symlink(root.path(), &linked).unwrap();
        assert!(create_at(&linked, 7).is_err());
    }

    #[test]
    fn unreadable_regular_history_is_an_error_and_installer_tail_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let (path, mut file) = create_at(root.path(), 7).unwrap();
        for _ in 0..2000 {
            file.write_all(b"token : synthetic-secret\n").unwrap();
        }
        let text = installation_tail(&path).unwrap();
        assert!(text.len() <= 8192 && !text.contains("synthetic-secret"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(list_at(root.path(), 7).is_err());
            assert!(read_at(root.path(), 7, path.file_name().unwrap().to_str().unwrap()).is_err());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
