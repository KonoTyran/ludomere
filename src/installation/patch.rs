use crate::{
    compatibility::{CompatibilityBackend, CompatibilityRunRequest},
    domain::InstalledGame,
};
use anyhow::{Context, Result, bail};
use std::{path::PathBuf, sync::mpsc, thread};

#[derive(Debug)]
pub enum PatchEvent {
    Started { log_path: PathBuf },
    Complete { exit_code: Option<i32> },
    Failed(String),
}

pub fn patch_target_version(label: Option<&str>) -> Option<String> {
    let label = label?.trim();
    for separator in [" to ", " → ", " -> "] {
        if let Some((_, target)) = label.rsplit_once(separator) {
            return Some(target.trim().trim_matches(['(', ')']).to_owned());
        }
    }
    Some(label.trim_matches(['(', ')']).to_owned())
}

pub fn run_patch(
    game: InstalledGame,
    patch: PathBuf,
    target_version: Option<String>,
) -> mpsc::Receiver<PatchEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        if let Err(error) = run_patch_worker(&game, &patch, target_version.as_deref(), &sender) {
            sender.send(PatchEvent::Failed(format!("{error:#}"))).ok();
        }
    });
    receiver
}

fn run_patch_worker(
    game: &InstalledGame,
    patch: &std::path::Path,
    target_version: Option<&str>,
    events: &mpsc::Sender<PatchEvent>,
) -> Result<()> {
    let mut game = game.clone();
    if !patch.is_file() {
        bail!("patch file is missing: {}", patch.display());
    }
    if !patch
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
    {
        bail!("the selected patch is not a Windows executable");
    }
    let compatibility = game
        .compatibility
        .as_ref()
        .context("this installation has no UMU compatibility environment")?
        .clone();
    let profile = crate::compatibility::profile_for_use(game.product_id, &compatibility.profile);
    if profile != compatibility.profile {
        game.compatibility.as_mut().unwrap().profile = profile.clone();
        game.updated_at = chrono::Utc::now().timestamp();
    }
    let library = game
        .installation_directory
        .parent()
        .context("installation directory has no library root")?;
    let prefix = crate::compatibility::prefix_path(library, &compatibility.prefix_slug);
    crate::compatibility::validate_ownership(&prefix, &compatibility.prefix_slug)?;
    crate::compatibility::configure_library_drive(&prefix, library)?;

    let log_path = super::patch_log_path(game.product_id)?;
    std::fs::File::create(&log_path)?;
    events
        .send(PatchEvent::Started {
            log_path: log_path.clone(),
        })
        .ok();
    let backend = crate::compatibility::default_backend();
    let mut process = backend.run_executable(CompatibilityRunRequest {
        prefix,
        profile,
        executable: patch.to_path_buf(),
        arguments: crate::compatibility::inno_patch_arguments(game.installer_language.as_deref()),
        working_directory: Some(game.installation_directory.clone()),
        log_path,
    })?;
    let status = process.wait()?;
    if !status.success() {
        bail!("patch exited unsuccessfully: {status}");
    }
    super::marker::record_successful_patch(&game, target_version)?;
    super::save_game_preferences(&crate::state::StateStore::open()?, &game)?;
    events
        .send(PatchEvent::Complete {
            exit_code: status.code(),
        })
        .ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_patch_target_version() {
        assert_eq!(
            patch_target_version(Some("1.3.0.5 to 1.3.0.6")).as_deref(),
            Some("1.3.0.6")
        );
        assert_eq!(
            patch_target_version(Some("1.3.0.6")).as_deref(),
            Some("1.3.0.6")
        );
    }
}
