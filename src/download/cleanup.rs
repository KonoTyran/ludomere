use crate::{
    config::{Config, GameLibrary, LibraryKind},
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
    #[serde(default)]
    blocked_libraries: Vec<String>,
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
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }
    pub fn blocked_libraries(&self) -> &[String] {
        &self.blocked_libraries
    }
}

#[derive(Debug, Default)]
pub struct CleanupResult {
    pub deleted: usize,
    pub failures: Vec<String>,
}

pub(super) struct Retention {
    snapshot: ManagedDownloads,
    replacement: ManagedDownloads,
    revision: i64,
    library: GameLibrary,
    trash: Vec<super::trash::PreparedTrash>,
}

pub(super) fn prepare_retention(
    product_id: i64,
    job_id: &str,
    artifacts: &[crate::domain::RemoteArtifact],
    paths: &[PathBuf],
    token: &str,
    session: u64,
) -> Result<Option<Retention>> {
    static VERIFYING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _verification = VERIFYING.try_lock().map_err(|_| {
        anyhow::anyhow!(
            "Another installer cleanup is being prepared; retry at the next update check"
        )
    })?;
    let _activity = crate::profile_reset::begin_activity("installer retention verification")?;
    let store = StateStore::open()?;
    crate::online::with_account_session(session, || Ok(()))?;
    if !crate::updates::UpdatePolicy::resolve(
        &read_config(&Config::path())?,
        store.game_preferences(product_id)?.as_ref(),
    )
    .prune_superseded_installers
    {
        return Ok(None);
    }
    ensure!(
        !artifacts.is_empty()
            && artifacts.len() == paths.len()
            && artifacts
                .iter()
                .all(|artifact| artifact.kind == crate::domain::ArtifactKind::Installer),
        "Only complete primary installers qualify for retention"
    );
    ensure_no_install_intent(&store, product_id)?;
    // Historical completed jobs are not replacement candidates; do not let their obsolete
    // checksum endpoints prevent a later, currently offered job from being considered.
    if !store.offered_installer_job(job_id)? {
        return Ok(None);
    }
    if let Some(revision) = store.verified_installer_revision_for_job(job_id)?
        && store
            .superseded_installer_files(product_id, revision)?
            .is_empty()
    {
        return Ok(None);
    }
    let config = read_config(&Config::path())?;
    let library =
        crate::storage::validate_path(&config, LibraryKind::OfflineInstallers, &paths[0])?;
    ensure!(
        paths.iter().all(|path| path.starts_with(&library.path)),
        "The replacement must be complete in one Offline Installers library"
    );
    let mut replacement = inspect(&store, &config, product_id)?;
    replacement.files.retain(|file| paths.contains(&file.path));
    ensure!(
        replacement.files.len() == paths.len(),
        "The replacement is not fully indexed as managed downloads"
    );
    let ordered_paths = artifact_files(artifacts, paths, &store.managed_files()?)?;
    for (artifact, path) in artifacts.iter().zip(&ordered_paths) {
        crate::online::with_account_session(session, || Ok(()))?;
        let checksum = super::gog_checksum(artifact, token)?;
        let (_, _, mut handle) = open_file(path)?;
        let before = handle.metadata()?;
        let expected = replacement
            .files
            .iter()
            .find(|file| file.path == *path)
            .unwrap();
        ensure!(
            matches_identity(&before, expected) && before.len() == checksum.size,
            "The replacement changed before verification"
        );
        let mut md5 = md5::Context::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            crate::online::with_account_session(session, || Ok(()))?;
            let count = std::io::Read::read(&mut handle, &mut buffer)?;
            if count == 0 {
                break;
            }
            md5.consume(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", md5.compute()).eq_ignore_ascii_case(&checksum.md5)
                && matches_identity(&handle.metadata()?, expected),
            "Replacement checksum verification failed; older installers were retained"
        );
        crate::online::with_account_session(session, || {
            store.mark_managed_file_verified(path, artifact, &checksum.md5)
        })?;
    }
    let revision = store
        .verified_installer_revision_for_job(job_id)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "The complete currently offered replacement revision could not be verified"
            )
        })?;
    let older = store.superseded_installer_files(product_id, revision)?;
    let mut snapshot = inspect(&store, &read_config(&Config::path())?, product_id)?;
    snapshot
        .files
        .retain(|file| older.contains(&file.path) && file.path.starts_with(&library.path));
    if snapshot.files.is_empty() {
        return Ok(None);
    }
    let mut trash = Vec::new();
    for file in &snapshot.files {
        let (_, _, mut handle) = open_file(&file.path)?;
        ensure!(
            matches_identity(&handle.metadata()?, file),
            "An old installer changed before preparing Trash"
        );
        trash.push(super::trash::prepare(&mut handle, &file.path, || {
            crate::online::account_session() != session
        })?);
    }
    Ok(Some(Retention {
        snapshot,
        replacement,
        revision,
        library,
        trash,
    }))
}

fn artifact_files(
    artifacts: &[crate::domain::RemoteArtifact],
    paths: &[PathBuf],
    managed: &[ManagedFileRecord],
) -> Result<Vec<PathBuf>> {
    let mut ordered = Vec::new();
    for artifact in artifacts {
        let matches = managed
            .iter()
            .filter(|file| {
                file.present
                    && file.matched
                    && paths.contains(&file.path)
                    && file.product_id == artifact.product_id
                    && file.kind == artifact.kind
                    && file.artifact_path.as_deref() == Some(artifact.download_path.as_str())
                    && file.version == artifact.version
                    && (artifact.provider_file_id.is_none()
                        || file.provider_file_id == artifact.provider_file_id)
            })
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1 && !ordered.contains(&matches[0].path),
            "The replacement file identity is ambiguous; older installers were retained"
        );
        ordered.push(matches[0].path.clone());
    }
    ensure!(
        ordered.len() == paths.len(),
        "The replacement file set is incomplete"
    );
    Ok(ordered)
}

pub(super) fn commit_retention(
    store: &StateStore,
    retention: Retention,
    job_id: &str,
) -> Result<CleanupResult> {
    let _permit = crate::operation_gate::try_acquire()?;
    commit_retention_with_config(store, retention, job_id, &read_config(&Config::path())?)
}

fn commit_retention_with_config(
    store: &StateStore,
    retention: Retention,
    job_id: &str,
    config: &Config,
) -> Result<CleanupResult> {
    let id = retention.snapshot.product_id;
    ensure!(
        crate::updates::UpdatePolicy::resolve(config, store.game_preferences(id)?.as_ref())
            .prune_superseded_installers,
        "Installer cleanup was disabled; the original files were retained"
    );
    ensure_no_install_intent(store, id)?;
    let statuses = crate::storage::inspect_libraries_with_store(config, store)?;
    ensure!(
        statuses
            .iter()
            .any(|status| status.kind == LibraryKind::OfflineInstallers
                && status.library_id == retention.library.id
                && status.path == retention.library.path
                && status.compatibility == crate::storage::LibraryCompatibility::Compatible),
        "The replacement library changed or is incompatible; older installers were retained"
    );
    ensure!(
        retention
            .snapshot
            .files
            .iter()
            .chain(&retention.replacement.files)
            .all(|file| file.path.starts_with(&retention.library.path)),
        "Installer retention requires a verified replacement in the same library"
    );
    ensure!(
        store.verified_installer_revision_for_job(job_id)? == Some(retention.revision),
        "The offered replacement changed; the original files were retained"
    );
    for file in &retention.replacement.files {
        let (_, _, handle) = open_file(&file.path)?;
        ensure!(
            matches_identity(&handle.metadata()?, file),
            "The verified replacement changed; the original files were retained"
        );
    }
    delete_locked_with_trash(
        store,
        retention.snapshot,
        false,
        Some(retention.trash),
        config,
    )
}

fn ensure_no_install_intent(store: &StateStore, product_id: i64) -> Result<()> {
    let jobs = store
        .download_jobs()?
        .into_iter()
        .filter(|job| job.product_id == product_id)
        .map(|job| job.job_id)
        .collect::<std::collections::HashSet<_>>();
    ensure!(
        !store
            .download_install_intents()?
            .iter()
            .any(|intent| (intent.product_id == product_id
                || intent.job_ids.iter().any(|id| jobs.contains(id)))
                && intent.state != "complete"),
        "A pending installation uses these files; installer cleanup is deferred"
    );
    Ok(())
}

fn matches_identity(metadata: &std::fs::Metadata, file: &DownloadFile) -> bool {
    metadata.is_file()
        && metadata.nlink() == 1
        && metadata.dev() == file.device
        && metadata.ino() == file.inode
        && metadata.len() == file.size
        && metadata.mtime() == file.modified
        && metadata.mtime_nsec() == file.modified_ns
}

/// Preview only indexed, recognized downloads; callers must confirm this exact snapshot.
pub fn managed_downloads(product_id: i64) -> Result<ManagedDownloads> {
    inspect(
        &StateStore::open()?,
        &read_config(&Config::path())?,
        product_id,
    )
}

pub fn managed_downloads_for_kind(product_id: i64, kind: LibraryKind) -> Result<ManagedDownloads> {
    inspect_kind(
        &StateStore::open()?,
        &read_config(&Config::path())?,
        product_id,
        kind,
    )
}

fn inspect_kind(
    store: &StateStore,
    config: &Config,
    product_id: i64,
    kind: LibraryKind,
) -> Result<ManagedDownloads> {
    ensure!(
        kind != LibraryKind::GameFiles,
        "Installed payloads are not downloaded archives"
    );
    let mut snapshot = inspect(store, config, product_id)?;
    snapshot.files.retain(|file| {
        config
            .libraries(kind)
            .iter()
            .any(|library| file.path.starts_with(&library.path))
    });
    snapshot.blocked_libraries.retain(|message| {
        config
            .libraries(kind)
            .iter()
            .any(|library| message.starts_with(&format!("{}:", library.path.display())))
    });
    Ok(snapshot)
}

fn read_config(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error.into()),
    }
}

fn inspect(store: &StateStore, config: &Config, product_id: i64) -> Result<ManagedDownloads> {
    let statuses = crate::storage::inspect_libraries_with_store(config, store)?;
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
    let mut blocked_libraries = std::collections::BTreeSet::new();
    for file in store
        .managed_files_for_products(&ids)?
        .into_iter()
        .filter(|file| file.present && file.matched && ids.contains(&file.product_id))
    {
        for status in statuses.iter().filter(|status| {
            status.kind != LibraryKind::GameFiles && file.path.starts_with(&status.path)
        }) {
            if let crate::storage::LibraryCompatibility::Incompatible(reason)
            | crate::storage::LibraryCompatibility::Unavailable(reason) = &status.compatibility
            {
                blocked_libraries.insert(format!("{}: {reason}", status.path.display()));
            }
        }
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
        let Some(library) = statuses.iter().find(|status| {
            status.kind != LibraryKind::GameFiles
                && status.compatibility == crate::storage::LibraryCompatibility::Compatible
                && file.path.starts_with(&status.path)
        }) else {
            continue;
        };
        let current = library.path.join(&relative) == file.path;
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
    Ok(ManagedDownloads {
        product_id,
        files,
        blocked_libraries: blocked_libraries.into_iter().collect(),
    })
}

/// Directory descriptors anchor every component; no parent or final symlink is followed.
pub(super) fn open_file(path: &Path) -> Result<(File, CString, File)> {
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
    delete_locked(
        store,
        snapshot,
        after_uninstall,
        &read_config(&Config::path())?,
    )
}

fn delete_locked(
    store: &StateStore,
    snapshot: ManagedDownloads,
    after_uninstall: bool,
    config: &Config,
) -> Result<CleanupResult> {
    delete_locked_with_trash(store, snapshot, after_uninstall, None, config)
}

fn delete_locked_with_trash(
    store: &StateStore,
    snapshot: ManagedDownloads,
    after_uninstall: bool,
    mut trash: Option<Vec<super::trash::PreparedTrash>>,
    config: &Config,
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
    let current = inspect(store, config, snapshot.product_id)?;
    // Revocation is checked and completed before any unlink, serialized by the queue manager.
    for job in &jobs {
        if trash.is_some() {
            continue;
        }
        if snapshot
            .files
            .iter()
            .any(|file| job.completed_files.contains(&file.path))
        {
            store.clear_download_install_intent_for_job(&job.job_id)?;
        }
    }
    let mut result = CleanupResult::default();
    if let Some(trash) = &mut trash {
        trash.reverse();
    }
    for file in snapshot.files {
        let mut prepared_trash = trash.as_mut().and_then(Vec::pop);
        let mut removed_payload = false;
        let removed = (|| -> Result<()> {
            ensure!(
                current
                    .files
                    .iter()
                    .any(|candidate| candidate.path == file.path),
                "This file is no longer in a compatible managed library; inspect it again"
            );
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
            if let Some((parent, name, handle)) = opened {
                let metadata = handle.metadata()?;
                ensure!(
                    matches_identity(&metadata, &file),
                    "The downloaded file was replaced or modified; inspect it again"
                );
                if let Some(trash) = &prepared_trash {
                    trash.verify_present()?;
                }
                ensure!(
                    unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == 0,
                    "Could not remove the downloaded file: {}",
                    std::io::Error::last_os_error()
                );
                removed_payload = true;
                if let Some(trash) = &mut prepared_trash {
                    trash.committed = true;
                }
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
            Ok(())
        })();
        result.deleted += usize::from(removed_payload);
        match removed {
            Ok(_) => {}
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
    fn verified_retention_preserves_pending_installs_and_replaced_files() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, older) = fixture(root.path());
        let mut old_artifacts = store.download_job("job").unwrap().unwrap().artifacts;
        for (index, artifact) in old_artifacts.iter_mut().enumerate() {
            artifact.version = Some("1".into());
            artifact.provider_group_id = Some("linux-en".into());
            artifact.provider_file_id = Some(format!("old-{index}"));
            artifact.provider_category = Some(crate::domain::DownloadCategory::Installer);
        }
        store.observe_download_manifest(7, &old_artifacts).unwrap();
        store
            .record_completed_artifacts("job", "game", &old_artifacts, &older)
            .unwrap();
        let mut new_artifacts = old_artifacts.clone();
        let replacement = older
            .iter()
            .enumerate()
            .map(|(index, path)| path.with_file_name(format!("new-{index}.bin")))
            .collect::<Vec<_>>();
        for (index, (artifact, path)) in new_artifacts.iter_mut().zip(&replacement).enumerate() {
            artifact.version = Some("2".into());
            artifact.provider_file_id = Some(format!("new-{index}"));
            artifact.download_path = format!("/new/{index}");
            std::fs::write(path, b"inert fixture").unwrap();
        }
        store.observe_download_manifest(7, &new_artifacts).unwrap();
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: "replacement",
                product_id: 7,
                title: "Game",
                artifacts: &new_artifacts,
                destination: replacement[0].parent().unwrap(),
                state: DownloadState::Complete,
                bytes_downloaded: 26,
                total_bytes: Some(26),
                completed_files: &replacement,
                error: None,
            })
            .unwrap();
        store
            .record_completed_artifacts("replacement", "game", &new_artifacts, &replacement)
            .unwrap();
        assert!(
            store
                .verified_installer_revision_for_job("replacement")
                .unwrap()
                .is_none()
        );
        for (artifact, path) in new_artifacts.iter().zip(&replacement) {
            store
                .mark_managed_file_verified(path, artifact, "fixture-verified")
                .unwrap();
        }
        let revision = store
            .verified_installer_revision_for_job("replacement")
            .unwrap()
            .unwrap();
        assert_eq!(
            store.superseded_installer_files(7, revision).unwrap().len(),
            2
        );
        let prepare = || {
            let mut snapshot = inspect(&store, &config, 7).unwrap();
            let mut replacement_snapshot = snapshot.clone();
            snapshot.files.retain(|file| older.contains(&file.path));
            replacement_snapshot
                .files
                .retain(|file| replacement.contains(&file.path));
            let trash = snapshot
                .files
                .iter()
                .map(|file| {
                    super::super::trash::prepare(
                        &mut File::open(&file.path).unwrap(),
                        &file.path,
                        || false,
                    )
                    .unwrap()
                })
                .collect();
            Retention {
                snapshot,
                replacement: replacement_snapshot,
                revision,
                library: config.offline_libraries[0].clone(),
                trash,
            }
        };
        store
            .set_game_update_preferences(7, None, None, Some(true), None)
            .unwrap();
        store
            .save_download_install_intent(&crate::state::DownloadInstallIntent {
                product_id: 7,
                intent_id: "consent".into(),
                job_ids: vec!["job".into()],
                plan_json: "{}".into(),
                state: "waiting".into(),
                error: None,
            })
            .unwrap();
        assert!(commit_retention_with_config(&store, prepare(), "replacement", &config).is_err());
        assert!(older.iter().all(|path| path.exists()));
        assert_eq!(
            store.download_install_intents().unwrap()[0].intent_id,
            "consent"
        );
        store.clear_download_install_intent(7).unwrap();
        let mut changed = config.clone();
        let other = GameLibrary {
            id: "another-root".into(),
            name: "Another root".into(),
            path: root.path().join("another-root"),
            default: false,
        };
        std::fs::create_dir(&other.path).unwrap();
        changed.offline_libraries.push(other.clone());
        let mut cross_root = prepare();
        cross_root.library = other;
        assert!(commit_retention_with_config(&store, cross_root, "replacement", &changed).is_err());
        assert!(older.iter().all(|path| path.is_file()));
        let pending = prepare();
        // A changed verified replacement blocks retention before any older file is removed.
        std::fs::rename(&replacement[1], replacement[1].with_extension("held")).unwrap();
        symlink(&older[1], &replacement[1]).unwrap();
        assert!(commit_retention_with_config(&store, pending, "replacement", &config).is_err());
        assert!(
            older
                .iter()
                .all(|path| std::fs::read(path).unwrap() == b"inert fixture")
        );
        assert!(
            std::fs::symlink_metadata(&replacement[1])
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_file(&replacement[1]).unwrap();
        std::fs::rename(replacement[1].with_extension("held"), &replacement[1]).unwrap();
        assert!(
            replacement
                .iter()
                .all(|path| std::fs::read(path).unwrap() == b"inert fixture")
        );
        assert_eq!(
            store.download_job("job").unwrap().unwrap().completed_files,
            older
        );
        assert_eq!(
            store
                .download_job("replacement")
                .unwrap()
                .unwrap()
                .completed_files,
            replacement
        );
        let pending = prepare();
        std::fs::rename(&older[1], older[1].with_extension("held")).unwrap();
        symlink(&replacement[1], &older[1]).unwrap();
        let result = commit_retention_with_config(&store, pending, "replacement", &config).unwrap();
        assert_eq!(result.deleted, 1);
        assert_eq!(result.failures.len(), 1);
        assert!(!older[0].exists());
        assert!(
            std::fs::symlink_metadata(&older[1])
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read(older[1].with_extension("held")).unwrap(),
            b"inert fixture"
        );
        assert!(
            replacement
                .iter()
                .all(|path| std::fs::read(path).unwrap() == b"inert fixture")
        );
        assert_eq!(
            store.download_job("job").unwrap().unwrap().completed_files,
            older[1..]
        );
        assert_eq!(
            store
                .download_job("replacement")
                .unwrap()
                .unwrap()
                .completed_files,
            replacement
        );
    }

    #[test]
    fn multipart_retention_matches_index_identity_even_when_completed_paths_are_reordered() {
        let root = tempfile::tempdir().unwrap();
        let (store, _, files) = fixture(root.path());
        let artifacts = store.download_job("job").unwrap().unwrap().artifacts;
        let mut reordered = files.clone();
        reordered.reverse();
        assert_eq!(
            artifact_files(&artifacts, &reordered, &store.managed_files().unwrap()).unwrap(),
            files
        );
        reordered[1] = reordered[0].clone();
        assert!(artifact_files(&artifacts, &reordered, &store.managed_files().unwrap()).is_err());
    }

    #[test]
    fn trash_copies_survive_index_failure_after_actual_unlink_and_count_is_truthful() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, files) = fixture(root.path());
        let snapshot = inspect(&store, &config, 7).unwrap();
        let trash = snapshot
            .files
            .iter()
            .map(|file| {
                super::super::trash::prepare(
                    &mut File::open(&file.path).unwrap(),
                    &file.path,
                    || false,
                )
                .unwrap()
            })
            .collect();
        let trash_path = dirs::data_dir().unwrap().join("Trash/files");
        let copies = std::fs::read_dir(&trash_path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
        database.execute_batch("CREATE TRIGGER fail_mark BEFORE UPDATE OF present ON managed_files WHEN NEW.present=0 BEGIN SELECT RAISE(ABORT,'fixture index failure'); END;").unwrap();
        let result =
            delete_locked_with_trash(&store, snapshot, false, Some(trash), &config).unwrap();
        assert_eq!(result.deleted, 2);
        assert_eq!(result.failures.len(), 2);
        assert!(files.iter().all(|file| !file.exists()));
        assert!(copies.iter().all(|copy| copy.exists()));
        assert!(
            store
                .managed_files()
                .unwrap()
                .iter()
                .all(|file| file.present)
        );
    }

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
    fn cleanup_includes_recorded_owned_dlc_in_a_configured_extras_root() {
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
        config.extras_libraries.push(GameLibrary {
            id: "extras".into(),
            name: "Extras".into(),
            path: root.path().join("old-downloads"),
            default: true,
        });
        let snapshot = inspect(&store, &config, 7).unwrap();
        assert_eq!(snapshot.count(), 3);
        assert!(snapshot.files.iter().any(|file| file.path == path));
        let result = delete_locked(&store, snapshot, false, &config).unwrap();
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
            download_directory: download.clone(),
            game_libraries: Vec::new(),
            offline_libraries: vec![GameLibrary {
                id: "offline".into(),
                name: "Offline".into(),
                path: download,
                default: true,
            }],
            ..Default::default()
        };
        (store, config, files)
    }

    #[test]
    fn typed_deletion_preserves_other_category_and_payload_and_explains_blocked_copies() {
        let root = tempfile::tempdir().unwrap();
        let (store, mut config, installers) = fixture(root.path());
        let extras_root = root.path().join("extras");
        let destination = extras_root.join("game/extra");
        std::fs::create_dir_all(&destination).unwrap();
        let extra = destination.join("bonus.zip");
        std::fs::write(&extra, b"inert fixture").unwrap();
        let mut artifact = store
            .download_job("job")
            .unwrap()
            .unwrap()
            .artifacts
            .remove(0);
        artifact.kind = ArtifactKind::Extra;
        artifact.operating_system = None;
        artifact.language = None;
        artifact.part_count = Some(1);
        artifact.part_number = Some(1);
        artifact.download_path = "/bonus".into();
        store
            .record_completed_artifacts("bonus", "game", &[artifact], std::slice::from_ref(&extra))
            .unwrap();
        config.extras_libraries.push(GameLibrary {
            id: "extras".into(),
            name: "Extras".into(),
            path: extras_root.clone(),
            default: true,
        });
        let payload = root.path().join("payload/save.dat");
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::write(&payload, b"preserve").unwrap();
        let offline = inspect_kind(&store, &config, 7, LibraryKind::OfflineInstallers).unwrap();
        assert_eq!(offline.count(), 2);
        assert_eq!(
            inspect_kind(&store, &config, 7, LibraryKind::Extras)
                .unwrap()
                .count(),
            1
        );
        std::fs::write(extras_root.join("unrecognized.txt"), b"preserve").unwrap();
        let available = inspect_kind(&store, &config, 7, LibraryKind::Extras).unwrap();
        assert_eq!(available.count(), 1);
        assert!(available.blocked_libraries().is_empty());
        std::os::unix::fs::symlink(root.path(), extras_root.join(".ludomere-staging")).unwrap();
        let blocked = inspect_kind(&store, &config, 7, LibraryKind::Extras).unwrap();
        assert_eq!(blocked.count(), 0);
        assert_eq!(blocked.blocked_libraries().len(), 1);
        std::fs::remove_file(extras_root.join(".ludomere-staging")).unwrap();
        assert_eq!(
            delete_locked(&store, offline, false, &config)
                .unwrap()
                .deleted,
            2
        );
        assert!(installers.iter().all(|path| !path.exists()));
        assert_eq!(std::fs::read(extra).unwrap(), b"inert fixture");
        assert_eq!(std::fs::read(payload).unwrap(), b"preserve");
        assert_eq!(
            std::fs::read(extras_root.join("unrecognized.txt")).unwrap(),
            b"preserve"
        );
    }

    #[test]
    fn exact_cleanup_preserves_payload_preferences_and_retries_partial_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let (store, config, files) = fixture(root.path());
        let payload = root.path().join("installed-game");
        std::fs::create_dir_all(&payload).unwrap();
        for name in [
            "start.sh",
            "save.dat",
            ".ludomere-install.json",
            "notes.txt",
        ] {
            std::fs::write(payload.join(name), b"preserve").unwrap();
        }
        let outside = root.path().join("other/save.dat");
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
        let staging = config.offline_libraries[0].path.join(".ludomere-staging");
        symlink(root.path(), &staging).unwrap();
        let result = delete_locked(&store, snapshot.clone(), false, &config).unwrap();
        assert_eq!(result.deleted, 0);
        assert_eq!(result.failures.len(), 2);
        assert!(store.download_install_intents().unwrap().is_empty());
        assert_eq!(
            store.download_job("job").unwrap().unwrap().completed_files,
            files
        );
        std::fs::remove_file(staging).unwrap();
        // Once the root is safe, unchanged owned files can be removed independently.
        let result = delete_locked(&store, snapshot.clone(), false, &config).unwrap();
        assert_eq!(result.deleted, 1);
        assert_eq!(result.failures.len(), 1);
        assert!(!files[0].exists());
        assert!(
            std::fs::symlink_metadata(&files[1])
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserve other");
        assert_eq!(
            store.download_job("job").unwrap().unwrap().completed_files,
            files[1..]
        );
        std::fs::remove_file(&files[1]).unwrap();
        std::fs::rename(files[1].with_extension("held"), &files[1]).unwrap();
        let retried =
            delete_locked(&store, inspect(&store, &config, 7).unwrap(), false, &config).unwrap();
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
            assert_eq!(std::fs::read(payload.join(name)).unwrap(), b"preserve");
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
        assert!(delete_locked(&store, snapshot.clone(), false, &config).is_err());
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
            delete_locked(&store, snapshot.clone(), false, &config)
                .unwrap()
                .failures
                .len(),
            2
        );
        std::fs::remove_file(parent).unwrap();
        std::fs::rename(&renamed, parent).unwrap();
        let result = delete_locked(&store, snapshot, false, &config).unwrap();
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
