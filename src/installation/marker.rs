use crate::{
    domain::{GalaxyDepotProvenance, InstallationSource, InstallationState, InstalledGame},
    state::StateStore,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MARKER_SCHEMA_VERSION: u32 = 2;
const MARKER_FILENAME: &str = "installation.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallationMarker {
    pub schema_version: u32,
    pub product_id: i64,
    pub slug: String,
    pub base: InstalledComponent,
    #[serde(default)]
    pub dlc: Vec<InstalledDlc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<InstalledCompatibility>,
    #[serde(
        default,
        skip_serializing_if = "InstallationSource::is_offline_installer"
    )]
    pub source: InstallationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub galaxy_depot: Option<GalaxyDepotProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<InstalledLaunch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl InstallationMarker {
    pub fn with_galaxy_depot(mut self, provenance: GalaxyDepotProvenance) -> Result<Self> {
        self.source = InstallationSource::GalaxyDepot;
        self.galaxy_depot = Some(provenance);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        let expected_schema = if self.compatibility.is_some() { 2 } else { 1 };
        if self.schema_version != expected_schema {
            bail!("installation marker schema does not match its platform runtime");
        }
        if let Some(launch) = &self.launch {
            validate_marker_relative_path(&launch.executable)?;
            if let Some(directory) = launch.working_directory.as_deref() {
                validate_marker_relative_path(directory)?;
            }
        }
        match (self.source, self.galaxy_depot.as_ref()) {
            (InstallationSource::OfflineInstaller, None)
            | (InstallationSource::GalaxyDepot, Some(_)) => Ok(()),
            (InstallationSource::OfflineInstaller, Some(_)) => {
                bail!("offline installation marker contains Galaxy depot provenance")
            }
            (InstallationSource::GalaxyDepot, None) => {
                bail!("Galaxy depot installation marker is missing provenance")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledLaunch {
    pub executable: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

fn validate_marker_relative_path(path: &str) -> Result<()> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("installation marker contains an unsafe launch path");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledCompatibility {
    pub backend: crate::compatibility::CompatibilityBackendKind,
    pub managed_by_ludomere: bool,
    pub prefix_slug: String,
    pub profile: crate::compatibility::UmuProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledComponent {
    pub operating_system: Option<String>,
    pub language: Option<String>,
    pub version: Option<String>,
    pub revision_id: Option<i64>,
    pub installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledDlc {
    pub product_id: i64,
    pub version: Option<String>,
    pub revision_id: Option<i64>,
    pub installed_at: i64,
}

pub fn marker_path(installation_directory: &Path) -> PathBuf {
    installation_directory
        .join(crate::identity::MARKER_DIRECTORY)
        .join(MARKER_FILENAME)
}

pub fn load(installation_directory: &Path) -> Result<Option<InstallationMarker>> {
    let path = marker_path(installation_directory);
    let directory = path.parent().unwrap();
    if fs::symlink_metadata(directory).is_ok_and(|value| value.file_type().is_symlink())
        || fs::symlink_metadata(&path).is_ok_and(|value| value.file_type().is_symlink())
    {
        bail!("installation marker path cannot be a symlink");
    }
    if !path.is_file() {
        return Ok(None);
    }
    let marker: InstallationMarker = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("could not read {}", path.display()))?,
    )
    .with_context(|| format!("could not parse {}", path.display()))?;
    if marker.schema_version > MARKER_SCHEMA_VERSION {
        bail!(
            "installation marker uses unsupported schema version {}",
            marker.schema_version
        );
    }
    marker.validate()?;
    Ok(Some(marker))
}

pub fn write(marker: &InstallationMarker, installation_directory: &Path) -> Result<()> {
    marker.validate()?;
    let directory = installation_directory.join(crate::identity::MARKER_DIRECTORY);
    if fs::symlink_metadata(&directory).is_ok_and(|value| value.file_type().is_symlink()) {
        bail!("installation marker directory cannot be a symlink");
    }
    fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let final_path = directory.join(MARKER_FILENAME);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temporary_path = directory.join(format!(
        ".{MARKER_FILENAME}.{}-{nonce}.tmp",
        std::process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary_path)
        .with_context(|| format!("could not create {}", temporary_path.display()))?;
    serde_json::to_writer_pretty(&mut file, marker)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary_path, &final_path).with_context(|| {
        format!(
            "could not replace installation marker {}",
            final_path.display()
        )
    })?;
    Ok(())
}

pub fn from_game(game: &InstalledGame, dlc: Vec<InstalledDlc>) -> InstallationMarker {
    InstallationMarker {
        schema_version: if game.compatibility.is_some() {
            MARKER_SCHEMA_VERSION
        } else {
            1
        },
        product_id: game.product_id,
        slug: game
            .installation_directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("game")
            .to_owned(),
        base: InstalledComponent {
            operating_system: game.installer_operating_system.clone(),
            language: game.installer_language.clone(),
            version: game.installed_version.clone(),
            revision_id: game.installer_revision_id,
            installed_at: game.installed_at.unwrap_or(game.updated_at),
        },
        dlc,
        compatibility: game
            .compatibility
            .as_ref()
            .map(|value| InstalledCompatibility {
                backend: value.backend,
                managed_by_ludomere: true,
                prefix_slug: value.prefix_slug.clone(),
                profile: value.profile.clone(),
            }),
        source: InstallationSource::OfflineInstaller,
        galaxy_depot: None,
        launch: None,
        dependencies: Vec::new(),
    }
}

pub fn ensure_for_game(_store: &StateStore, game: &InstalledGame) -> Result<InstallationMarker> {
    if let Some(marker) = load(&game.installation_directory)? {
        return Ok(marker);
    }
    let marker = from_game(game, Vec::new());
    write(&marker, &game.installation_directory)?;
    Ok(marker)
}

pub fn record_dlc(game: &InstalledGame, dlc: InstalledDlc) -> Result<()> {
    let store = StateStore::open()?;
    let mut marker = ensure_for_game(&store, game)?;
    marker
        .dlc
        .retain(|entry| entry.product_id != dlc.product_id);
    marker.dlc.push(dlc);
    marker.dlc.sort_by_key(|entry| entry.product_id);
    write(&marker, &game.installation_directory)
}

/// A successful GOG patch is treated as updating the complete installed game.
/// GOG patches may replace shared base/DLC payload, so advance the base and
/// every DLC already present in the marker to their currently offered
/// installer metadata. DLC that was not installed is never added.
pub fn record_successful_patch(game: &InstalledGame, target_version: Option<&str>) -> Result<()> {
    let store = StateStore::open()?;
    let mut marker = ensure_for_game(&store, game)?;
    let current_installer = |product_id| -> Option<crate::domain::DownloadRevision> {
        store
            .load_current_download_revisions(product_id)
            .ok()?
            .into_iter()
            .filter(|revision| {
                revision.provider_category == crate::domain::DownloadCategory::Installer
            })
            .filter(|revision| {
                product_id != game.product_id
                    || game
                        .installer_operating_system
                        .as_deref()
                        .is_none_or(|installed| {
                            revision
                                .operating_system
                                .as_deref()
                                .is_some_and(|current| current.eq_ignore_ascii_case(installed))
                        })
            })
            .max_by_key(|revision| revision.revision_id)
    };
    if let Some(current) = current_installer(game.product_id) {
        marker.base.revision_id = Some(current.revision_id);
        marker.base.version = target_version.map(str::to_owned).or(current.version);
    } else if let Some(version) = target_version {
        marker.base.revision_id = None;
        marker.base.version = Some(version.to_owned());
    }
    let now = chrono::Utc::now().timestamp();
    marker.base.installed_at = now;
    for dlc in &mut marker.dlc {
        if let Some(current) = current_installer(dlc.product_id) {
            dlc.revision_id = Some(current.revision_id);
            dlc.version = current.version;
            dlc.installed_at = now;
        }
    }
    write(&marker, &game.installation_directory)
}

pub fn remove(installation_directory: &Path) -> Result<()> {
    let path = marker_path(installation_directory);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("could not remove {}", path.display()))?;
    }
    if let Some(directory) = path.parent() {
        let _ = fs::remove_dir(directory);
    }
    Ok(())
}

pub fn game_from_marker(
    marker: &InstallationMarker,
    library_id: String,
    directory: PathBuf,
    executable: Option<PathBuf>,
) -> InstalledGame {
    let now = chrono::Utc::now().timestamp();
    let marker_executable = marker
        .launch
        .as_ref()
        .map(|launch| directory.join(&launch.executable));
    InstalledGame {
        product_id: marker.product_id,
        installed_version: marker.base.version.clone(),
        library_id,
        installation_directory: directory,
        installer_revision_id: marker.base.revision_id,
        installer_job_id: None,
        installer_files: Vec::new(),
        installer_complete: true,
        installer_operating_system: marker.base.operating_system.clone(),
        installer_language: marker.base.language.clone(),
        compatibility: marker.compatibility.as_ref().map(|value| {
            crate::compatibility::GameCompatibilityPreferences {
                backend: value.backend,
                prefix_slug: value.prefix_slug.clone(),
                profile: value.profile.clone(),
                pending_profile: None,
            }
        }),
        primary_executable: marker_executable.or(executable),
        launch_arguments: marker
            .launch
            .as_ref()
            .map(|launch| launch.arguments.clone())
            .unwrap_or_default(),
        state: InstallationState::Installed,
        error: None,
        installed_at: Some(marker.base.installed_at),
        verified_at: None,
        last_played_at: None,
        playtime_seconds: 0,
        created_at: marker.base.installed_at,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GalaxyDepotDlcProvenance, GalaxyDepotIdentity};

    fn example_marker() -> InstallationMarker {
        InstallationMarker {
            schema_version: 1,
            product_id: 42,
            slug: "example".into(),
            base: InstalledComponent {
                operating_system: Some("linux".into()),
                language: Some("en".into()),
                version: Some("1.0".into()),
                revision_id: Some(7),
                installed_at: 10,
            },
            dlc: vec![InstalledDlc {
                product_id: 43,
                version: Some("2".into()),
                revision_id: Some(8),
                installed_at: 11,
            }],
            compatibility: None,
            source: InstallationSource::OfflineInstaller,
            galaxy_depot: None,
            launch: None,
            dependencies: Vec::new(),
        }
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ludomere-marker-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ))
    }

    fn depot_provenance() -> GalaxyDepotProvenance {
        GalaxyDepotProvenance {
            build_id: "build-123".into(),
            repository_id: "repository-456".into(),
            manifest_fingerprint: "sha256:abcdef".into(),
            branch: None,
            language: Some("en-US".into()),
            architecture: Some("x86_64".into()),
            depots: vec![GalaxyDepotIdentity {
                depot_id: "base-depot".into(),
                manifest_id: "base-manifest".into(),
            }],
            dlc: vec![GalaxyDepotDlcProvenance {
                product_id: 43,
                depots: vec![GalaxyDepotIdentity {
                    depot_id: "dlc-depot".into(),
                    manifest_id: "dlc-manifest".into(),
                }],
                has_payload: true,
                entitlement_only_marker: false,
            }],
        }
    }

    #[test]
    fn legacy_schema_one_defaults_to_offline_source() {
        let root = test_root("legacy-schema-one");
        let directory = marker_path(&root).parent().unwrap().to_owned();
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            marker_path(&root),
            r#"{
                "schema_version": 1,
                "product_id": 42,
                "slug": "example",
                "base": {
                    "operating_system": "linux",
                    "language": "en",
                    "version": "1.0",
                    "revision_id": 7,
                    "installed_at": 10
                }
            }"#,
        )
        .unwrap();

        let marker = load(&root).unwrap().unwrap();
        assert_eq!(marker.source, InstallationSource::OfflineInstaller);
        assert_eq!(marker.galaxy_depot, None);
        write(&marker, &root).unwrap();
        let serialized = fs::read_to_string(marker_path(&root)).unwrap();
        assert!(!serialized.contains("\"source\""));
        assert!(!serialized.contains("galaxy_depot"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marker_round_trips_without_absolute_paths() {
        let root = std::env::temp_dir().join(format!(
            "ludomere-installation-marker-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = example_marker();
        write(&marker, &root).unwrap();
        assert_eq!(load(&root).unwrap(), Some(marker));
        let text = fs::read_to_string(marker_path(&root)).unwrap();
        assert!(!text.contains(root.to_string_lossy().as_ref()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_schema_two_round_trips_without_host_paths() {
        let root = test_root("windows-schema-two");
        fs::create_dir_all(&root).unwrap();
        let mut marker = example_marker();
        marker.schema_version = 2;
        marker.base.operating_system = Some("windows".into());
        marker.compatibility = Some(InstalledCompatibility {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            managed_by_ludomere: true,
            prefix_slug: "grim-dawn".into(),
            profile: crate::compatibility::UmuProfile {
                game_id: "umu-219990".into(),
                store: "gog".into(),
                source: crate::compatibility::UmuProfileSource::GogProductId,
            },
        });
        write(&marker, &root).unwrap();
        assert_eq!(load(&root).unwrap(), Some(marker));
        assert!(
            !fs::read_to_string(marker_path(&root))
                .unwrap()
                .contains(root.to_string_lossy().as_ref())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn galaxy_linux_marker_keeps_schema_one_and_provenance() {
        let marker = example_marker()
            .with_galaxy_depot(depot_provenance())
            .unwrap();
        assert_eq!(marker.schema_version, 1);
        assert_eq!(marker.source, InstallationSource::GalaxyDepot);
        assert_eq!(marker.galaxy_depot, Some(depot_provenance()));
    }

    #[test]
    fn galaxy_windows_marker_keeps_schema_two() {
        let mut marker = example_marker();
        marker.schema_version = 2;
        marker.base.operating_system = Some("windows".into());
        marker.compatibility = Some(InstalledCompatibility {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            managed_by_ludomere: true,
            prefix_slug: "example".into(),
            profile: crate::compatibility::UmuProfile {
                game_id: "umu-42".into(),
                store: "gog".into(),
                source: crate::compatibility::UmuProfileSource::GogProductId,
            },
        });
        let marker = marker.with_galaxy_depot(depot_provenance()).unwrap();
        assert_eq!(marker.schema_version, 2);
        assert!(marker.compatibility.is_some());
    }

    #[test]
    fn inconsistent_source_and_provenance_are_rejected() {
        let root = test_root("inconsistent");
        fs::create_dir_all(&root).unwrap();

        let mut missing = example_marker();
        missing.source = InstallationSource::GalaxyDepot;
        assert!(write(&missing, &root).is_err());

        let mut unexpected = example_marker();
        unexpected.galaxy_depot = Some(depot_provenance());
        assert!(write(&unexpected, &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn depot_provenance_serialization_has_no_secret_fields() {
        let marker = example_marker()
            .with_galaxy_depot(depot_provenance())
            .unwrap();
        let serialized = serde_json::to_string(&marker).unwrap();
        for forbidden in [
            "password",
            "credential",
            "token",
            "signed_url",
            "https://secret.invalid",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
