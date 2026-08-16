use super::{
    manager::{DepotOperationRequest, DepotSource, EntitlementDlc},
    marker::{InstallationMarker, InstalledCompatibility, InstalledComponent, InstalledDlc},
};
use crate::{
    domain::{
        DepotOperationKind, GalaxyBuild, GalaxyDepotDlcProvenance, GalaxyDepotIdentity,
        GalaxyDepotProvenance, InstallationSource,
    },
    gog::depot_acquisition::{Acquisition, Selection},
    state::{DepotManifestRecord, DepotRepositoryRecord, StateStore},
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub struct PrepareDepotRequest<'a> {
    pub store: &'a StateStore,
    pub acquisition: &'a Acquisition,
    pub build: &'a GalaxyBuild,
    pub selection: &'a Selection,
    pub operation_id: String,
    pub kind: DepotOperationKind,
    pub library_id: String,
    pub library_root: PathBuf,
    pub slug: String,
    pub access_token: String,
}

pub fn prepare(request: PrepareDepotRequest<'_>) -> Result<DepotOperationRequest> {
    validate(&request)?;
    cache_acquisition(request.store, request.acquisition, request.build)?;
    let now = chrono::Utc::now().timestamp();
    let destination = request.library_root.join(&request.slug);
    let existing = super::marker::load(&destination)?;
    validate_operation(request.kind, existing.as_ref(), request.build)?;
    let current_sources = existing
        .as_ref()
        .map(|marker| load_current_sources(request.store, marker))
        .transpose()?
        .unwrap_or_default();
    let base_product = request.build.product_id;
    let identities = |product_id| {
        request
            .acquisition
            .sources
            .iter()
            .filter(|source| source.product_id == product_id)
            .map(|source| GalaxyDepotIdentity {
                depot_id: source.depot_identity.clone(),
                manifest_id: source.manifest_id.clone(),
            })
            .collect::<Vec<_>>()
    };
    let entitlement = request
        .acquisition
        .entitlement_only_dlc
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let dlc = request
        .selection
        .selected_dlc
        .iter()
        .map(|product_id| GalaxyDepotDlcProvenance {
            product_id: *product_id,
            depots: identities(*product_id),
            has_payload: request.acquisition.sources.iter().any(|source| {
                source.product_id == *product_id
                    && source
                        .manifest
                        .split_support()
                        .is_ok_and(|(payload, _)| !payload.entries.is_empty())
            }),
            entitlement_only_marker: entitlement.contains(product_id),
        })
        .collect::<Vec<_>>();
    let operating_system = request.build.operating_system.clone();
    let compatibility = operating_system.eq_ignore_ascii_case("windows").then(|| {
        let (profile, _) = crate::compatibility::resolve_profile(base_product);
        InstalledCompatibility {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            managed_by_ludomere: true,
            prefix_slug: request.slug.clone(),
            profile,
        }
    });
    let mut marker = InstallationMarker {
        schema_version: if compatibility.is_some() { 2 } else { 1 },
        product_id: base_product,
        slug: request.slug.clone(),
        base: InstalledComponent {
            operating_system: Some(operating_system),
            language: Some(request.selection.language.clone()),
            version: request.build.version.clone(),
            revision_id: None,
            installed_at: now,
        },
        dlc: request
            .selection
            .selected_dlc
            .iter()
            .map(|product_id| InstalledDlc {
                product_id: *product_id,
                version: request.build.version.clone(),
                revision_id: None,
                installed_at: now,
            })
            .collect(),
        compatibility,
        source: InstallationSource::GalaxyDepot,
        galaxy_depot: Some(GalaxyDepotProvenance {
            build_id: request.build.build_id.clone(),
            repository_id: request.acquisition.repository_id.clone(),
            manifest_fingerprint: String::new(),
            branch: request.build.branch.clone(),
            language: Some(request.selection.language.clone()),
            architecture: request.selection.bitness.clone(),
            depots: identities(base_product),
            dlc,
        }),
        launch: None,
        dependencies: request.acquisition.repository.dependencies.clone(),
    };
    let sources = request
        .acquisition
        .sources
        .iter()
        .map(|source| {
            Ok(DepotSource {
                product_id: source.product_id,
                depot_id: source.depot_identity.clone(),
                manifest_id: source.manifest_id.clone(),
                manifest_json: Some(source.manifest.canonical_json()?),
                content_root: Some(source.content_root.clone()),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let entitlement_dlc = request
        .acquisition
        .entitlement_only_dlc
        .iter()
        .map(|product_id| {
            let name = request
                .acquisition
                .repository
                .products
                .iter()
                .find(|product| product.product_id.parse::<i64>() == Ok(*product_id))
                .and_then(|product| product.name.clone())
                .context("entitlement-only DLC has no product name")?;
            Ok(EntitlementDlc {
                product_id: *product_id,
                name,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let staging_path = super::depot::operation_staging_path(
        &request.library_root,
        &destination,
        &request.slug,
        &request.operation_id,
    )?;
    let mut operation = DepotOperationRequest {
        operation_id: request.operation_id,
        product_id: base_product,
        build_id: request.build.build_id.clone(),
        branch: request.build.branch.clone(),
        kind: request.kind,
        sources,
        current_sources,
        current_manifest_json: None,
        library_id: request.library_id,
        dependencies: request.acquisition.repository.dependencies.clone(),
        entitlement_dlc,
        library_root: request.library_root,
        slug: request.slug,
        destination,
        staging_path,
        target_marker: marker.clone(),
        access_token: request.access_token,
    };
    marker.galaxy_depot.as_mut().unwrap().manifest_fingerprint =
        super::manager::planned_manifest_identity(&operation)?;
    marker.validate()?;
    operation.target_marker = marker;
    Ok(operation)
}

fn validate(request: &PrepareDepotRequest<'_>) -> Result<()> {
    if request.acquisition.build_id != request.build.build_id
        || request.acquisition.branch != request.build.branch
        || request
            .acquisition
            .repository
            .root_product_id
            .parse::<i64>()
            != Ok(request.build.product_id)
    {
        bail!("acquired depot metadata does not match the selected build");
    }
    if request.library_id.is_empty() || request.access_token.is_empty() {
        bail!("depot preparation requires library and authentication identity");
    }
    if request.build.generation != 2 {
        bail!("only generation-2 Galaxy builds can be installed");
    }
    if !matches!(
        request.build.operating_system.to_ascii_lowercase().as_str(),
        "windows" | "linux"
    ) {
        bail!("Galaxy depot installation is supported only for Windows and Linux builds");
    }
    Ok(())
}

fn validate_operation(
    kind: DepotOperationKind,
    existing: Option<&InstallationMarker>,
    target: &GalaxyBuild,
) -> Result<()> {
    match (kind, existing) {
        (DepotOperationKind::Install, None) => Ok(()),
        (DepotOperationKind::Install, Some(_)) => bail!("install destination is already managed"),
        (_, None) => bail!("depot operation requires an existing installation"),
        (_, Some(marker)) if marker.source != InstallationSource::GalaxyDepot => {
            bail!("offline and depot installation semantics cannot be mixed")
        }
        (DepotOperationKind::Update, Some(marker))
            if marker
                .galaxy_depot
                .as_ref()
                .is_none_or(|installed| installed.branch != target.branch) =>
        {
            bail!("changing Galaxy branches requires a branch-switch operation")
        }
        (DepotOperationKind::BranchSwitch, Some(marker))
            if marker
                .galaxy_depot
                .as_ref()
                .is_some_and(|installed| installed.branch == target.branch) =>
        {
            bail!("target build is on the currently installed branch")
        }
        _ => Ok(()),
    }
}

pub fn cache_acquisition(
    store: &StateStore,
    acquisition: &Acquisition,
    build: &GalaxyBuild,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    store.save_depot_repository(&DepotRepositoryRecord {
        product_id: build.product_id,
        operating_system: build.operating_system.clone(),
        build_id: build.build_id.clone(),
        branch: build.branch.clone(),
        manifest_identity: acquisition.repository_id.clone(),
        repository_json: serde_json::to_string(&acquisition.repository)?,
        first_seen_at: now,
        last_seen_at: now,
    })?;
    for source in &acquisition.sources {
        store.save_depot_manifest(&DepotManifestRecord {
            manifest_identity: source.manifest.identity(),
            product_id: source.product_id,
            build_id: build.build_id.clone(),
            depot_id: source.depot_identity.clone(),
            manifest_json: source.manifest.canonical_json()?,
            first_seen_at: now,
            last_seen_at: now,
        })?;
    }
    Ok(())
}

pub fn cached_acquisition_available(
    store: &StateStore,
    build: &GalaxyBuild,
    selection: &Selection,
) -> Result<bool> {
    Ok(load_cached_acquisition(store, build, selection)?.is_some())
}

pub fn load_cached_acquisition(
    store: &StateStore,
    build: &GalaxyBuild,
    selection: &Selection,
) -> Result<Option<Acquisition>> {
    let Some(record) =
        store.depot_repository(build.product_id, &build.operating_system, &build.build_id)?
    else {
        return Ok(None);
    };
    if record.manifest_identity != build.repository_id.clone().unwrap_or_default() {
        return Ok(None);
    }
    let repository: crate::gog::types::GenerationTwoRepository =
        serde_json::from_str(&record.repository_json)?;
    let depots = crate::gog::depot_acquisition::select_depots(&repository, selection)?;
    let mut sources = Vec::with_capacity(depots.len());
    for depot in depots {
        let product_id = depot
            .product_id
            .parse()
            .context("cached repository depot product ID is invalid")?;
        let Some(manifest) =
            store.depot_manifest_for_depot(product_id, &build.build_id, &depot.manifest_id)?
        else {
            return Ok(None);
        };
        let bytes = manifest.manifest_json.into_bytes();
        let parsed = crate::gog::depot_manifest::parse(&bytes)?;
        if parsed.identity() != manifest.manifest_identity {
            return Ok(None);
        }
        sources.push(crate::gog::depot_acquisition::SelectedSource {
            product_id,
            depot_identity: depot.manifest_id.clone(),
            manifest_id: depot.manifest_id.clone(),
            content_root: "/".into(),
            manifest_bytes: bytes,
            manifest: parsed,
        });
    }
    let payload_products = sources
        .iter()
        .map(|source| source.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    let entitlement_only_dlc = selection
        .selected_dlc
        .iter()
        .filter(|id| selection.owned_dlc.contains(id) && !payload_products.contains(id))
        .copied()
        .collect();
    Ok(Some(Acquisition {
        build_id: build.build_id.clone(),
        branch: build.branch.clone(),
        repository_id: record.manifest_identity,
        repository,
        sources,
        entitlement_only_dlc,
    }))
}

fn load_current_sources(
    store: &StateStore,
    marker: &InstallationMarker,
) -> Result<Vec<DepotSource>> {
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .context("installed depot provenance is missing")?;
    provenance
        .depots
        .iter()
        .map(|depot| (marker.product_id, depot))
        .chain(
            provenance
                .dlc
                .iter()
                .flat_map(|dlc| dlc.depots.iter().map(move |depot| (dlc.product_id, depot))),
        )
        .map(|(product_id, depot)| {
            let record = store
                .depot_manifest_for_depot(product_id, &provenance.build_id, &depot.depot_id)?
                .context(
                    "installed depot manifest is not cached; repair metadata must be reacquired",
                )?;
            Ok(DepotSource {
                product_id,
                depot_id: depot.depot_id.clone(),
                manifest_id: depot.manifest_id.clone(),
                manifest_json: Some(record.manifest_json),
                content_root: Some("/".into()),
            })
        })
        .collect()
}

pub fn installation_directory(library_root: &Path, slug: &str) -> Result<PathBuf> {
    let destination = library_root.join(slug);
    super::depot::operation_staging_path(library_root, &destination, slug, "validate")?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gog::{
        depot_acquisition::SelectedSource,
        depot_manifest::{DepotChunk, DepotEntry, DepotFile, DepotManifest},
        types::{GenerationTwoRepository, RepositoryDepot, RepositoryProduct},
    };

    #[test]
    fn prepares_and_caches_a_secret_free_install_request() {
        let root = std::env::temp_dir().join(format!(
            "ludomere-depot-plan-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = StateStore::open_at(&root.join("state.db")).unwrap();
        let chunk = DepotChunk {
            compressed_md5: "0".repeat(32),
            compressed_size: 1,
            md5: "1".repeat(32),
            size: 1,
        };
        let manifest = DepotManifest {
            generation: 2,
            entries: vec![DepotEntry::File(DepotFile {
                path: "game.exe".into(),
                size: 1,
                executable: false,
                support: false,
                md5: None,
                sha256: None,
                chunks: vec![chunk],
                small_file: None,
            })],
            small_files_containers: Vec::new(),
        };
        let repository = GenerationTwoRepository {
            generation: 2,
            root_product_id: "7".into(),
            build_id: Some("build".into()),
            platform: Some("linux".into()),
            install_directory: "Game".into(),
            products: vec![RepositoryProduct {
                product_id: "7".into(),
                name: Some("Game".into()),
                script: None,
                temp_arguments: None,
                temp_executable: None,
                play_tasks: Vec::new(),
                support_tasks: Vec::new(),
            }],
            depots: vec![RepositoryDepot {
                manifest_id: "manifest".into(),
                product_id: "7".into(),
                languages: vec!["en-US".into()],
                os_bitness: None,
                compressed_size: Some(1),
                size: 1,
                is_gog_depot: false,
            }],
            dependencies: Vec::new(),
        };
        let acquisition = Acquisition {
            build_id: "build".into(),
            branch: None,
            repository_id: "repository".into(),
            repository,
            sources: vec![SelectedSource {
                product_id: 7,
                depot_identity: "manifest".into(),
                manifest_id: "manifest".into(),
                content_root: "/".into(),
                manifest_bytes: Vec::new(),
                manifest,
            }],
            entitlement_only_dlc: Vec::new(),
        };
        let build = GalaxyBuild {
            build_id: "build".into(),
            product_id: 7,
            operating_system: "linux".into(),
            version: Some("1".into()),
            branch: None,
            tags: Vec::new(),
            public: true,
            generation: 2,
            repository_url: "unused".into(),
            repository_id: Some("repository".into()),
            published_at: None,
            currently_returned: true,
            first_seen_at: 1,
            last_seen_at: 1,
        };
        let selection = Selection {
            language: "en-US".into(),
            bitness: None,
            owned_dlc: Default::default(),
            selected_dlc: Default::default(),
        };
        let operation = prepare(PrepareDepotRequest {
            store: &store,
            acquisition: &acquisition,
            build: &build,
            selection: &selection,
            operation_id: "op".into(),
            kind: DepotOperationKind::Install,
            library_id: "library".into(),
            library_root: root.clone(),
            slug: "game".into(),
            access_token: "secret".into(),
        })
        .unwrap();
        assert!(
            operation
                .target_marker
                .galaxy_depot
                .as_ref()
                .unwrap()
                .manifest_fingerprint
                .starts_with("sha256:")
        );
        assert!(
            store
                .depot_repository(7, "linux", "build")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .depot_manifest_for_depot(7, "build", "manifest")
                .unwrap()
                .is_some()
        );
        assert!(cached_acquisition_available(&store, &build, &selection).unwrap());
        let cached = load_cached_acquisition(&store, &build, &selection)
            .unwrap()
            .unwrap();
        assert_eq!(cached.repository_id, "repository");
        assert_eq!(cached.sources.len(), 1);
        assert!(!format!("{operation:?}").contains("secret"));
        let _ = std::fs::remove_dir_all(root);
    }
}
