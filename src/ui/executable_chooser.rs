use super::*;

/// Prompt for a Windows executable when metadata and confident ranking could
/// not select one. Returns true when launch should pause for the prompt.
pub(super) fn prompt_for_windows_executable(
    window: &adw::ApplicationWindow,
    game_title: &str,
    installed: &crate::domain::InstalledGame,
    retry_launch: Rc<dyn Fn()>,
) -> bool {
    if installed.primary_executable.is_some() || installed.compatibility.is_none() {
        return false;
    }
    let discovery = crate::installation::discover_windows_executable(
        &installed.installation_directory,
        installed.product_id,
        game_title,
    );
    let candidates = discovery
        .candidates
        .into_iter()
        .take(12)
        .collect::<Vec<_>>();
    let dialog = adw::AlertDialog::builder()
        .heading("Select the game executable")
        .body(if candidates.is_empty() {
            "Ludomere could not identify a likely game executable. Choose one later in Game Settings."
        } else {
            "Ludomere could not choose confidently. Select the executable that starts the game. This choice will be saved."
        })
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.set_close_response("cancel");
    for (index, candidate) in candidates.iter().enumerate() {
        let relative = candidate
            .path
            .strip_prefix(&installed.installation_directory)
            .unwrap_or(&candidate.path)
            .to_string_lossy();
        dialog.add_response(&format!("candidate-{index}"), &relative);
    }
    let window = window.clone();
    let callback_window = window.clone();
    let installed = installed.clone();
    dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
        let Some(index) = response
            .strip_prefix("candidate-")
            .and_then(|value| value.parse::<usize>().ok())
        else {
            return;
        };
        let Some(candidate) = candidates.get(index) else {
            return;
        };
        let mut updated = installed.clone();
        updated.primary_executable = Some(candidate.path.clone());
        updated.updated_at = chrono::Utc::now().timestamp();
        match StateStore::open()
            .and_then(|store| crate::installation::save_game_preferences(&store, &updated))
        {
            Ok(()) => retry_launch(),
            Err(error) => {
                let failure = adw::AlertDialog::builder()
                    .heading("Could not save executable")
                    .body(error.to_string())
                    .build();
                failure.add_response("close", "Close");
                failure.present(Some(&callback_window));
            }
        }
    });
    true
}
