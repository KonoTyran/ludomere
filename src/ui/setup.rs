use super::*;

pub(super) fn show_setup(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>, product_id: Option<i64>) {
    if model.borrow().logout_pending {
        return;
    }
    let dialog = adw::Dialog::new();
    let closed = Rc::new(std::cell::Cell::new(false));
    let completed = Rc::new(std::cell::Cell::new(false));
    dialog.set_title("Set up Ludomere");
    dialog.set_content_width(720);
    dialog.set_content_height(700);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    let folders = adw::PreferencesGroup::new();
    folders.set_title("1. Choose your folders");
    folders.set_description(Some("Existing libraries and saved Proton choices are preserved. Folder changes are saved when you finish."));
    let games = adw::EntryRow::new();
    games.set_title("Game folder (absolute path)");
    let downloads = adw::EntryRow::new();
    downloads.set_title("Download folder (absolute path)");
    {
        let state = model.borrow();
        games.set_text(
            &state
                .config
                .installer_library()
                .map(|library| library.path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        downloads.set_text(&state.config.download_directory.to_string_lossy());
    }
    folders.add(&games);
    folders.add(&downloads);
    {
        let downloads = downloads.clone();
        games.connect_changed(move |games| {
            downloads.set_text(
                &std::path::Path::new(games.text().as_str())
                    .join("downloads")
                    .to_string_lossy(),
            );
        });
    }
    page.add(&folders);
    let (selection, refresh) = proton::proton_selection_group(&w.window, product_id);
    selection.set_title(if product_id.is_some() {
        "2. Windows game Proton choice"
    } else {
        "2. Default Proton for Windows games"
    });
    page.add(&selection);
    page.add(&proton::acquisition_group(product_id, refresh));
    let requirements = adw::PreferencesGroup::new();
    let check = gtk::Button::with_label("Check Windows requirements");
    requirements.add(&check);
    let status = gtk::Label::new(Some(
        "Check an existing Proton installation, or explicitly download missing components above. Native games do not require these components.",
    ));
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_widget_name("setup-status");
    requirements.add(&status);
    let defer = gtk::CheckButton::with_label("Set up Windows games later");
    defer.set_active(model.borrow().config.windows_setup_deferred);
    requirements.add(&defer);
    page.add(&requirements);
    for (entry, title, name) in [
        (&games, "Choose game folder", "setup-choose-games"),
        (
            &downloads,
            "Choose download folder",
            "setup-choose-downloads",
        ),
    ] {
        let choose = gtk::Button::with_label("Choose…");
        choose.set_widget_name(name);
        choose.set_tooltip_text(Some(title));
        choose.set_valign(gtk::Align::Center);
        entry.add_suffix(&choose);
        let entry = entry.clone();
        let window = w.window.clone();
        let model = model.clone();
        let closed = closed.clone();
        let status = status.clone();
        choose.connect_clicked(move |button| {
            let epoch = model.borrow().account_epoch;
            let chooser = gtk::FileDialog::builder().title(title).build();
            if std::path::Path::new(entry.text().as_str()).is_absolute() {
                chooser.set_initial_folder(Some(&gio::File::for_path(entry.text().as_str())));
            }
            button.set_sensitive(false);
            let button = button.clone();
            let entry = entry.clone();
            let closed = closed.clone();
            let model = model.clone();
            let status = status.clone();
            chooser.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                button.set_sensitive(true);
                if closed.get()
                    || model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                {
                    return;
                }
                match result {
                    Ok(folder) => match folder.path() {
                        Some(path) => entry.set_text(&path.to_string_lossy()),
                        None => status.set_label("Choose a local folder."),
                    },
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled) => {}
                    Err(_) => status.set_label(
                        "The folder picker could not open. Try again or enter an absolute path.",
                    ),
                }
            });
        });
    }
    let next_step = gtk::Label::new(Some(
        "After these settings are saved, optional GOG sign-in opens separately. Close that window to skip sign-in.",
    ));
    next_step.set_wrap(true);
    requirements.add(&next_step);
    root.append(&page);
    page.set_vexpand(true);
    let finish = gtk::Button::with_label("Save settings and continue");
    finish.add_css_class("suggested-action");
    finish.set_margin_start(18);
    finish.set_margin_end(18);
    finish.set_margin_bottom(18);
    root.append(&finish);
    dialog.set_child(Some(&root));
    {
        let status = status.clone();
        let closed = closed.clone();
        let model = model.clone();
        check.connect_clicked(move |button| {
            let epoch = model.borrow().account_epoch;
            button.set_sensitive(false);
            status.set_label("Checking Windows requirements…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(crate::compatibility::preflight_windows(product_id));
            });
            let button = button.clone();
            let status = status.clone();
            let model = model.clone();
            let closed = closed.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if closed.get() || model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    button.set_sensitive(true);
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                Ok(result) => {
                    button.set_sensitive(true);
                    status.set_label(&match result {
                        Ok(()) => "Windows requirements are ready. Saved choices will be used automatically.".to_string(),
                        Err(error) => proton::compatibility_message(&error),
                    });
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    button.set_sensitive(true);
                    status.set_label("The check stopped. Try again.");
                    glib::ControlFlow::Break
                }
                }
            });
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let dialog = dialog.clone();
        let completed = completed.clone();
        let closed = closed.clone();
        finish.connect_clicked(move |button| {
            let epoch = model.borrow().account_epoch;
            let games = std::path::PathBuf::from(games.text().as_str());
            let downloads = std::path::PathBuf::from(downloads.text().as_str());
            let deferred = defer.is_active();
            button.set_sensitive(false);
            dialog.set_can_close(false);
            status.set_label("Saving setup…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
                    anyhow::ensure!(
                        games.is_absolute() && downloads.is_absolute(),
                        "Choose absolute game and download folder paths."
                    );
                    if !deferred {
                        crate::compatibility::preflight_windows(product_id).map_err(|error| {
                            anyhow::anyhow!(proton::compatibility_message(&error))
                        })?;
                    }
                    std::fs::create_dir_all(&games)?;
                    std::fs::create_dir_all(&downloads)?;
                    Ok((games, downloads))
                })();
                let _ = sender.send(result);
            });
            let w = w.clone();
            let model = model.clone();
            let dialog = dialog.clone();
            let status = status.clone();
            let button = button.clone();
            let completed = completed.clone();
            let closed = closed.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if closed.get() || model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    dialog.set_can_close(true);
                    dialog.close();
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok((games, downloads))) => {
                        let mut config = model.borrow().config.clone();
                        let id = config
                            .game_libraries
                            .iter()
                            .find(|library| library.path == games)
                            .map(|library| library.id.clone())
                            .unwrap_or_else(|| crate::config::game_library_id(&games));
                        if !config
                            .game_libraries
                            .iter()
                            .any(|library| library.id == id)
                        {
                            config
                                .game_libraries
                                .push(crate::config::GameLibrary {
                                    id: id.clone(),
                                    name: "Games".to_string(),
                                    path: games,
                                    default: false,
                                });
                        }
                        config.installer_library_id = Some(id);
                        config.download_directory = downloads;
                        config.setup_seen = true;
                        config.setup_completed = true;
                        config.windows_setup_deferred = deferred;
                        if let Err(error) = config.save() {
                            status.set_label(&format!("Settings could not be saved: {error}. Your edits are still here; try again."));
                            button.set_sensitive(true);
                            dialog.set_can_close(true);
                            return glib::ControlFlow::Break;
                        }
                        model.borrow_mut().config = config;
                        completed.set(true);
                        w.finish_setup.set_visible(deferred);
                        dialog.set_can_close(true);
                        dialog.close();
                        let signed_in = model.borrow().account_token.as_ref().is_some_and(|token| token.expires_at > chrono::Utc::now().timestamp());
                        if !signed_in {
                            show_gog_login(&w, &model);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        status.set_label(&format!("Setup was not saved: {error}"));
                        button.set_sensitive(true);
                        dialog.set_can_close(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => {
                        status.set_label("Setup stopped before completion. Try again.");
                        button.set_sensitive(true);
                        dialog.set_can_close(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }
    let window = w.window.clone();
    let model = model.clone();
    let w = w.clone();
    dialog.connect_closed(move |_| {
        closed.set(true);
        if completed.get() {
            return;
        }
        let config = {
            let mut state = model.borrow_mut();
            if state.logout_pending {
                return;
            }
            state.config.setup_seen = true;
            state.config.clone()
        };
        if config.save().is_err() {
            w.status.set_label(
                "Setup preferences could not be saved. Open Finish setup and try again.",
            );
            w.finish_setup.set_visible(true);
        }
    });
    dialog.present(Some(&window));
}
