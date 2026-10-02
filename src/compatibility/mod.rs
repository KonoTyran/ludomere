pub mod acquisition;
mod backend;
pub(crate) mod comet;
pub mod components;
mod database;
mod dll_overrides;
mod fixes;
mod paths;
mod process;
mod proton;
mod umu;

pub use backend::*;
pub use database::{UmuDatabaseEntry, profile_for_use, resolve_profile};
pub use dll_overrides::{DllLoadOrder, apply_game_dll_overrides, normalize_dll_overrides};
pub use fixes::{
    LaunchFixDefinition, LaunchFixOperation, available_fixes, effective_fixes, recommended_fix_ids,
};
pub use paths::*;
pub use process::CompatibilityProcess;
pub(crate) use process::append_step_log;
pub use proton::{
    ProtonFamily, ProtonInstallation, ProtonPreferences, discover_proton, game_dll_overrides,
    proton_preferences, proton_preferences_generation, reset_invalid_proton_preferences,
    select_proton, set_default_proton, set_game_dll_overrides, set_game_proton, validate_proton,
};
pub use umu::UmuBackend;

pub fn backend_for_game(product_id: i64) -> Result<UmuBackend> {
    let proton = select_proton(Some(product_id))?;
    check_prerequisites(&proton.path)?;
    Ok(UmuBackend::new(proton.path))
}

pub fn preflight_windows(product_id: Option<i64>) -> Result<()> {
    check_prerequisites(&select_proton(product_id)?.path)
}

fn check_prerequisites(proton: &std::path::Path) -> Result<()> {
    validate_proton(proton)?;
    if let Some(runtime) = acquisition::runtime_requirement(proton)
        .map_err(|error| CompatibilityFailure::Io(error.to_string()))?
        && !acquisition::runtime_ready(&runtime)
    {
        return Err(CompatibilityFailure::RuntimeMissing(runtime.name.into()));
    }
    if !UmuBackend::executable().is_file() {
        return Err(CompatibilityFailure::UmuUnavailable);
    }
    Ok(())
}
