use crate::{
    config::{Config, GameLibrary, LibraryKind},
    domain::{ArtifactKind, DownloadCategory, RemoteArtifact},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryCompatibility {
    Compatible,
    Incompatible(String),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStatus {
    pub kind: LibraryKind,
    pub library_id: String,
    pub path: PathBuf,
    pub compatibility: LibraryCompatibility,
}

pub fn read_config() -> Result<Config> {
    toml::from_str(&fs::read_to_string(Config::path())?).context("Reading storage configuration")
}

pub fn artifact_library_kind(artifact: &RemoteArtifact) -> LibraryKind {
    match artifact.provider_category {
        Some(DownloadCategory::Bonus) => LibraryKind::Extras,
        Some(_) => LibraryKind::OfflineInstallers,
        None => match artifact.kind {
            ArtifactKind::Installer | ArtifactKind::Patch => LibraryKind::OfflineInstallers,
            ArtifactKind::Extra => LibraryKind::Extras,
        },
    }
}

/// Pure presentation lookup; callers must freshly validate before accessing library content.
pub fn path_status<'a>(
    statuses: &'a [LibraryStatus],
    path: &Path,
) -> Option<&'a LibraryCompatibility> {
    statuses
        .iter()
        .filter(|status| path.starts_with(&status.path))
        .max_by_key(|status| status.path.components().count())
        .map(|status| &status.compatibility)
}

/// Worker-only inspection. No content is moved, created, deleted or executed.
pub fn inspect_libraries(config: &Config) -> Result<Vec<LibraryStatus>> {
    inspect_libraries_with_store(config, &crate::state::StateStore::open()?)
}

pub(crate) fn inspect_libraries_with_store(
    config: &Config,
    store: &crate::state::StateStore,
) -> Result<Vec<LibraryStatus>> {
    let files = store.managed_files()?;
    let jobs = store.download_jobs()?;
    let mut parts = HashMap::new();
    for product in files
        .iter()
        .map(|file| file.product_id)
        .collect::<std::collections::HashSet<_>>()
    {
        for revision in store.load_all_download_revisions(product)? {
            let kind = if revision.provider_category == DownloadCategory::Bonus {
                LibraryKind::Extras
            } else {
                LibraryKind::OfflineInstallers
            };
            for part in revision.parts {
                parts.insert(part.part_id, kind);
            }
        }
    }
    let mut evidence = files
        .iter()
        .filter(|file| file.present)
        .map(|file| {
            (
                file.path.clone(),
                file.part_id
                    .and_then(|id| parts.get(&id))
                    .copied()
                    .unwrap_or(match file.kind {
                        ArtifactKind::Extra => LibraryKind::Extras,
                        _ => LibraryKind::OfflineInstallers,
                    }),
            )
        })
        .collect::<Vec<_>>();
    evidence.extend(jobs.iter().filter_map(|job| {
        job.artifacts
            .first()
            .map(|artifact| (job.destination.clone(), artifact_library_kind(artifact)))
    }));
    Ok(inspect_with_evidence(config, &evidence))
}

fn inspect_with_evidence(
    config: &Config,
    evidence: &[(PathBuf, LibraryKind)],
) -> Vec<LibraryStatus> {
    LibraryKind::ALL
        .into_iter()
        .flat_map(|kind| {
            config
                .libraries(kind)
                .iter()
                .map(move |library| (kind, library))
        })
        .map(|(kind, library)| LibraryStatus {
            kind,
            library_id: library.id.clone(),
            path: library.path.clone(),
            compatibility: match inspect_library(config, kind, library, evidence) {
                Ok(None) => LibraryCompatibility::Compatible,
                Ok(Some(reason)) => LibraryCompatibility::Incompatible(reason),
                Err(error) if error.chain().any(|cause| cause.is::<std::io::Error>()) => {
                    LibraryCompatibility::Unavailable(format!(
                        "Could not inspect this library: {error}. Recheck after restoring access."
                    ))
                }
                Err(error) => LibraryCompatibility::Incompatible(error.to_string()),
            },
        })
        .collect()
}

pub fn validate_library(config: &Config, kind: LibraryKind, id: &str) -> Result<GameLibrary> {
    let library = config
        .libraries(kind)
        .iter()
        .find(|library| library.id == id)
        .context("The selected library is no longer configured for this type")?;
    let status = inspect_libraries(config)?
        .into_iter()
        .find(|status| status.kind == kind && status.library_id == id)
        .context("Library inspection did not finish")?;
    match status.compatibility {
        LibraryCompatibility::Compatible => Ok(library.clone()),
        LibraryCompatibility::Incompatible(reason) => bail!(
            "{} library is incompatible: {reason}. Correct its contents or choose another directory in Storage.",
            kind.label()
        ),
        LibraryCompatibility::Unavailable(reason) => {
            bail!("{} library is unavailable: {reason}", kind.label())
        }
    }
}

pub fn validate_path(config: &Config, kind: LibraryKind, path: &Path) -> Result<GameLibrary> {
    ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "Invalid library content path"
    );
    let library = config
        .libraries(kind)
        .iter()
        .find(|library| path.starts_with(&library.path))
        .context("This path is not in a configured library of the required type")?;
    let library = validate_library(config, kind, &library.id)?;
    contained_path(&library.path, path)?;
    Ok(library)
}

fn contained_path(root: &Path, path: &Path) -> Result<()> {
    ensure!(
        path.starts_with(root),
        "Path is outside the selected library"
    );
    let root = root.canonicalize()?;
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => {
                ensure!(
                    existing.canonicalize()?.starts_with(&root),
                    "Library content resolves outside its selected directory"
                );
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .context("Library path has no existing ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn metadata_json<T: serde::de::DeserializeOwned>(root: &Path, path: &Path) -> Result<Option<T>> {
    let mut current = root.to_path_buf();
    for component in path.strip_prefix(root)?.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "Library metadata cannot use symbolic links"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    let limit = if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".operation.json"))
    {
        64 * 1024 * 1024
    } else {
        4 * 1024 * 1024
    };
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Library metadata must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Library metadata exceeds its size limit"
    );
    Ok(Some(
        serde_json::from_slice(&bytes).context("Library metadata is malformed")?,
    ))
}

fn inspect_library(
    config: &Config,
    kind: LibraryKind,
    library: &GameLibrary,
    evidence: &[(PathBuf, LibraryKind)],
) -> Result<Option<String>> {
    if library.id.trim().is_empty()
        || !library.path.is_absolute()
        || !library
            .path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Ok(Some(
            "A library requires an identity and an absolute normalized path".into(),
        ));
    }
    for other_kind in LibraryKind::ALL {
        for other in config.libraries(other_kind) {
            if std::ptr::eq(other, library) {
                continue;
            }
            if other.id == library.id
                || other.path.starts_with(&library.path)
                || library.path.starts_with(&other.path)
            {
                return Ok(Some(
                    "Configured libraries have duplicate identities or overlapping paths".into(),
                ));
            }
        }
    }
    let data = crate::identity::data_root();
    let cache = crate::identity::cache_root();
    let mut protected = vec![crate::identity::config_root(), cache.clone()];
    protected.extend(
        [
            "account",
            "installation-logs",
            "runtime-logs",
            "install-targets",
            "library.sqlite3",
            "proton",
            "umu",
            "comet",
            "cloud-save-backups",
            "cloud-save-deletion-recovery",
        ]
        .map(|name| data.join(name)),
    );
    for protected in protected {
        if protected.starts_with(&library.path) || library.path.starts_with(&protected) {
            return Ok(Some(
                "The library overlaps application profile storage".into(),
            ));
        }
    }
    if data.starts_with(&library.path)
        || library.path.components().any(|part| {
            part.as_os_str() == crate::identity::MARKER_DIRECTORY
                || part.as_os_str() == crate::identity::STAGING_DIRECTORY
        })
    {
        return Ok(Some(
            "The library overlaps application or library infrastructure".into(),
        ));
    }
    let mut path = PathBuf::new();
    for component in library.path.components() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(Some(
                "Library ancestors must be real directories, not links".into(),
            ));
        }
    }
    for (path, actual) in evidence {
        if path.starts_with(&library.path) && *actual != kind && fs::symlink_metadata(path).is_ok()
        {
            return Ok(Some(format!(
                "Contains recorded {} content",
                actual.label()
            )));
        }
    }
    let mut budget = 100_000;
    for entry in fs::read_dir(&library.path)? {
        let entry = entry?;
        ensure!(
            budget > 0,
            "Library inspection exceeds its bounded entry limit"
        );
        budget -= 1;
        let name = entry.file_name();
        if !entry.file_type()?.is_dir() {
            return Ok(Some(
                "Contains files or links outside a recognized product directory".into(),
            ));
        }
        if name == crate::identity::STAGING_DIRECTORY {
            continue;
        }
        if name == crate::identity::MARKER_DIRECTORY {
            if kind == LibraryKind::GameFiles {
                continue;
            }
            return Ok(Some(
                "Contains Game Files infrastructure in an archive library".into(),
            ));
        }
        if kind == LibraryKind::GameFiles {
            for category in ["installer", "patch", "extra"] {
                let category = entry.path().join(category);
                let metadata = match fs::symlink_metadata(&category) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                if !metadata.is_dir() {
                    continue;
                }
                for platform in fs::read_dir(&category)? {
                    let platform = platform?;
                    ensure!(
                        budget > 0,
                        "Library inspection exceeds its bounded entry limit"
                    );
                    budget -= 1;
                    if platform.file_type()?.is_dir()
                        && matches!(
                            platform.file_name().to_str(),
                            Some("windows" | "linux" | "mac" | "macos" | "any")
                        )
                    {
                        return Ok(Some("Contains an archive in the managed platform layout; choose separate typed libraries".into()));
                    }
                }
            }
            let marker = metadata_json::<crate::installation::InstallationMarker>(
                &library.path,
                &entry.path().join(".ludomere/installation.json"),
            )?;
            if let Some(marker) = &marker {
                marker.validate()?;
                ensure!(
                    marker.slug == name.to_string_lossy(),
                    "Installation marker does not match its product directory"
                );
            }
            let journal = crate::installation::operation_journal::path(
                &library.path,
                &name.to_string_lossy(),
            )?;
            let journal = metadata_json::<crate::installation::operation_journal::OperationJournal>(
                &library.path,
                &journal,
            )?;
            if let Some(journal) = &journal {
                use crate::installation::operation_journal::OperationJournal;
                match journal {
                    OperationJournal::Depot { version, record } => ensure!(
                        *version == 1 && record.destination == entry.path(),
                        "Operation journal does not match its product directory"
                    ),
                    OperationJournal::Offline { version, record } => {
                        ensure!(*version == 1, "Unsupported operation journal version");
                        let plan: serde_json::Value = serde_json::from_str(&record.plan_json)
                            .context("Operation plan is malformed")?;
                        let game = plan.get("game").unwrap_or(&plan);
                        ensure!(
                            game.get("installation_directory")
                                .and_then(|value| value.as_str())
                                .is_some_and(|path| Path::new(path) == entry.path())
                                && game.get("product_id").and_then(|value| value.as_i64())
                                    == Some(record.product_id),
                            "Operation journal does not match its product directory"
                        );
                    }
                }
            }
            let recovery = metadata_json::<serde_json::Value>(
                &library.path,
                &library
                    .path
                    .join(".ludomere/staging")
                    .join(format!("{}.recovery.json", name.to_string_lossy())),
            )?;
            if let Some(recovery) = &recovery {
                use std::os::unix::fs::MetadataExt;
                let metadata = fs::symlink_metadata(entry.path())?;
                let identity: Option<(u64, u64)> =
                    serde_json::from_value(recovery.get("identity").cloned().unwrap_or_default())
                        .context("Recovery identity is malformed")?;
                ensure!(
                    recovery.get("version").and_then(|value| value.as_u64()) == Some(1)
                        && recovery
                            .get("product_id")
                            .and_then(|value| value.as_i64())
                            .is_some_and(|id| id > 0)
                        && recovery
                            .get("directory")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| Path::new(path) == entry.path())
                        && identity == Some((metadata.dev(), metadata.ino())),
                    "Recovery receipt does not match its product directory"
                );
            }
            let mut gog_info = false;
            if marker.is_none() && journal.is_none() && recovery.is_none() {
                for info in fs::read_dir(entry.path())? {
                    let info = info?;
                    ensure!(
                        budget > 0,
                        "Library inspection exceeds its bounded entry limit"
                    );
                    budget -= 1;
                    let name = info.file_name();
                    let Some(id) = name
                        .to_str()
                        .and_then(|name| name.strip_prefix("goggame-"))
                        .and_then(|name| name.strip_suffix(".info"))
                        .and_then(|id| id.parse::<i64>().ok())
                        .filter(|id| *id > 0)
                    else {
                        continue;
                    };
                    let info = metadata_json::<serde_json::Value>(&library.path, &info.path())?
                        .context("Game metadata disappeared during inspection")?;
                    ensure!(
                        info.get("gameId")
                            .is_some_and(|value| value.as_i64() == Some(id)
                                || value
                                    .as_str()
                                    .is_some_and(|value| value.parse::<i64>().ok() == Some(id))),
                        "Game metadata does not match its product identity"
                    );
                    gog_info = true;
                }
            }
            if marker.is_none()
                && journal.is_none()
                && recovery.is_none()
                && !gog_info
                && !plausible_payload(&entry.path(), 0, &mut budget)?
            {
                return Ok(Some(format!(
                    "Product directory {} has no recognized installation or operation",
                    name.to_string_lossy()
                )));
            }
        } else if let Some(reason) =
            inspect_product(&entry.path(), kind, evidence, &mut budget, true)?
        {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

fn plausible_payload(path: &Path, depth: usize, budget: &mut usize) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        ensure!(
            *budget > 0,
            "Library inspection exceeds its bounded entry limit"
        );
        *budget -= 1;
        if matches!(
            entry.file_name().to_str(),
            Some("installer" | "patch" | "extra" | "dlc" | ".ludomere" | ".ludomere-staging")
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_file()
            && (metadata.permissions().mode() & 0o111 != 0
                || entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["exe", "com", "bat"]
                            .iter()
                            .any(|known| extension.eq_ignore_ascii_case(known))
                    }))
        {
            return Ok(true);
        }
        if metadata.is_dir() && depth < 4 && plausible_payload(&entry.path(), depth + 1, budget)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn inspect_product(
    path: &Path,
    kind: LibraryKind,
    evidence: &[(PathBuf, LibraryKind)],
    budget: &mut usize,
    allow_dlc: bool,
) -> Result<Option<String>> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        ensure!(
            *budget > 0,
            "Library inspection exceeds its bounded entry limit"
        );
        *budget -= 1;
        let name = entry.file_name();
        if name == crate::identity::MARKER_DIRECTORY {
            return Ok(Some(
                "Contains installed-game metadata in an archive library".into(),
            ));
        }
        if !entry.file_type()?.is_dir() {
            return Ok(Some(
                "Archive content is outside the recognized category layout".into(),
            ));
        }
        if name == "dlc" && allow_dlc {
            for child in fs::read_dir(entry.path())? {
                let child = child?;
                ensure!(
                    *budget > 0,
                    "Library inspection exceeds its bounded entry limit"
                );
                *budget -= 1;
                if !child.file_type()?.is_dir() {
                    return Ok(Some(
                        "DLC archive directory contains an unexpected file or link".into(),
                    ));
                }
                if let Some(reason) = inspect_product(&child.path(), kind, evidence, budget, false)?
                {
                    return Ok(Some(reason));
                }
            }
            continue;
        }
        let expected = match name.to_str() {
            Some("installer" | "patch") => LibraryKind::OfflineInstallers,
            Some("extra") => {
                if evidence.iter().any(|(path, recorded)| {
                    *recorded == LibraryKind::OfflineInstallers && path.starts_with(entry.path())
                }) {
                    LibraryKind::OfflineInstallers
                } else {
                    LibraryKind::Extras
                }
            }
            _ => return Ok(Some("Contains an unknown archive category".into())),
        };
        if expected != kind {
            return Ok(Some(format!("Contains {} content", expected.label())));
        }
        inspect_archive_files(&entry.path(), 0, budget)?;
    }
    Ok(None)
}

fn inspect_archive_files(path: &Path, depth: usize, budget: &mut usize) -> Result<()> {
    ensure!(
        depth <= 3,
        "Archive directory is deeper than the managed layout"
    );
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        ensure!(
            *budget > 0,
            "Library inspection exceeds its bounded entry limit"
        );
        *budget -= 1;
        let metadata = entry.file_type()?;
        ensure!(
            metadata.is_file() || metadata.is_dir(),
            "Archive contains an unsafe link or special file"
        );
        if metadata.is_dir() {
            inspect_archive_files(&entry.path(), depth + 1, budget)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(root: &Path) -> Config {
        let mut config = Config::default();
        for (kind, name) in [
            (LibraryKind::GameFiles, "games"),
            (LibraryKind::OfflineInstallers, "installers"),
            (LibraryKind::Extras, "extras"),
        ] {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            *config.libraries_mut(kind) = vec![GameLibrary {
                id: name.into(),
                name: name.into(),
                path,
                default: true,
            }];
        }
        config
    }

    fn marker(path: &Path) {
        fs::create_dir_all(path.join(".ludomere")).unwrap();
        fs::write(path.join(".ludomere/installation.json"), serde_json::to_vec(&serde_json::json!({
            "schema_version":1, "product_id":7,"slug":path.file_name().unwrap().to_str().unwrap(),
            "base":{"installed_at":1}
        })).unwrap()).unwrap();
    }

    #[test]
    fn typed_libraries_classify_layout_without_confusing_game_category_names() {
        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let game = config.game_libraries[0].path.join("extra");
        marker(&game);
        fs::create_dir_all(game.join("installer")).unwrap();
        fs::write(game.join("extra.txt"), b"payload").unwrap();
        let installer = config.offline_libraries[0]
            .path
            .join("game/installer/windows/en");
        fs::create_dir_all(&installer).unwrap();
        fs::write(installer.join("setup.exe"), b"inert").unwrap();
        assert!(
            inspect_with_evidence(&config, &[])
                .iter()
                .all(|status| status.compatibility == LibraryCompatibility::Compatible)
        );
        fs::create_dir_all(game.join("installer/windows/en")).unwrap();
        fs::write(game.join("installer/windows/en/setup.exe"), b"inert").unwrap();
        assert!(matches!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        fs::create_dir_all(
            config.extras_libraries[0]
                .path
                .join(".ludomere/compatibility"),
        )
        .unwrap();
        assert!(matches!(
            inspect_with_evidence(&config, &[])[2].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        fs::remove_dir_all(&game).unwrap();
        let nested = config.game_libraries[0]
            .path
            .join("downloads/game/installer/windows/en");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("setup.exe"), b"inert").unwrap();
        assert!(matches!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
    }

    #[test]
    fn library_paths_reject_escapes_and_distinguish_malformed_from_unavailable() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let mut config = configured(root.path());
        let game = config.game_libraries[0].path.join("game");
        marker(&game);
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, game.join("escape")).unwrap();
        assert!(
            contained_path(
                &config.game_libraries[0].path,
                &game.join("escape/new/file")
            )
            .is_err()
        );
        assert!(contained_path(&config.game_libraries[0].path, &game.join("new/file")).is_ok());
        fs::write(game.join(".ludomere/installation.json"), b"broken").unwrap();
        assert!(matches!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        config.game_libraries[0].path = root.path().join("absent");
        assert!(matches!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Unavailable(_)
        ));
        config.game_libraries[0].path = crate::identity::data_root().join("runtime-logs/nested");
        assert!(matches!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        for protected in ["cloud-save-backups", "cloud-save-deletion-recovery"] {
            config.game_libraries[0].path =
                crate::identity::data_root().join(protected).join("nested");
            assert!(matches!(
                inspect_with_evidence(&config, &[])[0].compatibility,
                LibraryCompatibility::Incompatible(_)
            ));
        }
    }

    #[test]
    fn provider_language_packs_require_installer_storage_and_native_payloads_remain_valid() {
        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let game = config.game_libraries[0].path.join("native");
        fs::create_dir(&game).unwrap();
        fs::write(game.join("launch.exe"), b"inert").unwrap();
        assert_eq!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Compatible
        );
        fs::remove_file(game.join("launch.exe")).unwrap();
        fs::write(game.join("goggame-7.info"), br#"{"gameId":"7"}"#).unwrap();
        assert_eq!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Compatible
        );
        fs::remove_file(game.join("goggame-7.info")).unwrap();
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&game).unwrap();
        let receipt = config.game_libraries[0]
            .path
            .join(".ludomere/staging/native.recovery.json");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(receipt, serde_json::to_vec(&serde_json::json!({"version":1,"product_id":7,"directory":game,"identity":[metadata.dev(),metadata.ino()]})).unwrap()).unwrap();
        assert_eq!(
            inspect_with_evidence(&config, &[])[0].compatibility,
            LibraryCompatibility::Compatible
        );
        let artifact: RemoteArtifact = serde_json::from_value(serde_json::json!({"product_id":7,"kind":"extra","name":"language","download_path":"/file","provider_category":"language_pack"})).unwrap();
        assert_eq!(
            artifact_library_kind(&artifact),
            LibraryKind::OfflineInstallers
        );
    }
}
