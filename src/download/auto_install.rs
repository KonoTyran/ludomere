use super::{AutoInstallRequest, DownloadRequest};
use crate::{
    config::{GameLibrary, PreferredInstallationSource},
    domain::{ArtifactKind, InstallationState, InstalledGame},
    state::{DownloadInstallIntent, DownloadJobRecord, DownloadState, StateStore},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SelectedInstaller {
    job_id: String,
    product_id: i64,
    title: String,
    operating_system: String,
    language: Option<String>,
    version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstallPlan {
    product_id: i64,
    slug: String,
    title: String,
    library: GameLibrary,
    libraries: Vec<GameLibrary>,
    base: SelectedInstaller,
    dlcs: Vec<SelectedInstaller>,
    interactive_prompts: bool,
}

pub(super) fn intent(
    store: &StateStore,
    requests: &[DownloadRequest],
    choice: &AutoInstallRequest,
) -> Result<Option<DownloadInstallIntent>> {
    let job_ids = requests
        .iter()
        .map(|request| {
            super::job_id_in(
                store,
                &request.artifacts.iter().collect::<Vec<_>>(),
                &request.destination,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let groups = requests
        .iter()
        .zip(&job_ids)
        .filter_map(|(request, job_id)| {
            let first = request.artifacts.first()?;
            if first.kind != ArtifactKind::Installer
                || !request.artifacts.iter().all(|artifact| {
                    artifact.kind == ArtifactKind::Installer
                        && artifact.product_id == first.product_id
                })
            {
                return None;
            }
            let os = first.operating_system.as_deref()?.to_ascii_lowercase();
            if !matches!(os.as_str(), "linux" | "windows") {
                return None;
            }
            Some(SelectedInstaller {
                job_id: job_id.clone(),
                product_id: first.product_id,
                title: request.title.clone(),
                operating_system: os,
                language: first.language.clone(),
                version: first.version.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut bases = groups
        .iter()
        .filter(|group| group.product_id == choice.product_id)
        .collect::<Vec<_>>();
    bases.sort_by_key(|group| {
        let source = if group.operating_system == "linux" {
            PreferredInstallationSource::LinuxOffline
        } else {
            PreferredInstallationSource::WindowsOffline
        };
        let language = group.language.as_deref().unwrap_or("");
        (
            choice
                .config
                .installation_source_order
                .iter()
                .position(|preferred| *preferred == source)
                .unwrap_or(usize::MAX),
            if choice
                .config
                .installer_language
                .as_deref()
                .is_some_and(|preferred| preferred.eq_ignore_ascii_case(language))
            {
                0
            } else if language.eq_ignore_ascii_case("english")
                || language.eq_ignore_ascii_case("en")
            {
                1
            } else {
                2
            },
            group.job_id.clone(),
        )
    });
    let Some(base) = bases.first() else {
        return Ok(None);
    };
    ensure!(
        !choice.slug.is_empty()
            && !choice.slug.contains(['/', '\\'])
            && !matches!(choice.slug.as_str(), "." | ".."),
        "Invalid installation directory name"
    );
    let config = crate::storage::read_config()?;
    let library = crate::storage::validate_library(
        &config,
        crate::config::LibraryKind::GameFiles,
        &choice.library_id,
    )?;
    ensure!(
        choice
            .config
            .game_libraries
            .iter()
            .any(|selected| selected.id == library.id && selected.path == library.path),
        "The chosen Game Files library changed; review the download and installation choices again."
    );
    ensure!(
        library.path.is_absolute(),
        "The default game library must be an absolute path"
    );
    let mut dlcs = Vec::new();
    let mut ids = groups
        .iter()
        .filter(|group| group.product_id != choice.product_id)
        .map(|group| group.product_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    let known_dlcs = if ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        store
            .cached_product_game(choice.product_id)?
            .into_iter()
            .flat_map(|game| game.dlcs)
            .map(|dlc| dlc.product_id)
            .collect()
    };
    for id in ids {
        ensure!(
            known_dlcs.contains(&id),
            "A selected installer is not a known DLC of this game; download it separately or disable automatic installation"
        );
        let selected=groups.iter().filter(|group|group.product_id==id && group.operating_system==base.operating_system && compatible(group.language.as_deref(),base.language.as_deref()) && compatible(group.version.as_deref(),base.version.as_deref())).min_by_key(|group|&group.job_id).context("Selected DLC installers do not match the base installer's platform, language or version; adjust the selection or disable automatic installation")?;
        dlcs.push(selected.clone());
    }
    let plan = InstallPlan {
        product_id: choice.product_id,
        slug: choice.slug.clone(),
        title: choice.title.clone(),
        library,
        libraries: config.game_libraries.clone(),
        base: (*base).clone(),
        dlcs,
        interactive_prompts: choice.config.interactive_installer_prompts,
    };
    for selected in std::iter::once(&plan.base).chain(&plan.dlcs) {
        let request = requests
            .iter()
            .zip(&job_ids)
            .find(|(_, id)| **id == selected.job_id)
            .map(|(request, _)| request)
            .context(
                "The selected installer request is unavailable; review the download choices",
            )?;
        ensure!(
            request.artifacts.iter().all(|artifact| artifact
                .operating_system
                .as_deref()
                .is_some_and(|os| os.eq_ignore_ascii_case(&selected.operating_system))
                && compatible(artifact.language.as_deref(), selected.language.as_deref())
                && compatible(artifact.version.as_deref(), selected.version.as_deref())
                && artifact
                    .part_count
                    .is_none_or(|count| count as usize == request.artifacts.len())),
            "Select every matching installer part before installing automatically"
        );
        let parts = request
            .artifacts
            .iter()
            .filter_map(|artifact| artifact.part_number)
            .collect::<std::collections::HashSet<_>>();
        ensure!(
            parts.is_empty()
                || (parts.len() == request.artifacts.len()
                    && parts
                        .iter()
                        .all(|part| *part > 0 && *part as usize <= request.artifacts.len())),
            "Selected installer parts are duplicated or incomplete"
        );
    }
    Ok(Some(DownloadInstallIntent {
        product_id: choice.product_id,
        intent_id: format!(
            "{}-{}",
            choice.product_id,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ),
        job_ids: std::iter::once(plan.base.job_id.clone())
            .chain(plan.dlcs.iter().map(|dlc| dlc.job_id.clone()))
            .collect(),
        plan_json: serde_json::to_string(&plan)?,
        state: "waiting".into(),
        error: None,
    }))
}

fn compatible(left: Option<&str>, right: Option<&str>) -> bool {
    left.is_none_or(|left| right.is_none_or(|right| left.eq_ignore_ascii_case(right)))
}

fn completed_job(
    store: &StateStore,
    selected: &SelectedInstaller,
) -> Result<Option<DownloadJobRecord>> {
    let job = store.download_job(&selected.job_id)?.context(
        "A selected installer download was removed; add it again to install automatically",
    )?;
    if job.state != DownloadState::Complete {
        return Ok(None);
    }
    ensure!(
        job.product_id == selected.product_id
            && !job.artifacts.is_empty()
            && job.completed_files.len() == job.artifacts.len(),
        "The completed installer is missing required parts; retry the download"
    );
    ensure!(
        job.completed_files.iter().all(|path| path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)),
        "Downloaded installer files are missing; retry the download"
    );
    Ok(Some(job))
}

fn prepare(
    store: &StateStore,
    plan: &InstallPlan,
) -> Result<Option<(InstalledGame, Vec<crate::installation::AdditionalInstaller>)>> {
    let config = crate::storage::read_config()?;
    let library = crate::storage::validate_library(
        &config,
        crate::config::LibraryKind::GameFiles,
        &plan.library.id,
    )?;
    ensure!(
        library.path == plan.library.path,
        "The installation library changed; select Install again."
    );
    let Some(base) = completed_job(store, &plan.base)? else {
        return Ok(None);
    };
    let mut additional = Vec::new();
    for selected in &plan.dlcs {
        let Some(job) = completed_job(store, selected)? else {
            return Ok(None);
        };
        additional.push(crate::installation::AdditionalInstaller {
            product_id: selected.product_id,
            revision_id: None,
            version: selected.version.clone(),
            title: selected.title.clone(),
            files: job.completed_files,
        });
    }
    let directory = plan.library.path.join(&plan.slug);
    crate::installation::validate_offline_sources(
        &config,
        &base
            .completed_files
            .iter()
            .cloned()
            .chain(
                additional
                    .iter()
                    .flat_map(|installer| installer.files.clone()),
            )
            .collect::<Vec<_>>(),
    )?;
    ensure_target_available(store, plan, &directory, &config)?;
    ensure!(
        !plan.interactive_prompts,
        "Automatic installation cannot display installer prompts. Disable interactive installer prompts or install this game manually"
    );
    let expected_extension = if plan.base.operating_system == "windows" {
        "exe"
    } else {
        "sh"
    };
    ensure!(
        base.completed_files.iter().any(|path| path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))),
        "The installer has no supported launcher; install it manually"
    );
    let preferences = store.game_preferences(plan.product_id)?;
    let now = chrono::Utc::now().timestamp();
    let (last_played_at, playtime_seconds) = store.product_activity(plan.product_id)?;
    Ok(Some((
        InstalledGame {
            product_id: plan.product_id,
            library_id: plan.library.id.clone(),
            installed_version: plan.base.version.clone(),
            installation_directory: directory,
            installer_revision_id: None,
            installer_job_id: Some(base.job_id),
            installer_files: base.completed_files,
            installer_complete: true,
            installer_operating_system: Some(plan.base.operating_system.clone()),
            installer_language: plan.base.language.clone(),
            compatibility: if plan.base.operating_system == "windows" {
                preferences
                    .as_ref()
                    .and_then(|preferences| preferences.compatibility.clone())
            } else {
                None
            },
            primary_executable: None,
            launch_arguments: preferences
                .map_or_else(Vec::new, |preferences| preferences.launch_arguments),
            state: InstallationState::Pending,
            error: None,
            installed_at: None,
            verified_at: None,
            last_played_at,
            playtime_seconds,
            created_at: now,
            updated_at: now,
        },
        additional,
    )))
}

fn ensure_target_available(
    store: &StateStore,
    plan: &InstallPlan,
    directory: &std::path::Path,
    config: &crate::config::Config,
) -> Result<()> {
    for library in &config.game_libraries {
        crate::storage::validate_library(
            config,
            crate::config::LibraryKind::GameFiles,
            &library.id,
        )?;
        let entries = match std::fs::read_dir(&library.path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir()
                && crate::installation::load_installation_marker(&path)?
                    .is_some_and(|marker| marker.product_id == plan.product_id)
                && crate::installation::directory_has_installed_payload(&path)
            {
                anyhow::bail!(
                    "This game is already installed. Its payload was preserved; use its Install or Update action instead"
                );
            }
        }
    }
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_dir() && !metadata.is_symlink(),
        "The installation target is not a regular directory; its contents were preserved"
    );
    let files = store
        .download_jobs()?
        .into_iter()
        .flat_map(|job| job.completed_files)
        .filter(|path| path.starts_with(directory))
        .collect::<std::collections::HashSet<_>>();
    let mut pending = vec![(directory.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((parent, depth)) = pending.pop() {
        ensure!(
            depth < 32,
            "The existing installation directory needs manual inspection"
        );
        for entry in std::fs::read_dir(parent)? {
            let entry = entry?;
            visited += 1;
            ensure!(
                visited <= 10000,
                "The existing installation directory needs manual inspection"
            );
            let path = entry.path();
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "The installation directory contains a link; its contents were preserved"
            );
            if kind.is_dir() {
                pending.push((path, depth + 1));
            } else {
                ensure!(
                    kind.is_file() && files.contains(&path),
                    "The installation directory contains existing payload or untracked files. Its contents were preserved; install manually"
                );
            }
        }
    }
    Ok(())
}

/// Called serially by the download manager, never by a GTK callback.
pub(super) fn process(store: &StateStore) -> Result<()> {
    process_with(
        store,
        |game| {
            if game.installer_operating_system.as_deref() == Some("windows") {
                crate::compatibility::preflight_windows(Some(game.product_id))?;
            }
            Ok(())
        },
        |intent, game, additional| {
            crate::installation::enqueue_downloaded_installation(
                store,
                &intent.intent_id,
                game,
                additional,
            )
        },
    )
}

fn process_with(
    store: &StateStore,
    preflight: impl Fn(&InstalledGame) -> Result<()>,
    dispatch: impl Fn(
        &DownloadInstallIntent,
        InstalledGame,
        Vec<crate::installation::AdditionalInstaller>,
    ) -> Result<()>,
) -> Result<()> {
    for intent in store
        .download_install_intents()?
        .into_iter()
        .filter(|intent| intent.state == "waiting")
    {
        let result = (|| -> Result<()> {
            let plan: InstallPlan = serde_json::from_str(&intent.plan_json)?;
            let Some((game, additional)) = prepare(store, &plan)? else {
                return Ok(());
            };
            let _activity =
                crate::profile_reset::begin_activity("automatic installation preparation")?;
            preflight(&game)?;
            if let Some(base) = intent.job_ids.first() {
                store.set_download_job_status(base, None)?;
            }
            dispatch(&intent, game, additional)?;
            Ok(())
        })();
        let message = match result {
            // The dispatcher persists handed_off before starting its worker. It
            // may already have completed by now; never overwrite its terminal
            // state or replace completion feedback with another queued message.
            Ok(()) => continue,
            Err(error) => {
                let message = format!(
                    "Automatic installation needs attention: {error}. Retry installation after resolving this, or install manually."
                );
                store.set_download_install_state(&intent.intent_id, "blocked", Some(&message))?;
                message
            }
        };
        if let Some(base) = intent.job_ids.first() {
            store.set_download_job_status(base, Some(&message))?;
        }
    }
    Ok(())
}

pub(super) fn clear_for_requests(store: &StateStore, requests: &[DownloadRequest]) -> Result<()> {
    for request in requests {
        if let Some(artifact) = request.artifacts.first() {
            store.clear_download_install_intent(artifact.product_id)?;
        }
        if !request.artifacts.is_empty() {
            store.clear_download_install_intent_for_job(&super::job_id_in(
                store,
                &request.artifacts.iter().collect::<Vec<_>>(),
                &request.destination,
            )?)?;
        }
    }
    Ok(())
}

pub(super) fn block_replaced_job(
    store: &StateStore,
    previous: &str,
    replacement: &str,
) -> Result<()> {
    for mut intent in store
        .download_install_intents()?
        .into_iter()
        .filter(|intent| {
            intent.job_ids.iter().any(|id| id == previous) && intent.state != "handed_off"
        })
    {
        // Keep the old installer plan invalid; only transfer the visible queue association.
        for id in &mut intent.job_ids {
            if id == previous {
                *id = replacement.to_owned();
            }
        }
        intent.state = "blocked".into();
        intent.error = Some("The installer revision changed. Select Download again with Install after downloading to confirm the new installer.".into());
        store.save_download_install_intent(&intent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{Dlc, Game, RemoteArtifact},
        state::DownloadJobUpdate,
    };
    use std::{cell::Cell, path::Path, sync::mpsc};

    fn isolated(name: &str) -> bool {
        if std::env::var("LUDOMERE_AUTO_INSTALL_TEST").as_deref() == Ok(name) {
            return true;
        }
        let root = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &format!("download::auto_install::tests::{name}")])
            .env("LUDOMERE_AUTO_INSTALL_TEST", name);
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
        ] {
            let path = root.path().join(key);
            std::fs::create_dir(&path).unwrap();
            command.env(key, path);
        }
        assert!(command.status().unwrap().success());
        false
    }

    fn request(root: &Path, id: i64, os: &str) -> DownloadRequest {
        DownloadRequest {
            library_id: "offline".into(),
            artifacts: vec![RemoteArtifact {
                product_id: id,
                kind: ArtifactKind::Installer,
                name: format!("setup-{id}"),
                language: Some("English".into()),
                operating_system: Some(os.into()),
                version: Some("1".into()),
                release_date: None,
                size_label: None,
                size_bytes: Some(4),
                part_number: Some(1),
                part_count: Some(1),
                download_path: format!("/installer/{id}/{os}"),
                provider_group_id: None,
                provider_file_id: None,
                provider_category: None,
            }],
            title: format!("Game {id}"),
            access_token: String::new(),
            destination: root
                .with_file_name("offline")
                .join("game/installer")
                .join(os)
                .join(id.to_string()),
            events: mpsc::channel().0,
        }
    }

    fn choice(root: &Path) -> AutoInstallRequest {
        let library = GameLibrary {
            id: "test".into(),
            name: "Test".into(),
            path: root.into(),
            default: true,
        };
        let choice = AutoInstallRequest {
            library_id: "test".into(),
            product_id: 7,
            slug: "game".into(),
            title: "Game".into(),
            config: crate::config::Config {
                game_libraries: vec![library],
                offline_libraries: vec![GameLibrary {
                    id: "offline".into(),
                    name: "Offline".into(),
                    path: root.with_file_name("offline"),
                    default: true,
                }],
                installer_library_id: Some("test".into()),
                installation_source_order: vec![
                    PreferredInstallationSource::LinuxOffline,
                    PreferredInstallationSource::WindowsOffline,
                ],
                ..Default::default()
            },
        };
        for kind in crate::config::LibraryKind::ALL {
            for library in choice.config.libraries(kind) {
                std::fs::create_dir_all(&library.path).unwrap();
            }
        }
        std::fs::create_dir_all(crate::config::Config::path().parent().unwrap()).unwrap();
        choice.config.save().unwrap();
        choice
    }

    fn save_job(store: &StateStore, request: &DownloadRequest, complete: bool) {
        std::fs::create_dir_all(&request.destination).unwrap();
        let files = request
            .artifacts
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let path = request.destination.join(format!("setup-{index}.sh"));
                std::fs::write(&path, b"data").unwrap();
                path
            })
            .collect::<Vec<_>>();
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: &super::super::job_id_in(
                    store,
                    &request.artifacts.iter().collect::<Vec<_>>(),
                    &request.destination,
                )
                .unwrap(),
                product_id: request.artifacts[0].product_id,
                title: &request.title,
                artifacts: &request.artifacts,
                destination: &request.destination,
                state: if complete {
                    DownloadState::Complete
                } else {
                    DownloadState::Queued
                },
                bytes_downloaded: if complete { 4 } else { 0 },
                total_bytes: Some(4),
                completed_files: if complete { &files } else { &[] },
                error: None,
            })
            .unwrap();
    }

    #[test]
    #[ignore = "requires isolated HOME and all XDG directories under /tmp/ludomere-p352-"]
    fn native_and_windows_plans_keep_saved_runtime_preferences_separate() {
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
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let choice = choice(&root.path().join("library"));
        let preferences = crate::domain::GamePreferences {
            product_id: 7,
            launch_arguments: vec!["--retained".into()],
            compatibility: Some(crate::compatibility::GameCompatibilityPreferences {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                prefix_slug: "game".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
                pending_profile: Some(crate::compatibility::UmuProfile {
                    game_id: "umu-pending".into(),
                    store: "gog".into(),
                    source: crate::compatibility::UmuProfileSource::GogProductId,
                }),
            }),
            ..Default::default()
        };
        store.upsert_game_preferences(&preferences).unwrap();
        store.preserve_product_activity(7, Some(100), 123).unwrap();
        for os in ["linux", "windows"] {
            let mut request = request(&choice.config.game_libraries[0].path, 7, os);
            request.artifacts[0].part_count = Some(2);
            request.artifacts.push(RemoteArtifact {
                part_number: Some(2),
                download_path: format!("/installer/7/{os}/2"),
                ..request.artifacts[0].clone()
            });
            let record = intent(&store, std::slice::from_ref(&request), &choice)
                .unwrap()
                .unwrap();
            let plan: InstallPlan = serde_json::from_str(&record.plan_json).unwrap();
            save_job(&store, &request, true);
            if os == "windows" {
                let job = store.download_job(&plan.base.job_id).unwrap().unwrap();
                let files = job
                    .completed_files
                    .iter()
                    .map(|path| {
                        let renamed = path.with_extension("exe");
                        std::fs::rename(path, &renamed).unwrap();
                        renamed
                    })
                    .collect::<Vec<_>>();
                store
                    .save_download_job(&DownloadJobUpdate {
                        job_id: &job.job_id,
                        product_id: job.product_id,
                        title: &job.title,
                        artifacts: &job.artifacts,
                        destination: &job.destination,
                        state: DownloadState::Complete,
                        bytes_downloaded: 8,
                        total_bytes: Some(8),
                        completed_files: &files,
                        error: None,
                    })
                    .unwrap();
            }
            let (game, dlc) = prepare(&store, &plan).unwrap().unwrap();
            assert!(dlc.is_empty());
            assert_eq!(game.installer_files.len(), 2);
            assert_eq!(game.installer_operating_system.as_deref(), Some(os));
            assert_eq!(
                game.compatibility,
                if os == "windows" {
                    preferences.compatibility.clone()
                } else {
                    None
                }
            );
            assert_eq!(game.installation_directory, plan.library.path.join("game"));
            assert_eq!(game.launch_arguments, preferences.launch_arguments);
            assert_eq!(game.last_played_at, Some(100));
            assert_eq!(game.playtime_seconds, 123);
            assert_eq!(store.game_preferences(7).unwrap().unwrap(), preferences);
        }
    }

    #[test]
    fn selection_uses_defaults_and_requires_complete_matching_base_and_dlc() {
        if !isolated("selection_uses_defaults_and_requires_complete_matching_base_and_dlc") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let choice = choice(&root.path().join("library"));
        store
            .upsert_normalized_library(&[Game {
                product_id: 7,
                dlcs: (8..=10)
                    .map(|product_id| Dlc {
                        product_id,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }])
            .unwrap();
        let requests = vec![
            request(&choice.config.game_libraries[0].path, 7, "windows"),
            request(&choice.config.game_libraries[0].path, 7, "linux"),
            request(&choice.config.game_libraries[0].path, 8, "linux"),
            request(&choice.config.game_libraries[0].path, 10, "linux"),
            request(&choice.config.game_libraries[0].path, 9, "linux"),
        ];
        let retained = &requests[3];
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: "retained-legacy-dlc-job",
                product_id: 10,
                title: &retained.title,
                artifacts: &retained.artifacts,
                destination: &retained.destination,
                state: DownloadState::Paused,
                bytes_downloaded: 0,
                total_bytes: Some(4),
                completed_files: &[],
                error: None,
            })
            .unwrap();
        let record = intent(&store, &requests, &choice).unwrap().unwrap();
        let plan: InstallPlan = serde_json::from_str(&record.plan_json).unwrap();
        assert_eq!(plan.base.operating_system, "linux");
        assert_eq!(
            plan.dlcs
                .iter()
                .map(|dlc| dlc.product_id)
                .collect::<Vec<_>>(),
            [8, 9, 10]
        );
        assert_eq!(record.job_ids.len(), 4);
        assert_eq!(plan.dlcs[2].job_id, "retained-legacy-dlc-job");
        assert_eq!(record.job_ids[3], "retained-legacy-dlc-job");
        let mut selected = request(&choice.config.game_libraries[0].path, 7, "windows");
        selected.artifacts[0].language = Some("Polish".into());
        selected.artifacts[0].version = Some("selected-version".into());
        let selected = intent(&store, &[selected], &choice).unwrap().unwrap();
        let selected: InstallPlan = serde_json::from_str(&selected.plan_json).unwrap();
        assert_eq!(selected.base.operating_system, "windows");
        assert_eq!(selected.base.language.as_deref(), Some("Polish"));
        assert_eq!(selected.base.version.as_deref(), Some("selected-version"));
        assert_eq!(selected.library.id, choice.library_id);
        assert!(intent(&store, &requests[2..], &choice).unwrap().is_none());
        let mut partial = request(root.path(), 7, "linux");
        partial.artifacts[0].part_count = Some(2);
        assert!(intent(&store, &[partial], &choice).is_err());
        for (numbers, valid) in [
            ([Some(1), Some(2)], true),
            ([Some(2), Some(1)], true),
            ([None, None], true),
            ([Some(2), Some(3)], false),
            ([Some(0), Some(1)], false),
            ([Some(1), Some(1)], false),
            ([Some(1), None], false),
        ] {
            let mut multipart = request(root.path(), 7, "linux");
            multipart.artifacts = numbers
                .into_iter()
                .enumerate()
                .map(|(index, part_number)| RemoteArtifact {
                    part_number,
                    part_count: Some(2),
                    download_path: format!("/installer/7/linux/{index}"),
                    ..multipart.artifacts[0].clone()
                })
                .collect();
            assert_eq!(
                intent(&store, &[multipart], &choice).is_ok(),
                valid,
                "installer part numbers: {numbers:?}"
            );
        }
        let mut extra = request(root.path(), 7, "linux");
        extra.artifacts[0].kind = ArtifactKind::Extra;
        assert!(intent(&store, &[extra], &choice).unwrap().is_none());
        assert!(
            intent(&store, &[request(root.path(), 7, "mac")], &choice)
                .unwrap()
                .is_none()
        );
        assert!(
            intent(
                &store,
                &[
                    request(root.path(), 7, "linux"),
                    request(root.path(), 99, "linux")
                ],
                &choice
            )
            .is_err()
        );
    }

    #[test]
    fn completion_waits_for_all_parts_and_dlc_then_dispatches_once_across_reopen() {
        if !isolated("completion_waits_for_all_parts_and_dlc_then_dispatches_once_across_reopen") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.db");
        let store = StateStore::open_at(&database).unwrap();
        let choice = choice(&root.path().join("library"));
        store
            .upsert_normalized_library(&[Game {
                product_id: 7,
                dlcs: vec![Dlc {
                    product_id: 8,
                    ..Default::default()
                }],
                ..Default::default()
            }])
            .unwrap();
        let mut requests = vec![
            request(&choice.config.game_libraries[0].path, 7, "linux"),
            request(&choice.config.game_libraries[0].path, 8, "linux"),
        ];
        requests[0].artifacts[0].part_count = Some(2);
        let second_part = RemoteArtifact {
            part_number: Some(2),
            download_path: "/installer/7/linux/2".into(),
            ..requests[0].artifacts[0].clone()
        };
        requests[0].artifacts.push(second_part);
        let mut extras = request(&choice.config.game_libraries[0].path, 7, "linux");
        extras.artifacts[0].kind = ArtifactKind::Extra;
        extras.artifacts[0].download_path = "/extras/unavailable".into();
        requests.push(extras);
        let record = intent(&store, &requests, &choice).unwrap().unwrap();
        assert_eq!(
            record.job_ids.len(),
            2,
            "extras must not gate game installation"
        );
        let plan: InstallPlan = serde_json::from_str(&record.plan_json).unwrap();
        store.save_download_install_intent(&record).unwrap();
        save_job(&store, &requests[0], true);
        save_job(&store, &requests[1], true);
        let base = store.download_job(&plan.base.job_id).unwrap().unwrap();
        assert_eq!(base.completed_files.len(), 2);
        for state in [
            DownloadState::Downloading,
            DownloadState::Queued,
            DownloadState::Paused,
            DownloadState::Failed,
            DownloadState::Complete,
        ] {
            store
                .save_download_job(&DownloadJobUpdate {
                    job_id: &base.job_id,
                    product_id: base.product_id,
                    title: &base.title,
                    artifacts: &base.artifacts,
                    destination: &base.destination,
                    state,
                    bytes_downloaded: 4,
                    total_bytes: Some(8),
                    completed_files: &base.completed_files[..1],
                    error: None,
                })
                .unwrap();
            if state == DownloadState::Complete {
                assert!(completed_job(&store, &plan.base).is_err());
            } else {
                process_with(
                    &store,
                    |_| panic!("unfinished multipart installer reached preflight"),
                    |_, _, _| panic!("unfinished multipart installer dispatched"),
                )
                .unwrap();
                assert_eq!(
                    store.download_install_intents().unwrap()[0].state,
                    "waiting"
                );
            }
        }
        save_job(&store, &requests[0], true);
        std::fs::write(&base.completed_files[1], b"").unwrap();
        assert!(completed_job(&store, &plan.base).is_err());
        std::fs::remove_file(&base.completed_files[1]).unwrap();
        assert!(completed_job(&store, &plan.base).is_err());
        save_job(&store, &requests[0], true);
        save_job(&store, &requests[1], false);
        let count = Cell::new(0);
        process_with(
            &store,
            |_| Ok(()),
            |_, _, _| {
                count.set(count.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count.get(), 0);
        save_job(&store, &requests[1], true);
        process_with(
            &store,
            |_| Ok(()),
            |intent, game, dlc| {
                assert_eq!(game.product_id, 7);
                assert_eq!(game.installer_files.len(), 2);
                assert_eq!(dlc.len(), 1);
                assert_eq!(game.library_id, choice.library_id);
                assert_eq!(
                    game.installation_directory,
                    choice.config.game_libraries[0].path.join("game")
                );
                // Match the production dispatcher's durable handoff, then model
                // a fast worker completing before the enqueue call returns.
                store.set_download_install_state(&intent.intent_id, "handed_off", None)?;
                store.complete_download_install_intent(game.product_id, &intent.intent_id)?;
                store.set_download_job_status(&plan.base.job_id, Some("Installation completed"))?;
                count.set(count.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count.get(), 1);
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "complete"
        );
        assert_eq!(
            store
                .download_job(&plan.base.job_id)
                .unwrap()
                .unwrap()
                .status_message
                .as_deref(),
            Some("Installation completed")
        );
        drop(store);
        let store = StateStore::open_at(&database).unwrap();
        process_with(
            &store,
            |_| Ok(()),
            |_, _, _| panic!("replayed completed intent"),
        )
        .unwrap();
        assert!(requests[0].destination.join("setup-0.sh").is_file());
    }

    #[test]
    fn blocked_prerequisite_retries_explicitly_and_unchecked_or_removed_clears_consent() {
        if !isolated(
            "blocked_prerequisite_retries_explicitly_and_unchecked_or_removed_clears_consent",
        ) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let choice = choice(&root.path().join("library"));
        let requests = vec![request(&choice.config.game_libraries[0].path, 7, "linux")];
        save_job(&store, &requests[0], true);
        process_with(
            &store,
            |_| Ok(()),
            |_, _, _| panic!("historical completed download installed"),
        )
        .unwrap();
        let record = intent(&store, &requests, &choice).unwrap().unwrap();
        store.save_download_install_intent(&record).unwrap();
        process_with(
            &store,
            |_| anyhow::bail!("Set up the runtime"),
            |_, _, _| panic!("missing prerequisite dispatched"),
        )
        .unwrap();
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "blocked"
        );
        assert!(
            store.download_jobs().unwrap()[0]
                .status_message
                .as_deref()
                .unwrap()
                .contains("Set up the runtime")
        );
        process_with(
            &store,
            |_| Ok(()),
            |_, _, _| panic!("blocked intent retried without user action"),
        )
        .unwrap();
        store
            .set_download_install_state(&record.intent_id, "waiting", None)
            .unwrap();
        process_with(
            &store,
            |_| Ok(()),
            |intent, _, _| store.set_download_install_state(&intent.intent_id, "handed_off", None),
        )
        .unwrap();
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "handed_off"
        );
        assert!(store.download_jobs().unwrap()[0].status_message.is_none());
        clear_for_requests(&store, &requests).unwrap();
        assert!(store.download_install_intents().unwrap().is_empty());
        store.save_download_install_intent(&record).unwrap();
        block_replaced_job(&store, &record.job_ids[0], "replacement-job").unwrap();
        let blocked = &store.download_install_intents().unwrap()[0];
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.job_ids, ["replacement-job"]);
        assert_eq!(blocked.plan_json, record.plan_json);
        store
            .clear_download_install_intent_for_job("replacement-job")
            .unwrap();
        assert!(store.download_install_intents().unwrap().is_empty());
    }

    #[test]
    fn existing_payload_elsewhere_and_untracked_target_files_are_preserved() {
        if !isolated("existing_payload_elsewhere_and_untracked_target_files_are_preserved") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let mut choice = choice(&root.path().join("library"));
        let requests = vec![request(&choice.config.game_libraries[0].path, 7, "linux")];
        save_job(&store, &requests[0], true);
        let record = intent(&store, &requests, &choice).unwrap().unwrap();
        let plan: InstallPlan = serde_json::from_str(&record.plan_json).unwrap();
        let (mut game, _) = prepare(&store, &plan).unwrap().unwrap();
        let sentinel = plan.library.path.join("game/keep.dat");
        std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        std::fs::write(&sentinel, b"preserve").unwrap();
        assert!(prepare(&store, &plan).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve");
        std::fs::remove_file(&sentinel).unwrap();
        std::fs::remove_dir(sentinel.parent().unwrap()).unwrap();
        let other = root.path().join("other");
        let target = other.join("renamed-game");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("start.sh"), b"never execute").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            target.join("start.sh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        game.installation_directory = target.clone();
        crate::installation::write_installation_marker(
            &crate::installation::installation_marker_from_game(&game, vec![]),
            &target,
        )
        .unwrap();
        choice.config.game_libraries.push(GameLibrary {
            id: "other".into(),
            name: "Other".into(),
            path: other,
            default: false,
        });
        choice.config.save().unwrap();
        let record = intent(&store, &requests, &choice).unwrap().unwrap();
        let plan: InstallPlan = serde_json::from_str(&record.plan_json).unwrap();
        assert!(prepare(&store, &plan).is_err());
        assert!(target.join("start.sh").is_file());
    }
}
