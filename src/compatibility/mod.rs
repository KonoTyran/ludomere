mod backend;
pub(crate) mod comet;
mod database;
mod fixes;
mod paths;
mod process;
mod umu;

pub use backend::*;
pub use database::{UmuDatabaseEntry, profile_for_use, resolve_profile};
pub use fixes::{
    LaunchFixDefinition, LaunchFixOperation, available_fixes, effective_fixes, recommended_fix_ids,
};
pub use paths::*;
pub use process::CompatibilityProcess;
pub use umu::UmuBackend;

pub fn default_backend() -> UmuBackend {
    UmuBackend
}
