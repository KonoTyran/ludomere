use crate::{
    auth::Token,
    config::Config,
    domain::{ArtifactKind, Game, GamePreferences, InstallationSource},
    state::{DownloadState, StateStore},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

static CHECK_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateCheckReport {
    pub already_running: bool,
    pub galaxy_updates_queued: usize,
    pub offline_installers_queued: usize,
    pub skipped_running: usize,
    pub failures: Vec<(i64, String)>,
}

struct CheckContext<'a> {
    config: &'a Config,
    token: &'a Token,
    store: &'a StateStore,
    client: &'a reqwest::blocking::Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
                .and_then(|value| value.auto_update_galaxy)
                .unwrap_or(config.auto_update_galaxy_installations),
            auto_download_offline_installer: preferences
                .and_then(|value| value.auto_download_offline_installer)
                .unwrap_or(config.auto_download_offline_installers),
            prune_superseded_installers: preferences
                .and_then(|value| value.prune_superseded_installers)
                .unwrap_or(config.prune_superseded_offline_installers),
            galaxy_language: preferences
                .and_then(|value| value.galaxy_language.clone())
                .or_else(|| config.installer_language.clone()),
        }
    }
}

pub fn check_and_queue(
    config: &Config,
    games: &[Game],
    token: &Token,
    mode: CheckMode,
) -> Result<UpdateCheckReport> {
    if CHECK_RUNNING.swap(true, Ordering::AcqRel) {
        return Ok(UpdateCheckReport {
            already_running: true,
            ..UpdateCheckReport::default()
        });
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CHECK_RUNNING.store(false, Ordering::Release);
        }
    }
    let _reset = Reset;
    let store = StateStore::open()?;
    let installed = crate::installation::reconcile_installed_games(&store, &config.game_libraries)?;
    let client = reqwest::blocking::Client::new();
    let context = CheckContext {
        config,
        token,
        store: &store,
        client: &client,
    };
    let mut report = UpdateCheckReport::default();
    for installed_game in installed {
        let Some(game) = games
            .iter()
            .find(|game| game.product_id == installed_game.product_id)
        else {
            continue;
        };
        let preferences = store.game_preferences(game.product_id)?;
        let policy = UpdatePolicy::resolve(config, preferences.as_ref());
        if crate::installation::is_game_running(game.product_id) {
            report.skipped_running += 1;
            continue;
        }
        let result = (|| -> Result<()> {
            let marker = crate::installation::load_installation_marker(
                &installed_game.installation_directory,
            )?
            .ok_or_else(|| anyhow::anyhow!("managed installation marker is missing"))?;
            if marker.source == InstallationSource::GalaxyDepot
                && (mode == CheckMode::Manual || policy.auto_update_galaxy)
                && crate::installation::depot_operation_snapshot_for_product(game.product_id)
                    .is_none()
                && queue_galaxy_update(&context, game, &installed_game, &marker, &policy, false)?
            {
                report.galaxy_updates_queued += 1;
            }
            if (mode == CheckMode::Manual || policy.auto_download_offline_installer)
                && queue_offline_installer(&context, game, &installed_game)
            {
                report.offline_installers_queued += 1;
            }
            Ok(())
        })();
        if let Err(error) = result {
            report
                .failures
                .push((game.product_id, format!("{error:#}")));
        }
    }
    Ok(report)
}

fn queue_galaxy_update(
    context: &CheckContext<'_>,
    game: &Game,
    installed: &crate::domain::InstalledGame,
    marker: &crate::installation::InstallationMarker,
    policy: &UpdatePolicy,
    force_reconcile: bool,
) -> Result<bool> {
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Galaxy installation has no depot provenance"))?;
    let platform = marker
        .base
        .operating_system
        .clone()
        .unwrap_or_else(|| "windows".into());
    let builds = crate::gog::depot_service::list_builds(
        context.store,
        context.client,
        &crate::gog::depot_service::BuildRequest {
            user_id: context.token.user_id.clone(),
            product_id: game.product_id,
            platform,
            generation: 2,
            branch: provenance.branch.clone(),
            supplied_password: None,
        },
    )?;
    let forced_build = force_reconcile.then(|| {
        builds
            .iter()
            .filter(|build| {
                build.generation == 2
                    && build.currently_returned
                    && build.branch == provenance.branch
                    && build.operating_system.eq_ignore_ascii_case(
                        marker.base.operating_system.as_deref().unwrap_or("windows"),
                    )
            })
            .max_by_key(|build| build.published_at)
            .cloned()
    });
    let build = match forced_build.flatten().map(Ok).unwrap_or_else(|| {
        crate::gog::depot_service::resolve_operation_build(
            &builds,
            marker,
            crate::domain::DepotOperationKind::Update,
            None,
        )
        .cloned()
    }) {
        Ok(build) => build,
        Err(error)
            if error.downcast_ref::<crate::gog::depot_service::BuildResolutionError>()
                == Some(&crate::gog::depot_service::BuildResolutionError::NoUpdate) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let selected_dlc = provenance
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    let language = depot_language(game, policy.galaxy_language.as_deref())
        .or_else(|| provenance.language.clone())
        .unwrap_or_else(|| "en".into());
    let library_root = context
        .config
        .game_libraries
        .iter()
        .find(|library| library.id == installed.library_id)
        .map(|library| library.path.clone())
        .ok_or_else(|| anyhow::anyhow!("installed game library is no longer configured"))?;
    crate::gog::depot_service::start_operation(
        context.store,
        context.client,
        crate::gog::depot_service::PrepareOperationRequest {
            build,
            selection: crate::gog::depot_acquisition::Selection {
                language,
                bitness: provenance.architecture.clone(),
                owned_dlc: selected_dlc.clone(),
                selected_dlc,
            },
            operation_id: format!(
                "{}-automatic-update-{}",
                game.product_id,
                chrono::Utc::now().timestamp_millis()
            ),
            kind: crate::domain::DepotOperationKind::Update,
            library_id: installed.library_id.clone(),
            library_root,
            slug: game.slug.clone(),
        },
    )?;
    Ok(true)
}

pub fn queue_language_reconciliation(config: &Config, game: &Game, token: &Token) -> Result<bool> {
    if crate::installation::is_game_running(game.product_id) {
        anyhow::bail!("close the game before changing its installed language");
    }
    if crate::installation::depot_operation_snapshot_for_product(game.product_id).is_some() {
        anyhow::bail!("another Galaxy operation is already queued for this game");
    }
    let store = StateStore::open()?;
    let installed = crate::installation::reconcile_installed_games(&store, &config.game_libraries)?
        .into_iter()
        .find(|installed| installed.product_id == game.product_id)
        .ok_or_else(|| anyhow::anyhow!("the game is not installed"))?;
    let marker = crate::installation::load_installation_marker(&installed.installation_directory)?
        .ok_or_else(|| anyhow::anyhow!("managed installation marker is missing"))?;
    if marker.source != InstallationSource::GalaxyDepot {
        anyhow::bail!("installed language reconciliation requires a Galaxy installation");
    }
    let policy = UpdatePolicy::resolve(config, store.game_preferences(game.product_id)?.as_ref());
    let selected = depot_language(game, policy.galaxy_language.as_deref())
        .or_else(|| policy.galaxy_language.clone())
        .unwrap_or_else(|| "en".into());
    if marker
        .galaxy_depot
        .as_ref()
        .and_then(|provenance| provenance.language.as_ref())
        .is_some_and(|current| current.eq_ignore_ascii_case(&selected))
    {
        return Ok(false);
    }
    let client = reqwest::blocking::Client::new();
    let context = CheckContext {
        config,
        token,
        store: &store,
        client: &client,
    };
    queue_galaxy_update(&context, game, &installed, &marker, &policy, true)
}

fn queue_offline_installer(
    context: &CheckContext<'_>,
    game: &Game,
    installed: &crate::domain::InstalledGame,
) -> bool {
    let preferred_language = context.config.installer_language.as_deref();
    let group = crate::download_selection::group_artifacts(&game.remote_artifacts)
        .into_iter()
        .filter(|group| group.kind == ArtifactKind::Installer && complete_group(group))
        .filter(|group| {
            installed
                .installer_operating_system
                .as_deref()
                .is_none_or(|os| {
                    group
                        .operating_system
                        .as_deref()
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(os))
                })
        })
        .filter(|group| {
            preferred_language.is_none_or(|language| {
                group
                    .language
                    .as_deref()
                    .is_none_or(|candidate| candidate.eq_ignore_ascii_case(language))
            })
        })
        .max_by(|left, right| {
            (
                left.release_sort_key(),
                left.version.as_deref().unwrap_or_default(),
            )
                .cmp(&(
                    right.release_sort_key(),
                    right.version.as_deref().unwrap_or_default(),
                ))
        });
    let Some(group) = group else {
        return false;
    };
    if context
        .store
        .download_job(&group.job_id)
        .ok()
        .flatten()
        .is_some_and(|job| {
            matches!(
                job.state,
                DownloadState::Queued | DownloadState::Downloading | DownloadState::Complete
            )
        })
    {
        return false;
    }
    let artifacts = group.artifacts;
    let refs = artifacts.iter().collect::<Vec<_>>();
    let destination =
        crate::download::destination(&context.config.download_directory, &game.slug, None, &refs);
    let (events, _) = mpsc::channel();
    crate::download::enqueue(crate::download::DownloadRequest {
        artifacts,
        title: game.title.clone(),
        access_token: context.token.access_token.clone(),
        destination,
        events,
    });
    true
}

fn complete_group(group: &crate::download_selection::ArtifactGroup) -> bool {
    let expected = group
        .artifacts
        .iter()
        .filter_map(|artifact| artifact.part_count)
        .max()
        .unwrap_or(1);
    expected as usize == group.artifacts.len()
        && (expected == 1
            || group
                .artifacts
                .iter()
                .filter_map(|artifact| artifact.part_number)
                .collect::<BTreeSet<_>>()
                == (1..=expected).collect())
}

fn depot_language(game: &Game, preferred: Option<&str>) -> Option<String> {
    preferred.and_then(|preferred| {
        game.metadata
            .localizations
            .iter()
            .find(|localization| {
                localization.language_code.eq_ignore_ascii_case(preferred)
                    || localization.name.eq_ignore_ascii_case(preferred)
            })
            .map(|localization| localization.language_code.clone())
    })
}

pub fn verify_and_prune_offline_installer(
    product_id: i64,
    job_id: &str,
    artifacts: &[crate::domain::RemoteArtifact],
    files: &[std::path::PathBuf],
    access_token: &str,
) -> Result<usize> {
    let config = Config::load_or_create()?;
    let store = StateStore::open()?;
    let preferences = store.game_preferences(product_id)?;
    if !UpdatePolicy::resolve(&config, preferences.as_ref()).prune_superseded_installers {
        return Ok(0);
    }
    if artifacts.len() != files.len()
        || artifacts
            .iter()
            .any(|artifact| artifact.kind != ArtifactKind::Installer)
    {
        return Ok(0);
    }
    for (artifact, file) in artifacts.iter().zip(files) {
        let checksum = crate::download::gog_checksum(artifact, access_token)?;
        if file.metadata()?.len() != checksum.size {
            anyhow::bail!("downloaded installer size does not match GOG metadata");
        }
        let actual = crate::download::file_md5_with_progress(file, |_, _| {})?;
        if !actual.eq_ignore_ascii_case(&checksum.md5) {
            anyhow::bail!("downloaded installer checksum does not match GOG metadata");
        }
        store.mark_managed_file_verified(file, artifact, &checksum.md5)?;
    }
    let Some(revision_id) = store.verified_installer_revision_for_job(job_id)? else {
        anyhow::bail!("replacement installer revision is not completely verified");
    };
    let superseded = store.superseded_installer_files(product_id, revision_id)?;
    use gtk::gio::prelude::FileExt;
    let mut trashed = 0;
    for path in superseded {
        gtk::gio::File::for_path(&path).trash(gtk::gio::Cancellable::NONE)?;
        store.mark_managed_file_absent(&path)?;
        trashed += 1;
    }
    Ok(trashed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_game_choices_override_global_defaults() {
        let config = Config {
            installer_language: Some("English".into()),
            ..Config::default()
        };
        let preferences = GamePreferences {
            product_id: 1,
            executable_path: None,
            launch_arguments: Vec::new(),
            compatibility: None,
            auto_update_galaxy: Some(false),
            auto_download_offline_installer: Some(true),
            prune_superseded_installers: None,
            galaxy_language: Some("de-DE".into()),
            created_at: 0,
            updated_at: 0,
        };

        let policy = UpdatePolicy::resolve(&config, Some(&preferences));
        assert!(!policy.auto_update_galaxy);
        assert!(policy.auto_download_offline_installer);
        assert!(!policy.prune_superseded_installers);
        assert_eq!(policy.galaxy_language.as_deref(), Some("de-DE"));
    }
}
