use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub product_id: i64,
    pub slug: String,
    pub title: String,
    pub release_date: Option<DateTime<FixedOffset>>,
    pub description: String,
    pub changelog: String,
    pub platforms: Platforms,
    pub features: Vec<String>,
    pub languages: Vec<String>,
    #[serde(default)]
    pub metadata: ProductMetadata,
    #[serde(default)]
    pub galaxy_builds: Vec<GalaxyBuild>,
    pub location: PathBuf,
    pub artwork: Option<PathBuf>,
    #[serde(default)]
    pub detail_artwork: Option<PathBuf>,
    #[serde(default)]
    pub hero_logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    #[serde(default)]
    pub screenshots: Vec<Screenshot>,
    #[serde(default)]
    pub links: ExternalLinks,
    pub installers: Vec<LibraryFile>,
    pub patches: Vec<LibraryFile>,
    pub extras: Vec<LibraryFile>,
    #[serde(default)]
    pub remote_artifacts: Vec<RemoteArtifact>,
    pub dlc_count: usize,
    #[serde(default)]
    pub dlcs: Vec<Dlc>,
    pub disk_usage: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dlc {
    pub product_id: i64,
    #[serde(default = "default_true")]
    pub owned: bool,
    pub slug: String,
    pub title: String,
    pub release_date: Option<DateTime<FixedOffset>>,
    pub description: String,
    pub changelog: String,
    pub platforms: Platforms,
    pub languages: Vec<String>,
    #[serde(default)]
    pub metadata: ProductMetadata,
    #[serde(default)]
    pub galaxy_builds: Vec<GalaxyBuild>,
    pub location: PathBuf,
    pub artwork: Option<PathBuf>,
    #[serde(default)]
    pub detail_artwork: Option<PathBuf>,
    #[serde(default)]
    pub hero_logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    #[serde(default)]
    pub screenshots: Vec<Screenshot>,
    #[serde(default)]
    pub links: ExternalLinks,
    pub installers: Vec<LibraryFile>,
    pub extras: Vec<LibraryFile>,
    #[serde(default)]
    pub remote_artifacts: Vec<RemoteArtifact>,
    pub disk_usage: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExternalLinks {
    pub store: Option<String>,
    pub forum: Option<String>,
    pub support: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Screenshot {
    pub id: String,
    pub thumbnail_url: String,
    pub full_url: String,
}

impl Dlc {
    pub fn is_catalog_visible(&self) -> bool {
        self.owned
            || self
                .metadata
                .store_release_status
                .as_deref()
                .is_some_and(|status| status != "unavailable")
    }

    pub fn kind(&self) -> &'static str {
        let title = self.title.to_lowercase();
        if title.contains("soundtrack") {
            "Soundtrack"
        } else if title.contains("cosmetic") || title.contains("character pack") {
            "Cosmetic pack"
        } else if title.contains("expansion") || title.contains("ancient gods") {
            "Expansion"
        } else if title.contains("level pack") {
            "Level pack"
        } else {
            "DLC"
        }
    }

    pub fn platform_label(&self) -> String {
        platform_label(&self.platforms)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Platforms {
    pub windows: bool,
    pub linux: bool,
    pub macos: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteArtifact {
    pub product_id: i64,
    pub kind: ArtifactKind,
    pub name: String,
    pub language: Option<String>,
    pub operating_system: Option<String>,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub size_label: Option<String>,
    pub size_bytes: Option<u64>,
    pub part_number: Option<u32>,
    pub part_count: Option<u32>,
    pub download_path: String,
    #[serde(default)]
    pub provider_group_id: Option<String>,
    #[serde(default)]
    pub provider_file_id: Option<String>,
    #[serde(default)]
    pub provider_category: Option<DownloadCategory>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadCategory {
    Installer,
    Patch,
    LanguagePack,
    Bonus,
}

impl DownloadCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installer => "installer",
            Self::Patch => "patch",
            Self::LanguagePack => "language_pack",
            Self::Bonus => "bonus",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductMetadata {
    pub tags: Vec<MetadataTerm>,
    pub properties: Vec<MetadataTerm>,
    pub features: Vec<MetadataTerm>,
    pub genres: Vec<MetadataTerm>,
    pub themes: Vec<MetadataTerm>,
    pub game_modes: Vec<MetadataTerm>,
    pub localizations: Vec<ProductLocalization>,
    pub developers: Vec<Company>,
    pub publishers: Vec<Company>,
    pub series: Option<Series>,
    pub editions: Vec<ProductReference>,
    pub system_requirements: Vec<SystemRequirements>,
    pub copyright: Option<String>,
    pub gamesdb_summary: Option<String>,
    #[serde(default)]
    pub gamesdb_background_url: Option<String>,
    #[serde(default)]
    pub gamesdb_artwork_url: Option<String>,
    #[serde(default)]
    pub gamesdb_horizontal_artwork_url: Option<String>,
    #[serde(default)]
    pub store_galaxy_background_url: Option<String>,
    #[serde(default)]
    pub store_wordmark_url: Option<String>,
    #[serde(default)]
    pub store_wordmark_checked: bool,
    #[serde(default)]
    pub gamesdb_media_checked: bool,
    #[serde(default)]
    pub gamesdb_media_version: u32,
    #[serde(default)]
    pub store_release_status: Option<String>,
    #[serde(default)]
    pub store_description: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataTerm {
    pub provider_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub source: MetadataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    ProductApi,
    StoreApi,
    GamesDb,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductLocalization {
    pub language_code: String,
    pub name: String,
    pub text: bool,
    pub audio: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Company {
    pub provider_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub provider_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductReference {
    pub product_id: i64,
    pub title: String,
    pub relationship: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemRequirements {
    pub operating_system: String,
    pub minimum: Option<String>,
    pub recommended: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalaxyBuild {
    pub build_id: String,
    pub product_id: i64,
    pub operating_system: String,
    pub version: Option<String>,
    pub branch: Option<String>,
    pub tags: Vec<String>,
    pub public: bool,
    pub generation: u32,
    pub repository_url: String,
    pub repository_id: Option<String>,
    pub published_at: Option<i64>,
    pub currently_returned: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationSource {
    #[default]
    OfflineInstaller,
    GalaxyDepot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepotOperationKind {
    Install,
    Update,
    BranchSwitch,
    Repair,
}

impl InstallationSource {
    pub const fn is_offline_installer(value: &Self) -> bool {
        matches!(value, Self::OfflineInstaller)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyDepotIdentity {
    pub depot_id: String,
    pub manifest_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyDepotDlcProvenance {
    pub product_id: i64,
    #[serde(default)]
    pub depots: Vec<GalaxyDepotIdentity>,
    pub has_payload: bool,
    pub entitlement_only_marker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyDepotProvenance {
    pub build_id: String,
    pub repository_id: String,
    pub manifest_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(default)]
    pub depots: Vec<GalaxyDepotIdentity>,
    #[serde(default)]
    pub dlc: Vec<GalaxyDepotDlcProvenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DownloadPart {
    pub part_id: i64,
    pub revision_id: i64,
    pub provider_file_id: String,
    pub part_index: u32,
    pub expected_size: Option<u64>,
    pub downlink: String,
    pub checksum: Option<String>,
    pub checksum_fetched_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DownloadRevision {
    pub revision_id: i64,
    pub slot_id: i64,
    pub product_id: i64,
    pub provider_group_id: String,
    pub provider_category: DownloadCategory,
    pub name: String,
    pub operating_system: Option<String>,
    pub language_code: Option<String>,
    pub language_name: Option<String>,
    pub version: Option<String>,
    pub total_size: Option<u64>,
    pub manifest_fingerprint: String,
    pub currently_offered: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub retired_at: Option<i64>,
    pub parts: Vec<DownloadPart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    Pending,
    Installing,
    Installed,
    Uninstalling,
    UninstallFailed,
    Failed,
}

impl InstallationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Uninstalling => "uninstalling",
            Self::UninstallFailed => "uninstall_failed",
            Self::Failed => "failed",
        }
    }

    pub fn from_database(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "installing" => Self::Installing,
            "installed" => Self::Installed,
            "uninstalling" => Self::Uninstalling,
            "uninstall_failed" => Self::UninstallFailed,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledGame {
    pub product_id: i64,
    pub library_id: String,
    pub installed_version: Option<String>,
    pub installation_directory: std::path::PathBuf,
    pub installer_revision_id: Option<i64>,
    pub installer_job_id: Option<String>,
    pub installer_files: Vec<std::path::PathBuf>,
    pub installer_complete: bool,
    pub installer_operating_system: Option<String>,
    pub installer_language: Option<String>,
    pub compatibility: Option<crate::compatibility::GameCompatibilityPreferences>,
    pub primary_executable: Option<std::path::PathBuf>,
    pub launch_arguments: Vec<String>,
    pub state: InstallationState,
    pub error: Option<String>,
    pub installed_at: Option<i64>,
    pub verified_at: Option<i64>,
    pub last_played_at: Option<i64>,
    pub playtime_seconds: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePreferences {
    pub product_id: i64,
    pub executable_path: Option<std::path::PathBuf>,
    pub launch_arguments: Vec<String>,
    pub compatibility: Option<crate::compatibility::GameCompatibilityPreferences>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSavePreference {
    #[default]
    Undecided,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSaveAvailability {
    #[default]
    Unknown,
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSaveDiscovery {
    pub availability: CloudSaveAvailability,
    pub locations: Vec<CloudSaveLocation>,
    pub metadata_build_id: Option<String>,
    pub checked_at: i64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSaveLocation {
    pub name: String,
    pub path: PathBuf,
    pub remote_namespace: String,
    pub user_override: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSaveStatus {
    #[default]
    NeverSynced,
    Syncing,
    Synchronized,
    Conflict,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSaveFileMetadata {
    pub location: String,
    pub relative_path: PathBuf,
    pub size: u64,
    pub modified_at: i64,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSaveConflict {
    pub location: String,
    pub relative_path: PathBuf,
    pub local: Option<CloudSaveFileMetadata>,
    pub remote: Option<CloudSaveFileMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSyncMode {
    Normal,
    ForceDownload,
    ForceUpload,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncResult {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: Vec<CloudSaveConflict>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Installer,
    Patch,
    Extra,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installer => "installer",
            Self::Patch => "patch",
            Self::Extra => "extra",
        }
    }
}

impl Game {
    pub fn release_year(&self) -> Option<i32> {
        use chrono::Datelike;
        self.release_date.as_ref().map(|date| date.year())
    }

    pub fn platform_label(&self) -> String {
        platform_label(&self.platforms)
    }
}

fn platform_label(platforms: &Platforms) -> String {
    let mut labels = Vec::new();
    if platforms.windows {
        labels.push("Windows");
    }
    if platforms.linux {
        labels.push("Linux");
    }
    if platforms.macos {
        labels.push("macOS");
    }
    labels.join(" · ")
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
