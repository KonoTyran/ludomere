use super::*;
use std::cell::Cell;

const STEPS: [&str; 5] = [
    "Welcome",
    "Game folder",
    "Download folder",
    "Select Proton Version",
    "Verify Steam Linux Runtime",
];

fn validate_folder(path: &std::path::Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.is_absolute(),
        "Enter an absolute folder path or choose a folder."
    );
    for ancestor in path.ancestors() {
        match std::fs::metadata(ancestor) {
            Ok(metadata) => {
                anyhow::ensure!(metadata.is_dir(), "{} is not a folder.", ancestor.display());
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("The folder location is unavailable.")
}

pub(super) fn show_setup(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>, product_id: Option<i64>) {
    if model.borrow().logout_pending {
        return;
    }
    let dialog = adw::Dialog::new();
    let epoch = model.borrow().account_epoch;
    let closed = Rc::new(Cell::new(false));
    let completed = Rc::new(Cell::new(false));
    let step = Rc::new(Cell::new(0usize));
    let busy = Rc::new(Cell::new(false));
    dialog.set_widget_name("setup-wizard");
    dialog.set_title("Welcome to Ludomere");
    dialog.set_content_width(720);
    dialog.set_content_height(700);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&adw::HeaderBar::new());
    let position = gtk::Label::new(None);
    position.set_widget_name("setup-step");
    position.set_margin_bottom(12);
    root.append(&position);
    let pages = gtk::Stack::new();
    pages.set_vexpand(true);
    pages.set_vhomogeneous(false);
    let welcome = adw::StatusPage::builder()
        .title("Welcome to Ludomere")
        .description("Let's make a home for your games. Choose your folders, then get Windows support ready if you need it. You can skip setup and return anytime.")
        .icon_name("applications-games-symbolic")
        .build();
    pages.add_named(&welcome, Some("0"));
    let game_page = adw::PreferencesPage::new();
    pages.add_named(&game_page, Some("1"));
    let download_page = adw::PreferencesPage::new();
    pages.add_named(&download_page, Some("2"));
    let proton_page = adw::PreferencesPage::new();
    pages.add_named(&proton_page, Some("3"));
    let runtime_page = adw::PreferencesPage::new();
    pages.add_named(&runtime_page, Some("4"));
    let folders = adw::PreferencesGroup::new();
    folders.set_title("Where should your games live?");
    folders.set_description(Some(
        "Choose the directory where your games will be installed.",
    ));
    let games = adw::EntryRow::new();
    games.set_title("Game folder (absolute path)");
    let downloads = adw::EntryRow::new();
    downloads.set_title("Download folder (absolute path)");
    games.set_widget_name("setup-game-folder");
    downloads.set_widget_name("setup-download-folder");
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
    game_page.add(&folders);
    let download_folders = adw::PreferencesGroup::new();
    download_folders.set_title("Where should downloads go?");
    download_folders.set_description(Some(
        "Keep installers and extras here. Changing this folder won't change your game folder.",
    ));
    download_folders.add(&downloads);
    download_page.add(&download_folders);
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
    let active: Rc<dyn Fn() -> bool> = Rc::new({
        let model = model.clone();
        let closed = closed.clone();
        move || {
            !closed.get() && model.borrow().account_epoch == epoch && !model.borrow().logout_pending
        }
    });
    let acquisition_busy = Rc::new(Cell::new(false));
    let proton::ProtonSelection {
        group: selection,
        refresh,
        busy: selection_busy,
        detected,
        selected_path,
    } = proton::proton_selection_group_guarded(
        &w.window,
        product_id,
        Some(active.clone()),
        Some(acquisition_busy.clone()),
        false,
    );
    selection.set_title("Select Proton Version");
    selection.set_description(Some(if product_id.is_some() {
        "Select the version of Proton to use for this Windows game. Changes are saved automatically; other games keep their existing choices."
    } else {
        "Select the version of Proton you wish to use by default below. This version will be used to launch all Windows games. You may override this choice on a per-game basis later."
    }));
    proton_page.add(&selection);
    let proton_download = proton::acquisition_group_guarded(
        product_id,
        refresh.clone(),
        proton::ComponentScope::Proton,
        acquisition_busy,
        Some(selection_busy.clone()),
    );
    proton_download.group.set_title("Download Proton");
    proton_download.group.set_description(Some("No Proton versions have been detected on your system. You may browse for and download Proton runtimes here."));
    proton_download.group.set_visible(false);
    proton_page.add(&proton_download.group);
    let runtime_download =
        proton::acquisition_group(product_id, refresh, proton::ComponentScope::Runtime);
    runtime_download.runtime_button.set_sensitive(false);
    runtime_download
        .group
        .set_title("Verify Steam Linux Runtime");
    runtime_download.group.set_description(Some("Proton also needs a Steam Linux Runtime. If it has not been detected on your system, click the button below to download it."));
    runtime_page.add(&runtime_download.group);
    let status = gtk::Label::new(None);
    status.set_widget_name("setup-status");
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_selectable(true);
    status.set_margin_start(18);
    status.set_margin_end(18);
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
    root.append(&pages);
    root.append(&status);
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    navigation.set_margin_top(12);
    navigation.set_margin_start(18);
    navigation.set_margin_end(18);
    navigation.set_margin_bottom(18);
    let skip = gtk::Button::with_label("Skip for now");
    skip.set_widget_name("setup-skip");
    let back = gtk::Button::with_label("Back");
    back.set_widget_name("setup-back");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let finish = gtk::Button::with_label("Save settings and continue");
    finish.set_widget_name("setup-next");
    finish.add_css_class("suggested-action");
    navigation.append(&skip);
    navigation.append(&spacer);
    navigation.append(&back);
    navigation.append(&finish);
    root.append(&navigation);
    dialog.set_child(Some(&root));
    let render: Rc<dyn Fn()> = Rc::new({
        let step = step.clone();
        let pages = pages.clone();
        let position = position.clone();
        let finish = finish.clone();
        let back = back.clone();
        let status = status.clone();
        move || {
            pages.set_visible_child_name(&step.get().to_string());
            position.set_label(&format!(
                "Step {} of {} — {}",
                step.get() + 1,
                STEPS.len(),
                STEPS[step.get()]
            ));
            back.set_visible(step.get() > 0);
            finish.set_label(if step.get() == 0 {
                "Let's get started"
            } else if step.get() == STEPS.len() - 1 {
                "Save settings and continue"
            } else {
                "Next"
            });
            status.set_label("");
        }
    });
    render();
    let (check_runtime, checking_runtime) = proton::runtime_check(
        product_id,
        &runtime_download,
        Rc::new({
            let active = active.clone();
            let step = step.clone();
            move || active() && step.get() == 4
        }),
    );
    let components_busy: Rc<dyn Fn() -> bool> = Rc::new({
        let proton_busy = proton_download.busy.clone();
        let runtime_busy = runtime_download.busy.clone();
        let step = step.clone();
        let checking = checking_runtime.clone();
        let selection_busy = selection_busy.clone();
        move || {
            proton_busy.get()
                || runtime_busy.get()
                || checking.get()
                || (step.get() == 3 && selection_busy.get())
        }
    });
    skip.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    back.connect_clicked({
        let step = step.clone();
        let busy = busy.clone();
        let active = active.clone();
        let components_busy = components_busy.clone();
        let render = render.clone();
        move |_| {
            if active() && !busy.get() && !components_busy() {
                step.set(step.get().saturating_sub(1));
                render();
            }
        }
    });
    glib::timeout_add_local(Duration::from_millis(100), {
        let active = active.clone();
        let closed = closed.clone();
        let dialog = dialog.clone();
        let busy = busy.clone();
        let components_busy = components_busy.clone();
        let finish = finish.clone();
        let back = back.clone();
        let skip = skip.clone();
        let step = step.clone();
        let proton_group = proton_download.group.clone();
        let runtime_busy = runtime_download.busy.clone();
        let mut last_step = step.get();
        let mut was_downloading = false;
        let runtime_succeeded = runtime_download.succeeded.clone();
        let proton_busy = proton_download.busy.clone();
        let selection = selection.clone();
        let selection_busy = selection_busy.clone();
        move || {
            if closed.get() {
                return glib::ControlFlow::Break;
            }
            selection.set_sensitive(!proton_busy.get());
            proton_group.set_sensitive(!selection_busy.get());
            proton_group.set_visible(proton_busy.get() || detected.get() == Some(false));
            if step.get() == 4
                && (last_step != 4
                    || (was_downloading && !runtime_busy.get() && runtime_succeeded.get()))
            {
                check_runtime();
            }
            last_step = step.get();
            was_downloading = runtime_busy.get();
            if !active() {
                dialog.set_can_close(true);
                dialog.close();
                return glib::ControlFlow::Break;
            }
            let enabled = !busy.get() && !components_busy();
            finish.set_sensitive(enabled);
            back.set_sensitive(enabled);
            skip.set_sensitive(!busy.get());
            glib::ControlFlow::Continue
        }
    });
    {
        let w = w.clone();
        let model = model.clone();
        let dialog = dialog.clone();
        let completed = completed.clone();
        let closed = closed.clone();
        let active = active.clone();
        let busy = busy.clone();
        finish.connect_clicked(move |button| {
            if !active() || busy.get() || components_busy() {
                return;
            }
            if step.get() == 0 {
                step.set(1);
                render();
                return;
            }
            if step.get() < STEPS.len() - 1 {
                let current = step.get();
                let path = std::path::PathBuf::from(if current == 1 { games.text() } else { downloads.text() }.as_str());
                let proton_path = selected_path();
                busy.set(true);
                pages.set_sensitive(false);
                button.set_sensitive(false);
                status.set_label("Checking your choice…");
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = if current < 3 {
                        validate_folder(&path)
                    } else {
                        (|| -> anyhow::Result<()> {
                            if let Some(path) = proton_path? {
                                let preferences = crate::compatibility::proton_preferences()?;
                                let saved = product_id
                                    .and_then(|id| preferences.overrides.get(&id.to_string()))
                                    .or(preferences.default.as_ref());
                                if saved != Some(&path) {
                                    match product_id {
                                        Some(id) => crate::compatibility::set_game_proton(id, Some(&path))?,
                                        None => crate::compatibility::set_default_proton(&path)?,
                                    }
                                }
                            }
                            proton::saved_proton(product_id).map(|_| ())
                        })()
                    };
                    let _ = sender.send(result);
                });
                let active = active.clone(); let busy = busy.clone(); let pages = pages.clone();
                let status = status.clone(); let step = step.clone(); let render = render.clone();
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    if !active() { return glib::ControlFlow::Break; }
                    match receiver.try_recv() {
                        Ok(result) => {
                            busy.set(false); pages.set_sensitive(true);
                            match result {
                                Ok(()) => { step.set(current + 1); render(); }
                                Err(error) => status.set_label(&format!("Please check this step: {error}")),
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            busy.set(false); pages.set_sensitive(true);
                            status.set_label("The check stopped. Your choices are still here; try again.");
                            glib::ControlFlow::Break
                        }
                    }
                });
                return;
            }
            let games = std::path::PathBuf::from(games.text().as_str());
            let downloads = std::path::PathBuf::from(downloads.text().as_str());
            button.set_sensitive(false);
            busy.set(true);
            pages.set_sensitive(false);
            dialog.set_can_close(false);
            status.set_label("Saving setup…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
                    validate_folder(&games)?;
                    validate_folder(&downloads)?;
                    crate::compatibility::preflight_windows(product_id).map_err(|error| {
                        anyhow::anyhow!(proton::compatibility_message(&error))
                    })?;
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
            let busy = busy.clone();
            let pages = pages.clone();
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
                        config.windows_setup_deferred = false;
                        if let Err(error) = config.save() {
                            status.set_label(&format!("Settings could not be saved: {error}. Your edits are still here; try again."));
                            button.set_sensitive(true);
                            busy.set(false);
                            pages.set_sensitive(true);
                            dialog.set_can_close(true);
                            return glib::ControlFlow::Break;
                        }
                        model.borrow_mut().config = config;
                        completed.set(true);
                        w.finish_setup.set_visible(false);
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
                        busy.set(false);
                        pages.set_sensitive(true);
                        dialog.set_can_close(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => {
                        status.set_label("Setup stopped before completion. Try again.");
                        button.set_sensitive(true);
                        busy.set(false);
                        pages.set_sensitive(true);
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
        proton_download
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        runtime_download
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        if completed.get() {
            return;
        }
        let config = {
            let mut state = model.borrow_mut();
            if state.logout_pending || state.account_epoch != epoch {
                return;
            }
            state.config.setup_seen = true;
            state.config.clone()
        };
        if !config.setup_completed || config.windows_setup_deferred {
            w.finish_setup.set_visible(true);
        }
        if config.save().is_err() {
            w.status.set_label(
                "Setup preferences could not be saved. Open Finish setup and try again.",
            );
            w.finish_setup.set_visible(true);
        }
    });
    dialog.present(Some(&window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires private HOME/all XDG"]
    fn runtime_detection_uses_saved_manifest_without_saving_or_running_helpers() {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p166-")
        );
        let root = tempfile::tempdir().unwrap();
        let proton = root.path().join("GE-Proton-inert");
        std::fs::create_dir_all(proton.join("files/bin")).unwrap();
        for name in ["proton", "files/bin/wine"] {
            std::fs::write(proton.join(name), "inert; never execute\n").unwrap();
            std::fs::set_permissions(proton.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let manifest = proton.join("toolmanifest.vdf");
        std::fs::write(&manifest, "manifest {}").unwrap();
        assert!(proton::runtime_readiness(None).is_err());
        crate::compatibility::set_default_proton(&proton).unwrap();
        let preferences = crate::identity::config_root().join("proton.json");
        let original = std::fs::read(&preferences).unwrap();
        let modified = std::fs::metadata(&preferences).unwrap().modified().unwrap();
        assert_eq!(proton::runtime_readiness(None).unwrap(), None);
        std::fs::write(&manifest, "manifest { require_tool_appid 4183110 }").unwrap();
        assert_eq!(proton::runtime_readiness(None).unwrap(), Some(false));
        let runtime = crate::compatibility::acquisition::runtime_requirement(&proton)
            .unwrap()
            .unwrap();
        for file in [
            ".installed.ok",
            "_v2-entry-point",
            "toolmanifest.vdf",
            "pressure-vessel/bin/pv-verify",
            "VERSIONS.txt",
        ] {
            let path = runtime.path.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "inert metadata fixture").unwrap();
        }
        std::fs::create_dir_all(runtime.path.join("steamrt4_platform_fixture/files")).unwrap();
        assert_eq!(proton::runtime_readiness(None).unwrap(), Some(true));
        std::fs::remove_file(runtime.path.join(".installed.ok")).unwrap();
        assert_eq!(proton::runtime_readiness(None).unwrap(), Some(false));
        std::fs::write(&manifest, "manifest { require_tool_appid 999 }").unwrap();
        assert!(proton::runtime_readiness(None).is_err());
        assert_eq!(std::fs::read(&preferences).unwrap(), original);
        assert_eq!(
            std::fs::metadata(&preferences).unwrap().modified().unwrap(),
            modified
        );
    }

    #[test]
    fn folder_step_accepts_existing_or_new_directories_without_creating_them() {
        let root = tempfile::tempdir().unwrap();
        let draft = root.path().join("New library with spaces/downloads");
        validate_folder(root.path()).unwrap();
        validate_folder(&draft).unwrap();
        assert!(!draft.exists());
    }

    #[test]
    fn folder_step_rejects_relative_paths_and_file_ancestors_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("not a folder");
        std::fs::write(&file, b"preserved").unwrap();
        assert!(validate_folder(std::path::Path::new("relative/games")).is_err());
        assert!(validate_folder(&file).is_err());
        assert!(validate_folder(&file.join("games")).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"preserved");
    }
}
