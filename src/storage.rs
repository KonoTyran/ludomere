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
    pub game_issues: Vec<GameDirectoryIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDirectoryIssue {
    pub path: PathBuf,
    pub reason: String,
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

pub fn inspect_library_status(
    config: &Config,
    kind: LibraryKind,
    id: &str,
) -> Result<LibraryStatus> {
    let library = config
        .libraries(kind)
        .iter()
        .find(|library| library.id == id)
        .context("The selected library is no longer configured")?;
    let evidence = library_evidence(&crate::state::StateStore::open()?, Some(&library.path))?;
    let mut game_issues = Vec::new();
    let compatibility =
        library_compatibility(config, kind, library, &evidence, &mut game_issues, true);
    Ok(LibraryStatus {
        kind,
        library_id: library.id.clone(),
        path: library.path.clone(),
        compatibility,
        game_issues,
    })
}

pub(crate) fn inspect_libraries_with_store(
    config: &Config,
    store: &crate::state::StateStore,
) -> Result<Vec<LibraryStatus>> {
    Ok(inspect_with_evidence(
        config,
        &library_evidence(store, None)?,
        true,
    ))
}

/// Root compatibility for targeted game refreshes, without inspecting sibling games.
/// This omits Game Files child diagnostics; Storage uses the full inspection APIs.
/// Offline Installers and Extras retain their full typed-content checks.
pub(crate) fn inspect_libraries_for_refresh(
    config: &Config,
    store: &crate::state::StateStore,
) -> Result<Vec<LibraryStatus>> {
    Ok(inspect_with_evidence(
        config,
        &library_evidence(store, None)?,
        false,
    ))
}

fn library_evidence(
    store: &crate::state::StateStore,
    root: Option<&Path>,
) -> Result<Vec<(PathBuf, LibraryKind)>> {
    let files = store
        .managed_files()?
        .into_iter()
        .filter(|file| file.present && root.is_none_or(|root| file.path.starts_with(root)))
        .collect::<Vec<_>>();
    let mut parts = HashMap::new();
    let products = files
        .iter()
        .map(|file| file.product_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for products in products.chunks(400) {
        for revision in store
            .load_download_revisions_for(products, false)?
            .into_values()
            .flatten()
        {
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
    evidence.extend(store.download_jobs()?.iter().filter_map(|job| {
        if root.is_some_and(|root| !job.destination.starts_with(root)) {
            return None;
        }
        job.artifacts
            .first()
            .map(|artifact| (job.destination.clone(), artifact_library_kind(artifact)))
    }));
    Ok(evidence)
}

fn inspect_with_evidence(
    config: &Config,
    evidence: &[(PathBuf, LibraryKind)],
    inspect_game_children: bool,
) -> Vec<LibraryStatus> {
    LibraryKind::ALL
        .into_iter()
        .flat_map(|kind| {
            config
                .libraries(kind)
                .iter()
                .map(move |library| (kind, library))
        })
        .map(|(kind, library)| {
            let mut game_issues = Vec::new();
            let compatibility = library_compatibility(
                config,
                kind,
                library,
                evidence,
                &mut game_issues,
                inspect_game_children || kind != LibraryKind::GameFiles,
            );
            LibraryStatus {
                kind,
                library_id: library.id.clone(),
                path: library.path.clone(),
                compatibility,
                game_issues,
            }
        })
        .collect()
}

fn library_compatibility(
    config: &Config,
    kind: LibraryKind,
    library: &GameLibrary,
    evidence: &[(PathBuf, LibraryKind)],
    game_issues: &mut Vec<GameDirectoryIssue>,
    inspect_children: bool,
) -> LibraryCompatibility {
    match inspect_library(
        config,
        kind,
        library,
        evidence,
        game_issues,
        inspect_children,
    ) {
        Ok(None) => LibraryCompatibility::Compatible,
        Ok(Some(reason)) => LibraryCompatibility::Incompatible(reason),
        Err(error) if error.chain().any(|cause| cause.is::<std::io::Error>()) => {
            LibraryCompatibility::Unavailable(format!(
                "Could not inspect this library: {error}. Recheck after restoring access."
            ))
        }
        Err(error) => LibraryCompatibility::Incompatible(error.to_string()),
    }
}

pub fn validate_library(config: &Config, kind: LibraryKind, id: &str) -> Result<GameLibrary> {
    let library = config
        .libraries(kind)
        .iter()
        .find(|library| library.id == id)
        .context("The selected library is no longer configured for this type")?;
    // Admission checks the selected root only. Config-wide overlap checks remain in
    // inspect_library, but unrelated archive trees need not be traversed for every launch.
    let evidence = if kind == LibraryKind::GameFiles {
        Vec::new()
    } else {
        library_evidence(&crate::state::StateStore::open()?, Some(&library.path))?
    };
    match library_compatibility(config, kind, library, &evidence, &mut Vec::new(), false) {
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
    if kind == LibraryKind::GameFiles
        && let Some(name) = path.strip_prefix(&library.path)?.components().next()
    {
        let directory = library.path.join(name.as_os_str());
        validate_game_directory_location(config, &directory)?;
        match fs::symlink_metadata(&directory) {
            Ok(_) => {
                let evidence =
                    library_evidence(&crate::state::StateStore::open()?, Some(&directory))?;
                if let Some(reason) =
                    inspect_game_directory(&library, &directory, &evidence, &mut 100_000)?
                {
                    bail!(
                        "This game's files need attention: {reason}. Browse its files, repair it, or choose confirmed recovery; other games remain available"
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(library)
}

/// Location safety only, for browsing or explicitly confirmed recovery. Does not
/// establish installedness, payload ownership, or permission to execute files.
pub fn validate_game_directory_location(config: &Config, directory: &Path) -> Result<GameLibrary> {
    let library = config
        .game_libraries
        .iter()
        .find(|library| directory.parent() == Some(library.path.as_path()))
        .context("Choose a direct game directory in a configured Game Files library")?;
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid game directory name")?;
    crate::compatibility::validate_slug(slug)?;
    ensure!(
        slug != crate::identity::MARKER_DIRECTORY && slug != crate::identity::STAGING_DIRECTORY,
        "Library infrastructure is not a game directory"
    );
    let library = validate_library(config, LibraryKind::GameFiles, &library.id)?;
    match fs::symlink_metadata(directory) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "The game directory must be a real directory, not a link"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    contained_path(&library.path, directory)?;
    Ok(library)
}

pub fn game_directories_for_browsing(config: &Config, slug: &str) -> Result<Vec<PathBuf>> {
    crate::compatibility::validate_slug(slug)?;
    let mut directories = Vec::new();
    let mut failures = Vec::new();
    for library in &config.game_libraries {
        let directory = library.path.join(slug);
        match fs::symlink_metadata(&directory) {
            Ok(_) => match validate_game_directory_location(config, &directory) {
                Ok(_) => directories.push(directory),
                Err(error) => failures.push(format!("{}: {error}", directory.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
        }
    }
    ensure!(
        !directories.is_empty() || failures.is_empty(),
        "Could not safely browse this game's directories: {}",
        failures.join("; ")
    );
    Ok(directories)
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

fn library_file(root: &Path, path: &Path) -> Result<Option<fs::File>> {
    use std::os::fd::{AsRawFd, FromRawFd};
    ensure!(
        path.starts_with(root) && path.is_absolute(),
        "Metadata is outside its library"
    );
    let mut file = fs::File::open("/")?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => std::ffi::CString::new(name.as_encoded_bytes())?,
            _ => bail!("Invalid library metadata path"),
        };
        let fd = unsafe {
            libc::openat(
                file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC
                    | if components.peek().is_some() {
                        libc::O_DIRECTORY
                    } else {
                        0
                    },
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            ensure!(
                !matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)),
                "Library metadata cannot use symbolic links or non-directory ancestors"
            );
            return Err(error.into());
        }
        file = unsafe { fs::File::from_raw_fd(fd) };
    }
    ensure!(
        file.metadata()?.is_file(),
        "Library metadata must be a regular file"
    );
    Ok(Some(file))
}

fn metadata_json<T: serde::de::DeserializeOwned>(
    root: &Path,
    path: &Path,
    limit: u64,
) -> Result<Option<T>> {
    let Some(file) = library_file(root, path)? else {
        return Ok(None);
    };
    let metadata = file.metadata()?;
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
    game_issues: &mut Vec<GameDirectoryIssue>,
    inspect_children: bool,
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
    if kind == LibraryKind::GameFiles {
        for name in [
            crate::identity::MARKER_DIRECTORY,
            crate::identity::STAGING_DIRECTORY,
        ] {
            match fs::symlink_metadata(library.path.join(name)) {
                Ok(metadata) => ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "Library infrastructure must be a real directory, not a link"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        // Check root accessibility without touching any sibling game's content.
        fs::read_dir(&library.path)?;
    }
    for (path, actual) in evidence {
        if kind != LibraryKind::GameFiles
            && path.starts_with(&library.path)
            && *actual != kind
            && fs::symlink_metadata(path).is_ok()
        {
            return Ok(Some(format!(
                "Contains recorded {} content",
                actual.label()
            )));
        }
    }
    if kind == LibraryKind::GameFiles && !inspect_children {
        return Ok(None);
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
        let entry_type = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if kind == LibraryKind::GameFiles => {
                game_issues.push(GameDirectoryIssue {
                    path: entry.path(),
                    reason: format!("Could not inspect this game entry: {error}"),
                });
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if !entry_type.is_dir() {
            if kind == LibraryKind::GameFiles
                && name != crate::identity::MARKER_DIRECTORY
                && name != crate::identity::STAGING_DIRECTORY
            {
                game_issues.push(GameDirectoryIssue {
                    path: entry.path(),
                    reason: "This game entry is not a real directory; links are not followed"
                        .into(),
                });
                continue;
            }
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
            let result = inspect_game_directory(library, &entry.path(), evidence, &mut 100_000);
            let reason = match result {
                Ok(reason) => reason,
                Err(error) => Some(error.to_string()),
            };
            if let Some(reason) = reason {
                game_issues.push(GameDirectoryIssue {
                    path: entry.path(),
                    reason,
                });
            }
        } else if let Some(reason) =
            inspect_product(&entry.path(), kind, evidence, &mut budget, true)?
        {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

fn inspect_game_directory(
    library: &GameLibrary,
    directory: &Path,
    evidence: &[(PathBuf, LibraryKind)],
    budget: &mut usize,
) -> Result<Option<String>> {
    let name = directory
        .file_name()
        .context("Game directory has no name")?;
    for (path, actual) in evidence {
        if path.starts_with(directory)
            && *actual != LibraryKind::GameFiles
            && fs::symlink_metadata(path).is_ok()
        {
            return Ok(Some(format!(
                "Contains recorded {} content",
                actual.label()
            )));
        }
    }
    let mut archive = managed_archive_layout(directory, budget)?;
    let dlc = directory.join("dlc");
    let dlc_directory = match fs::symlink_metadata(&dlc) {
        Ok(metadata) => metadata.is_dir(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if !archive && dlc_directory {
        for child in fs::read_dir(&dlc)? {
            let child = child?;
            ensure!(
                *budget > 0,
                "Library inspection exceeds its bounded entry limit"
            );
            *budget -= 1;
            if child.file_type()?.is_dir() {
                archive |= managed_archive_layout(&child.path(), budget)?;
            }
        }
    }
    if archive {
        return Ok(Some(
            "Contains an archive in the managed platform layout; choose separate typed libraries"
                .into(),
        ));
    }
    let marker = metadata_json::<crate::installation::InstallationMarker>(
        &library.path,
        &directory.join(".ludomere/installation.json"),
        4 * 1024 * 1024,
    )?;
    if let Some(marker) = &marker {
        marker.validate()?;
        ensure!(
            marker.slug == name.to_string_lossy(),
            "Installation marker does not match its product directory"
        );
    }
    let journal =
        crate::installation::operation_journal::path(&library.path, &name.to_string_lossy())?;
    let journal = metadata_json::<crate::installation::operation_journal::OperationJournal>(
        &library.path,
        &journal,
        64 * 1024 * 1024,
    )?;
    if let Some(journal) = &journal {
        use crate::installation::operation_journal::OperationJournal;
        match journal {
            OperationJournal::Depot { version, record } => ensure!(
                *version == 1 && record.destination == directory,
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
                        .is_some_and(|path| Path::new(path) == directory)
                        && game.get("product_id").and_then(|value| value.as_i64())
                            == Some(record.product_id),
                    "Operation journal does not match its product directory"
                );
            }
        }
    }
    let mut retained = false;
    for suffix in ["recovery", "retained"] {
        if suffix == "retained" && (marker.is_some() || journal.is_some()) {
            continue;
        }
        let receipt = metadata_json::<serde_json::Value>(
            &library.path,
            &library
                .path
                .join(".ludomere/staging")
                .join(format!("{}.{suffix}.json", name.to_string_lossy())),
            4 * 1024 * 1024,
        )?;
        if let Some(recovery) = &receipt {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::symlink_metadata(directory)?;
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
                        .is_some_and(|path| Path::new(path) == directory)
                    && identity == Some((metadata.dev(), metadata.ino())),
                "Retained file identity does not match its product directory"
            );
            retained = true;
        }
    }
    let mut gog_info = false;
    if marker.is_none() && journal.is_none() && !retained {
        for info in fs::read_dir(directory)? {
            let info = info?;
            ensure!(
                *budget > 0,
                "Library inspection exceeds its bounded entry limit"
            );
            *budget -= 1;
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
            let info =
                metadata_json::<serde_json::Value>(&library.path, &info.path(), 4 * 1024 * 1024)?
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
        && !retained
        && !gog_info
        && !depot_payload_evidence(&library.path, directory)?
        && !plausible_payload(directory, 0, budget)?
    {
        return Ok(Some(format!(
            "Product directory {} has no recognized installation or operation. Browse its files, repair the game, or choose confirmed recovery; its files have not been changed",
            name.to_string_lossy()
        )));
    }
    Ok(None)
}

fn managed_archive_layout(directory: &Path, budget: &mut usize) -> Result<bool> {
    for category in ["installer", "patch", "extra"] {
        let category = directory.join(category);
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
                *budget > 0,
                "Library inspection exceeds its bounded entry limit"
            );
            *budget -= 1;
            if platform.file_type()?.is_dir()
                && matches!(
                    platform.file_name().to_str(),
                    Some("windows" | "linux" | "mac" | "macos" | "any")
                )
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

// A factory reset discards runnable operations, but older builds leave the Depot's
// materialization journal. One matching chunk establishes category, not installedness,
// ownership for deletion, or permission to resume its old operation.
fn depot_payload_evidence(library: &Path, directory: &Path) -> Result<bool> {
    #[derive(serde::Deserialize)]
    struct Chunk {
        index: usize,
        offset: u64,
        size: u64,
        md5: String,
    }
    #[derive(serde::Deserialize)]
    struct File {
        path: String,
        identity: String,
        chunks: Vec<Chunk>,
    }
    #[derive(serde::Deserialize)]
    struct Journal {
        version: u32,
        manifest_identity: String,
        files: Vec<File>,
    }
    let Some(journal) = metadata_json::<Journal>(
        library,
        &library.join(".ludomere/staging").join(format!(
            "{}.json",
            directory
                .file_name()
                .context("Missing product name")?
                .to_string_lossy()
        )),
        crate::download::depot::JOURNAL_LIMIT,
    )?
    else {
        return Ok(false);
    };
    let hash = |value: &str, size| {
        value.len() == size && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    ensure!(
        journal.version == 1 && hash(&journal.manifest_identity, 64),
        "Unrecognized Depot materialization journal"
    );
    for file in &journal.files {
        ensure!(
            !file.path.is_empty()
                && Path::new(&file.path)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                && !file.path.contains('\\')
                && hash(&file.identity, 64),
            "Unsafe Depot materialization entry"
        );
        let mut offset = 0_u64;
        for (index, chunk) in file.chunks.iter().enumerate() {
            ensure!(
                chunk.index == index && chunk.offset == offset && hash(&chunk.md5, 32),
                "Invalid Depot materialization chunk"
            );
            offset = offset
                .checked_add(chunk.size)
                .context("Depot materialization size overflow")?;
        }
    }
    let mut budget = 16 * 1024 * 1024;
    for file in journal.files {
        let Some(chunk) = file
            .chunks
            .first()
            .filter(|chunk| chunk.size > 0 && chunk.size <= budget)
        else {
            continue;
        };
        let Some(payload) = library_file(library, &directory.join(&file.path))? else {
            continue;
        };
        if payload.metadata()?.len() < chunk.size {
            continue;
        }
        budget -= chunk.size;
        let mut digest = md5::Context::new();
        let mut input = payload.take(chunk.size);
        let mut buffer = [0_u8; 64 * 1024];
        let mut read = 0;
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            digest.consume(&buffer[..count]);
        }
        if read == chunk.size && format!("{:x}", digest.compute()) == chunk.md5 {
            return Ok(true);
        }
    }
    Ok(false)
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

    #[test]
    fn library_evidence_batches_catalogs_without_changing_scope_or_categories() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("library.sqlite3");
        let store = crate::state::StateStore::open_at(&database).unwrap();
        let mut connection = rusqlite::Connection::open(&database).unwrap();
        let selected = root.path().join("selected");
        let other = root.path().join("other");
        let transaction = connection.transaction().unwrap();
        for product in 1..=500 {
            transaction.execute(
                "INSERT INTO download_slots(slot_id,product_id,provider_group_id,provider_category,name,first_seen_at,last_seen_at)
                 VALUES(?1,?1,'group',?2,'fixture',1,1)",
                rusqlite::params![product, if product % 2 == 1 { "bonus" } else { "installer" }],
            ).unwrap();
            transaction.execute(
                "INSERT INTO download_revisions(revision_id,slot_id,manifest_fingerprint,currently_offered,first_seen_at,last_seen_at,retired_at)
                 VALUES(?1,?1,'fixture',0,1,1,2)", [product],
            ).unwrap();
            transaction.execute(
                "INSERT INTO download_parts(part_id,revision_id,provider_file_id,part_index,downlink)
                 VALUES(?1,?1,'file',0,'fixture')", [product],
            ).unwrap();
            transaction.execute(
                "INSERT INTO managed_files(path,product_id,product_slug,artifact_kind,filename,size,part_id)
                 VALUES(?1,?2,'fixture','extra','fixture',1,?2)",
                rusqlite::params![(if product == 500 { &other } else { &selected }).join(format!("file-{product}")).to_str().unwrap(), product],
            ).unwrap();
        }
        for (name, kind, part) in [
            ("legacy", "patch", None),
            ("missing-part", "extra", Some(9000)),
            ("cross-product", "extra", Some(500)),
        ] {
            transaction.execute(
                "INSERT INTO managed_files(path,product_id,product_slug,artifact_kind,filename,size,part_id)
                 VALUES(?1,1,'fixture',?2,'fixture',1,?3)",
                rusqlite::params![selected.join(name).to_str().unwrap(), kind, part],
            ).unwrap();
        }
        transaction.execute_batch(
            "INSERT INTO download_slots VALUES(999,999,'group','bonus',X'FF',NULL,NULL,NULL,1,1);
             INSERT INTO download_revisions VALUES(999,999,NULL,NULL,'fixture',1,1,1,NULL);
             INSERT INTO managed_files(path,product_id,product_slug,artifact_kind,filename,size,present)
             VALUES('/synthetic/absent',999,'fixture','extra','fixture',1,0);",
        ).unwrap();
        transaction.commit().unwrap();

        // Reference the former per-product loading path using exactly the represented
        // products, including retired revisions. No unrelated catalog is materialized.
        let started = std::time::Instant::now();
        let files = store.managed_files().unwrap();
        let mut categories = HashMap::new();
        for product in files
            .iter()
            .filter(|file| file.present)
            .map(|file| file.product_id)
            .collect::<std::collections::HashSet<_>>()
        {
            for revision in store.load_all_download_revisions(product).unwrap() {
                let kind = if revision.provider_category == DownloadCategory::Bonus {
                    LibraryKind::Extras
                } else {
                    LibraryKind::OfflineInstallers
                };
                categories.extend(revision.parts.into_iter().map(|part| (part.part_id, kind)));
            }
        }
        let expected = files
            .into_iter()
            .filter(|file| file.present)
            .map(|file| {
                let kind = file
                    .part_id
                    .and_then(|part| categories.get(&part))
                    .copied()
                    .unwrap_or(if file.kind == ArtifactKind::Extra {
                        LibraryKind::Extras
                    } else {
                        LibraryKind::OfflineInstallers
                    });
                (file.path, kind)
            })
            .collect::<Vec<_>>();
        let previous_elapsed = started.elapsed();
        let started = std::time::Instant::now();
        let actual = library_evidence(&store, None).unwrap();
        eprintln!(
            "500 product catalogs: per-product {previous_elapsed:?}, batched {:?}",
            started.elapsed()
        );
        assert_eq!(actual, expected);
        let actual = actual.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(actual[&selected.join("file-1")], LibraryKind::Extras);
        assert_eq!(
            actual[&selected.join("legacy")],
            LibraryKind::OfflineInstallers
        );
        assert_eq!(actual[&selected.join("missing-part")], LibraryKind::Extras);
        assert_eq!(
            actual[&selected.join("cross-product")],
            LibraryKind::OfflineInstallers
        );
        let scoped = library_evidence(&store, Some(&selected))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(scoped[&selected.join("cross-product")], LibraryKind::Extras);
        assert_eq!(scoped.len(), 502);
        connection
            .execute(
                "UPDATE download_slots SET name=X'FF' WHERE product_id=500",
                [],
            )
            .unwrap();
        assert!(library_evidence(&store, Some(&selected)).is_ok());
        assert!(library_evidence(&store, None).is_err());
        connection.execute("DELETE FROM managed_files", []).unwrap();
        assert!(library_evidence(&store, None).unwrap().is_empty());
    }

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
    fn targeted_refresh_checks_roots_and_archives_without_scanning_game_siblings() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let store = crate::state::StateStore::open_at(&root.path().join("state.db")).unwrap();
        for id in 0..500 {
            let partial = config.game_libraries[0].path.join(format!("partial-{id}"));
            fs::create_dir(&partial).unwrap();
            fs::write(partial.join("incomplete.bin"), b"partial payload").unwrap();
        }
        let healthy = config.game_libraries[0].path.join("healthy");
        marker(&healthy);
        fs::write(
            healthy.join("game.sh"),
            b"inert synthetic executable; never run",
        )
        .unwrap();
        fs::set_permissions(healthy.join("game.sh"), fs::Permissions::from_mode(0o700)).unwrap();
        let started = std::time::Instant::now();
        let full = inspect_libraries_with_store(&config, &store).unwrap();
        let full_time = started.elapsed();
        let started = std::time::Instant::now();
        let targeted = inspect_libraries_for_refresh(&config, &store).unwrap();
        eprintln!(
            "500 partial siblings: full inspection {full_time:?}, targeted roots {:?}",
            started.elapsed()
        );
        assert_eq!(full[0].game_issues.len(), 500);
        assert!(targeted[0].game_issues.is_empty());
        assert_eq!(targeted[0].compatibility, LibraryCompatibility::Compatible);
        assert_eq!(full[0].compatibility, targeted[0].compatibility);
        assert_eq!(
            crate::installation::reconcile_installed_products(
                &store,
                &config.game_libraries,
                &[(7, "healthy".into())],
                &HashMap::new(),
            )
            .unwrap()[0]
                .installation_directory,
            healthy
        );
        assert!(
            validate_path(
                &config,
                LibraryKind::GameFiles,
                &config.game_libraries[0].path.join("partial-0")
            )
            .is_err()
        );

        for (kind, category) in [
            (LibraryKind::OfflineInstallers, "extra"),
            (LibraryKind::Extras, "installer"),
        ] {
            fs::create_dir_all(config.libraries(kind)[0].path.join("game").join(category)).unwrap();
        }
        let full = inspect_libraries_with_store(&config, &store).unwrap();
        let targeted = inspect_libraries_for_refresh(&config, &store).unwrap();
        for index in [1, 2] {
            assert!(matches!(
                targeted[index].compatibility,
                LibraryCompatibility::Incompatible(_)
            ));
            assert_eq!(targeted[index].compatibility, full[index].compatibility);
        }
        symlink(
            &config.game_libraries[0].path,
            root.path().join("linked-games"),
        )
        .unwrap();
        for path in [
            root.path().join("linked-games"),
            crate::identity::config_root(),
            config.offline_libraries[0].path.join("overlap"),
        ] {
            let mut invalid = config.clone();
            invalid.game_libraries[0].path = path;
            assert!(matches!(
                inspect_libraries_for_refresh(&invalid, &store).unwrap()[0].compatibility,
                LibraryCompatibility::Incompatible(_)
            ));
        }
        symlink(root.path(), config.game_libraries[0].path.join(".ludomere")).unwrap();
        assert!(matches!(
            inspect_libraries_for_refresh(&config, &store).unwrap()[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
    }

    #[test]
    fn broken_game_entries_do_not_block_healthy_siblings_or_new_targets() {
        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let library = &config.game_libraries[0];
        let terraria = library.path.join("terraria");
        let grim_dawn = library.path.join("grim_dawn");
        fs::create_dir(&terraria).unwrap();
        fs::write(terraria.join("partial.bin"), b"incomplete").unwrap();
        marker(&grim_dawn);
        let check = || {
            let statuses = inspect_with_evidence(&config, &[], true);
            assert_eq!(statuses[0].compatibility, LibraryCompatibility::Compatible);
            assert!(
                statuses[0]
                    .game_issues
                    .iter()
                    .any(|issue| issue.path == terraria)
            );
            assert!(
                !statuses[0]
                    .game_issues
                    .iter()
                    .any(|issue| issue.path == grim_dawn)
            );
            assert!(validate_library(&config, LibraryKind::GameFiles, &library.id).is_ok());
            assert!(validate_path(&config, LibraryKind::GameFiles, &grim_dawn).is_ok());
            assert!(
                validate_path(
                    &config,
                    LibraryKind::GameFiles,
                    &library.path.join("new_game")
                )
                .is_ok()
            );
            assert!(validate_path(&config, LibraryKind::GameFiles, &terraria).is_err());
            assert!(validate_game_directory_location(&config, &terraria).is_ok());
        };
        check();
        fs::create_dir(terraria.join(".ludomere")).unwrap();
        fs::write(terraria.join(".ludomere/installation.json"), b"{broken").unwrap();
        check();
        fs::create_dir_all(terraria.join("installer/windows/en")).unwrap();
        check();
        fs::write(library.path.join("stray-file"), b"leave alone").unwrap();
        std::os::unix::fs::symlink(root.path(), library.path.join("unsafe-link")).unwrap();
        check();
        assert!(
            validate_game_directory_location(&config, &library.path.join("unsafe-link")).is_err()
        );
        assert!(
            validate_game_directory_location(&config, &library.path.join(".ludomere")).is_err()
        );
        assert!(validate_game_directory_location(&config, &grim_dawn.join("nested")).is_err());
        assert_eq!(
            fs::read(terraria.join("partial.bin")).unwrap(),
            b"incomplete"
        );
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
            inspect_with_evidence(&config, &[], true)
                .iter()
                .all(|status| status.compatibility == LibraryCompatibility::Compatible)
        );
        fs::create_dir_all(game.join("installer/windows/en")).unwrap();
        fs::write(game.join("installer/windows/en/setup.exe"), b"inert").unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        fs::remove_dir_all(game.join("installer")).unwrap();
        fs::create_dir_all(game.join("dlc/expansion/extra/any/en")).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        fs::create_dir_all(
            config.extras_libraries[0]
                .path
                .join(".ludomere/compatibility"),
        )
        .unwrap();
        assert!(matches!(
            inspect_with_evidence(&config, &[], true)[2].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        fs::remove_dir_all(&game).unwrap();
        let nested = config.game_libraries[0]
            .path
            .join("downloads/game/installer/windows/en");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("setup.exe"), b"inert").unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
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
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        config.game_libraries[0].path = root.path().join("absent");
        assert!(matches!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Unavailable(_)
        ));
        config.game_libraries[0].path = crate::identity::data_root().join("runtime-logs/nested");
        assert!(matches!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Incompatible(_)
        ));
        for protected in ["cloud-save-backups", "cloud-save-deletion-recovery"] {
            config.game_libraries[0].path =
                crate::identity::data_root().join(protected).join("nested");
            assert!(matches!(
                inspect_with_evidence(&config, &[], true)[0].compatibility,
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
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Compatible
        );
        fs::remove_file(game.join("launch.exe")).unwrap();
        fs::write(game.join("goggame-7.info"), br#"{"gameId":"7"}"#).unwrap();
        assert_eq!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
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
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Compatible
        );
        let artifact: RemoteArtifact = serde_json::from_value(serde_json::json!({"product_id":7,"kind":"extra","name":"language","download_path":"/file","provider_category":"language_pack"})).unwrap();
        assert_eq!(
            artifact_library_kind(&artifact),
            LibraryKind::OfflineInstallers
        );
    }

    #[test]
    fn reset_depot_files_require_matching_bounded_materialization_evidence() {
        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let library = &config.game_libraries[0].path;
        let game = library.join("partial");
        fs::create_dir(&game).unwrap();
        fs::write(game.join("content.bin"), b"partial payload").unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        let path = library.join(".ludomere/staging/partial.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut files = (0..16_011)
            .map(|index| {
                serde_json::json!({
                    "path": format!("content/{index}/{}.bin", "x".repeat(200)), "identity": "a".repeat(64), "chunks": []
                })
            })
            .collect::<Vec<_>>();
        files[0] = serde_json::json!({"path":"content.bin", "identity":"a".repeat(64),
            "chunks":[{"index":0,"offset":0,"size":15,"md5":format!("{:x}",md5::compute(b"partial payload"))}]});
        let mut journal =
            serde_json::json!({"version":1,"manifest_identity":"b".repeat(64),"files":files});
        fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 4 * 1024 * 1024);
        assert_eq!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Compatible
        );
        fs::write(game.join("content.bin"), b"changed payload").unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        fs::remove_file(game.join("content.bin")).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"partial payload").unwrap();
        std::os::unix::fs::symlink(&outside, game.join("content.bin")).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        fs::remove_file(game.join("content.bin")).unwrap();
        fs::write(game.join("content.bin"), b"partial payload").unwrap();
        journal["files"][1]["path"] = serde_json::json!("../outside");
        fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        journal["files"][1]["path"] = serde_json::json!("other.bin");
        fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        fs::create_dir_all(game.join("installer/windows/en")).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
    }

    #[test]
    fn retained_identity_does_not_accept_replaced_or_linked_directories() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let config = configured(root.path());
        let library = &config.game_libraries[0].path;
        let game = library.join("partial");
        fs::create_dir(&game).unwrap();
        let metadata = fs::metadata(&game).unwrap();
        let receipt = library.join(".ludomere/staging/partial.retained.json");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, serde_json::to_vec(&serde_json::json!({
            "version":1,"product_id":7,"directory":game,"identity":[metadata.dev(),metadata.ino()]
        })).unwrap()).unwrap();
        assert_eq!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Compatible
        );
        let outside = root.path().join("prior");
        fs::rename(&game, &outside).unwrap();
        fs::create_dir(&game).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
        // An independently validated new installation supersedes an old reset receipt.
        marker(&game);
        assert_eq!(
            inspect_with_evidence(&config, &[], true)[0].compatibility,
            LibraryCompatibility::Compatible
        );
        fs::remove_dir_all(game.join(".ludomere")).unwrap();
        fs::remove_dir(&game).unwrap();
        symlink(&outside, &game).unwrap();
        assert!(
            !inspect_with_evidence(&config, &[], true)[0]
                .game_issues
                .is_empty()
        );
    }

    #[test]
    fn selected_library_inspection_still_checks_other_configured_paths_for_overlap() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured(root.path());
        config.extras_libraries[0].path = root.path().join("unavailable");
        assert_eq!(
            library_compatibility(
                &config,
                LibraryKind::GameFiles,
                &config.game_libraries[0],
                &[],
                &mut Vec::new(),
                false,
            ),
            LibraryCompatibility::Compatible
        );
        config.extras_libraries[0].path = config.game_libraries[0].path.join("extras");
        assert!(matches!(
            library_compatibility(
                &config,
                LibraryKind::GameFiles,
                &config.game_libraries[0],
                &[],
                &mut Vec::new(),
                false,
            ),
            LibraryCompatibility::Incompatible(_)
        ));
    }
}
