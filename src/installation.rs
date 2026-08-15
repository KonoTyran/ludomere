use crate::{
    config::{Config, GameLibrary},
    domain::{ArtifactKind, DownloadCategory, DownloadRevision},
    state::ManagedFileRecord,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

mod executor;
mod launcher;
mod manager;
mod marker;
mod patch;
mod windows_executable;
pub use executor::{
    AdditionalInstaller, InstallationEvent, InstallationHandle, UninstallationEvent,
    UninstallationHandle, installation_log_path, start_installation, start_uninstallation,
    uninstallation_log_path,
};
pub use launcher::{
    LaunchEvent, is_game_running, is_game_stopping, launch_game, runtime_log_path, stop_all_games,
    stop_game,
};
pub use manager::{
    InstallationManagerEvent, InstallationOperationSnapshot, cancel_operation,
    enqueue_installation, enqueue_uninstallation, installation_operation_snapshot,
    recover_interrupted_operations, respond_to_installation, shutdown, start_recovered_operations,
    subscribe_installation_events,
};
pub use marker::{
    InstallationMarker, InstalledDlc, from_game as installation_marker_from_game,
    load as load_installation_marker, write as write_installation_marker,
};
pub use patch::{PatchEvent, patch_target_version, run_patch};
pub use windows_executable::{
    WindowsExecutableCandidate, WindowsExecutableDiscovery, discover_windows_executable,
};

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
                    discovered.insert(marker.product_id, (library.id.clone(), directory, marker));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(path = %directory.display(), %error, "could not read installation marker")
                }
            }
        }
    }
    let mut reconciled = Vec::with_capacity(discovered.len());
    for (_, (library_id, directory, found_marker)) in discovered {
        if !directory_has_installed_payload(&directory) {
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
            game.compatibility = preferences.compatibility.clone();
        }
        let (last_played, playtime) = store.product_activity(game.product_id)?;
        game.last_played_at = last_played;
        game.playtime_seconds = playtime;
        if preferences
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
        compatibility: game.compatibility.clone(),
        created_at: game.created_at,
        updated_at: game.updated_at,
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

fn directory_has_installed_payload(directory: &std::path::Path) -> bool {
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

        fs::remove_dir_all(&moved).unwrap();
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
}
