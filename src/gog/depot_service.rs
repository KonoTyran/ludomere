use crate::{domain::GalaxyBuild, state::StateStore};
use anyhow::{Result, bail};

pub struct BranchPassword(String);

impl BranchPassword {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

pub struct BuildRequest {
    pub user_id: String,
    pub product_id: i64,
    pub platform: String,
    pub generation: u32,
    pub branch: Option<String>,
    pub supplied_password: Option<BranchPassword>,
}

impl std::fmt::Debug for BuildRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BuildRequest")
            .field("user_id", &self.user_id)
            .field("product_id", &self.product_id)
            .field("platform", &self.platform)
            .field("generation", &self.generation)
            .field("branch", &self.branch)
            .field(
                "supplied_password",
                &self.supplied_password.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    NeedsBranchPassword,
    InvalidBranchPassword,
}

#[derive(Debug)]
pub struct ServiceError {
    pub kind: ServiceErrorKind,
    pub product_id: i64,
    pub saved_credential: bool,
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match (self.kind, self.saved_credential) {
            (ServiceErrorKind::NeedsBranchPassword, _) => "protected branch password is required",
            (ServiceErrorKind::InvalidBranchPassword, true) => {
                "saved protected branch password was rejected; enter a new password or forget it"
            }
            (ServiceErrorKind::InvalidBranchPassword, false) => {
                "protected branch password was rejected"
            }
        })
    }
}

impl std::error::Error for ServiceError {}

pub fn list_builds(
    store: &StateStore,
    client: &reqwest::blocking::Client,
    request: &BuildRequest,
) -> Result<Vec<GalaxyBuild>> {
    let builds = list_with(
        request,
        || {
            request
                .branch
                .as_deref()
                .map(|branch| {
                    crate::branch_credentials::load(
                        store,
                        &request.user_id,
                        request.product_id,
                        branch,
                    )
                })
                .transpose()
                .map(Option::flatten)
        },
        |password| {
            crate::gog::builds::fetch_authenticated_generation(
                client,
                crate::auth::load_saved_token()?
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("sign in to GOG to list Galaxy builds"))?
                    .access_token
                    .as_str(),
                password,
                request.product_id,
                &request.platform,
                request.generation,
            )
        },
        |password| {
            let branch = request
                .branch
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("branch password requires a branch"))?;
            crate::branch_credentials::save(
                store,
                &request.user_id,
                request.product_id,
                branch,
                password,
            )
        },
    )?;
    store.observe_galaxy_builds(request.product_id, &request.platform, &builds)?;
    Ok(builds)
}

pub fn forget_one(store: &StateStore, user_id: &str, product_id: i64, branch: &str) -> Result<()> {
    crate::branch_credentials::forget(store, user_id, product_id, branch)
}

pub fn forget_all(store: &StateStore, user_id: &str) -> Result<usize> {
    crate::branch_credentials::forget_all(store, user_id)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PrepareOperationRequest {
    pub build: GalaxyBuild,
    pub selection: crate::gog::depot_acquisition::Selection,
    pub operation_id: String,
    pub kind: crate::domain::DepotOperationKind,
    pub library_id: String,
    pub library_root: std::path::PathBuf,
    pub slug: String,
}

pub fn prepare_operation(
    store: &StateStore,
    client: &reqwest::blocking::Client,
    request: PrepareOperationRequest,
) -> Result<crate::installation::DepotOperationRequest> {
    let token = crate::auth::load_saved_token()?
        .ok_or_else(|| anyhow::anyhow!("sign in to GOG to prepare a Galaxy depot operation"))?;
    let acquisition = crate::installation::depot_planner::load_cached_acquisition(
        store,
        &request.build,
        &request.selection,
    )?
    .map(Ok)
    .unwrap_or_else(|| {
        crate::gog::depot_acquisition::acquire(
            client,
            &token.access_token,
            &request.build,
            &request.selection,
        )
    })?;
    crate::installation::depot_planner::prepare(
        crate::installation::depot_planner::PrepareDepotRequest {
            store,
            acquisition: &acquisition,
            build: &request.build,
            selection: &request.selection,
            operation_id: request.operation_id,
            kind: request.kind,
            library_id: request.library_id,
            library_root: request.library_root,
            slug: request.slug,
            access_token: token.access_token,
        },
    )
}

pub fn start_operation(
    store: &StateStore,
    client: &reqwest::blocking::Client,
    request: PrepareOperationRequest,
) -> Result<String> {
    let operation = prepare_operation(store, client, request)?;
    let operation_id = operation.operation_id.clone();
    if !crate::installation::enqueue_depot_operation(operation) {
        bail!("Galaxy depot operation conflicts with active work or could not be persisted");
    }
    Ok(operation_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildResolutionError {
    InstalledBuildUnavailable,
    BranchUnavailable,
    NoUpdate,
}

impl std::fmt::Display for BuildResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InstalledBuildUnavailable => {
                "the installed Galaxy build is no longer available; choose another build"
            }
            Self::BranchUnavailable => "the selected Galaxy branch has no available build",
            Self::NoUpdate => "the installed Galaxy build is already current",
        })
    }
}

impl std::error::Error for BuildResolutionError {}

pub fn resolve_operation_build<'a>(
    builds: &'a [GalaxyBuild],
    marker: &crate::installation::InstallationMarker,
    kind: crate::domain::DepotOperationKind,
    target_branch: Option<&str>,
) -> Result<&'a GalaxyBuild> {
    let installed = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("installation has no Galaxy depot provenance"))?;
    let matching_os = |build: &&GalaxyBuild| {
        build.generation == 2
            && build.currently_returned
            && marker
                .base
                .operating_system
                .as_deref()
                .is_none_or(|os| build.operating_system.eq_ignore_ascii_case(os))
    };
    match kind {
        crate::domain::DepotOperationKind::Install => {
            bail!("install build resolution does not use an existing marker")
        }
        crate::domain::DepotOperationKind::Repair => builds
            .iter()
            .filter(matching_os)
            .find(|build| build.build_id == installed.build_id)
            .ok_or_else(|| BuildResolutionError::InstalledBuildUnavailable.into()),
        crate::domain::DepotOperationKind::Update => {
            let target = builds
                .iter()
                .filter(matching_os)
                .filter(|build| build.branch == installed.branch)
                .max_by_key(|build| build.published_at)
                .ok_or(BuildResolutionError::BranchUnavailable)?;
            if target.build_id == installed.build_id {
                return Err(BuildResolutionError::NoUpdate.into());
            }
            Ok(target)
        }
        crate::domain::DepotOperationKind::BranchSwitch => {
            if target_branch == installed.branch.as_deref() {
                bail!("branch switch target is the installed branch");
            }
            builds
                .iter()
                .filter(matching_os)
                .filter(|build| build.branch.as_deref() == target_branch)
                .max_by_key(|build| build.published_at)
                .ok_or_else(|| BuildResolutionError::BranchUnavailable.into())
        }
    }
}

fn list_with<L, F, S>(
    request: &BuildRequest,
    load: L,
    mut fetch: F,
    save: S,
) -> Result<Vec<GalaxyBuild>>
where
    L: FnOnce() -> Result<Option<String>>,
    F: FnMut(Option<&str>) -> Result<Vec<GalaxyBuild>>,
    S: FnOnce(&str) -> Result<()>,
{
    if !matches!(request.generation, 1 | 2) {
        bail!("unsupported Galaxy build-list generation");
    }
    if request.supplied_password.is_some() && request.branch.is_none() {
        bail!("branch password requires a branch");
    }
    let supplied = request
        .supplied_password
        .as_ref()
        .map(BranchPassword::expose);
    let saved = if supplied.is_none() && request.branch.is_some() {
        load()?
    } else {
        None
    };
    let password = supplied.or(saved.as_deref());
    let builds = fetch(password).map_err(|error| {
        let kind = error
            .downcast_ref::<crate::gog::depot_acquisition::AcquisitionError>()
            .map(|error| error.0);
        match kind {
            Some(crate::gog::depot_acquisition::AcquisitionErrorKind::NeedsBranchPassword) => {
                ServiceError {
                    kind: ServiceErrorKind::NeedsBranchPassword,
                    product_id: request.product_id,
                    saved_credential: saved.is_some(),
                }
                .into()
            }
            Some(crate::gog::depot_acquisition::AcquisitionErrorKind::InvalidBranchPassword) => {
                ServiceError {
                    kind: ServiceErrorKind::InvalidBranchPassword,
                    product_id: request.product_id,
                    saved_credential: saved.is_some(),
                }
                .into()
            }
            _ => error,
        }
    })?;
    if let Some(password) = supplied {
        save(password)?;
    }
    Ok(builds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn request(password: Option<&str>) -> BuildRequest {
        BuildRequest {
            user_id: "user".into(),
            product_id: 7,
            platform: "windows".into(),
            generation: 2,
            branch: Some("beta".into()),
            supplied_password: password.map(|value| BranchPassword::new(value.into())),
        }
    }

    fn auth_error(kind: crate::gog::depot_acquisition::AcquisitionErrorKind) -> anyhow::Error {
        crate::gog::depot_acquisition::AcquisitionError(kind).into()
    }

    #[test]
    fn reuses_saved_and_saves_supplied_only_after_success() {
        let seen = RefCell::new(String::new());
        list_with(
            &request(None),
            || Ok(Some("saved".into())),
            |password| {
                *seen.borrow_mut() = password.unwrap().into();
                Ok(Vec::new())
            },
            |_| panic!("saved credential must not be rewritten"),
        )
        .unwrap();
        assert_eq!(seen.into_inner(), "saved");

        let saved = RefCell::new(None);
        list_with(
            &request(Some("new")),
            || panic!(),
            |_| Ok(Vec::new()),
            |password| {
                *saved.borrow_mut() = Some(password.to_owned());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(saved.into_inner().as_deref(), Some("new"));
    }

    #[test]
    fn failed_auth_never_saves_and_reports_saved_context() {
        let called = Cell::new(false);
        let error = list_with(
            &request(Some("secret")),
            || panic!(),
            |_| {
                Err(auth_error(
                    crate::gog::depot_acquisition::AcquisitionErrorKind::InvalidBranchPassword,
                ))
            },
            |_| {
                called.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(!called.get());
        assert!(!format!("{error:?}").contains("secret"));

        let error = list_with(
            &request(None),
            || Ok(Some("old".into())),
            |_| {
                Err(auth_error(
                    crate::gog::depot_acquisition::AcquisitionErrorKind::InvalidBranchPassword,
                ))
            },
            |_| panic!(),
        )
        .unwrap_err();
        let service = error.downcast_ref::<ServiceError>().unwrap();
        assert!(service.saved_credential);
        assert!(service.to_string().contains("forget"));
    }

    #[test]
    fn request_debug_and_missing_password_are_secret_safe() {
        let debug_request = request(Some("password-sentinel"));
        assert!(!format!("{debug_request:?}").contains("password-sentinel"));
        let error = list_with(
            &request(None),
            || Ok(None),
            |_| {
                Err(auth_error(
                    crate::gog::depot_acquisition::AcquisitionErrorKind::NeedsBranchPassword,
                ))
            },
            |_| panic!(),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>().unwrap().kind,
            ServiceErrorKind::NeedsBranchPassword
        );
    }

    #[test]
    fn forget_routes_are_user_scoped() {
        let path = std::env::temp_dir().join(format!(
            "ludomere-depot-service-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        for (user, product, branch) in [("user", 1, "a"), ("user", 2, "b"), ("other", 1, "a")] {
            store
                .save_galaxy_branch_credential(user, product, branch, 1, &[1; 12], b"opaque")
                .unwrap();
        }
        forget_one(&store, "user", 1, "a").unwrap();
        assert!(
            store
                .galaxy_branch_credential("user", 1, "a")
                .unwrap()
                .is_none()
        );
        assert_eq!(forget_all(&store, "user").unwrap(), 1);
        assert!(
            store
                .galaxy_branch_credential("other", 1, "a")
                .unwrap()
                .is_some()
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
