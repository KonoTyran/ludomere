//! Policy-driven acquisition uses fresh metadata and the existing installation queues.
use crate::{
    auth::Token,
    config::{Config, GameLibrary, LibraryKind},
    domain::{DepotOperationKind, Game, GamePreferences, InstallationSource, InstalledGame},
    state::{DownloadState, StateStore},
};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

static CHECK_RUNNING: AtomicBool = AtomicBool::new(false);

pub enum ManualUpdateCheck {
    UpToDate,
    Available(Box<ManualUpdateOffer>),
}

#[derive(Clone)]
pub struct ManualUpdateOffer {
    pub source: InstallationSource,
    pub version: Option<String>,
    pub download_required: bool,
    game: Game,
    installed: InstalledGame,
    marker: crate::installation::InstallationMarker,
    target: ManualUpdateTarget,
    session: u64,
    recovery_generation: u64,
}

#[derive(Clone)]
enum ManualUpdateTarget {
    Depot(crate::domain::GalaxyBuild),
    Offline(Vec<crate::domain::RemoteArtifact>),
}

#[derive(Debug)]
pub enum ManualUpdateQueued {
    Depot,
    OfflineDownload,
    OfflineReady,
}

/// Worker-only discovery. It neither queues work nor prepares/runs installers.
pub fn check_installed_update(
    game: &Game,
    installed: &InstalledGame,
    token: &Token,
    session: u64,
) -> Result<ManualUpdateCheck> {
    (|| {
        let _activity = crate::profile_reset::begin_activity("manual update discovery")?;
        let store = StateStore::open()?;
        let config = crate::storage::read_config()?;
        let generation = crate::installation::recovery::generation(game.product_id);
        let marker =
            validate_manual_installation(&config, &store, game, installed, session, generation)?;
        let target = match marker.source {
            InstallationSource::GalaxyDepot => {
                let builds = manual_builds(&store, game, &marker, token)?;
                match crate::gog::depot_service::resolve_operation_build(
                    &builds,
                    &marker,
                    DepotOperationKind::Update,
                    None,
                ) {
                    Ok(build) => Some(ManualUpdateTarget::Depot(build.clone())),
                    Err(error)
                        if error
                            .downcast_ref::<crate::gog::depot_service::BuildResolutionError>()
                            == Some(&crate::gog::depot_service::BuildResolutionError::NoUpdate) =>
                    {
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
            InstallationSource::OfflineInstaller => manual_offline_artifacts(
                &marker,
                game,
                &store.load_all_download_revisions(game.product_id)?,
                &crate::online::fetch_product_file_metadata(
                    &token.access_token,
                    game.product_id,
                    &[],
                )?
                .into_iter()
                .find(|(id, _, _)| *id == game.product_id)
                .map(|(_, artifacts, _)| artifacts)
                .unwrap_or_default(),
            )?
            .map(ManualUpdateTarget::Offline),
        };
        ensure!(
            validate_manual_installation(
                &crate::storage::read_config()?,
                &store,
                game,
                installed,
                session,
                generation
            )? == marker,
            "The installation changed; check for updates again"
        );
        let Some(target) = target else {
            return Ok(ManualUpdateCheck::UpToDate);
        };
        let (version, download_required) = match &target {
            ManualUpdateTarget::Depot(build) => (build.version.clone(), true),
            ManualUpdateTarget::Offline(artifacts) => (
                artifacts[0].version.clone(),
                !manual_installer_cached(&config, &store, artifacts)?,
            ),
        };
        Ok(ManualUpdateCheck::Available(Box::new(ManualUpdateOffer {
            source: marker.source,
            version,
            download_required,
            game: game.clone(),
            installed: installed.clone(),
            marker,
            target,
            session,
            recovery_generation: generation,
        })))
    })()
    .map_err(|error| anyhow::anyhow!(status_error(&error)))
}

/// The offer is rechecked after explicit consent; it never substitutes another revision.
pub fn confirm_installed_update(
    offer: ManualUpdateOffer,
    token: &Token,
    session: u64,
    offline_library: Option<GameLibrary>,
) -> Result<ManualUpdateQueued> {
    (|| {
        let _activity = crate::profile_reset::begin_activity("manual update confirmation")?;
        ensure!(
            session == offer.session,
            "Account changed; check for updates again"
        );
        let config = crate::storage::read_config()?;
        let store = StateStore::open()?;
        ensure!(
            validate_manual_installation(
                &config,
                &store,
                &offer.game,
                &offer.installed,
                session,
                offer.recovery_generation
            )? == offer.marker,
            "The installation changed; check for updates again"
        );
        match &offer.target {
            ManualUpdateTarget::Depot(build) => {
                let client = reqwest::blocking::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(Duration::from_secs(45))
                    .user_agent(crate::identity::USER_AGENT)
                    .build()?;
                ensure!(
                    queue_galaxy(
                        &config,
                        &offer.game,
                        &offer.installed,
                        &offer.marker,
                        token,
                        session,
                        &client,
                        &store,
                        &UpdatePolicy::resolve(
                            &config,
                            store.game_preferences(offer.game.product_id)?.as_ref()
                        ),
                        false,
                        Some((build, offer.recovery_generation))
                    )?,
                    "The offered update changed; check for updates again"
                );
                Ok(ManualUpdateQueued::Depot)
            }
            ManualUpdateTarget::Offline(artifacts) => {
                let current = crate::online::fetch_product_file_metadata(
                    &token.access_token,
                    offer.game.product_id,
                    &[],
                )?
                .into_iter()
                .find(|(id, _, _)| *id == offer.game.product_id)
                .map(|(_, artifacts, _)| artifacts)
                .unwrap_or_default();
                ensure!(
                    manual_offline_artifacts(
                        &offer.marker,
                        &offer.game,
                        &store.load_all_download_revisions(offer.game.product_id)?,
                        &current
                    )?
                    .as_ref()
                        == Some(artifacts),
                    "The offered installer changed; check for updates again"
                );
                // Root edits and recovery cleanup also hold this existing gate. Keep the
                // final identity check and registration together, without network under it.
                let _gate = crate::operation_gate::try_acquire().map_err(|_| {
                    anyhow::anyhow!(
                        "Finish or pause active file operations, then confirm the update again"
                    )
                })?;
                let config = crate::storage::read_config()?;
                ensure!(
                    validate_manual_installation(
                        &config,
                        &store,
                        &offer.game,
                        &offer.installed,
                        session,
                        offer.recovery_generation
                    )? == offer.marker,
                    "The installation changed; check for updates again"
                );
                if manual_installer_cached(&config, &store, artifacts)? {
                    crate::online::with_account_session(session, || {
                        store.observe_download_manifest(offer.game.product_id, &current)?;
                        store.cache_download_manifest(offer.game.product_id, &current)
                    })?;
                    return Ok(ManualUpdateQueued::OfflineReady);
                }
                let chosen = offline_library.ok_or_else(|| {
                    anyhow::anyhow!("Choose an Offline Installers library for this update")
                })?;
                let library = crate::storage::validate_library(
                    &config,
                    LibraryKind::OfflineInstallers,
                    &chosen.id,
                )?;
                ensure!(
                    library.path == chosen.path,
                    "The selected installer library changed; choose it again"
                );
                crate::online::with_account_session(session, || {
                    store.observe_download_manifest(offer.game.product_id, &current)?;
                    store.cache_download_manifest(offer.game.product_id, &current)
                })?;
                crate::download::enqueue_with_install(
                    vec![backup_request(
                        &library,
                        &offer.game,
                        token,
                        artifacts.clone(),
                    )],
                    None,
                    session,
                )?;
                Ok(ManualUpdateQueued::OfflineDownload)
            }
        }
    })()
    .map_err(|error| anyhow::anyhow!(status_error(&error)))
}

fn validate_manual_installation(
    config: &Config,
    store: &StateStore,
    game: &Game,
    installed: &InstalledGame,
    session: u64,
    generation: u64,
) -> Result<crate::installation::InstallationMarker> {
    crate::online::with_account_session(session, || Ok(()))?;
    ensure!(
        crate::installation::recovery::current(game.product_id, generation),
        "Recovery changed this game; check for updates again"
    );
    ensure!(
        game.product_id == installed.product_id,
        "The installation does not match this game"
    );
    crate::installation::validate_game_library(
        config,
        &installed.library_id,
        &installed.installation_directory,
    )?;
    ensure!(
        !crate::installation::is_game_running(game.product_id) && !busy(store, game.product_id)?,
        "Close the game and finish its active operations before checking for updates"
    );
    ensure!(
        !crate::installation::recovery::pending(
            &installed.installation_directory,
            game.product_id
        )?,
        "Finish this game's interrupted recovery before updating"
    );
    let marker = crate::installation::load_installation_marker(&installed.installation_directory)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "The installation has no source marker; repair or reinstall it before updating"
            )
        })?;
    ensure!(
        marker.product_id == game.product_id
            && marker.slug == game.slug
            && installed
                .installation_directory
                .file_name()
                .is_some_and(|name| name == game.slug.as_str())
            && crate::installation::directory_has_installed_payload(
                &installed.installation_directory
            ),
        "The installation identity or payload changed; check for updates again"
    );
    Ok(marker)
}

fn manual_builds(
    store: &StateStore,
    game: &Game,
    marker: &crate::installation::InstallationMarker,
    token: &Token,
) -> Result<Vec<crate::domain::GalaxyBuild>> {
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Installed Depot provenance is missing"))?;
    let password = provenance
        .branch
        .as_deref()
        .map(|branch| {
            crate::branch_credentials::load(store, &token.user_id, game.product_id, branch)
        })
        .transpose()?
        .flatten();
    crate::gog::builds::fetch_authenticated_generation(
        &reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .user_agent(crate::identity::USER_AGENT)
            .build()?,
        &token.access_token,
        password.as_deref(),
        game.product_id,
        marker.base.operating_system.as_deref().unwrap_or("windows"),
        2,
    )
}

fn manual_offline_artifacts(
    marker: &crate::installation::InstallationMarker,
    game: &Game,
    revisions: &[crate::domain::DownloadRevision],
    artifacts: &[crate::domain::RemoteArtifact],
) -> Result<Option<Vec<crate::domain::RemoteArtifact>>> {
    let os = marker.base.operating_system.as_deref().ok_or_else(|| {
        anyhow::anyhow!("The installed platform is unknown; choose its offline installer manually")
    })?;
    let language = |value: &str| {
        game.metadata
            .localizations
            .iter()
            .find(|entry| {
                entry.language_code.eq_ignore_ascii_case(value)
                    || entry.name.eq_ignore_ascii_case(value)
            })
            .map_or_else(
                || value.to_ascii_lowercase(),
                |entry| entry.language_code.to_ascii_lowercase(),
            )
    };
    let mut groups = crate::download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| {
            group.product_id == game.product_id
                && group.kind == crate::domain::ArtifactKind::Installer
                && group.artifacts.iter().all(|part| {
                    part.provider_category
                        .is_none_or(|kind| kind == crate::domain::DownloadCategory::Installer)
                })
                && group
                    .operating_system
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(os))
                && group.language.as_deref().map(&language)
                    == marker.base.language.as_deref().map(&language)
        })
        .collect::<Vec<_>>();
    ensure!(
        groups.len() == 1,
        "The current installer for the installed platform and language is missing or ambiguous; choose an offline installer manually"
    );
    let group = groups.remove(0);
    let expected = group
        .artifacts
        .iter()
        .filter_map(|part| part.part_count)
        .max()
        .unwrap_or(1) as usize;
    ensure!(
        expected == group.artifacts.len()
            && (expected == 1
                || group
                    .artifacts
                    .iter()
                    .filter_map(|part| part.part_number)
                    .collect::<BTreeSet<_>>()
                    == (1..=expected as u32).collect()),
        "The current installer has an incomplete multipart listing; retry discovery"
    );
    let installed_version = marker.base.version.as_deref().ok_or_else(|| {
        anyhow::anyhow!("The installed version is unknown; choose its offline installer manually")
    })?;
    let version = group.version.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "The current installer version is unknown; choose an offline installer manually"
        )
    })?;
    let changed_parts = marker
        .base
        .revision_id
        .and_then(|id| revisions.iter().find(|revision| revision.revision_id == id))
        .is_some_and(|revision| {
            revision.parts.len() != group.artifacts.len()
                || group.artifacts.iter().any(|part| {
                    part.provider_group_id
                        .as_ref()
                        .is_some_and(|id| id != &revision.provider_group_id)
                        || !revision.parts.iter().any(|installed| {
                            installed.downlink == part.download_path
                                && installed.expected_size == part.size_bytes
                                && part
                                    .provider_file_id
                                    .as_ref()
                                    .is_none_or(|id| id == &installed.provider_file_id)
                        })
                })
        });
    Ok((version != installed_version || changed_parts).then_some(group.artifacts))
}

fn manual_installer_cached(
    config: &Config,
    store: &StateStore,
    artifacts: &[crate::domain::RemoteArtifact],
) -> Result<bool> {
    let managed = store.managed_files_for_products(&[artifacts[0].product_id])?;
    for library in config.libraries(LibraryKind::OfflineInstallers) {
        if crate::storage::validate_library(config, LibraryKind::OfflineInstallers, &library.id)
            .is_err()
        {
            continue;
        }
        for directory in managed
            .iter()
            .filter(|file| file.path.starts_with(&library.path))
            .filter_map(|file| file.path.parent())
            .collect::<BTreeSet<_>>()
        {
            if artifacts.iter().all(|part| {
                managed.iter().any(|file| {
                    file.present
                        && file.matched
                        && file.path.parent() == Some(directory)
                        && file.version == part.version
                        && part
                            .provider_file_id
                            .as_ref()
                            .is_none_or(|id| file.provider_file_id.as_ref() == Some(id))
                        && file.artifact_path.as_deref() == Some(part.download_path.as_str())
                        && crate::storage::validate_path(
                            config,
                            LibraryKind::OfflineInstallers,
                            &file.path,
                        )
                        .is_ok()
                        && std::fs::metadata(&file.path).is_ok_and(|metadata| {
                            metadata.is_file()
                                && part.size_bytes.is_none_or(|size| metadata.len() == size)
                        })
                })
            }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CheckMode {
    Automatic,
    Manual,
}

#[derive(Default)]
pub struct UpdateCheckReport {
    pub already_running: bool,
    pub galaxy_updates_queued: usize,
    pub offline_installers_queued: usize,
    pub extras_queued: usize,
    pub skipped_running: usize,
    pub skipped_busy: usize,
    pub failures: Vec<(i64, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UpdatePolicy {
    pub auto_update_galaxy: bool,
    pub auto_download_offline_installer: bool,
    pub prune_superseded_installers: bool,
    pub galaxy_language: Option<String>,
}

impl UpdatePolicy {
    pub fn resolve(config: &Config, preferences: Option<&GamePreferences>) -> Self {
        Self {
            auto_update_galaxy: preferences
                .and_then(|p| p.auto_update_galaxy)
                .unwrap_or(config.auto_update_galaxy_installations),
            auto_download_offline_installer: preferences
                .and_then(|p| p.auto_download_offline_installer)
                .unwrap_or(config.auto_download_offline_installers),
            prune_superseded_installers: preferences
                .and_then(|p| p.prune_superseded_installers)
                .unwrap_or(config.prune_superseded_offline_installers),
            galaxy_language: preferences
                .and_then(|p| p.galaxy_language.clone())
                .or_else(|| config.installer_language.clone()),
        }
    }
}

fn busy(store: &StateStore, id: i64) -> Result<bool> {
    Ok(
        crate::installation::depot_operation_snapshot_for_product(id).is_some_and(|s| {
            !matches!(
                s.state.as_str(),
                "complete" | "failed" | "cancelled" | "abandoned"
            )
        }) || crate::installation::installation_operation_snapshot(id).is_some_and(|s| {
            s.queued
                || matches!(
                    s.state,
                    crate::domain::InstallationState::Installing
                        | crate::domain::InstallationState::Uninstalling
                )
        }) || store
            .download_install_intents()?
            .iter()
            .any(|intent| intent.product_id == id && intent.state != "complete")
            || store.download_jobs()?.iter().any(|job| {
                job.product_id == id
                    && matches!(
                        job.state,
                        DownloadState::Queued | DownloadState::Downloading
                    )
            }),
    )
}

pub fn check_and_queue(
    config: &Config,
    games: &[Game],
    token: &Token,
    _mode: CheckMode,
    session: u64,
) -> Result<UpdateCheckReport> {
    if CHECK_RUNNING.swap(true, Ordering::AcqRel) {
        return Ok(UpdateCheckReport {
            already_running: true,
            ..Default::default()
        });
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CHECK_RUNNING.store(false, Ordering::Release);
        }
    }
    let _reset = Reset;
    let _activity = crate::profile_reset::begin_activity("game update discovery")?;
    crate::online::with_account_session(session, || Ok(()))?;
    let store = StateStore::open()?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    let mut report = UpdateCheckReport::default();
    let statuses = crate::storage::inspect_libraries(config)?;
    let libraries = config
        .game_libraries
        .iter()
        .filter(|library| {
            statuses.iter().any(|status| {
                status.kind == LibraryKind::GameFiles
                    && status.library_id == library.id
                    && status.compatibility == crate::storage::LibraryCompatibility::Compatible
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    for status in statuses
        .iter()
        .filter(|status| status.kind == LibraryKind::GameFiles)
    {
        if let crate::storage::LibraryCompatibility::Incompatible(reason)
        | crate::storage::LibraryCompatibility::Unavailable(reason) = &status.compatibility
        {
            report
                .failures
                .push((0, format!("{}: {reason}", status.path.display())));
        }
    }
    // Serial per-product discovery bounds both request concurrency and memory use.
    let installed = match crate::installation::reconcile_installed_games(&store, &libraries) {
        Ok(installed) => installed,
        Err(error) => {
            report.failures.push((0, status_error(&error)));
            Vec::new()
        }
    };
    for installed in installed {
        crate::online::with_account_session(session, || Ok(()))?;
        let Some(game) = games
            .iter()
            .find(|game| game.product_id == installed.product_id)
        else {
            continue;
        };
        if crate::installation::is_game_running(game.product_id) {
            report.skipped_running += 1;
            continue;
        }
        if busy(&store, game.product_id)? {
            report.skipped_busy += 1;
            continue;
        }
        let policy =
            UpdatePolicy::resolve(config, store.game_preferences(game.product_id)?.as_ref());
        let marker = match crate::installation::load_installation_marker(
            &installed.installation_directory,
        ) {
            Ok(marker) => marker,
            Err(error) => {
                report
                    .failures
                    .push((game.product_id, status_error(&error)));
                continue;
            }
        };
        if let Some(marker) =
            marker.filter(|marker| marker.source == InstallationSource::GalaxyDepot)
            && policy.auto_update_galaxy
        {
            match queue_galaxy(
                config, game, &installed, &marker, token, session, &client, &store, &policy, false,
                None,
            ) {
                Ok(true) => report.galaxy_updates_queued += 1,
                Ok(false) => {}
                Err(error) => report
                    .failures
                    .push((game.product_id, status_error(&error))),
            }
        }
        if policy.prune_superseded_installers
            && let Err(error) = crate::download::check_installer_retention(
                game.product_id,
                &token.access_token,
                session,
            )
        {
            report
                .failures
                .push((game.product_id, status_error(&error)));
        }
    }
    let managed = store.managed_files()?;
    for product_id in managed
        .iter()
        .filter(|file| file.present && file.matched)
        .map(|file| file.product_id)
        .collect::<BTreeSet<_>>()
    {
        crate::online::with_account_session(session, || Ok(()))?;
        if crate::installation::is_game_running(product_id) {
            report.skipped_running += 1;
            continue;
        }
        if busy(&store, product_id)? {
            report.skipped_busy += 1;
            continue;
        }
        let game = games
            .iter()
            .find(|game| game.product_id == product_id)
            .cloned()
            .or(store.cached_product_game(product_id)?);
        let Some(game) = game else { continue };
        if let Err(error) = queue_archives(config, &game, token, session, &store, &mut report) {
            report.failures.push((product_id, status_error(&error)));
        }
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn queue_galaxy(
    config: &Config,
    game: &Game,
    installed: &InstalledGame,
    marker: &crate::installation::InstallationMarker,
    token: &Token,
    session: u64,
    client: &reqwest::blocking::Client,
    store: &StateStore,
    policy: &UpdatePolicy,
    language_change: bool,
    offered: Option<(&crate::domain::GalaxyBuild, u64)>,
) -> Result<bool> {
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Installed Galaxy provenance is missing"))?;
    let recovery_generation = offered.map_or_else(
        || crate::installation::recovery::generation(game.product_id),
        |(_, generation)| generation,
    );
    let platform = marker.base.operating_system.as_deref().unwrap_or("windows");
    ensure!(
        platform.eq_ignore_ascii_case("windows") || platform.eq_ignore_ascii_case("linux"),
        "Automatic Depot updates require a supported Windows or Linux installation"
    );
    let password = provenance
        .branch
        .as_deref()
        .map(|branch| {
            crate::branch_credentials::load(store, &token.user_id, game.product_id, branch)
        })
        .transpose()?
        .flatten();
    let builds = crate::gog::builds::fetch_authenticated_generation(
        client,
        &token.access_token,
        password.as_deref(),
        game.product_id,
        platform,
        2,
    )?;
    let build = if language_change {
        builds
            .iter()
            .filter(|build| {
                build.generation == 2
                    && build.branch == provenance.branch
                    && build.operating_system.eq_ignore_ascii_case(platform)
            })
            .max_by_key(|build| build.published_at)
            .ok_or_else(|| {
                anyhow::anyhow!("No supported build is available on the installed branch")
            })?
    } else {
        match crate::gog::depot_service::resolve_operation_build(
            &builds,
            marker,
            DepotOperationKind::Update,
            None,
        ) {
            Ok(build) => build,
            Err(error)
                if error.downcast_ref::<crate::gog::depot_service::BuildResolutionError>()
                    == Some(&crate::gog::depot_service::BuildResolutionError::NoUpdate) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
    };
    ensure!(
        build.generation == 2,
        "Only generation-two Depot updates are supported"
    );
    if let Some((offered, _)) = offered {
        ensure!(
            same_offered_build(build, offered),
            "The offered Depot build changed; check for updates again"
        );
    }
    let selected_dlc = provenance
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    let owned_dlc = game
        .dlcs
        .iter()
        .filter(|dlc| dlc.owned)
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    ensure!(
        selected_dlc.is_subset(&owned_dlc),
        "Refresh your library to confirm the installed DLC ownership before updating"
    );
    let desired_language = if language_change {
        policy.galaxy_language.clone()
    } else {
        store
            .game_preferences(game.product_id)?
            .and_then(|preferences| preferences.galaxy_language)
    };
    let selection = crate::gog::depot_acquisition::Selection {
        language: desired_language
            .as_deref()
            .and_then(|language| {
                game.metadata
                    .localizations
                    .iter()
                    .find(|entry| {
                        entry.language_code.eq_ignore_ascii_case(language)
                            || entry.name.eq_ignore_ascii_case(language)
                    })
                    .map(|entry| entry.language_code.clone())
            })
            .or(desired_language)
            .or_else(|| provenance.language.clone())
            .unwrap_or_else(|| "en".into()),
        bitness: provenance.architecture.clone(),
        owned_dlc,
        selected_dlc,
    };
    if language_change && provenance.language.as_deref() == Some(selection.language.as_str()) {
        return Ok(false);
    }
    let acquisition =
        crate::gog::depot_acquisition::acquire(client, &token.access_token, build, &selection)?;
    validate_language(
        &acquisition.repository.depots,
        game.product_id,
        &selection.language,
    )?;
    let library = config
        .game_libraries
        .iter()
        .find(|library| library.id == installed.library_id)
        .ok_or_else(|| anyhow::anyhow!("The installation library is no longer configured"))?;
    if platform.eq_ignore_ascii_case("windows") {
        crate::compatibility::preflight_windows(Some(game.product_id))?;
    }
    crate::online::with_account_session(session, || Ok(()))?;
    let request = crate::installation::depot_planner::prepare(
        crate::installation::depot_planner::PrepareDepotRequest {
            account_session: session,
            recovery_generation,
            store,
            acquisition: &acquisition,
            build,
            selection: &selection,
            operation_id: format!(
                "{}-policy-{}",
                game.product_id,
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ),
            kind: DepotOperationKind::Update,
            library_id: library.id.clone(),
            library_root: library.path.clone(),
            slug: game.slug.clone(),
            access_token: token.access_token.clone(),
        },
    )?;
    let _manual_gate = offered
        .map(|_| {
            crate::operation_gate::try_acquire().map_err(|_| {
                anyhow::anyhow!(
                    "Finish or pause active file operations, then confirm the update again"
                )
            })
        })
        .transpose()?;
    if offered.is_some() {
        ensure!(
            validate_manual_installation(
                &crate::storage::read_config()?,
                store,
                game,
                installed,
                session,
                recovery_generation
            )? == *marker,
            "The installation changed; check for updates again"
        );
    }
    crate::online::with_account_session(session, || {
        ensure!(
            !crate::installation::is_game_running(game.product_id)
                && !busy(store, game.product_id)?,
            "The game became busy; retry after it finishes"
        );
        store.observe_galaxy_builds(game.product_id, platform, &builds)?;
        ensure!(
            crate::installation::enqueue_depot_operation(request),
            "The update could not be queued; finish active work and retry"
        );
        Ok(true)
    })
}

fn same_offered_build(
    current: &crate::domain::GalaxyBuild,
    offered: &crate::domain::GalaxyBuild,
) -> bool {
    current.product_id == offered.product_id
        && current.generation == offered.generation
        && current.build_id == offered.build_id
        && current.repository_id == offered.repository_id
        && current.repository_url == offered.repository_url
        && current.branch == offered.branch
        && current.operating_system == offered.operating_system
        && current.version == offered.version
}

fn queue_archives(
    config: &Config,
    game: &Game,
    token: &Token,
    session: u64,
    store: &StateStore,
    report: &mut UpdateCheckReport,
) -> Result<()> {
    let policy = UpdatePolicy::resolve(config, store.game_preferences(game.product_id)?.as_ref());
    let managed = store.managed_files_for_products(&[game.product_id])?;
    let mut libraries = Vec::new();
    for kind in [LibraryKind::OfflineInstallers, LibraryKind::Extras] {
        if !match kind {
            LibraryKind::OfflineInstallers => policy.auto_download_offline_installer,
            LibraryKind::Extras => config.auto_download_extras,
            LibraryKind::GameFiles => false,
        } {
            continue;
        }
        for library in config.libraries(kind) {
            if managed
                .iter()
                .any(|file| file.present && file.matched && file.path.starts_with(&library.path))
            {
                let library = match crate::storage::validate_library(config, kind, &library.id) {
                    Ok(library) => library,
                    Err(error) => {
                        report.failures.push((
                            game.product_id,
                            format!("{}: {}", library.path.display(), status_error(&error)),
                        ));
                        continue;
                    }
                };
                if managed.iter().any(|file| {
                    file.present
                        && file.matched
                        && file.path.starts_with(&library.path)
                        && file.path.is_file()
                }) {
                    libraries.push((kind, library));
                }
            }
        }
    }
    if libraries.is_empty() {
        return Ok(());
    }
    let manifests =
        crate::online::fetch_product_file_metadata(&token.access_token, game.product_id, &[])?;
    let artifacts = manifests
        .into_iter()
        .find(|(id, _, _)| *id == game.product_id)
        .map(|(_, artifacts, _)| artifacts)
        .unwrap_or_default();
    crate::online::with_account_session(session, || {
        store.observe_download_manifest(game.product_id, &artifacts)?;
        store.cache_download_manifest(game.product_id, &artifacts)?;
        Ok(())
    })?;
    let revisions = store.load_all_download_revisions(game.product_id)?;
    for (kind, library) in libraries {
        for (group, destination) in
            archive_updates(&artifacts, &revisions, &managed, &library.path, kind)
        {
            let queued = (|| -> Result<bool> {
                let current = crate::storage::read_config()?;
                let current_library =
                    crate::storage::validate_library(&current, kind, &library.id)?;
                ensure!(
                    current_library.path == library.path,
                    "The archive library changed; retry update discovery"
                );
                let policy = UpdatePolicy::resolve(
                    &current,
                    store.game_preferences(game.product_id)?.as_ref(),
                );
                if !(match kind {
                    LibraryKind::OfflineInstallers => policy.auto_download_offline_installer,
                    LibraryKind::Extras => current.auto_download_extras,
                    LibraryKind::GameFiles => false,
                }) {
                    return Ok(false);
                }
                // Recheck the existing copy after metadata fetch; never turn an update into a fresh backup.
                if !archive_updates(
                    &artifacts,
                    &revisions,
                    &store.managed_files_for_products(&[game.product_id])?,
                    &library.path,
                    kind,
                )
                .iter()
                .any(|(candidate, path)| candidate.job_id == group.job_id && *path == destination)
                {
                    return Ok(false);
                }
                let id = crate::download::resolve_job_id(
                    &group.artifacts.iter().collect::<Vec<_>>(),
                    &destination,
                )?;
                if store.download_job(&id)?.is_some_and(|job| {
                    matches!(
                        job.state,
                        DownloadState::Queued | DownloadState::Downloading
                    ) || job.state == DownloadState::Complete
                        && !job.completed_files.is_empty()
                        && job.completed_files.iter().all(|path| path.is_file())
                }) {
                    return Ok(false);
                }
                let mut request = backup_request(&library, game, token, group.artifacts);
                request.destination = destination;
                crate::download::enqueue_backup(request, session)?;
                Ok(true)
            })();
            match queued {
                Ok(true) => match kind {
                    LibraryKind::OfflineInstallers => report.offline_installers_queued += 1,
                    LibraryKind::Extras => report.extras_queued += 1,
                    LibraryKind::GameFiles => unreachable!(),
                },
                Ok(false) => {}
                Err(error) => report
                    .failures
                    .push((game.product_id, status_error(&error))),
            }
        }
    }
    Ok(())
}

pub(crate) fn backup_request(
    library: &GameLibrary,
    game: &Game,
    token: &Token,
    artifacts: Vec<crate::domain::RemoteArtifact>,
) -> crate::download::DownloadRequest {
    let destination = crate::download::destination(
        &library.path,
        &game.slug,
        None,
        &artifacts.iter().collect::<Vec<_>>(),
    );
    let (events, _) = mpsc::channel();
    crate::download::DownloadRequest {
        artifacts,
        title: game.title.clone(),
        access_token: token.access_token.clone(),
        destination,
        library_id: library.id.clone(),
        events,
    }
}

fn archive_updates(
    artifacts: &[crate::domain::RemoteArtifact],
    revisions: &[crate::domain::DownloadRevision],
    managed: &[crate::state::ManagedFileRecord],
    root: &std::path::Path,
    kind: LibraryKind,
) -> Vec<(crate::download_selection::ArtifactGroup, std::path::PathBuf)> {
    crate::download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| {
            let expected = group
                .artifacts
                .iter()
                .filter_map(|part| part.part_count)
                .max()
                .unwrap_or(1) as usize;
            crate::storage::artifact_library_kind(&group.artifacts[0]) == kind
                && expected == group.artifacts.len()
                && (expected == 1
                    || group
                        .artifacts
                        .iter()
                        .filter_map(|part| part.part_number)
                        .collect::<BTreeSet<_>>()
                        == (1..=expected as u32).collect())
        })
        .flat_map(|group| {
            let Some(current) = revisions.iter().find(|revision| {
                revision.currently_offered
                    && revision.parts.len() == group.artifacts.len()
                    && group.artifacts.iter().all(|artifact| {
                        revision.parts.iter().any(|part| {
                            part.downlink == artifact.download_path
                                && artifact.provider_file_id.as_deref()
                                    == Some(part.provider_file_id.as_str())
                        })
                    })
            }) else {
                return Vec::new();
            };
            managed
                .iter()
                .filter(|file| {
                    file.present
                        && file.matched
                        && file.path.starts_with(root)
                        && file.path.is_file()
                        && file.operating_system == group.operating_system
                        && file.language == group.language
                        && file.revision_id.is_some_and(|id| {
                            revisions.iter().any(|revision| {
                                revision.revision_id == id
                                    && revision.slot_id == current.slot_id
                                    && revision.revision_id != current.revision_id
                            })
                        })
                })
                .filter_map(|file| file.path.parent().map(std::path::Path::to_path_buf))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|destination| {
                    !current.parts.iter().all(|part| {
                        managed.iter().any(|file| {
                            file.present
                                && file.matched
                                && file.part_id == Some(part.part_id)
                                && file.path.parent() == Some(destination.as_path())
                                && file.path.is_file()
                        })
                    })
                })
                .map(|destination| (group.clone(), destination))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn queue_language_reconciliation_inner(
    config: &Config,
    game: &Game,
    token: &Token,
    session: u64,
) -> Result<bool> {
    let _activity = crate::profile_reset::begin_activity("installed language change")?;
    crate::online::with_account_session(session, || Ok(()))?;
    let store = StateStore::open()?;
    ensure!(
        !crate::installation::is_game_running(game.product_id) && !busy(&store, game.product_id)?,
        "Finish active game operations before changing language"
    );
    let installed = crate::installation::reconcile_installed_games(&store, &config.game_libraries)?
        .into_iter()
        .find(|installed| installed.product_id == game.product_id)
        .ok_or_else(|| anyhow::anyhow!("The game is not installed"))?;
    let marker = crate::installation::load_installation_marker(&installed.installation_directory)?
        .ok_or_else(|| anyhow::anyhow!("The installation marker is missing"))?;
    ensure!(
        marker.source == InstallationSource::GalaxyDepot,
        "Installed language changes require a Depot installation"
    );
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    queue_galaxy(
        config,
        game,
        &installed,
        &marker,
        token,
        session,
        &client,
        &store,
        &UpdatePolicy::resolve(config, store.game_preferences(game.product_id)?.as_ref()),
        true,
        None,
    )
}

pub fn queue_language_reconciliation(
    config: &Config,
    game: &Game,
    token: &Token,
    session: u64,
) -> Result<bool> {
    queue_language_reconciliation_inner(config, game, token, session)
        .map_err(|error| anyhow::anyhow!(status_error(&error)))
}

pub(crate) fn status_error(error: &anyhow::Error) -> String {
    if error.chain().any(|cause| cause.is::<reqwest::Error>()) {
        return crate::online::sync_error_message(error)
            .replace("Retry synchronization", "Retry this operation")
            .replace("retry synchronization", "retry this operation");
    }
    let message = error.to_string();
    if message.contains("://") {
        "Could not prepare this operation. Check setup and retry.".into()
    } else {
        message.chars().take(512).collect()
    }
}

fn validate_language(
    depots: &[crate::gog::types::RepositoryDepot],
    product_id: i64,
    selected: &str,
) -> Result<()> {
    let languages = depots
        .iter()
        .filter(|depot| depot.product_id.parse::<i64>() == Ok(product_id))
        .flat_map(|depot| &depot.languages)
        .filter(|language| language.as_str() != "*")
        .collect::<Vec<_>>();
    ensure!(
        languages.is_empty()
            || languages
                .iter()
                .any(|language| crate::gog::depot_acquisition::language_matches(
                    language, selected
                )),
        "The selected language is not supported by this build; choose another installed language"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual_fixture() -> (
        Game,
        crate::installation::InstallationMarker,
        Vec<crate::domain::RemoteArtifact>,
    ) {
        let game = Game {
            product_id: 914_080,
            slug: "manual-game".into(),
            ..Default::default()
        };
        let marker = serde_json::from_value(serde_json::json!({
            "schema_version":1,"product_id":game.product_id,"slug":game.slug,
            "base":{"operating_system":"linux","language":"en","version":"1","installed_at":1}
        }))
        .unwrap();
        let parts = (1..=2)
            .map(|part| crate::domain::RemoteArtifact {
                product_id: game.product_id,
                kind: crate::domain::ArtifactKind::Installer,
                name: "Game installer".into(),
                language: Some("en".into()),
                operating_system: Some("linux".into()),
                version: Some("1".into()),
                release_date: None,
                size_label: None,
                size_bytes: Some(3),
                part_number: Some(part),
                part_count: Some(2),
                download_path: format!("/installer/{part}"),
                provider_group_id: Some("linux-en".into()),
                provider_file_id: Some(format!("file-{part}")),
                provider_category: Some(crate::domain::DownloadCategory::Installer),
            })
            .collect();
        (game, marker, parts)
    }

    #[test]
    fn manual_offline_discovery_detects_same_version_revisions_without_queuing() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let (game, mut marker, mut parts) = manual_fixture();
        store
            .observe_download_manifest(game.product_id, &parts)
            .unwrap();
        let revisions = store.load_all_download_revisions(game.product_id).unwrap();
        marker.base.revision_id = Some(revisions[0].revision_id);
        assert!(
            manual_offline_artifacts(&marker, &game, &revisions, &parts)
                .unwrap()
                .is_none()
        );
        parts[0].provider_file_id = Some("replacement-same-version-and-path".into());
        assert_eq!(
            manual_offline_artifacts(&marker, &game, &revisions, &parts).unwrap(),
            Some(parts.clone())
        );
        let mut french = parts.clone();
        for part in &mut french {
            part.language = Some("fr".into());
            part.provider_group_id = Some("linux-fr".into());
        }
        let mut all = parts.clone();
        all.extend(french);
        assert_eq!(
            manual_offline_artifacts(&marker, &game, &revisions, &all).unwrap(),
            Some(parts.clone())
        );
        assert!(manual_offline_artifacts(&marker, &game, &revisions, &parts[..1]).is_err());
        marker.base.operating_system = Some("windows".into());
        assert!(manual_offline_artifacts(&marker, &game, &revisions, &parts).is_err());
        assert!(store.download_jobs().unwrap().is_empty());
        assert!(store.download_install_intents().unwrap().is_empty());
        assert_eq!(
            store.load_all_download_revisions(game.product_id).unwrap(),
            revisions
        );
    }

    #[test]
    fn manual_cached_installer_requires_exact_parts_in_one_compatible_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let (game, _, parts) = manual_fixture();
        let mut config = Config {
            game_libraries: Vec::new(),
            offline_libraries: Vec::new(),
            ..Default::default()
        };
        for id in ["first", "second"] {
            let library = GameLibrary {
                id: id.into(),
                name: id.into(),
                path: root.path().join(id),
                default: id == "first",
            };
            std::fs::create_dir_all(&library.path).unwrap();
            config.offline_libraries.push(library);
        }
        store
            .observe_download_manifest(game.product_id, &parts)
            .unwrap();
        let mut paths = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let directory = crate::download::destination(
                &config.offline_libraries[index].path,
                &game.slug,
                None,
                &parts.iter().collect::<Vec<_>>(),
            );
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("part{index}.sh"));
            std::fs::write(&path, b"one").unwrap();
            store
                .record_completed_artifacts(
                    "fixture",
                    &game.slug,
                    std::slice::from_ref(part),
                    std::slice::from_ref(&path),
                )
                .unwrap();
            paths.push(path);
        }
        assert!(
            !manual_installer_cached(&config, &store, &parts).unwrap(),
            "must not combine libraries"
        );
        let second = paths[0].parent().unwrap().join("part1.sh");
        std::fs::write(&second, b"two").unwrap();
        store
            .record_completed_artifacts(
                "fixture",
                &game.slug,
                &parts[1..],
                std::slice::from_ref(&second),
            )
            .unwrap();
        assert!(manual_installer_cached(&config, &store, &parts).unwrap());
        let mut replacement = parts.clone();
        replacement[0].provider_file_id = Some("new-id-same-path-size-version".into());
        assert!(!manual_installer_cached(&config, &store, &replacement).unwrap());
        store
            .observe_download_manifest(game.product_id, &replacement)
            .unwrap();
        store
            .record_completed_artifacts("new-fixture", &game.slug, &replacement[..1], &paths[..1])
            .unwrap();
        assert!(manual_installer_cached(&config, &store, &replacement).unwrap());
        std::fs::write(&second, b"wrong-size").unwrap();
        assert!(!manual_installer_cached(&config, &store, &replacement).unwrap());
    }

    #[test]
    fn manual_confirmation_rejects_stale_session_source_and_recovery_without_transfer() {
        const CHILD: &str = "LUDOMERE_MANUAL_CONFIRM_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "updates::tests::manual_confirmation_rejects_stale_session_source_and_recovery_without_transfer", "--nocapture"]).env(CHILD, "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
                "TMPDIR",
            ] {
                let path = root.path().join(key);
                std::fs::create_dir(&path).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let (game, marker, parts) = manual_fixture();
        let library = GameLibrary {
            id: "games".into(),
            name: "Games".into(),
            path: root.path().join("games"),
            default: true,
        };
        let directory = library.path.join(&game.slug);
        std::fs::create_dir_all(directory.join(".ludomere")).unwrap();
        std::fs::write(
            directory.join("start.sh"),
            b"inert game payload, never executed",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            directory.join("start.sh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let marker_path = directory.join(".ludomere/installation.json");
        std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        let config = Config {
            game_libraries: vec![library],
            ..Default::default()
        };
        std::fs::create_dir_all(Config::path().parent().unwrap()).unwrap();
        config.save().unwrap();
        let store = StateStore::open().unwrap();
        let installed: InstalledGame = serde_json::from_value(serde_json::json!({
            "product_id":game.product_id,"library_id":"games","installation_directory":directory,
            "installer_files":[],"installer_complete":true,"launch_arguments":[],"state":"installed",
            "playtime_seconds":0,"created_at":1,"updated_at":1
        })).unwrap();
        let session = crate::online::account_session();
        let generation = crate::installation::recovery::generation(game.product_id);
        assert_eq!(
            validate_manual_installation(&config, &store, &game, &installed, session, generation)
                .unwrap(),
            marker
        );
        let offer = ManualUpdateOffer {
            source: marker.source,
            version: Some("2".into()),
            download_required: true,
            game: game.clone(),
            installed,
            marker: marker.clone(),
            target: ManualUpdateTarget::Offline(parts),
            session,
            recovery_generation: generation,
        };
        let token = Token {
            access_token: "inert-never-sent".into(),
            refresh_token: "inert".into(),
            user_id: "fixture".into(),
            expires_at: 0,
        };
        assert!(
            confirm_installed_update(offer.clone(), &token, session.wrapping_add(1), None)
                .unwrap_err()
                .to_string()
                .contains("Account changed")
        );
        let mut changed = marker.clone();
        changed.source = InstallationSource::GalaxyDepot;
        changed.galaxy_depot = Some(
            serde_json::from_value(serde_json::json!({
                "build_id":"changed","repository_id":"fixture","manifest_fingerprint":"fixture"
            }))
            .unwrap(),
        );
        std::fs::write(&marker_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            confirm_installed_update(offer.clone(), &token, session, None)
                .unwrap_err()
                .to_string()
                .contains("installation changed")
        );
        std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        let reservation =
            crate::installation::recovery::Reservation::reserve(&[game.product_id]).unwrap();
        assert!(
            confirm_installed_update(offer, &token, session, None)
                .unwrap_err()
                .to_string()
                .contains("Recovery changed")
        );
        drop(reservation);
        assert!(store.download_jobs().unwrap().is_empty());
        assert!(store.download_install_intents().unwrap().is_empty());
        assert_eq!(
            std::fs::read(directory.join("start.sh")).unwrap(),
            b"inert game payload, never executed"
        );
    }

    #[test]
    fn manual_depot_offer_identity_ignores_observation_time_but_not_target_changes() {
        let build: crate::domain::GalaxyBuild = serde_json::from_value(serde_json::json!({
            "build_id":"one","product_id":1,"operating_system":"windows","version":"1","branch":"beta",
            "tags":[],"public":false,"generation":2,"repository_url":"https://example.invalid/one","repository_id":"repo1",
            "published_at":1,"currently_returned":true,"first_seen_at":1,"last_seen_at":2
        })).unwrap();
        let mut current = build.clone();
        current.last_seen_at += 10;
        assert!(same_offered_build(&current, &build));
        for field in [
            "build_id",
            "repository_id",
            "repository_url",
            "branch",
            "version",
            "operating_system",
        ] {
            let mut value = serde_json::to_value(&build).unwrap();
            value[field] = serde_json::json!("changed");
            assert!(
                !same_offered_build(&serde_json::from_value(value).unwrap(), &build),
                "{field}"
            );
        }
    }

    #[test]
    fn effective_policies_keep_opt_ins_independent_and_off_overrides_default() {
        let config = Config::default();
        let inherited = UpdatePolicy::resolve(&config, None);
        assert!(inherited.auto_update_galaxy);
        assert!(
            !inherited.auto_download_offline_installer && !inherited.prune_superseded_installers
        );
        let preferences = GamePreferences {
            auto_update_galaxy: Some(false),
            auto_download_offline_installer: Some(true),
            galaxy_language: Some("fr".into()),
            ..Default::default()
        };
        let effective = UpdatePolicy::resolve(&config, Some(&preferences));
        assert!(
            !effective.auto_update_galaxy
                && effective.auto_download_offline_installer
                && !effective.prune_superseded_installers
        );
        assert_eq!(effective.galaxy_language.as_deref(), Some("fr"));
    }

    #[test]
    fn unsupported_build_language_is_rejected_even_with_universal_base_files() {
        let depots: Vec<crate::gog::types::RepositoryDepot> =
            serde_json::from_value(serde_json::json!([
                {"manifest":"common","productId":"7","languages":["*"],"size":0},
                {"manifest":"en","productId":"7","languages":["en-US"],"size":1}
            ]))
            .unwrap();
        assert!(validate_language(&depots, 7, "en").is_ok());
        assert!(
            validate_language(&depots, 7, "fr")
                .unwrap_err()
                .to_string()
                .contains("not supported")
        );
        assert!(validate_language(&depots[..1], 7, "fr").is_ok());
    }

    #[test]
    fn archive_updates_preserve_existing_root_and_slot_without_requiring_installation() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let library = root.path().join("offline");
        let other_library = root.path().join("other-offline");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(&other_library).unwrap();
        let mut artifacts = vec![];
        for part in 1..=2 {
            artifacts.push(crate::domain::RemoteArtifact {
                product_id: 1,
                kind: crate::domain::ArtifactKind::Installer,
                name: "Game".into(),
                language: Some("en".into()),
                operating_system: Some("windows".into()),
                version: Some("1".into()),
                release_date: None,
                size_label: None,
                size_bytes: Some(13),
                part_number: Some(part),
                part_count: Some(2),
                download_path: format!("/part{part}"),
                provider_group_id: Some("base".into()),
                provider_file_id: Some(part.to_string()),
                provider_category: Some(crate::domain::DownloadCategory::Installer),
            });
        }
        store.observe_download_manifest(1, &artifacts).unwrap();
        let paths = [library.join("part1"), library.join("part2")];
        for path in &paths {
            std::fs::write(path, b"inert fixture").unwrap();
        }
        store
            .record_completed_artifacts("old", "game", &artifacts, &paths)
            .unwrap();
        for artifact in &mut artifacts {
            artifact.version = Some("2".into());
            artifact.download_path.push_str("-new");
        }
        let mut other_language = artifacts.clone();
        for artifact in &mut other_language {
            artifact.language = Some("fr".into());
            artifact.provider_group_id = Some("french".into());
        }
        let mut offered = artifacts.clone();
        offered.extend(other_language);
        store.observe_download_manifest(1, &offered).unwrap();
        let revisions = store.load_all_download_revisions(1).unwrap();
        let managed = store.managed_files().unwrap();
        let candidates = archive_updates(
            &offered,
            &revisions,
            &managed,
            &library,
            LibraryKind::OfflineInstallers,
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0.artifacts, artifacts);
        assert_eq!(candidates[0].1, library);
        assert!(
            archive_updates(
                &offered,
                &revisions,
                &managed,
                &other_library,
                LibraryKind::OfflineInstallers
            )
            .is_empty()
        );
        assert!(
            archive_updates(
                &offered,
                &revisions,
                &managed,
                &library,
                LibraryKind::Extras
            )
            .is_empty()
        );
        assert!(
            archive_updates(
                &artifacts[..1],
                &revisions,
                &managed,
                &library,
                LibraryKind::OfflineInstallers
            )
            .is_empty()
        );
        artifacts[1].part_number = Some(1);
        assert!(
            archive_updates(
                &artifacts,
                &revisions,
                &managed,
                &library,
                LibraryKind::OfflineInstallers
            )
            .is_empty()
        );
        for path in &paths {
            std::fs::remove_file(path).unwrap();
        }
        assert!(
            archive_updates(
                &offered,
                &revisions,
                &managed,
                &library,
                LibraryKind::OfflineInstallers
            )
            .is_empty()
        );
    }

    #[test]
    fn stale_scheduled_check_is_rejected_before_state_or_network_access() {
        let token = Token {
            access_token: "inert".into(),
            refresh_token: "inert".into(),
            user_id: "fixture".into(),
            expires_at: 0,
        };
        let error = check_and_queue(
            &Config::default(),
            &[],
            &token,
            CheckMode::Manual,
            crate::online::account_session().wrapping_sub(1),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("account changed"));
        assert!(!CHECK_RUNNING.load(Ordering::Acquire));
    }
}
