//! Policy-driven acquisition uses fresh metadata and the existing installation queues.
use crate::{
    auth::Token,
    config::Config,
    domain::{
        ArtifactKind, DepotOperationKind, Game, GamePreferences, InstallationSource, InstalledGame,
    },
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
    // Serial per-product discovery bounds both request concurrency and memory use.
    for installed in crate::installation::reconcile_installed_games(&store, &config.game_libraries)?
    {
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
        let marker =
            crate::installation::load_installation_marker(&installed.installation_directory)?;
        if let Some(marker) =
            marker.filter(|marker| marker.source == InstallationSource::GalaxyDepot)
            && policy.auto_update_galaxy
        {
            match queue_galaxy(
                config, game, &installed, &marker, token, session, &client, &store, &policy, false,
            ) {
                Ok(true) => report.galaxy_updates_queued += 1,
                Ok(false) => {}
                Err(error) => report
                    .failures
                    .push((game.product_id, status_error(&error))),
            }
        }
        if policy.auto_download_offline_installer {
            match queue_offline(config, game, &installed, token, session, &store) {
                Ok(true) => report.offline_installers_queued += 1,
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
) -> Result<bool> {
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Installed Galaxy provenance is missing"))?;
    let recovery_generation = crate::installation::recovery::generation(game.product_id);
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

fn queue_offline(
    config: &Config,
    game: &Game,
    installed: &InstalledGame,
    token: &Token,
    session: u64,
    store: &StateStore,
) -> Result<bool> {
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
    let Some(group) = newest_complete_installer(
        &artifacts,
        installed.installer_operating_system.as_deref(),
        config.installer_language.as_deref(),
    ) else {
        return Ok(false);
    };
    if store.download_job(&group.job_id)?.is_some_and(|job| {
        matches!(
            job.state,
            DownloadState::Queued | DownloadState::Downloading
        ) || job.state == DownloadState::Complete
            && !job.completed_files.is_empty()
            && job.completed_files.iter().all(|path| path.is_file())
    }) {
        return Ok(false);
    }
    crate::download::enqueue_backup(
        backup_request(config, game, token, group.artifacts),
        session,
    )?;
    Ok(true)
}

pub(crate) fn backup_request(
    config: &Config,
    game: &Game,
    token: &Token,
    artifacts: Vec<crate::domain::RemoteArtifact>,
) -> crate::download::DownloadRequest {
    let destination = crate::download::destination(
        &config.download_directory,
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
        events,
    }
}

fn newest_complete_installer(
    artifacts: &[crate::domain::RemoteArtifact],
    os: Option<&str>,
    language: Option<&str>,
) -> Option<crate::download_selection::ArtifactGroup> {
    crate::download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| {
            let expected = group
                .artifacts
                .iter()
                .filter_map(|part| part.part_count)
                .max()
                .unwrap_or(1) as usize;
            group.kind == ArtifactKind::Installer
                && expected == group.artifacts.len()
                && (expected == 1
                    || group
                        .artifacts
                        .iter()
                        .filter_map(|part| part.part_number)
                        .collect::<BTreeSet<_>>()
                        == (1..=expected as u32).collect())
                && os.is_none_or(|os| {
                    group
                        .operating_system
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(os))
                })
                && language.is_none_or(|language| {
                    group
                        .language
                        .as_deref()
                        .is_none_or(|value| value.eq_ignore_ascii_case(language))
                })
        })
        .max_by(|a, b| {
            (a.release_sort_key(), a.version.as_deref())
                .cmp(&(b.release_sort_key(), b.version.as_deref()))
        })
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
    fn offline_candidate_requires_complete_matching_installer_and_ignores_extras() {
        let mut artifacts = vec![];
        for part in 1..=2 {
            artifacts.push(crate::domain::RemoteArtifact {
                product_id: 1,
                kind: ArtifactKind::Installer,
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
        assert!(newest_complete_installer(&artifacts[..1], Some("windows"), Some("en")).is_none());
        assert!(newest_complete_installer(&artifacts, Some("linux"), Some("en")).is_none());
        assert!(newest_complete_installer(&artifacts, Some("windows"), Some("fr")).is_none());
        assert_eq!(
            newest_complete_installer(&artifacts, Some("windows"), Some("en"))
                .unwrap()
                .artifacts
                .len(),
            2
        );
        artifacts[1].part_number = Some(1);
        assert!(newest_complete_installer(&artifacts, Some("windows"), Some("en")).is_none());
        artifacts[0].kind = ArtifactKind::Extra;
        assert!(newest_complete_installer(&artifacts, None, None).is_none());
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
