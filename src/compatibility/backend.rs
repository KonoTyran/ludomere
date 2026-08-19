use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

pub type Result<T> = std::result::Result<T, CompatibilityFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityBackendKind {
    Umu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityBackendStatus {
    pub kind: CompatibilityBackendKind,
    pub available: bool,
    pub version: Option<String>,
    pub healthy: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UmuProfileSource {
    GogProductId,
    DefaultFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmuProfile {
    pub game_id: String,
    pub store: String,
    pub source: UmuProfileSource,
}
impl UmuProfile {
    pub fn fallback() -> Self {
        Self {
            game_id: "umu-default".into(),
            store: "gog".into(),
            source: UmuProfileSource::DefaultFallback,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameCompatibilityPreferences {
    pub backend: CompatibilityBackendKind,
    pub prefix_slug: String,
    pub profile: UmuProfile,
    pub pending_profile: Option<UmuProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityPrefix {
    pub library_id: String,
    pub relative_path: PathBuf,
    pub managed_by_ludomere: bool,
}
pub struct InitializePrefixRequest {
    pub library_id: String,
    pub library: PathBuf,
    pub slug: String,
    pub profile: UmuProfile,
    pub log_path: PathBuf,
}
pub struct CompatibilityRunRequest {
    pub prefix: PathBuf,
    pub profile: UmuProfile,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub log_path: PathBuf,
    pub background: bool,
}

pub trait CompatibilityBackend: Send + Sync {
    fn status(&self) -> Result<CompatibilityBackendStatus>;
    fn initialize_prefix(&self, request: InitializePrefixRequest) -> Result<CompatibilityPrefix>;
    fn run_executable(
        &self,
        request: CompatibilityRunRequest,
    ) -> Result<super::CompatibilityProcess>;
    fn stop(&self, process: &mut super::CompatibilityProcess) -> Result<()>;
}

#[derive(Debug)]
pub enum CompatibilityFailure {
    UmuUnavailable,
    UmuVersionIncompatible(String),
    UmuUnhealthy,
    RuntimePreparationFailed,
    RuntimeDownloadFailed,
    ProfileDatabaseUnavailable,
    ProfileDatabaseInvalid,
    PrefixConflict(PathBuf),
    PrefixOwnershipAmbiguous(PathBuf),
    PrefixInitializationFailed,
    PrefixMissing(PathBuf),
    PrefixCorrupt(PathBuf),
    LibraryUnavailable(PathBuf),
    LibraryNotWritable(PathBuf),
    DriveMappingConflict,
    DriveMappingFailed,
    DriveMappingMismatch,
    InstallerMissing(PathBuf),
    InstallerLaunchRejected,
    InstallerExitedUnsuccessfully(Option<i32>),
    InstallerDestinationMismatch,
    ExecutableDiscoveryAmbiguous,
    ExecutableMissing(PathBuf),
    GameLaunchRejected,
    StopFailed,
    PrefixDeletionFailed,
    Io(String),
}
impl fmt::Display for CompatibilityFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CompatibilityFailure {}
impl From<std::io::Error> for CompatibilityFailure {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
