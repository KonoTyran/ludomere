use crate::{
    domain::{InstallationState, InstalledGame},
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
    Ok(Some(marker))
}

pub fn write(marker: &InstallationMarker, installation_directory: &Path) -> Result<()> {
    let directory = installation_directory.join(crate::identity::MARKER_DIRECTORY);
    fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let final_path = directory.join(MARKER_FILENAME);
    let temporary_path = directory.join(format!("{MARKER_FILENAME}.tmp"));
    let mut file = fs::File::create(&temporary_path)
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
        primary_executable: executable,
        launch_arguments: Vec::new(),
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
        }
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ludomere-marker-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ))
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
}
