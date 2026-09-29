use crate::{
    config::Config,
    state::{DownloadJobUpdate, DownloadState, ManagedFileRecord, StateStore},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedDownloads {
    pub(super) product_id: i64,
    files: Vec<DownloadFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DownloadFile {
    path: PathBuf,
    product_id: i64,
    artifact_id: Option<String>,
    device: u64,
    inode: u64,
    size: u64,
    modified: i64,
    modified_ns: i64,
}

impl ManagedDownloads {
    pub fn count(&self) -> usize {
        self.files.len()
    }
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

#[derive(Debug, Default)]
pub struct CleanupResult {
    pub deleted: usize,
    pub failures: Vec<String>,
}

/// Preview only indexed, recognized downloads; callers must confirm this exact snapshot.
pub fn managed_downloads(product_id: i64) -> Result<ManagedDownloads> {
    inspect(
        &StateStore::open()?,
        &read_config(&Config::path())?,
        product_id,
    )
}

fn read_config(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error.into()),
    }
}

fn inspect(store: &StateStore, config: &Config, product_id: i64) -> Result<ManagedDownloads> {
    let game = store.cached_product_game(product_id)?;
    let mut ids = vec![product_id];
    if let Some(game) = &game {
        ids.extend(
            game.dlcs
                .iter()
                .filter(|dlc| dlc.owned)
                .map(|dlc| dlc.product_id),
        );
    }
    let jobs = store.download_jobs()?;
    let mut files = Vec::new();
    for file in store
        .managed_files()?
        .into_iter()
        .filter(|file| file.present && file.matched && ids.contains(&file.product_id))
    {
        let base = game
            .as_ref()
            .map_or(file.product_slug.as_str(), |game| game.slug.as_str());
        let child = game
            .as_ref()
            .and_then(|game| {
                game.dlcs
                    .iter()
                    .find(|dlc| dlc.product_id == file.product_id)
            })
            .map(|dlc| dlc.slug.as_str());
        let mut relative = PathBuf::from(super::layout::key(base));
        if let Some(child) = child {
            relative.push("dlc");
            relative.push(super::layout::key(child));
        }
        relative.push(file.kind.as_str());
        if let Some(os) = file
            .operating_system
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            relative.push(super::layout::key(os));
        }
        if let Some(language) = file.language.as_deref().filter(|value| !value.is_empty()) {
            relative.push(super::layout::key(language));
        }
        if Path::new(&file.filename).components().count() != 1 {
            continue;
        }
        relative.push(&file.filename);
        let current = config.download_directory.join(&relative) == file.path;
        let recorded = jobs.iter().any(|job| {
            job.product_id == file.product_id
                && job.completed_files.contains(&file.path)
                && file.path.parent() == Some(job.destination.as_path())
                && file.path.ends_with(&relative)
        });
        if !current && !recorded {
            continue;
        }
        let Ok((_, _, handle)) = open_file(&file.path) else {
            continue;
        };
        let metadata = handle.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            continue;
        }
        files.push(DownloadFile {
            path: file.path,
            product_id: file.product_id,
            artifact_id: file.artifact_id,
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: metadata.mtime(),
            modified_ns: metadata.mtime_nsec(),
        });
    }
    Ok(ManagedDownloads { product_id, files })
}

/// Directory descriptors anchor every component; no parent or final symlink is followed.
fn open_file(path: &Path) -> Result<(File, CString, File)> {
    ensure!(path.is_absolute(), "Downloaded file path must be absolute");
    let mut parent = File::open("/")?;
    let components = path
        .components()
        .filter(|part| !matches!(part, Component::RootDir))
        .collect::<Vec<_>>();
    ensure!(
        !components.is_empty()
            && components
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "Unsafe downloaded file path"
    );
    for component in &components[..components.len() - 1] {
        let name = CString::new(component.as_os_str().as_bytes())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    let name = CString::new(components.last().unwrap().as_os_str().as_bytes())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((parent, name, unsafe { File::from_raw_fd(fd) }))
}

pub(super) fn delete(
    store: &StateStore,
    snapshot: ManagedDownloads,
    after_uninstall: bool,
) -> Result<CleanupResult> {
    let _permit = crate::operation_gate::try_acquire()?;
    delete_locked(store, snapshot, after_uninstall)
}

fn delete_locked(
    store: &StateStore,
    snapshot: ManagedDownloads,
    after_uninstall: bool,
) -> Result<CleanupResult> {
    let ids = snapshot
        .files
        .iter()
        .map(|file| file.product_id)
        .collect::<std::collections::HashSet<_>>();
    for id in &ids {
        ensure!(
            !crate::installation::is_game_running(*id),
            "Close this game before deleting its downloaded files"
        );
        if !after_uninstall || *id != snapshot.product_id {
            ensure!(
                !crate::installation::installation_operation_snapshot(*id).is_some_and(
                    |operation| operation.queued
                        || matches!(
                            operation.state,
                            crate::domain::InstallationState::Installing
                                | crate::domain::InstallationState::Uninstalling
                        )
                ),
                "An installation uses these files; finish or cancel it first"
            );
        }
    }
    let jobs = store.download_jobs()?;
    ensure!(
        !jobs.iter().any(|job| ids.contains(&job.product_id)
            && matches!(
                job.state,
                DownloadState::Queued | DownloadState::Downloading
            )),
        "Pause or finish this game's downloads before deleting files"
    );
    let indexed = store.managed_files()?;
    let current = inspect(store, &read_config(&Config::path())?, snapshot.product_id)?;
    // Revocation is checked and completed before any unlink, serialized by the queue manager.
    for job in &jobs {
        if snapshot
            .files
            .iter()
            .any(|file| job.completed_files.contains(&file.path))
        {
            store.clear_download_install_intent_for_job(&job.job_id)?;
        }
    }
    let mut result = CleanupResult::default();
    for file in snapshot.files {
        let removed = (|| -> Result<bool> {
            ensure!(
                indexed.iter().any(|current| same_record(current, &file)),
                "The downloaded file changed in the index; inspect it again"
            );
            let opened = match open_file(&file.path) {
                Ok(value) => Some(value),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                {
                    None
                }
                Err(error) => return Err(error),
            };
            let unlinked = opened.is_some();
            if let Some((parent, name, handle)) = opened {
                ensure!(
                    current
                        .files
                        .iter()
                        .any(|candidate| candidate.path == file.path),
                    "This file is no longer a recognized managed download; inspect it again"
                );
                let metadata = handle.metadata()?;
                ensure!(
                    metadata.is_file()
                        && metadata.nlink() == 1
                        && metadata.dev() == file.device
                        && metadata.ino() == file.inode
                        && metadata.len() == file.size
                        && metadata.mtime() == file.modified
                        && metadata.mtime_nsec() == file.modified_ns,
                    "The downloaded file was replaced or modified; inspect it again"
                );
                ensure!(
                    unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == 0,
                    "Could not remove the downloaded file: {}",
                    std::io::Error::last_os_error()
                );
            }
            store.mark_managed_file_absent(&file.path)?;
            for mut job in store
                .download_jobs()?
                .into_iter()
                .filter(|job| job.completed_files.contains(&file.path))
            {
                job.completed_files.retain(|path| path != &file.path);
                if job.completed_files.is_empty() {
                    store.delete_download_job(&job.job_id)?;
                } else {
                    store.save_download_job(&DownloadJobUpdate {
                        job_id: &job.job_id,
                        product_id: job.product_id,
                        title: &job.title,
                        artifacts: &job.artifacts,
                        destination: &job.destination,
                        state: DownloadState::Paused,
                        bytes_downloaded: job
                            .completed_files
                            .iter()
                            .filter_map(|path| path.metadata().ok())
                            .map(|meta| meta.len())
                            .sum(),
                        total_bytes: job.total_bytes,
                        completed_files: &job.completed_files,
                        error: None,
                    })?;
                    store.set_download_job_status(
                        &job.job_id,
                        Some("Some downloaded files were removed"),
                    )?;
                }
            }
            Ok(unlinked)
        })();
        match removed {
            Ok(unlinked) => result.deleted += usize::from(unlinked),
            Err(error) => result.failures.push(format!(
                "{}: {error}",
                file.path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    Ok(result)
}

fn same_record(current: &ManagedFileRecord, file: &DownloadFile) -> bool {
    current.matched
        && current.path == file.path
        && current.product_id == file.product_id
        && current.artifact_id == file.artifact_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactKind, Game, RemoteArtifact};
    use std::os::unix::fs::symlink;

    #[test]
    fn cleanup_config_inspection_never_writes_preferences() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        assert!(read_config(&path).is_ok());
        assert!(!path.exists());
        let text = "download_directory = '/fixture/downloads'\n# keep my comment\n";
        std::fs::write(&path, text).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            read_config(&path).unwrap().download_directory,
            Path::new("/fixture/downloads")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn cleanup_includes_recorded_owned_dlc_in_an_old_download_root() {
        let root = tempfile::tempdir().unwrap();
        let (store, mut config, _) = fixture(root.path());
        store
            .upsert_normalized_library(&[Game {
                product_id: 7,
                slug: "game".into(),
                dlcs: vec![crate::domain::Dlc {
                    product_id: 8,
                    slug: "child".into(),
                    owned: true,
                    ..Default::default()
                }],
                ..Default::default()
            }])
            .unwrap();
        let directory = root.path().join("old-downloads/game/dlc/child/extra");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("goodies.zip");
        std::fs::write(&path, b"inert goodies").unwrap();
        let mut artifact = store
            .download_job("job")
            .unwrap()
            .unwrap()
            .artifacts
            .remove(0);
        artifact.product_id = 8;
        artifact.kind = ArtifactKind::Extra;
        artifact.operating_system = None;
        artifact.language = None;
        artifact.download_path = "/child/extra".into();
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: "child",
                product_id: 8,
                title: "Child",
                artifacts: std::slice::from_ref(&artifact),
                destination: &directory,
                state: DownloadState::Complete,
                bytes_downloaded: 13,
                total_bytes: Some(13),
                completed_files: std::slice::from_ref(&path),
                error: None,
            })
            .unwrap();
        store
            .record_completed_artifacts("child", "child", &[artifact], std::slice::from_ref(&path))
            .unwrap();
        config.download_directory = root.path().join("new-download-root");
        let snapshot = inspect(&store, &config, 7).unwrap();
        assert_eq!(snapshot.count(), 3);
        assert!(snapshot.files.iter().any(|file| file.path == path));
        let result = delete_locked(&store, snapshot, false).unwrap();
        assert_eq!(result.deleted, 3);
        assert!(result.failures.is_empty());
        assert!(!path.exists());
    }

    fn fixture(root: &Path) -> (StateStore, Config, Vec<PathBuf>) {
        let store = StateStore::open_at(&root.join("state.db")).unwrap();
        let download = root.join("library");
        let directory = download.join("game/installer/linux/en");
        std::fs::create_dir_all(&directory).unwrap();
        let files = vec![directory.join("setup.sh"), directory.join("setup.bin")];
        for path in &files {
            std::fs::write(path, b"inert fixture").unwrap();
        }
        let artifacts = files
            .iter()
            .enumerate()
            .map(|(index, _)| RemoteArtifact {
                product_id: 7,
                kind: ArtifactKind::Installer,
                name: "installer".into(),
                language: Some("en".into()),
                operating_system: Some("linux".into()),
                version: None,
                release_date: None,
                size_label: None,
                size_bytes: Some(13),
                part_number: Some(index as u32 + 1),
                part_count: Some(2),
                download_path: format!("/fixture/{index}"),
                provider_group_id: None,
                provider_file_id: None,
                provider_category: None,
            })
            .collect::<Vec<_>>();
        store
            .upsert_normalized_library(&[Game {
                product_id: 7,
                slug: "game".into(),
                ..Default::default()
            }])
            .unwrap();
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: "job",
                product_id: 7,
                title: "Game",
                artifacts: &artifacts,
                destination: &directory,
                state: DownloadState::Complete,
                bytes_downloaded: 26,
                total_bytes: Some(26),
                completed_files: &files,
                error: None,
            })
            .unwrap();
        store
            .record_completed_artifacts("job", "game", &artifacts, &files)
            .unwrap();
        let config = Config {
            download_directory: download,
            ..Default::default()
        };
        (store, config, files)
    }

    #[test]
    fn exact_cleanup_preserves_payload_preferences_and_retries_partial_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, files) = fixture(root.path());
        for name in [
            "start.sh",
            "save.dat",
            ".ludomere-install.json",
            "notes.txt",
        ] {
            std::fs::write(
                config.download_directory.join("game").join(name),
                b"preserve",
            )
            .unwrap();
        }
        let outside = config.download_directory.join("other/save.dat");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"preserve other").unwrap();
        store.set_favorite(7, true).unwrap();
        store
            .save_download_install_intent(&crate::state::DownloadInstallIntent {
                product_id: 7,
                intent_id: "pending".into(),
                job_ids: vec!["job".into()],
                plan_json: "{}".into(),
                state: "waiting".into(),
                error: None,
            })
            .unwrap();
        let snapshot = inspect(&store, &config, 7).unwrap();
        assert_eq!(snapshot.count(), 2);
        // Merely opening or declining the preview has no effect.
        assert!(files.iter().all(|path| path.is_file()));
        assert_eq!(store.download_install_intents().unwrap().len(), 1);
        std::fs::rename(&files[1], files[1].with_extension("held")).unwrap();
        symlink(&outside, &files[1]).unwrap();
        let result = delete_locked(&store, snapshot.clone(), false).unwrap();
        assert_eq!(result.deleted, 1);
        assert_eq!(result.failures.len(), 1);
        assert!(store.download_install_intents().unwrap().is_empty());
        assert_eq!(
            store.download_job("job").unwrap().unwrap().completed_files,
            vec![files[1].clone()]
        );
        std::fs::remove_file(&files[1]).unwrap();
        std::fs::rename(files[1].with_extension("held"), &files[1]).unwrap();
        let retried = delete_locked(&store, snapshot, false).unwrap();
        assert!(retried.failures.is_empty());
        assert_eq!(retried.deleted, 1);
        assert!(!files[1].exists());
        assert!(store.download_job("job").unwrap().is_none());
        assert!(store.favorites().unwrap().contains(&7));
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve other");
        for name in [
            "start.sh",
            "save.dat",
            ".ludomere-install.json",
            "notes.txt",
        ] {
            assert_eq!(
                std::fs::read(config.download_directory.join("game").join(name)).unwrap(),
                b"preserve"
            );
        }
        assert_eq!(inspect(&store, &config, 7).unwrap().count(), 0);
    }

    #[test]
    fn cleanup_rejects_replacements_parent_links_and_active_jobs_before_unlink() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, files) = fixture(root.path());
        let snapshot = inspect(&store, &config, 7).unwrap();
        let mut job = store.download_job("job").unwrap().unwrap();
        job.state = DownloadState::Queued;
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: &job.job_id,
                product_id: 7,
                title: "Game",
                artifacts: &job.artifacts,
                destination: &job.destination,
                state: job.state,
                bytes_downloaded: 26,
                total_bytes: Some(26),
                completed_files: &files,
                error: None,
            })
            .unwrap();
        assert!(delete_locked(&store, snapshot.clone(), false).is_err());
        assert!(files.iter().all(|path| path.is_file()));
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: &job.job_id,
                product_id: 7,
                title: "Game",
                artifacts: &job.artifacts,
                destination: &job.destination,
                state: DownloadState::Complete,
                bytes_downloaded: 26,
                total_bytes: Some(26),
                completed_files: &files,
                error: None,
            })
            .unwrap();
        std::fs::write(&files[0], b"replacement bytes").unwrap();
        let parent = files[0].parent().unwrap();
        let renamed = parent.with_extension("held");
        std::fs::rename(parent, &renamed).unwrap();
        symlink(&renamed, parent).unwrap();
        assert_eq!(inspect(&store, &config, 7).unwrap().count(), 0);
        assert_eq!(
            delete_locked(&store, snapshot.clone(), false)
                .unwrap()
                .failures
                .len(),
            2
        );
        std::fs::remove_file(parent).unwrap();
        std::fs::rename(&renamed, parent).unwrap();
        let result = delete_locked(&store, snapshot, false).unwrap();
        assert_eq!(result.failures.len(), 1);
        assert_eq!(std::fs::read(&files[0]).unwrap(), b"replacement bytes");
    }

    #[test]
    fn post_uninstall_cleanup_refuses_busy_gate_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, _) = fixture(root.path());
        let snapshot = inspect(&store, &config, 7).unwrap();
        let _active = crate::operation_gate::acquire(|| false).unwrap();
        let started = std::time::Instant::now();
        assert!(delete(&store, snapshot, true).is_err());
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    }
}
