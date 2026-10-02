//! GOG cloud saves for managed Windows/UMU installations.

pub mod api;
pub mod backup;
pub mod metadata;
pub mod paths;
pub mod sync;

use crate::domain::{
    CloudSaveAvailability, CloudSaveDiscovery, CloudSaveLocation, CloudSyncMode, CloudSyncResult,
    InstalledGame,
};
use anyhow::{Context, Result, bail};
use api::Storage;

static OPERATIONS: std::sync::Mutex<Vec<i64>> = std::sync::Mutex::new(Vec::new());

fn ensure_session(session: u64) -> Result<()> {
    anyhow::ensure!(
        crate::auth::session_is_current(session),
        "GOG session changed; sign in again before synchronizing saves"
    );
    Ok(())
}

struct OperationGuard(i64);

impl Drop for OperationGuard {
    fn drop(&mut self) {
        OPERATIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|id| *id != self.0);
    }
}

fn begin_operation(product_id: i64) -> Result<OperationGuard> {
    let mut operations = OPERATIONS.lock().unwrap_or_else(|error| error.into_inner());
    if operations.contains(&product_id) {
        bail!("another cloud-save operation is running for this game; wait and retry");
    }
    operations.push(product_id);
    Ok(OperationGuard(product_id))
}

fn authenticated_storage(game: &InstalledGame, account_id: &str) -> Result<api::CloudClient> {
    let session = crate::auth::session();
    ensure_session(session)?;
    if game.compatibility.is_none()
        || !game
            .installer_operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        bail!("cloud saves require an installed managed Windows game");
    }
    let marker = crate::installation::load_installation_marker(&game.installation_directory)?
        .context("the game is no longer installed")?;
    if marker.product_id != game.product_id {
        bail!("the installed game changed; reopen its settings");
    }
    let store = crate::state::StateStore::open()?;
    if store.cloud_save_record(game.product_id)?.availability != CloudSaveAvailability::Supported {
        bail!("GOG cloud saves are not supported for this game");
    }
    let token = crate::auth::load_saved_token()?.context("sign in to GOG to manage cloud saves")?;
    if token.user_id != account_id {
        bail!("the GOG account changed; reopen cloud-save management");
    }
    let builds = windows_builds(&store, game.product_id)?;
    let exact = marker.galaxy_depot.map(|depot| depot.build_id);
    let build =
        metadata::select_build(&builds, exact.as_deref(), game.installed_version.as_deref())
            .context("no generation-2 Windows build is available")?;
    let client = api::client()?;
    let credentials = metadata::fetch_credentials(&client, &build.repository_url)?;
    ensure_session(session)?;
    let scoped = api::exchange_scoped_token(&client, &token.refresh_token, &credentials)?;
    ensure_session(session)?;
    Ok(
        api::CloudClient::new(client, token.user_id, credentials.client_id, scoped)
            .for_session(session),
    )
}

#[derive(Debug, Clone)]
pub struct CloudSyncRequest {
    pub game: InstalledGame,
    pub locations: Vec<CloudSaveLocation>,
    pub mode: CloudSyncMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudSaveInventory {
    pub file_count: usize,
    pub total_size: u64,
    pub latest_modified_at: Option<i64>,
}

pub fn discover(
    game: &InstalledGame,
    stored_locations: &[CloudSaveLocation],
) -> Result<CloudSaveDiscovery> {
    let checked_at = chrono::Utc::now().timestamp();
    let overrides = stored_locations
        .iter()
        .filter(|location| location.user_override)
        .cloned()
        .collect::<Vec<_>>();
    let unavailable = |reason: &str| CloudSaveDiscovery {
        availability: CloudSaveAvailability::Unavailable,
        locations: overrides.clone(),
        checked_at,
        reason: Some(reason.into()),
        ..Default::default()
    };
    if game.compatibility.is_none()
        || !game
            .installer_operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        return Ok(unavailable(
            "cloud saves require a managed Windows installation",
        ));
    }
    let store = crate::state::StateStore::open()?;
    let builds = windows_builds(&store, game.product_id)?;
    let exact = crate::installation::load_installation_marker(&game.installation_directory)?
        .and_then(|marker| marker.galaxy_depot.map(|depot| depot.build_id));
    let Some(build) =
        metadata::select_build(&builds, exact.as_deref(), game.installed_version.as_deref())
    else {
        return Ok(unavailable("no generation-2 Windows build is available"));
    };
    let build_id = Some(build.build_id.clone());
    let client = api::client()?;
    let credentials = match metadata::fetch_credentials(&client, &build.repository_url) {
        Ok(credentials) => credentials,
        Err(error)
            if error
                .downcast_ref::<metadata::MissingGameCredentials>()
                .is_some() =>
        {
            return Ok(CloudSaveDiscovery {
                metadata_build_id: build_id,
                ..unavailable("the generation-2 repository has no game credentials")
            });
        }
        Err(error) => return Err(error),
    };
    let config = metadata::fetch_remote_configuration(&client, &credentials.client_id)?;
    let Some(storage) = config
        .content
        .windows
        .and_then(|windows| windows.cloud_storage)
    else {
        return Ok(CloudSaveDiscovery {
            metadata_build_id: build_id,
            ..unavailable("GOG has no Windows cloud-storage configuration")
        });
    };
    if !storage.enabled {
        return Ok(CloudSaveDiscovery {
            availability: CloudSaveAvailability::Unsupported,
            locations: overrides.clone(),
            metadata_build_id: build_id,
            checked_at,
            reason: Some("GOG reports cloud saves are disabled for this game".into()),
        });
    }
    let locations = if overrides.is_empty() {
        let compatibility = game
            .compatibility
            .as_ref()
            .context("managed UMU prefix is unavailable")?;
        let library = game
            .installation_directory
            .parent()
            .context("installation has no library root")?;
        paths::resolve_locations(
            &storage.locations,
            &game.installation_directory,
            &crate::compatibility::prefix_path(library, &compatibility.prefix_slug),
            &credentials.client_id,
        )?
    } else {
        overrides
    };
    Ok(CloudSaveDiscovery {
        availability: CloudSaveAvailability::Supported,
        locations,
        metadata_build_id: build_id,
        checked_at,
        reason: None,
    })
}

pub fn discover_and_store(
    game: &InstalledGame,
    stored_locations: &[CloudSaveLocation],
) -> Result<CloudSaveDiscovery> {
    discover_and_store_for_session(game, stored_locations, crate::auth::session())
}

pub fn discover_and_store_for_session(
    game: &InstalledGame,
    stored_locations: &[CloudSaveLocation],
    session: u64,
) -> Result<CloudSaveDiscovery> {
    ensure_session(session)?;
    let store = crate::state::StateStore::open()?;
    match discover(game, stored_locations) {
        Ok(discovery) => {
            ensure_session(session)?;
            store.set_cloud_save_discovery(game.product_id, &discovery)?;
            Ok(discovery)
        }
        Err(error) => {
            ensure_session(session)?;
            let previous = store.cloud_save_record(game.product_id)?;
            store.set_cloud_save_discovery(
                game.product_id,
                &CloudSaveDiscovery {
                    availability: CloudSaveAvailability::Unknown,
                    locations: previous.locations,
                    metadata_build_id: previous.metadata_build_id,
                    checked_at: previous.metadata_checked_at.unwrap_or(0),
                    reason: Some(error.to_string()),
                },
            )?;
            Err(error)
        }
    }
}

fn windows_builds(
    store: &crate::state::StateStore,
    product_id: i64,
) -> Result<Vec<crate::domain::GalaxyBuild>> {
    Ok(store
        .load_galaxy_builds(product_id)?
        .into_iter()
        .filter(|build| build.operating_system.eq_ignore_ascii_case("windows"))
        .collect())
}

pub fn inventory(game: &InstalledGame, session: u64) -> Result<CloudSaveInventory> {
    ensure_session(session)?;
    let _activity = crate::profile_reset::begin_activity("checking cloud-save storage")?;
    let store = crate::state::StateStore::open()?;
    let record = store.cloud_save_record(game.product_id)?;
    if record.availability != CloudSaveAvailability::Supported {
        bail!("GOG cloud saves are not supported for this game");
    }
    let token = crate::auth::load_saved_token()?.context("sign in to GOG to check cloud saves")?;
    let builds = windows_builds(&store, game.product_id)?;
    let exact = crate::installation::load_installation_marker(&game.installation_directory)?
        .and_then(|marker| marker.galaxy_depot.map(|depot| depot.build_id));
    let build =
        metadata::select_build(&builds, exact.as_deref(), game.installed_version.as_deref())
            .context("no generation-2 Windows build is available")?;
    let client = api::client()?;
    let credentials = metadata::fetch_credentials(&client, &build.repository_url)?;
    ensure_session(session)?;
    let scoped = api::exchange_scoped_token(&client, &token.refresh_token, &credentials)?;
    ensure_session(session)?;
    let cloud = api::CloudClient::new(client, token.user_id, credentials.client_id, scoped)
        .for_session(session);
    Ok(summarize_inventory(&cloud.list()?))
}

fn summarize_inventory(objects: &[api::RemoteObject]) -> CloudSaveInventory {
    CloudSaveInventory {
        file_count: objects.len(),
        total_size: objects
            .iter()
            .fold(0, |total, object| total.saturating_add(object.size)),
        latest_modified_at: objects.iter().map(|object| object.modified_at).max(),
    }
}

pub fn sync_for_session(mut request: CloudSyncRequest, session: u64) -> Result<CloudSyncResult> {
    ensure_session(session)?;
    let _activity = crate::profile_reset::begin_activity("cloud sync")?;
    let _operation = begin_operation(request.game.product_id)?;
    if request.game.compatibility.is_none()
        || !request
            .game
            .installer_operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        bail!("cloud saves are supported only for managed Windows installations");
    }
    let store = crate::state::StateStore::open()?;
    let discovery = discover_and_store_for_session(&request.game, &request.locations, session)?;
    if discovery.availability != CloudSaveAvailability::Supported {
        bail!(
            "{}",
            discovery
                .reason
                .as_deref()
                .unwrap_or("GOG cloud saves are unavailable")
        );
    }
    request.locations = discovery.locations;
    let token = crate::auth::load_saved_token()?.context("sign in to GOG to synchronize saves")?;
    let builds = windows_builds(&store, request.game.product_id)?;
    let exact =
        crate::installation::load_installation_marker(&request.game.installation_directory)?
            .and_then(|marker| marker.galaxy_depot.map(|depot| depot.build_id));
    let build = metadata::select_build(
        &builds,
        exact.as_deref(),
        request.game.installed_version.as_deref(),
    )
    .context("no generation-2 Windows build is available")?;
    let client = api::client()?;
    let credentials = metadata::fetch_credentials(&client, &build.repository_url)?;
    ensure_session(session)?;
    let scoped = api::exchange_scoped_token(&client, &token.refresh_token, &credentials)?;
    ensure_session(session)?;
    let cloud = api::CloudClient::new(client, token.user_id, credentials.client_id, scoped)
        .for_session(session);
    sync::synchronize(
        &store,
        request.game.product_id,
        &request.locations,
        request.mode,
        &cloud,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_operations_exclude_concurrent_sync_for_the_same_product() {
        let first = begin_operation(987654321).unwrap();
        assert!(begin_operation(987654321).is_err());
        assert!(begin_operation(987654322).is_ok());
        drop(first);
        assert!(begin_operation(987654321).is_ok());
    }

    #[test]
    fn inventory_counts_files_size_and_latest_change() {
        let objects = [
            api::RemoteObject {
                namespace: "saves".into(),
                path: "one.sav".into(),
                size: 12,
                modified_at: 10,
                etag: "one".into(),
            },
            api::RemoteObject {
                namespace: "saves".into(),
                path: "two.sav".into(),
                size: 30,
                modified_at: 20,
                etag: "two".into(),
            },
        ];
        assert_eq!(
            summarize_inventory(&objects),
            CloudSaveInventory {
                file_count: 2,
                total_size: 42,
                latest_modified_at: Some(20),
            }
        );
    }
}
