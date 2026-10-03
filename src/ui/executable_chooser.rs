use super::*;

/// Resolve a sole executable automatically; ask only when a choice or recovery is needed.
pub(super) fn prompt_for_windows_executable(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game_title: &str,
    installed: &crate::domain::InstalledGame,
    notification: &gtk::Label,
    set_busy: Rc<dyn Fn(bool)>,
    retry_launch: Rc<dyn Fn()>,
) -> bool {
    if installed.primary_executable.is_some() || installed.compatibility.is_none() {
        return false;
    }
    let session = online::account_session();
    let auth_session = auth::session();
    let epoch = model.borrow().account_epoch;
    let generation = model.borrow().detail_generation;
    let selection = (epoch, installed.product_id);
    if !model.borrow_mut().executable_selections.insert(selection) {
        return true;
    }
    let set_busy: Rc<dyn Fn(bool)> = Rc::new({
        let model = Rc::downgrade(model);
        let finished = std::cell::Cell::new(false);
        move |busy| {
            if !busy {
                if finished.replace(true) {
                    return;
                }
                if let Some(model) = model.upgrade() {
                    model.borrow_mut().executable_selections.remove(&selection);
                }
            }
            set_busy(busy);
        }
    });
    let current: Rc<dyn Fn() -> bool> = Rc::new({
        let model = Rc::downgrade(model);
        let window = window.downgrade();
        let installed = installed.clone();
        move || {
            online::account_session() == session
                && auth::session() == auth_session
                && window.upgrade().is_some_and(|window| window.is_visible())
                && model.upgrade().is_some_and(|model| {
                    let model = model.borrow();
                    !model.logout_pending
                        && model.account_epoch == epoch
                        && model.detail_generation == generation
                        && model
                            .installed_games
                            .get(&installed.product_id)
                            .is_some_and(|game| {
                                game.installation_directory == installed.installation_directory
                                    && game.primary_executable == installed.primary_executable
                            })
                })
        }
    });
    set_busy(true);
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
    dialog.set_can_close(false);
    dialog.connect_close_attempt({
        let closed = closed.clone();
        let set_busy = set_busy.clone();
        move |dialog| {
            closed.set(true);
            set_busy(false);
            dialog.force_close();
        }
    });
    dialog.connect_closed({
        let closed = closed.clone();
        let set_busy = set_busy.clone();
        move |_| {
            if !closed.replace(true) {
                set_busy(false);
            }
        }
    });
    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    let (sender, receiver) = mpsc::channel();
    let game = installed.clone();
    let title = game_title.to_owned();
    std::thread::spawn(move || {
        let result = (|| {
            let _activity = crate::profile_reset::begin_activity("finding game executable")?;
            anyhow::ensure!(
                online::account_session() == session && auth::session() == auth_session,
                "Your account changed. Try Play again."
            );
            anyhow::ensure!(
                game.installation_directory.is_dir(),
                "The game folder is unavailable. Restore it or reinstall the game, then try Play again."
            );
            Ok(crate::installation::discover_windows_executable(
                &game.installation_directory,
                game.product_id,
                &title,
            ))
        })();
        let _ = sender.send(result);
    });
    let window = window.downgrade();
    let model = Rc::downgrade(model);
    let notification = notification.clone();
    let installed = installed.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(window) = window.upgrade() else {
            set_busy(false);
            return glib::ControlFlow::Break;
        };
        if closed.get() {
            return glib::ControlFlow::Break;
        }
        if !current() {
            set_busy(false);
            if dialog.root().is_some() {
                dialog.close();
            }
            return glib::ControlFlow::Break;
        }
        let discovery = match receiver.try_recv() {
            Ok(Ok(discovery)) => discovery,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            result => {
                spinner.stop();
                spinner.set_visible(false);
                let error = match result {
                    Ok(Err(error)) => format!("{error:#}"),
                    _ => "Executable discovery stopped unexpectedly. Close this window and try Play again.".into(),
                };
                let message =
                    notifications::failure_message("Could not find game executable", &error);
                status.set_label(&message);
                notification.set_label(&message);
                dialog.present(Some(&window));
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
        let automatic = discovery.candidates.len() == 1;
        if !automatic {
            if discovery.candidates.is_empty() {
                notification.set_label(&status.label());
            }
            dialog.present(Some(&window));
        }
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
            let current = current.clone();
            let model = model.clone();
            let window = window.downgrade();
            let notification = notification.clone();
            let set_busy = set_busy.clone();
            button.connect_clicked(move |_| {
                if !current() || closed.get() {
                    set_busy(false);
                    return;
                }
                let Some(choices) = choices.upgrade() else {
                    return;
                };
                let Some(pending_dialog) = dialog.upgrade() else {
                    return;
                };
                choices.set_sensitive(false);
                status.set_label("Saving the selected executable…");
                spinner.set_visible(true);
                spinner.start();
                let mut game = game.clone();
                game.updated_at = chrono::Utc::now().timestamp();
                let saved_game = game.clone();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = (|| {
                        let _activity = crate::profile_reset::begin_activity("saving game executable")?;
                        anyhow::ensure!(online::account_session() == session && auth::session() == auth_session,
                            "Your account changed. Try Play again.");
                        let path = game.primary_executable.as_ref().unwrap();
                        anyhow::ensure!(path.is_file() && path.canonicalize()?.starts_with(game.installation_directory.canonicalize()?),
                            "The selected executable is unavailable or outside the game folder. Restore it or choose another executable.");
                        online::with_account_session(session, || {
                            anyhow::ensure!(auth::session() == auth_session, "Your account changed. Try Play again.");
                            crate::installation::save_game_preferences(&StateStore::open()?, &game)
                        })
                    })();
                    let _ = sender.send(result);
                });
                let choices = choices.clone();
                let spinner = spinner.clone();
                let status = status.clone();
                let dialog = pending_dialog.clone();
                let closed = closed.clone();
                let retry = retry.clone();
                let current = current.clone();
                let model = model.clone();
                let window = window.clone();
                let notification = notification.clone();
                let set_busy = set_busy.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if closed.get() {
                        return glib::ControlFlow::Break;
                    }
                    if !current() {
                        set_busy(false);
                        if dialog.root().is_some() {
                            dialog.close();
                        }
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
                            let Some(model) = model.upgrade() else { return glib::ControlFlow::Break; };
                            if let Some(game) = model.borrow_mut().installed_games.get_mut(&saved_game.product_id) {
                                game.primary_executable = saved_game.primary_executable.clone();
                                game.updated_at = saved_game.updated_at;
                            }
                            closed.set(true);
                            notification.set_label("Game executable saved. Starting the game…");
                            set_busy(false);
                            if dialog.root().is_some() {
                                dialog.force_close();
                            }
                            retry();
                        }
                        Err(error) => {
                            let message = notifications::failure_message("Could not save executable", &format!("{error:#}"));
                            status.set_label(&message);
                            notification.set_label(&message);
                            if dialog.root().is_none() && let Some(window) = window.upgrade() {
                                dialog.present(Some(&window));
                            }
                        }
                    }
                    glib::ControlFlow::Break
                });
            });
            if automatic {
                button.emit_clicked();
            }
        }
        glib::ControlFlow::Break
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires private HOME/all XDG, GTK display and D-Bus"]
    fn executable_resolution_saves_before_retry_and_preserves_choice_errors_and_cancel() {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(
            home.starts_with("/tmp")
                && home
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ludomere-p305-")
        );
        adw::init().unwrap();
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.label().as_deref() == Some(label)
            {
                return Some(button.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = button(&widget, label) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.ExecutableTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.present();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let notification = gtk::Label::new(None);
        let busy = Rc::new(std::cell::Cell::new(false));
        let retries = Rc::new(std::cell::Cell::new(0));
        let mut game = crate::domain::InstalledGame {
            product_id: 9305001,
            library_id: "fixture".into(),
            installed_version: None,
            installation_directory: home.join("game"),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some("windows".into()),
            installer_language: None,
            compatibility: Some(crate::compatibility::GameCompatibilityPreferences {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                prefix_slug: "fixture".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
                pending_profile: None,
            }),
            primary_executable: None,
            launch_arguments: vec!["preserved".into()],
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: None,
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 1,
            updated_at: 1,
        };
        std::fs::create_dir_all(&game.installation_directory).unwrap();
        std::fs::write(
            game.installation_directory.join("game.exe"),
            b"synthetic, never executed",
        )
        .unwrap();
        let start = |game: &crate::domain::InstalledGame| {
            model
                .borrow_mut()
                .installed_games
                .insert(game.product_id, game.clone());
            let product_id = game.product_id;
            assert!(prompt_for_windows_executable(
                &window,
                &model,
                "Fixture",
                game,
                &notification,
                Rc::new({
                    let busy = busy.clone();
                    move |value| busy.set(value)
                }),
                Rc::new({
                    let model = model.clone();
                    let busy = busy.clone();
                    let retries = retries.clone();
                    move || {
                        assert!(
                            model.borrow().installed_games[&product_id]
                                .primary_executable
                                .is_some()
                        );
                        assert!(!busy.get());
                        retries.set(retries.get() + 1);
                        // Simulate the next launch owning the flag; a delayed dialog close must not clear it.
                        busy.set(true);
                    }
                })
            ));
        };
        start(&game);
        wait(|| {
            assert!(window.visible_dialog().is_none());
            retries.get() == 1
        });
        assert_eq!(
            StateStore::open()
                .unwrap()
                .game_preferences(game.product_id)
                .unwrap()
                .unwrap()
                .executable_path,
            Some("game.exe".into())
        );
        assert_eq!(
            model.borrow().installed_games[&game.product_id].launch_arguments,
            ["preserved"]
        );

        game.product_id += 1;
        std::fs::write(game.installation_directory.join("other.exe"), b"synthetic").unwrap();
        start(&game);
        wait(|| window.visible_dialog().is_some());
        assert_eq!(retries.get(), 1);
        let dialog = window.visible_dialog().unwrap();
        let choose = button(dialog.child().unwrap().upcast_ref(), "other.exe").unwrap();
        std::fs::remove_file(game.installation_directory.join("other.exe")).unwrap();
        choose.emit_clicked();
        wait(|| notification.label().contains("Could not save executable"));
        assert_eq!(retries.get(), 1);
        assert!(
            model.borrow().installed_games[&game.product_id]
                .primary_executable
                .is_none()
        );
        assert!(choose.is_sensitive());
        std::fs::write(game.installation_directory.join("other.exe"), b"synthetic").unwrap();
        choose.emit_clicked();
        wait(|| retries.get() == 2 && window.visible_dialog().is_none());
        assert!(
            busy.get(),
            "closing the old chooser must not clear the new launch flag"
        );
        assert_eq!(
            StateStore::open()
                .unwrap()
                .game_preferences(game.product_id)
                .unwrap()
                .unwrap()
                .executable_path,
            Some("other.exe".into())
        );

        game.product_id += 1;
        start(&game);
        wait(|| window.visible_dialog().is_some());
        let dialog = window.visible_dialog().unwrap();
        button(dialog.child().unwrap().upcast_ref(), "Cancel")
            .unwrap()
            .emit_clicked();
        wait(|| window.visible_dialog().is_none() && !busy.get());
        assert_eq!(retries.get(), 2);
        assert!(
            StateStore::open()
                .unwrap()
                .game_preferences(game.product_id)
                .unwrap()
                .is_none()
        );

        for cancel_button in [true, false] {
            game.product_id += 1;
            start(&game);
            wait(|| window.visible_dialog().is_some());
            let dialog = window.visible_dialog().unwrap();
            button(dialog.child().unwrap().upcast_ref(), "game.exe")
                .unwrap()
                .emit_clicked();
            if cancel_button {
                button(dialog.child().unwrap().upcast_ref(), "Cancel")
                    .unwrap()
                    .emit_clicked();
            } else {
                // The same close-attempt path as Escape or the window close control.
                dialog.close();
            }
            assert!(!busy.get());
            wait(|| window.visible_dialog().is_none());
            let deadline = std::time::Instant::now() + Duration::from_millis(200);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                retries.get(),
                2,
                "Dismissal must suppress pending-save continuation"
            );
        }

        game.product_id += 1;
        start(&game);
        model.borrow_mut().detail_generation += 1;
        wait(|| !busy.get());
        assert!(window.visible_dialog().is_none());
        assert_eq!(retries.get(), 2);

        // A failed automatic save must present its previously hidden recovery dialog.
        game.product_id += 1;
        std::fs::remove_file(game.installation_directory.join("other.exe")).unwrap();
        let database = rusqlite::Connection::open(crate::identity::database()).unwrap();
        database.execute_batch("CREATE TRIGGER reject_executable_fixture BEFORE INSERT ON game_preferences BEGIN SELECT RAISE(FAIL, 'synthetic executable save failure'); END;").unwrap();
        notification.set_label("");
        start(&game);
        wait(|| {
            window.visible_dialog().is_some()
                && notification
                    .label()
                    .contains("synthetic executable save failure")
        });
        assert_eq!(retries.get(), 2);
        assert!(
            model.borrow().installed_games[&game.product_id]
                .primary_executable
                .is_none()
        );
        window.visible_dialog().unwrap().close();
        wait(|| !busy.get() && window.visible_dialog().is_none());
        database
            .execute_batch("DROP TRIGGER reject_executable_fixture")
            .unwrap();
        window.close();
    }
}
