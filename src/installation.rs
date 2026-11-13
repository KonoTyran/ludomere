use crate::{
    config::{Config, GameLibrary},
    domain::{ArtifactKind, DownloadCategory, DownloadRevision},
    state::ManagedFileRecord,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

pub(crate) mod dependency_setup;
pub(crate) use manager::normalize_signed_out_operations;
pub(crate) use manager::{finish_sign_out_pause, request_sign_out_pause};
pub mod depot;
pub mod depot_actions;
pub mod depot_metadata;
pub mod depot_planner;
mod executor;
mod launcher;
mod manager;
mod marker;
pub(crate) mod operation_journal;
mod patch;
mod prefix_recovery;
pub use prefix_recovery::{
    PrefixRebuildPlan, PrefixRebuildResult, prepare_prefix_rebuild, rebuild_prefix,
};
pub mod recovery;
pub use recovery::{
    GameResetPlan, GameResetResult, UninstallPreparation, prepare_uninstall, reset_game,
    uninstall_prefix,
};
pub mod runtime_logs;
pub mod source_migration;
mod windows_executable;
pub use depot::{delete_abandoned_depot_staging, inspect_abandoned_depot_staging};
pub use executor::{
    AdditionalInstaller, InstallationEvent, InstallationHandle, UninstallationEvent,
    UninstallationHandle, installation_log_path, start_installation, start_uninstallation,
    uninstallation_log_path,
};
pub use launcher::{
    CloudSyncPhase, LaunchEvent, is_game_running, is_game_stopping, launch_game, runtime_log_path,
    stop_all_games, stop_game,
};
pub use manager::{
    DepotManagerEvent, DepotOperationRequest, DepotOperationSnapshot, DepotSetupProgress,
    DepotSource, InstallationManagerEvent, InstallationOperationSnapshot, TrackedInstallation,
    TrackedInstallationControl, abandon_depot_operation, cancel_depot_operation, cancel_operation,
    depot_operation_snapshot, depot_operation_snapshot_for_product, depot_operation_snapshots,
    enqueue_depot_operation, enqueue_downloaded_installation, enqueue_installation,
    enqueue_installation_tracked, enqueue_uninstallation, enqueue_uninstallation_with_cleanup,
    installation_operation_snapshot, pause_for_sign_out, prepare_depot_resume,
    recover_depot_operations, recover_interrupted_operations, respond_to_installation,
    resume_depot_operation, shutdown, start_recovered_operations, subscribe_depot_events,
    subscribe_installation_events, wait_for_paused,
};
pub use marker::{
    InstallationMarker, InstalledDlc, from_game as installation_marker_from_game,
    load as load_installation_marker, write as write_installation_marker,
};
pub use patch::{PatchEvent, patch_target_version, run_patch};
pub use windows_executable::{
    WindowsExecutableCandidate, WindowsExecutableDiscovery, discover_windows_executable,
};

pub fn recover_backend_operations() -> anyhow::Result<(usize, usize)> {
    let installers = recover_interrupted_operations()?;
    let depots = recover_depot_operations()?;
    Ok((installers, depots))
}

pub(crate) fn validate_game_library(
    config: &Config,
    library_id: &str,
    directory: &std::path::Path,
) -> anyhow::Result<()> {
    prefix_recovery::ensure_quiescent(directory)?;
    let library = crate::storage::validate_library(
        config,
        crate::config::LibraryKind::GameFiles,
        library_id,
    )?;
    anyhow::ensure!(
        directory.parent() == Some(library.path.as_path()),
        "The game's configured library changed; reopen this action."
    );
    crate::storage::validate_path(config, crate::config::LibraryKind::GameFiles, directory)?;
    Ok(())
}

pub(crate) fn validate_offline_sources(config: &Config, files: &[PathBuf]) -> anyhow::Result<()> {
    let mut selected = None;
    for path in files {
        let library = crate::storage::validate_path(
            config,
            crate::config::LibraryKind::OfflineInstallers,
            path,
        )?;
        anyhow::ensure!(
            selected.as_ref().is_none_or(|id| id == &library.id),
            "Select an installer set from one compatible Offline Installers library."
        );
        selected = Some(library.id);
    }
    Ok(())
}

pub fn patch_log_path(product_id: i64) -> anyhow::Result<PathBuf> {
    let root = crate::identity::installation_logs();
    std::fs::create_dir_all(&root)?;
    Ok(root.join(format!("{product_id}-patch.log")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerCandidate {
    pub product_id: i64,
    pub revision_id: Option<i64>,
    pub version: Option<String>,
    pub operating_system: Option<String>,
    pub language: Option<String>,
    pub paths: Vec<PathBuf>,
    pub launcher: Option<PathBuf>,
    pub method: InstallationMethod,
    pub total_size: u64,
    pub currently_offered: bool,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshInstallSource {
    OfflineInstaller(usize),
    GalaxyWindows,
}

pub fn rank_fresh_install_sources(
    config: &Config,
    candidates: &[InstallerCandidate],
    galaxy_windows_available: bool,
) -> Vec<FreshInstallSource> {
    use crate::config::PreferredInstallationSource::*;
    let mut ranked = Vec::new();
    for preferred in &config.installation_source_order {
        match preferred {
            LinuxOffline => ranked.extend(
                candidates
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| candidate.method == InstallationMethod::NativeLinux)
                    .map(|(index, _)| FreshInstallSource::OfflineInstaller(index)),
            ),
            WindowsGalaxy if galaxy_windows_available => {
                ranked.push(FreshInstallSource::GalaxyWindows)
            }
            WindowsOffline => ranked.extend(
                candidates
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.method == InstallationMethod::WindowsCompatibility
                    })
                    .map(|(index, _)| FreshInstallSource::OfflineInstaller(index)),
            ),
            _ => {}
        }
    }
    ranked
}

pub fn resolve_installation_directory(
    game: &crate::domain::InstalledGame,
    libraries: &[GameLibrary],
) -> Option<(String, PathBuf)> {
    let slug = game.installation_directory.file_name()?;
    let mut candidates = vec![(game.library_id.clone(), game.installation_directory.clone())];
    for library in libraries {
        let candidate = (library.id.clone(), library.path.join(slug));
        if !candidates.iter().any(|existing| existing.1 == candidate.1) {
            candidates.push(candidate);
        }
    }
    let executable_relative = game
        .primary_executable
        .as_ref()
        .and_then(|path| path.strip_prefix(&game.installation_directory).ok())
        .map(PathBuf::from);
    candidates.into_iter().find(|(_, directory)| {
        if let Some(relative) = &executable_relative {
            return directory.join(relative).is_file();
        }
        directory_has_installed_payload(directory)
    })
}

pub fn reconcile_installed_games(
    store: &crate::state::StateStore,
    libraries: &[GameLibrary],
) -> anyhow::Result<Vec<crate::domain::InstalledGame>> {
    let mut discovered = HashMap::new();
    for library in libraries {
        let Ok(entries) = std::fs::read_dir(&library.path) else {
            continue;
        };
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let directory = entry.path();
            match marker::load(&directory) {
                Ok(Some(marker)) => {
                    if depot::operation_staging_path(
                        &library.path,
                        &directory,
                        &marker.slug,
                        "reconcile",
                    )
                    .is_ok_and(|journal| journal.is_file())
                    {
                        continue;
                    }
                    discovered.insert(marker.product_id, (library.id.clone(), directory, marker));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(path = %directory.display(), %error, "could not read installation marker")
                }
            }
        }
    }
    reconcile_discovered(store, discovered, true)
}

/// Refresh known products without enumerating other games or changing preferences.
pub fn reconcile_installed_products(
    store: &crate::state::StateStore,
    libraries: &[GameLibrary],
    products: &[(i64, String)],
    existing: &HashMap<i64, crate::domain::InstalledGame>,
) -> anyhow::Result<Vec<crate::domain::InstalledGame>> {
    let mut discovered = HashMap::new();
    for (id, slug) in products {
        anyhow::ensure!(
            std::path::Path::new(slug).components().count() == 1
                && matches!(
                    std::path::Path::new(slug).components().next(),
                    Some(std::path::Component::Normal(_))
                ),
            "The game folder name is invalid; refresh the library before retrying"
        );
        for library in libraries {
            let mut paths = vec![library.path.join(slug)];
            if let Some(game) = existing.get(id)
                && game.library_id == library.id
                && game.installation_directory.starts_with(&library.path)
                && !paths.contains(&game.installation_directory)
            {
                paths.push(game.installation_directory.clone());
            }
            for directory in paths {
                match std::fs::metadata(marker::marker_path(&directory)) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                }
                let Some(found_marker) = marker::load(&directory)? else {
                    continue;
                };
                if found_marker.product_id != *id
                    || depot::operation_staging_path(
                        &library.path,
                        &directory,
                        &found_marker.slug,
                        "reconcile",
                    )
                    .is_ok_and(|journal| journal.is_file())
                {
                    continue;
                }
                discovered.insert(*id, (library.id.clone(), directory, found_marker));
            }
        }
    }
    reconcile_discovered(store, discovered, false)
}

fn reconcile_discovered(
    store: &crate::state::StateStore,
    discovered: HashMap<i64, (String, PathBuf, marker::InstallationMarker)>,
    save_discovered_preferences: bool,
) -> anyhow::Result<Vec<crate::domain::InstalledGame>> {
    let mut reconciled = Vec::with_capacity(discovered.len());
    for (_, (library_id, directory, found_marker)) in discovered {
        let has_payload = if save_discovered_preferences {
            directory_has_installed_payload(&directory)
        } else {
            checked_installed_payload(&directory, 0)?
        };
        if !has_payload {
            continue;
        }
        let preferences = store.game_preferences(found_marker.product_id)?;
        let saved_executable = preferences
            .as_ref()
            .and_then(|preferences| preferences.executable_path.as_ref())
            .map(|path| directory.join(path))
            .filter(|path| path.is_file());
        let executable = saved_executable.or_else(|| {
            if found_marker
                .base
                .operating_system
                .as_deref()
                .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
            {
                discover_windows_executable(&directory, found_marker.product_id, &found_marker.slug)
                    .selected
            } else {
                executor::discover_linux_executable(&directory)
            }
        });
        let mut game = marker::game_from_marker(&found_marker, library_id, directory, executable);
        if let Some(preferences) = &preferences {
            game.launch_arguments = preferences.launch_arguments.clone();
            match game.installer_operating_system.as_deref() {
                Some(os) if os.eq_ignore_ascii_case("linux") => game.compatibility = None,
                Some(os) if os.eq_ignore_ascii_case("windows") => {
                    if preferences.compatibility.is_some() {
                        game.compatibility = preferences.compatibility.clone();
                    }
                }
                _ => game.compatibility = preferences.compatibility.clone(),
            }
        }
        let (last_played, playtime) = store.product_activity(game.product_id)?;
        game.last_played_at = last_played;
        game.playtime_seconds = playtime;
        if save_discovered_preferences
            && preferences
                .as_ref()
                .and_then(|preferences| preferences.executable_path.as_ref())
                .is_none()
            && game.primary_executable.is_some()
        {
            save_game_preferences(store, &game)?;
        }
        reconciled.push(game);
    }
    reconciled.sort_by_key(|game| game.product_id);
    Ok(reconciled)
}

pub fn save_game_preferences(
    store: &crate::state::StateStore,
    game: &crate::domain::InstalledGame,
) -> anyhow::Result<()> {
    let executable_path = game.primary_executable.as_ref().and_then(|path| {
        path.strip_prefix(&game.installation_directory)
            .ok()
            .map(PathBuf::from)
    });
    store.upsert_game_preferences(&crate::domain::GamePreferences {
        product_id: game.product_id,
        executable_path,
        launch_arguments: game.launch_arguments.clone(),
        // Native runtime state must not erase retained Windows preferences.
        compatibility: if game
            .installer_operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("linux"))
        {
            store
                .game_preferences(game.product_id)?
                .and_then(|preferences| preferences.compatibility)
        } else {
            game.compatibility.clone()
        },
        created_at: game.created_at,
        updated_at: game.updated_at,
        ..Default::default()
    })?;
    store.preserve_product_activity(game.product_id, game.last_played_at, game.playtime_seconds)
}

pub fn installed_dlc_ids(
    store: &crate::state::StateStore,
    parent_product_id: i64,
) -> anyhow::Result<HashSet<i64>> {
    let game = find_installed_game(store, parent_product_id)?;
    let Some(game) = game else {
        return Ok(HashSet::new());
    };
    Ok(marker::load(&game.installation_directory)?
        .map(|marker| marker.dlc.into_iter().map(|dlc| dlc.product_id).collect())
        .unwrap_or_default())
}

pub fn installed_dlc_updates(
    store: &crate::state::StateStore,
    parent_product_id: i64,
) -> anyhow::Result<HashSet<i64>> {
    let game = find_installed_game(store, parent_product_id)?;
    let Some(game) = game else {
        return Ok(HashSet::new());
    };
    let Some(marker) = marker::load(&game.installation_directory)? else {
        return Ok(HashSet::new());
    };
    let updates = marker
        .dlc
        .into_iter()
        .filter_map(|dlc| dlc.revision_id.map(|revision| (dlc.product_id, revision)))
        .filter_map(|(product, revision)| {
            store
                .revision_has_update(revision)
                .ok()
                .filter(|value| *value)
                .map(|_| product)
        })
        .collect::<HashSet<_>>();
    Ok(updates)
}

fn find_installed_game(
    store: &crate::state::StateStore,
    product_id: i64,
) -> anyhow::Result<Option<crate::domain::InstalledGame>> {
    let config = Config::load_or_create()?;
    Ok(reconcile_installed_games(store, &config.game_libraries)?
        .into_iter()
        .find(|game| game.product_id == product_id))
}

pub(crate) fn directory_has_installed_payload(directory: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(
            name.as_ref(),
            "installer" | "patch" | "extra" | "dlc" | crate::identity::STAGING_DIRECTORY
        ) {
            return false;
        }
        let path = entry.path();
        launchable_file(&path) || (path.is_dir() && directory_contains_launchable(&path, 0))
    })
}

fn checked_installed_payload(directory: &std::path::Path, depth: usize) -> std::io::Result<bool> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if depth == 0
            && matches!(
                entry.file_name().to_str(),
                Some("installer" | "patch" | "extra" | "dlc" | crate::identity::STAGING_DIRECTORY)
            )
        {
            continue;
        }
        let path = entry.path();
        let metadata = match path.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if metadata.is_file() {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0
                || path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["exe", "com", "bat"]
                            .iter()
                            .any(|known| extension.eq_ignore_ascii_case(known))
                    })
            {
                return Ok(true);
            }
        } else if metadata.is_dir() && depth < 4 && checked_installed_payload(&path, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_contains_launchable(directory: &std::path::Path, depth: usize) -> bool {
    if depth >= 4 {
        return false;
    }
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let path = entry.path();
            launchable_file(&path)
                || (path.is_dir() && directory_contains_launchable(&path, depth + 1))
        })
    })
}

fn launchable_file(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["exe", "com", "bat"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationMethod {
    WindowsCompatibility,
    NativeLinux,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncompleteInstaller {
    pub revision_id: i64,
    pub version: Option<String>,
    pub missing_parts: usize,
    pub invalid_parts: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallerCandidates {
    pub usable: Vec<InstallerCandidate>,
    pub incomplete: Vec<IncompleteInstaller>,
    pub preferred: Option<usize>,
}

pub fn detect_installer_candidates(
    product_id: i64,
    revisions: &[DownloadRevision],
    managed_files: &[ManagedFileRecord],
    config: &Config,
) -> InstallerCandidates {
    let mut result = InstallerCandidates::default();
    let mut revision_paths = std::collections::HashSet::new();
    for revision in revisions
        .iter()
        .filter(|revision| revision.provider_category == DownloadCategory::Installer)
    {
        // Resolve each physical installer set independently. A partial copy in one
        // library must not borrow parts from another or hide its complete copy.
        let mut directories = managed_files
            .iter()
            .filter(|file| {
                file.product_id == revision.product_id
                    && revision.parts.iter().any(|part| {
                        file.part_id == Some(part.part_id)
                            || (file.revision_id == Some(revision.revision_id)
                                && file.provider_file_id.as_deref()
                                    == Some(part.provider_file_id.as_str()))
                    })
            })
            .filter_map(|file| file.path.parent().map(std::path::Path::to_path_buf))
            .collect::<std::collections::BTreeSet<_>>();
        if directories.is_empty() {
            directories.insert(PathBuf::new());
        }
        for directory in directories {
            let mut complete = true;
            let mut paths = Vec::with_capacity(revision.parts.len());
            let mut total_size = 0_u64;
            let mut missing_parts = 0;
            let mut invalid_parts = 0;
            for part in &revision.parts {
                let matching_files = managed_files
                    .iter()
                    .filter(|file| {
                        file.product_id == revision.product_id
                            && file.path.parent() == Some(directory.as_path())
                            && (file.part_id == Some(part.part_id)
                                || (file.revision_id == Some(revision.revision_id)
                                    && file.provider_file_id.as_deref()
                                        == Some(part.provider_file_id.as_str())))
                    })
                    .collect::<Vec<_>>();
                revision_paths.extend(matching_files.iter().map(|file| file.path.clone()));
                let local = matching_files
                    .into_iter()
                    .filter_map(|file| {
                        let metadata = file.path.metadata().ok()?;
                        metadata.is_file().then_some((file, metadata.len()))
                    })
                    .max_by_key(|(file, size)| {
                        (
                            part.expected_size == Some(*size),
                            launcher_matches(
                                &file.path,
                                installation_method(revision.operating_system.as_deref()),
                            ),
                            *size,
                        )
                    })
                    .map(|(file, _)| file);
                let Some(local) = local else {
                    missing_parts += 1;
                    continue;
                };
                // A file associated with a known revision must never fall through to
                // preserved-file discovery when that revision proves it incomplete.
                let Ok(metadata) = local.path.metadata() else {
                    missing_parts += 1;
                    continue;
                };
                if !metadata.is_file() {
                    invalid_parts += 1;
                    continue;
                }
                // GOG's product manifest commonly reports rounded part sizes. The
                // managed-file size is captured from the completed response and is
                // therefore the exact local identity to validate here.
                if local.size != metadata.len() || unresolved_download_descriptor(&local.path) {
                    invalid_parts += 1;
                }
                total_size += metadata.len();
                paths.push(local.path.clone());
            }
            if missing_parts > 0 || invalid_parts > 0 || revision.parts.is_empty() {
                complete = false;
                result.incomplete.push(IncompleteInstaller {
                    revision_id: revision.revision_id,
                    version: revision.version.clone(),
                    missing_parts: missing_parts + usize::from(revision.parts.is_empty()),
                    invalid_parts,
                });
                let method = installation_method(revision.operating_system.as_deref());
                let has_launcher = paths.iter().any(|path| launcher_matches(path, method));
                if revision.parts.is_empty() || !has_launcher {
                    continue;
                }
            }
            let method = installation_method(revision.operating_system.as_deref());
            let launcher = paths
                .iter()
                .find(|path| launcher_matches(path, method))
                .cloned();
            result.usable.push(InstallerCandidate {
                product_id: revision.product_id,
                revision_id: Some(revision.revision_id),
                version: revision.version.clone(),
                operating_system: revision.operating_system.clone(),
                language: revision
                    .language_name
                    .clone()
                    .or_else(|| revision.language_code.clone()),
                paths,
                launcher,
                method,
                total_size,
                currently_offered: revision.currently_offered,
                complete,
            });
        }
    }

    let mut preserved = BTreeMap::<
        (Option<String>, Option<String>, Option<String>, PathBuf),
        Vec<&ManagedFileRecord>,
    >::new();
    for file in managed_files.iter().filter(|file| {
        file.product_id == product_id
            && file.kind == ArtifactKind::Installer
            && !revision_paths.contains(&file.path)
            && file.path.is_file()
    }) {
        preserved
            .entry((
                file.version.clone(),
                file.operating_system.clone(),
                file.language.clone(),
                file.path
                    .parent()
                    .map_or_else(PathBuf::new, std::path::Path::to_path_buf),
            ))
            .or_default()
            .push(file);
    }
    for ((version, operating_system, language, _), files) in preserved {
        let paths = files
            .iter()
            .filter(|file| {
                file.path
                    .metadata()
                    .is_ok_and(|metadata| metadata.is_file())
            })
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        if paths.len() != files.len() {
            continue;
        }
        let method = installation_method(operating_system.as_deref());
        let launcher = paths
            .iter()
            .find(|path| launcher_matches(path, method))
            .cloned();
        result.usable.push(InstallerCandidate {
            product_id: files[0].product_id,
            revision_id: None,
            version,
            operating_system,
            language,
            total_size: files.iter().map(|file| file.size).sum(),
            paths,
            launcher,
            method,
            currently_offered: false,
            complete: true,
        });
    }

    result.usable.sort_by_key(|candidate| {
        (
            !candidate.complete,
            platform_rank(candidate.operating_system.as_deref(), config),
            language_rank(candidate.language.as_deref(), config),
            std::cmp::Reverse(version_sort_key(candidate.version.as_deref())),
            !candidate.currently_offered,
            std::cmp::Reverse(candidate.revision_id),
        )
    });
    result.preferred = result
        .usable
        .iter()
        .position(|candidate| {
            candidate.complete
                && candidate.method != InstallationMethod::Unsupported
                && is_enabled_platform(candidate.operating_system.as_deref(), config)
        })
        .or_else(|| {
            result.usable.iter().position(|candidate| {
                candidate.method != InstallationMethod::Unsupported
                    && is_enabled_platform(candidate.operating_system.as_deref(), config)
            })
        });
    result
}

fn version_sort_key(version: Option<&str>) -> Vec<u64> {
    version
        .unwrap_or_default()
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn unresolved_download_descriptor(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if metadata.len() > 64 * 1024 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes);
    text.trim_start().starts_with('{')
        && (text.contains("\"downlink\"") || text.contains("\"url\""))
}

fn installation_method(platform: Option<&str>) -> InstallationMethod {
    match normalize_platform(platform).as_str() {
        "windows" => InstallationMethod::WindowsCompatibility,
        "linux" => InstallationMethod::NativeLinux,
        _ => InstallationMethod::Unsupported,
    }
}

fn launcher_matches(path: &std::path::Path, method: InstallationMethod) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    match method {
        InstallationMethod::WindowsCompatibility => extension.eq_ignore_ascii_case("exe"),
        InstallationMethod::NativeLinux => extension.eq_ignore_ascii_case("sh"),
        InstallationMethod::Unsupported => false,
    }
}

fn is_enabled_platform(platform: Option<&str>, config: &Config) -> bool {
    match normalize_platform(platform).as_str() {
        "windows" => config.installer_windows,
        "linux" => config.installer_linux,
        "macos" => config.installer_macos,
        _ => true,
    }
}

fn platform_rank(platform: Option<&str>, config: &Config) -> u8 {
    if !is_enabled_platform(platform, config) {
        return 10;
    }
    let platform = normalize_platform(platform);
    if platform == normalize_platform(Some(std::env::consts::OS)) {
        return 0;
    }
    match platform.as_str() {
        "windows" => 1,
        "linux" => 2,
        "macos" => 3,
        _ => 4,
    }
}

fn language_rank(language: Option<&str>, config: &Config) -> u8 {
    let Some(language) = language else {
        return 2;
    };
    if config
        .installer_language
        .as_deref()
        .is_some_and(|preferred| preferred.eq_ignore_ascii_case(language))
    {
        return 0;
    }
    if language.eq_ignore_ascii_case("english") || language.eq_ignore_ascii_case("en") {
        return 1;
    }
    2
}

fn normalize_platform(platform: Option<&str>) -> String {
    match platform.unwrap_or_default().to_ascii_lowercase().as_str() {
        "win" | "windows" => "windows".to_owned(),
        "mac" | "macos" | "osx" => "macos".to_owned(),
        value => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DownloadPart, DownloadRevision};
    use std::{fs, time::SystemTime};

    #[test]
    #[ignore = "requires isolated HOME and all XDG directories under /tmp/ludomere-p352-"]
    fn recovered_native_marker_passes_protected_launch_admission() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p352-"),
                "{key}"
            );
        }
        let root = tempfile::tempdir().unwrap();
        let library = GameLibrary {
            id: "native".into(),
            name: "Native".into(),
            path: root.path().join("games"),
            default: true,
        };
        let config = Config {
            game_libraries: vec![library.clone()],
            offline_libraries: vec![],
            extras_libraries: vec![],
            ..Default::default()
        };
        let directory = library.path.join("enter_the_gungeon");
        let marker_path = marker::marker_path(&directory);
        fs::create_dir_all(marker_path.parent().unwrap()).unwrap();
        let executable = directory.join("start.sh");
        fs::write(&executable, b"inert native payload; never execute").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mixed = serde_json::json!({
            "schema_version": 2, "product_id": 7, "slug": "enter_the_gungeon",
            "base": { "operating_system": "linux", "language": "English", "version": "2.1.9", "revision_id": null, "installed_at": 1791283470 },
            "compatibility": { "backend": "umu", "managed_by_ludomere": true, "prefix_slug": "enter_the_gungeon", "profile": { "game_id": "umu-default", "store": "gog", "source": "default_fallback" } }
        });
        let bytes = serde_json::to_vec(&mixed).unwrap();
        fs::write(&marker_path, &bytes).unwrap();
        for _ in 0..2 {
            // The actual first admission used by run_game, including storage's
            // bounded, component-wise no-follow metadata read.
            validate_game_library(&config, &library.id, &directory).unwrap();
            let loaded = marker::load(&directory).unwrap().unwrap();
            assert_eq!(loaded.schema_version, 1);
            assert!(loaded.compatibility.is_none());
            let game = marker::game_from_marker(
                &loaded,
                library.id.clone(),
                directory.clone(),
                Some(executable.clone()),
            );
            prefix_recovery::ensure_ready(&game).unwrap();
            assert!(game.compatibility.is_none());
            assert_eq!(fs::read(&marker_path).unwrap(), bytes);
        }
        let raw: marker::InstallationMarker = serde_json::from_slice(&bytes).unwrap();
        assert!(
            marker::write(&raw, &directory).is_err(),
            "writer must remain strict"
        );
        assert_eq!(fs::read(&marker_path).unwrap(), bytes);
        for (field, value, expected) in [
            ("schema_version", serde_json::json!(3), "unsupported schema"),
            ("slug", serde_json::json!("another_game"), "reinstall"),
            ("dependencies", serde_json::json!(["dotnet"]), "reinstall"),
        ] {
            let mut rejected = mixed.clone();
            rejected[field] = value;
            let rejected = serde_json::to_vec(&rejected).unwrap();
            fs::write(&marker_path, &rejected).unwrap();
            let error = validate_game_library(&config, &library.id, &directory).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
            assert_eq!(fs::read(&marker_path).unwrap(), rejected);
        }
        // Canonical marker identities remain checked after normalization too.
        let mut mismatch = mixed.clone();
        mismatch["schema_version"] = serde_json::json!(1);
        mismatch["compatibility"] = serde_json::Value::Null;
        mismatch["slug"] = serde_json::json!("another_game");
        fs::write(&marker_path, serde_json::to_vec(&mismatch).unwrap()).unwrap();
        assert!(
            validate_game_library(&config, &library.id, &directory)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
        for rejected in [b"malformed".to_vec(), vec![b' '; 4 * 1024 * 1024 + 1]] {
            fs::write(&marker_path, &rejected).unwrap();
            assert!(validate_game_library(&config, &library.id, &directory).is_err());
            assert_eq!(fs::read(&marker_path).unwrap(), rejected);
        }
        fs::remove_file(&marker_path).unwrap();
        let outside = root.path().join("outside.json");
        fs::write(&outside, &bytes).unwrap();
        symlink(&outside, &marker_path).unwrap();
        assert!(validate_game_library(&config, &library.id, &directory).is_err());
        fs::remove_file(&marker_path).unwrap();
        fs::create_dir(&marker_path).unwrap();
        assert!(validate_game_library(&config, &library.id, &directory).is_err());
        fs::remove_dir(&marker_path).unwrap();
        fs::remove_dir(marker_path.parent().unwrap()).unwrap();
        let outside_directory = root.path().join("outside-marker");
        fs::create_dir(&outside_directory).unwrap();
        fs::write(outside_directory.join("installation.json"), &bytes).unwrap();
        symlink(&outside_directory, marker_path.parent().unwrap()).unwrap();
        assert!(validate_game_library(&config, &library.id, &directory).is_err());
        assert_eq!(fs::read(&outside).unwrap(), bytes);
        assert_eq!(
            fs::read(outside_directory.join("installation.json")).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(&executable).unwrap(),
            b"inert native payload; never execute"
        );
        assert!(!library.path.join(".ludomere/compatibility").exists());
    }

    #[test]
    #[ignore = "requires isolated HOME and all XDG directories under /tmp/ludomere-p352-"]
    fn native_reconciliation_preserves_windows_preferences_and_os_boundaries() {
        use crate::compatibility::{
            CompatibilityBackendKind, GameCompatibilityPreferences, UmuProfile, UmuProfileSource,
        };
        use crate::domain::{GamePreferences, InstallationSource};
        use std::os::unix::fs::PermissionsExt;
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p352-"),
                "{key}"
            );
        }
        let root = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::open_at(&root.path().join("state.db")).unwrap();
        let library = GameLibrary {
            id: "test".into(),
            name: "Test".into(),
            path: root.path().join("games"),
            default: true,
        };
        let proton_path = crate::identity::config_root().join("proton.json");
        fs::create_dir_all(proton_path.parent().unwrap()).unwrap();
        let proton_bytes = br#"{"default":"/inert/Proton","overrides":{"1":"/inert/Other"},"dll_overrides":{"1":{"dxgi":"native"}}}"#;
        fs::write(&proton_path, proton_bytes).unwrap();
        for (index, (os, mixed, saved)) in [
            (Some("linux"), false, 2),
            (Some("LiNuX"), true, 2),
            (Some("linux"), false, 0),
            (Some("windows"), false, 2),
            (Some("windows"), false, 1),
            (Some("windows"), false, 0),
            (Some("unknown"), false, 2),
            (None, false, 2),
        ]
        .into_iter()
        .enumerate()
        {
            let id = index as i64 + 1;
            let slug = format!("game-{id}");
            let directory = library.path.join(&slug);
            fs::create_dir_all(marker::marker_path(&directory).parent().unwrap()).unwrap();
            let windows = os == Some("windows");
            let linux = os.is_some_and(|os| os.eq_ignore_ascii_case("linux"));
            let executable = directory.join(if windows {
                format!("{slug}.exe")
            } else {
                "start.sh".into()
            });
            fs::write(&executable, b"inert payload").unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let runtime = GameCompatibilityPreferences {
                backend: CompatibilityBackendKind::Umu,
                prefix_slug: slug.clone(),
                profile: UmuProfile::fallback(),
                pending_profile: None,
            };
            let mut stored_runtime = runtime.clone();
            stored_runtime.profile.game_id = "umu-retained".into();
            stored_runtime.pending_profile = Some(UmuProfile {
                game_id: "umu-pending".into(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            });
            if saved != 0 {
                store
                    .upsert_game_preferences(&GamePreferences {
                        product_id: id,
                        // Force full reconciliation to discover and save the executable.
                        executable_path: None,
                        compatibility: (saved == 2).then_some(stored_runtime.clone()),
                        launch_arguments: vec!["--retained".into()],
                        ..Default::default()
                    })
                    .unwrap();
            }
            store.set_favorite(id, true).unwrap();
            store.add_tag(id, "Retained").unwrap();
            store.preserve_product_activity(id, Some(100), 321).unwrap();
            if saved != 0 {
                store
                    .set_game_update_preferences(
                        id,
                        Some(false),
                        Some(true),
                        Some(false),
                        Some("pl"),
                    )
                    .unwrap();
            }
            let marker = marker::InstallationMarker {
                schema_version: if windows || mixed { 2 } else { 1 },
                product_id: id,
                slug: slug.clone(),
                base: marker::InstalledComponent {
                    operating_system: os.map(str::to_owned),
                    language: Some("English".into()),
                    version: Some("2.1.9".into()),
                    revision_id: None,
                    installed_at: 1_791_283_470,
                },
                dlc: vec![],
                compatibility: (windows || mixed).then(|| marker::InstalledCompatibility {
                    backend: runtime.backend,
                    managed_by_ludomere: true,
                    prefix_slug: slug.clone(),
                    profile: runtime.profile.clone(),
                }),
                source: InstallationSource::OfflineInstaller,
                galaxy_depot: None,
                launch: None,
                dependencies: vec![],
            };
            let bytes = serde_json::to_vec(&marker).unwrap();
            fs::write(marker::marker_path(&directory), &bytes).unwrap();
            let expected_runtime = if linux {
                None
            } else if saved == 2 {
                Some(stored_runtime.clone())
            } else if windows {
                Some(runtime.clone())
            } else {
                None
            };
            for _ in 0..2 {
                let full =
                    reconcile_installed_games(&store, std::slice::from_ref(&library)).unwrap();
                let game = full.iter().find(|game| game.product_id == id).unwrap();
                assert_eq!(game.compatibility, expected_runtime, "case {index}");
                assert_eq!(game.primary_executable.as_ref(), Some(&executable));
                assert_eq!(game.last_played_at, Some(100));
                assert_eq!(game.playtime_seconds, 321);
                let targeted = reconcile_installed_products(
                    &store,
                    std::slice::from_ref(&library),
                    &[(id, slug.clone())],
                    &HashMap::from([(id, game.clone())]),
                )
                .unwrap();
                assert_eq!(targeted.len(), 1);
                assert_eq!(targeted[0].compatibility, expected_runtime);
                assert_eq!(targeted[0].primary_executable.as_ref(), Some(&executable));
                if linux {
                    let mut stale_plan = game.clone();
                    stale_plan.compatibility = Some(runtime.clone());
                    save_game_preferences(&store, &stale_plan).unwrap();
                }
                let preferences = store.game_preferences(id).unwrap().unwrap();
                assert_eq!(
                    preferences.compatibility,
                    if linux {
                        (saved == 2).then_some(stored_runtime.clone())
                    } else {
                        expected_runtime.clone()
                    }
                );
                if saved != 0 {
                    assert_eq!(preferences.launch_arguments, ["--retained"]);
                    assert_eq!(preferences.auto_update_galaxy, Some(false));
                    assert_eq!(preferences.auto_download_offline_installer, Some(true));
                    assert_eq!(preferences.prune_superseded_installers, Some(false));
                    assert_eq!(preferences.galaxy_language.as_deref(), Some("pl"));
                }
                assert!(store.favorites().unwrap().contains(&id));
                assert_eq!(store.tags().unwrap()[&id], ["Retained"]);
                assert_eq!(store.product_activity(id).unwrap(), (Some(100), 321));
                assert_eq!(fs::read(marker::marker_path(&directory)).unwrap(), bytes);
                assert_eq!(fs::read(&proton_path).unwrap(), proton_bytes);
                assert!(!library.path.join(".ludomere/compatibility").exists());
            }
        }
    }

    #[test]
    fn typed_library_operation_gates_preserve_mixed_content_and_separate_copies() {
        if std::env::var_os("LUDOMERE_TYPED_GATE_TEST").is_none() {
            let profile = tempfile::tempdir().unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "installation::tests::typed_library_operation_gates_preserve_mixed_content_and_separate_copies"])
                .env("LUDOMERE_TYPED_GATE_TEST", "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
            ] {
                let path = profile.path().join(key);
                fs::create_dir(&path).unwrap();
                child.env(key, path);
            }
            assert!(child.status().unwrap().success());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            game_libraries: vec![],
            ..Default::default()
        };
        for (kind, name) in [
            (crate::config::LibraryKind::GameFiles, "games"),
            (crate::config::LibraryKind::OfflineInstallers, "first"),
            (crate::config::LibraryKind::OfflineInstallers, "second"),
        ] {
            let path = root.path().join(name);
            fs::create_dir(&path).unwrap();
            config.libraries_mut(kind).push(GameLibrary {
                id: name.into(),
                name: name.into(),
                path,
                default: name != "second",
            });
        }
        validate_game_library(&config, "games", &root.path().join("games/game")).unwrap();
        assert!(validate_game_library(&config, "first", &root.path().join("first/game")).is_err());
        let mut files = Vec::new();
        for name in ["first", "second"] {
            let path = root
                .path()
                .join(name)
                .join("game/installer/windows/en/setup.exe");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"inert").unwrap();
            files.push(path);
        }
        validate_offline_sources(&config, &files[..1]).unwrap();
        assert!(validate_offline_sources(&config, &files).is_err());
        let mixed = root.path().join("games/installer.zip");
        fs::write(&mixed, b"preserved").unwrap();
        validate_game_library(&config, "games", &root.path().join("games/game")).unwrap();
        assert_eq!(fs::read(mixed).unwrap(), b"preserved");
        assert!(files.iter().all(|path| path.is_file()));
    }

    #[test]
    fn typed_library_operation_gates_choose_complete_copy_without_cross_root_parts() {
        let root = tempfile::tempdir().unwrap();
        for name in ["first", "second"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        let first = root.path().join("first/setup.exe");
        fs::write(&first, b"data").unwrap();
        let second = root.path().join("second/setup.exe");
        fs::write(&second, b"data").unwrap();
        let companion = root.path().join("second/setup.bin");
        fs::write(&companion, b"data").unwrap();
        let files = vec![
            managed(first, 3, 30, "part-0"),
            managed(second.clone(), 3, 30, "part-0"),
            managed(companion.clone(), 3, 31, "part-1"),
        ];
        let candidates = detect_installer_candidates(
            7,
            &[revision(3, "windows", true, 2)],
            &files,
            &Config::default(),
        );
        let preferred = &candidates.usable[candidates.preferred.unwrap()];
        assert!(preferred.complete);
        assert_eq!(preferred.paths, vec![second, companion]);
        assert!(
            candidates
                .usable
                .iter()
                .any(|candidate| !candidate.complete)
        );
    }

    fn revision(id: i64, os: &str, current: bool, part_count: usize) -> DownloadRevision {
        DownloadRevision {
            revision_id: id,
            slot_id: id,
            product_id: 7,
            provider_group_id: format!("installer_{os}_en"),
            provider_category: DownloadCategory::Installer,
            name: "Game".into(),
            operating_system: Some(os.into()),
            language_code: Some("en".into()),
            language_name: Some("English".into()),
            version: Some("1.0".into()),
            total_size: Some((part_count * 4) as u64),
            manifest_fingerprint: format!("fingerprint-{id}"),
            currently_offered: current,
            first_seen_at: 1,
            last_seen_at: 1,
            retired_at: (!current).then_some(1),
            parts: (0..part_count)
                .map(|index| DownloadPart {
                    part_id: id * 10 + index as i64,
                    revision_id: id,
                    provider_file_id: format!("part-{index}"),
                    part_index: index as u32,
                    expected_size: Some(4),
                    downlink: format!("/part-{index}"),
                    checksum: None,
                    checksum_fetched_at: None,
                })
                .collect(),
        }
    }

    fn managed(path: PathBuf, revision: i64, part: i64, provider: &str) -> ManagedFileRecord {
        ManagedFileRecord {
            path,
            product_id: 7,
            product_slug: "game".into(),
            kind: crate::domain::ArtifactKind::Installer,
            operating_system: Some("windows".into()),
            language: Some("English".into()),
            filename: provider.into(),
            size: 4,
            artifact_path: None,
            matched: true,
            present: true,
            artifact_id: None,
            job_id: None,
            version: Some("1.0".into()),
            expected_size: Some(4),
            gog_checksum: None,
            verified_at: None,
            revision_id: Some(revision),
            part_id: Some(part),
            provider_file_id: Some(provider.into()),
        }
    }

    #[test]
    fn requires_every_multipart_companion() {
        let root =
            std::env::temp_dir().join(format!("gog-install-candidate-{:?}", SystemTime::now()));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("part-0.bin");
        fs::write(&first, b"data").unwrap();
        let revision = revision(3, "windows", true, 2);
        let files = vec![managed(first, 3, 30, "part-0")];

        let detected = detect_installer_candidates(7, &[revision], &files, &Config::default());
        assert!(detected.usable.is_empty());
        assert_eq!(detected.incomplete[0].missing_parts, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offers_incomplete_tracked_installer_when_launcher_is_present() {
        let root = std::env::temp_dir().join(format!(
            "gog-install-incomplete-launcher-{:?}",
            SystemTime::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let launcher = root.join("setup.exe");
        fs::write(&launcher, b"data").unwrap();
        let revision = revision(3, "windows", true, 2);
        let files = vec![managed(launcher, 3, 30, "part-0")];

        let detected = detect_installer_candidates(7, &[revision], &files, &Config::default());
        assert_eq!(detected.usable.len(), 1);
        assert!(!detected.usable[0].complete);
        assert_eq!(detected.incomplete[0].missing_parts, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filesystem_truth_prefers_real_installer_over_stale_json_record() {
        let root = std::env::temp_dir().join(format!(
            "gog-install-duplicate-part-{:?}",
            SystemTime::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let descriptor = root.join("Game.sh");
        let installer = root.join("game_1_0.sh");
        fs::write(
            &descriptor,
            br#"{"downlink":"https://example.invalid/game.sh"}"#,
        )
        .unwrap();
        fs::write(&installer, b"data").unwrap();
        let revision = revision(3, "linux", true, 1);
        let mut stale = managed(descriptor, 3, 30, "part-0");
        stale.present = false;
        stale.operating_system = Some("linux".into());
        let mut real = managed(installer.clone(), 3, 30, "part-0");
        real.present = false;
        real.operating_system = Some("linux".into());

        let detected =
            detect_installer_candidates(7, &[revision], &[stale, real], &Config::default());
        assert_eq!(detected.usable.len(), 1);
        assert_eq!(detected.usable[0].launcher.as_ref(), Some(&installer));
        assert_eq!(detected.usable[0].paths, vec![installer]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeps_complete_historical_installers_but_prefers_current() {
        let root =
            std::env::temp_dir().join(format!("gog-install-history-{:?}", SystemTime::now()));
        fs::create_dir_all(&root).unwrap();
        let old_path = root.join("old.exe");
        let current_path = root.join("current.exe");
        fs::write(&old_path, b"data").unwrap();
        fs::write(&current_path, b"data").unwrap();
        let files = vec![
            managed(old_path, 1, 10, "part-0"),
            managed(current_path, 2, 20, "part-0"),
        ];

        let detected = detect_installer_candidates(
            7,
            &[
                revision(1, "windows", false, 1),
                revision(2, "windows", true, 1),
            ],
            &files,
            &Config::default(),
        );
        assert_eq!(detected.usable.len(), 2);
        assert_eq!(
            detected.usable[detected.preferred.unwrap()].revision_id,
            Some(2)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_preserved_installer_without_revision_identity() {
        let root =
            std::env::temp_dir().join(format!("gog-install-preserved-{:?}", SystemTime::now()));
        fs::create_dir_all(&root).unwrap();
        let launcher = root.join("setup_game_1.0.exe");
        let payload = root.join("setup_game_1.0-1.bin");
        fs::write(&launcher, b"data").unwrap();
        fs::write(&payload, b"data").unwrap();
        let mut files = vec![
            managed(launcher, 1, 10, "installer"),
            managed(payload, 1, 11, "payload"),
        ];
        for file in &mut files {
            file.revision_id = None;
            file.part_id = None;
            file.provider_file_id = None;
            file.expected_size = None;
        }

        let detected = detect_installer_candidates(7, &[], &files, &Config::default());
        assert_eq!(detected.usable.len(), 1);
        assert_eq!(detected.usable[0].revision_id, None);
        assert_eq!(detected.usable[0].paths.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_installed_payload_across_libraries_and_ignores_backups() {
        let root =
            std::env::temp_dir().join(format!("gog-install-resolution-{:?}", SystemTime::now()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(first.join("game/installer/linux/english")).unwrap();
        fs::write(
            first.join("game/installer/linux/english/setup.sh"),
            b"installer",
        )
        .unwrap();
        let libraries = vec![
            GameLibrary {
                id: "first".into(),
                name: "First".into(),
                path: first.clone(),
                default: true,
            },
            GameLibrary {
                id: "second".into(),
                name: "Second".into(),
                path: second.clone(),
                default: false,
            },
        ];
        let mut game = crate::domain::InstalledGame {
            product_id: 7,
            library_id: "first".into(),
            installed_version: Some("1.0".into()),
            installation_directory: first.join("game"),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some("linux".into()),
            installer_language: Some("English".into()),
            compatibility: None,
            primary_executable: None,
            launch_arguments: Vec::new(),
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: Some(1),
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 1,
            updated_at: 1,
        };
        assert_eq!(resolve_installation_directory(&game, &libraries), None);

        fs::create_dir_all(second.join("game/bin")).unwrap();
        let executable = second.join("game/bin/game");
        fs::write(&executable, b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let resolved = resolve_installation_directory(&game, &libraries).unwrap();
        assert_eq!(resolved, ("second".into(), second.join("game")));

        game.primary_executable = Some(first.join("game/bin/game"));
        assert_eq!(
            resolve_installation_directory(&game, &libraries),
            Some(("second".into(), second.join("game")))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_persists_moves_and_preserves_activity_when_install_disappears() {
        let root =
            std::env::temp_dir().join(format!("gog-install-reconcile-{:?}", SystemTime::now()));
        let database = root.join("state.sqlite3");
        let first = root.join("first");
        let second = root.join("second");
        let original = first.join("game");
        let moved = second.join("game");
        fs::create_dir_all(original.join("bin")).unwrap();
        let old_executable = original.join("bin/game");
        fs::write(&old_executable, b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&old_executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let store = crate::state::StateStore::open_at(&database).unwrap();
        let game = crate::domain::InstalledGame {
            product_id: 77,
            library_id: "first".into(),
            installed_version: Some("1.0".into()),
            installation_directory: original.clone(),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some("linux".into()),
            installer_language: Some("English".into()),
            compatibility: None,
            primary_executable: Some(old_executable),
            launch_arguments: Vec::new(),
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: Some(1),
            verified_at: None,
            last_played_at: Some(10),
            playtime_seconds: 120,
            created_at: 1,
            updated_at: 1,
        };
        marker::write(&marker::from_game(&game, Vec::new()), &original).unwrap();
        save_game_preferences(&store, &game).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::rename(&original, &moved).unwrap();
        let libraries = vec![
            GameLibrary {
                id: "first".into(),
                name: "First".into(),
                path: first,
                default: true,
            },
            GameLibrary {
                id: "second".into(),
                name: "Second".into(),
                path: second.clone(),
                default: false,
            },
        ];
        let reconciled = reconcile_installed_games(&store, &libraries).unwrap();
        assert_eq!(reconciled[0].library_id, "second");
        assert_eq!(reconciled[0].installation_directory, moved);
        assert_eq!(
            reconciled[0].primary_executable,
            Some(moved.join("bin/game"))
        );

        // The portable marker is sufficient to rebuild installation state.
        let recovered = reconcile_installed_games(&store, &libraries).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].product_id, 77);
        assert_eq!(recovered[0].installation_directory, moved);

        let known = HashMap::from([(77, recovered[0].clone())]);
        let writer = rusqlite::Connection::open(&database).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let targeted =
            reconcile_installed_products(&store, &libraries, &[(77, "game".into())], &known)
                .unwrap();
        assert_eq!(targeted, recovered);
        writer.execute_batch("ROLLBACK").unwrap();
        #[cfg(unix)]
        if unsafe { libc::geteuid() } != 0 {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::metadata(&moved).unwrap().permissions();
            fs::set_permissions(&moved, fs::Permissions::from_mode(0o111)).unwrap();
            let result =
                reconcile_installed_products(&store, &libraries, &[(77, "game".into())], &known);
            fs::set_permissions(&moved, permissions).unwrap();
            assert!(
                result.is_err(),
                "Unreadable payload must not become an absent game"
            );
            let nested = moved.join("bin");
            let permissions = fs::metadata(&nested).unwrap().permissions();
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o111)).unwrap();
            let result =
                reconcile_installed_products(&store, &libraries, &[(77, "game".into())], &known);
            fs::set_permissions(&nested, permissions).unwrap();
            assert!(
                result.is_err(),
                "Unreadable nested payload must preserve the prior snapshot"
            );
        }

        fs::remove_dir_all(&moved).unwrap();
        assert!(
            reconcile_installed_products(&store, &libraries, &[(77, "game".into())], &known)
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(&second).unwrap();
        // With the library unavailable, no on-disk marker can assert installation.
        assert_eq!(
            reconcile_installed_games(&store, &libraries).unwrap().len(),
            0
        );
        fs::create_dir_all(&second).unwrap();
        // Once the library is accessible, a genuinely absent game is removed.
        assert!(
            reconcile_installed_games(&store, &libraries)
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.product_activity(77).unwrap(), (Some(10), 120));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserved_offline_installer_does_not_require_a_known_launcher_extension() {
        let root = std::env::temp_dir().join(format!(
            "gog-install-preserved-unknown-launcher-{:?}",
            SystemTime::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("offline-installer.run");
        fs::write(&path, b"data").unwrap();
        let mut file = managed(path, 1, 10, "installer");
        file.revision_id = None;
        file.part_id = None;
        file.provider_file_id = None;

        let detected = detect_installer_candidates(7, &[], &[file], &Config::default());
        assert_eq!(detected.usable.len(), 1);
        assert_eq!(detected.usable[0].launcher, None);
        assert_eq!(detected.preferred, Some(0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macos_downloads_are_complete_but_not_installable_on_arch() {
        let root = std::env::temp_dir().join(format!("gog-install-macos-{:?}", SystemTime::now()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("game.pkg");
        fs::write(&path, b"data").unwrap();
        let files = vec![managed(path, 4, 40, "part-0")];

        let detected = detect_installer_candidates(
            7,
            &[revision(4, "macos", true, 1)],
            &files,
            &Config::default(),
        );
        assert_eq!(detected.usable[0].method, InstallationMethod::Unsupported);
        assert_eq!(detected.preferred, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fresh_sources_follow_preference_and_keep_offline_candidate_order() {
        let candidate = |method| InstallerCandidate {
            product_id: 1,
            revision_id: None,
            version: None,
            operating_system: None,
            language: None,
            paths: Vec::new(),
            launcher: None,
            method,
            total_size: 0,
            currently_offered: true,
            complete: true,
        };
        let candidates = [
            candidate(InstallationMethod::WindowsCompatibility),
            candidate(InstallationMethod::WindowsCompatibility),
            candidate(InstallationMethod::NativeLinux),
        ];
        assert_eq!(
            rank_fresh_install_sources(&Config::default(), &candidates, true),
            [
                FreshInstallSource::GalaxyWindows,
                FreshInstallSource::OfflineInstaller(2),
                FreshInstallSource::OfflineInstaller(0),
                FreshInstallSource::OfflineInstaller(1),
            ]
        );
    }
}
