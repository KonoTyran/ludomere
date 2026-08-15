use crate::domain::RemoteArtifact;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn delete_completed_files(destination: &Path, files: &[PathBuf]) -> Result<()> {
    for file in files {
        if !file.starts_with(destination) {
            bail!("refusing to delete a file outside its managed download directory");
        }
        match fs::remove_file(file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("deleting {}", file.display()));
            }
        }
    }
    let _ = fs::remove_dir(destination);
    Ok(())
}

pub fn prune_empty_directories(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    prune_empty_children(root)
}

fn prune_empty_children(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() && !file_type.is_symlink() {
            let path = entry.path();
            prune_empty_children(&path)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
            }
        }
    }
    Ok(())
}

pub(super) fn fallback_filename(artifact: &RemoteArtifact, index: usize, count: usize) -> String {
    let base = sanitize_filename(&artifact.name).unwrap_or_else(|| "gog-download".into());
    let extension = match artifact.operating_system.as_deref() {
        Some("windows") if index == 0 => Some("exe"),
        Some("linux") if index == 0 => Some("sh"),
        Some("windows" | "linux") if count > 1 => Some("bin"),
        Some("mac") | Some("osx") | Some("macos") => Some("pkg"),
        _ => None,
    };
    extension.map_or(base.clone(), |extension| format!("{base}.{extension}"))
}

pub(super) fn sanitize_filename(value: &str) -> Option<String> {
    let filename = Path::new(value).file_name()?.to_string_lossy();
    let filename = filename.trim();
    (!filename.is_empty() && filename != "." && filename != "..").then(|| filename.to_owned())
}
