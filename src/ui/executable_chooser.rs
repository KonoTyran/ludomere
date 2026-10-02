use super::*;

/// Keep the explicit launch prompt open while discovery and persistence run off GTK.
pub(super) fn prompt_for_windows_executable(
    window: &adw::ApplicationWindow,
    game_title: &str,
    installed: &crate::domain::InstalledGame,
    retry_launch: Rc<dyn Fn()>,
) -> bool {
    if installed.primary_executable.is_some() || installed.compatibility.is_none() {
        return false;
    }
    let session = online::account_session();
    let auth_session = auth::session();
    let dialog = adw::Dialog::builder()
        .title("Select the game executable")
        .content_width(520)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);
    let status = gtk::Label::new(Some("Looking for game executables…"));
    status.set_wrap(true);
    status.set_selectable(true);
    content.append(&status);
    let spinner = gtk::Spinner::new();
    spinner.start();
    content.append(&spinner);
    let choices = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let scroll = gtk::ScrolledWindow::builder()
        .max_content_height(360)
        .propagate_natural_height(true)
        .child(&choices)
        .build();
    content.append(&scroll);
    let cancel = gtk::Button::with_label("Cancel");
    content.append(&cancel);
    dialog.set_child(Some(&content));
    let closed = Rc::new(std::cell::Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    dialog.present(Some(window));
    let (sender, receiver) = mpsc::channel();
    let game = installed.clone();
    let title = game_title.to_owned();
    std::thread::spawn(move || {
        let _ = sender.send(crate::installation::discover_windows_executable(
            &game.installation_directory,
            game.product_id,
            &title,
        ));
    });
    let dialog = dialog.downgrade();
    let installed = installed.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(dialog) = dialog.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if closed.get() {
            return glib::ControlFlow::Break;
        }
        if online::account_session() != session || auth::session() != auth_session {
            dialog.close();
            return glib::ControlFlow::Break;
        }
        let discovery = match receiver.try_recv() {
            Ok(discovery) => discovery,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                spinner.stop();
                spinner.set_visible(false);
                status.set_label("Executable discovery stopped unexpectedly. Close this window and try Play again.");
                return glib::ControlFlow::Break;
            }
        };
        spinner.stop();
        spinner.set_visible(false);
        status.set_label(if discovery.candidates.is_empty() {
            "No likely game executable was found. Choose one later in Game Settings."
        } else {
            "Select the executable that starts the game. This choice will be saved."
        });
        for candidate in discovery.candidates.into_iter().take(12) {
            let button = gtk::Button::with_label(
                &candidate
                    .path
                    .strip_prefix(&installed.installation_directory)
                    .unwrap_or(&candidate.path)
                    .to_string_lossy(),
            );
            button.set_tooltip_text(Some(&candidate.path.to_string_lossy()));
            choices.append(&button);
            let mut game = installed.clone();
            game.primary_executable = Some(candidate.path);
            let choices = choices.downgrade();
            let spinner = spinner.clone();
            let status = status.clone();
            let dialog = dialog.downgrade();
            let closed = closed.clone();
            let retry = retry_launch.clone();
            button.connect_clicked(move |_| {
                let Some(choices) = choices.upgrade() else {
                    return;
                };
                choices.set_sensitive(false);
                status.set_label("Saving the selected executable…");
                spinner.set_visible(true);
                spinner.start();
                let mut game = game.clone();
                game.updated_at = chrono::Utc::now().timestamp();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = online::with_account_session(session, || {
                        crate::installation::save_game_preferences(&StateStore::open()?, &game)
                    });
                    let _ = sender.send(result);
                });
                let choices = choices.clone();
                let spinner = spinner.clone();
                let status = status.clone();
                let dialog = dialog.clone();
                let closed = closed.clone();
                let retry = retry.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    let Some(dialog) = dialog.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if closed.get() {
                        return glib::ControlFlow::Break;
                    }
                    if online::account_session() != session || auth::session() != auth_session {
                        dialog.close();
                        return glib::ControlFlow::Break;
                    }
                    let result = match receiver.try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!(
                            "Saving stopped unexpectedly. Select an executable to try again."
                        )),
                    };
                    spinner.stop();
                    spinner.set_visible(false);
                    choices.set_sensitive(true);
                    match result {
                        Ok(()) => {
                            dialog.close();
                            retry();
                        }
                        Err(error) => {
                            status.set_label(&format!("Could not save executable: {error}"))
                        }
                    }
                    glib::ControlFlow::Break
                });
            });
        }
        glib::ControlFlow::Break
    });
    true
}
