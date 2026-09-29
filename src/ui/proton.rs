use super::*;
use crate::compatibility::{self, acquisition};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) fn proton_page(window: &adw::ApplicationWindow) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Proton");
    let (selection, refresh) = proton_selection_group(window, None);
    page.add(&selection);
    page.add(&acquisition_group(None, refresh));
    page
}

pub(super) fn proton_selection_group(
    window: &adw::ApplicationWindow,
    product_id: Option<i64>,
) -> (adw::PreferencesGroup, Rc<dyn Fn()>) {
    let group = adw::PreferencesGroup::new();
    group.set_title(if product_id.is_some() {
        "Proton override"
    } else {
        "Default Proton"
    });
    group.set_description(Some("Existing installations are used in place. Automatic selection prefers GE-Proton, then UMU-Proton, then Valve Proton. The saved default stays fixed until you change it."));
    let choices = gtk::StringList::new(&[]);
    let selected = adw::ComboRow::new();
    selected.set_title("Proton version");
    selected.set_model(Some(&choices));
    let popup = gtk::SignalListItemFactory::new();
    popup.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(60);
        item.set_child(Some(&label));
    });
    popup.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        if let Some(value) = item.item().and_downcast::<gtk::StringObject>()
            && let Some(label) = item.child().and_downcast::<gtk::Label>()
        {
            label.set_label(&value.string());
        }
    });
    selected.set_list_factory(Some(&popup));
    group.add(&selected);
    let selected_path = gtk::Label::new(Some("No Proton version selected"));
    selected_path.set_widget_name("selected-proton-path");
    selected_path.set_xalign(0.0);
    selected_path.set_wrap(true);
    selected_path.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    selected_path.set_selectable(true);
    group.add(&selected_path);
    selected.connect_selected_item_notify(move |selected| {
        let value = selected.selected_item().and_downcast::<gtk::StringObject>();
        selected_path.set_label(
            value
                .as_ref()
                .map(|value| value.string())
                .as_deref()
                .unwrap_or("No Proton version selected"),
        );
    });
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_margin_top(8);
    let apply = gtk::Button::with_label("Use selected version");
    let browse = gtk::Button::with_label("Choose folder…");
    let refresh = gtk::Button::with_label("Refresh");
    buttons.append(&apply);
    buttons.append(&browse);
    buttons.append(&refresh);
    group.add(&buttons);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_selectable(true);
    group.add(&status);
    let paths = Rc::new(RefCell::new(Vec::<Option<PathBuf>>::new()));
    let reload: Rc<dyn Fn()> = Rc::new({
        let choices = choices.clone();
        let selected = selected.clone();
        let paths = paths.clone();
        let status = status.clone();
        let buttons = buttons.clone();
        let apply = apply.clone();
        move || {
            buttons.set_sensitive(false);
            status.set_label("Looking for installed Proton versions…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = compatibility::proton_preferences().map(|preferences| {
                    let mut installations = compatibility::discover_proton();
                    for path in preferences
                        .default
                        .iter()
                        .chain(preferences.overrides.values())
                    {
                        if !installations
                            .iter()
                            .any(|installation| &installation.path == path)
                            && let Ok(installation) = compatibility::validate_proton(path)
                        {
                            installations.push(installation);
                        }
                    }
                    (preferences, installations)
                });
                sender.send(result).ok();
            });
            let choices = choices.clone();
            let selected = selected.clone();
            let paths = paths.clone();
            let status = status.clone();
            let buttons = buttons.clone();
            let apply = apply.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(Ok((preferences, installations))) => {
                        let saved = product_id
                            .and_then(|id| preferences.overrides.get(&id.to_string()))
                            .or(preferences.default.as_ref());
                        let mut entries = Vec::new();
                        let mut labels = Vec::new();
                        if product_id.is_some() {
                            entries.push(None);
                            labels.push("Use application default".to_string());
                        }
                        for installation in installations {
                            labels.push(format!(
                                "{} — {}",
                                installation.name,
                                installation.path.display()
                            ));
                            entries.push(Some(installation.path));
                        }
                        if let Some(saved) = saved
                            && !entries.iter().any(|path| path.as_ref() == Some(saved))
                        {
                            labels.push(format!(
                                "Unavailable — {} (choose a replacement)",
                                saved.display()
                            ));
                            entries.push(Some(saved.clone()));
                        }
                        let index = if product_id
                            .is_some_and(|id| !preferences.overrides.contains_key(&id.to_string()))
                        {
                            0
                        } else {
                            entries
                                .iter()
                                .position(|path| path.as_ref() == saved)
                                .unwrap_or(0)
                        };
                        choices.splice(
                            0,
                            choices.n_items(),
                            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
                        );
                        *paths.borrow_mut() = entries;
                        apply.set_sensitive(!paths.borrow().is_empty());
                        selected.set_selected(index as u32);
                        status.set_label(&preferences.default.map_or_else(
                            || "No default saved. Choose a detected version, a folder, or download one below.".into(),
                            |path| format!("Application default: {}", path.display()),
                        ));
                        buttons.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        status.set_label(&format!("Could not read Proton preferences: {error}"));
                        buttons.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        status.set_label("Proton discovery stopped unexpectedly");
                        buttons.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });
    apply.connect_clicked({
        let status = status.clone();
        let buttons = buttons.clone();
        let reload = reload.clone();
        move |_| {
            let Some(path) = paths.borrow().get(selected.selected() as usize).cloned() else {
                return;
            };
            save_selection(product_id, path, &status, &buttons, reload.clone());
        }
    });
    browse.connect_clicked({
        let window = window.clone();
        let reload = reload.clone();
        move |_| {
            let chooser = gtk::FileDialog::builder()
                .title("Choose a Proton installation folder")
                .build();
            let status = status.clone();
            let buttons = buttons.clone();
            let reload = reload.clone();
            chooser.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(folder) = result
                    && let Some(path) = folder.path()
                {
                    save_selection(product_id, Some(path), &status, &buttons, reload);
                }
            });
        }
    });
    refresh.connect_clicked({
        let reload = reload.clone();
        move |_| reload()
    });
    reload();
    (group, reload)
}

fn save_selection(
    product_id: Option<i64>,
    path: Option<PathBuf>,
    status: &gtk::Label,
    buttons: &gtk::Box,
    reload: Rc<dyn Fn()>,
) {
    status.set_label("Validating and saving Proton selection…");
    buttons.set_sensitive(false);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = match product_id {
            Some(id) => compatibility::set_game_proton(id, path.as_deref()),
            None => compatibility::set_default_proton(
                path.as_deref().expect("global selection has a path"),
            ),
        };
        sender.send(result.map_err(|error| error.to_string())).ok();
    });
    let status = status.clone();
    let buttons = buttons.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(result) => {
                buttons.set_sensitive(true);
                match result {
                    Ok(()) => reload(),
                    Err(error) => status.set_label(&error),
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                status.set_label("Saving Proton selection stopped unexpectedly");
                buttons.set_sensitive(true);
                glib::ControlFlow::Break
            }
        }
    });
}

pub(super) fn acquisition_group(
    product_id: Option<i64>,
    refresh: Rc<dyn Fn()>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Download compatibility components");
    group.set_description(Some("Downloads start only when requested. Proton and the Steam Linux Runtime are stored separately from your games. External installations are never removed."));
    let family = adw::ComboRow::new();
    family.set_title("Proton family");
    family.set_model(Some(&gtk::StringList::new(&["GE-Proton", "UMU-Proton"])));
    group.add(&family);
    let versions = gtk::StringList::new(&[]);
    let version = adw::ComboRow::new();
    version.set_title("Stable release");
    version.set_subtitle("Load releases to include current and older versions");
    version.set_model(Some(&versions));
    group.add(&version);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_margin_top(8);
    let catalog = gtk::Button::with_label("Load releases");
    let download = gtk::Button::with_label("Download and select");
    download.set_sensitive(false);
    let runtime = gtk::Button::with_label("Download missing runtime");
    controls.append(&catalog);
    controls.append(&download);
    controls.append(&runtime);
    group.add(&controls);
    let progress = gtk::ProgressBar::new();
    progress.set_visible(false);
    group.add(&progress);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    group.add(&status);
    let cancel = gtk::Button::with_label("Cancel download");
    cancel.set_halign(gtk::Align::Start);
    cancel.set_visible(false);
    group.add(&cancel);
    let cancelled = Arc::new(AtomicBool::new(false));
    group.connect_unrealize({
        let cancelled = cancelled.clone();
        move |_| cancelled.store(true, Ordering::Release)
    });
    cancel.connect_clicked({
        let cancelled = cancelled.clone();
        let status = status.clone();
        move |button| {
            cancelled.store(true, Ordering::Release);
            button.set_sensitive(false);
            status.set_label("Cancelling…");
        }
    });
    let releases = Rc::new(RefCell::new(Vec::<acquisition::Release>::new()));
    family.connect_selected_notify({
        let releases = releases.clone();
        let versions = versions.clone();
        let download = download.clone();
        move |_| {
            releases.borrow_mut().clear();
            versions.splice(0, versions.n_items(), &[]);
            download.set_sensitive(false);
        }
    });
    catalog.connect_clicked({
        let family = family.clone();
        let controls = controls.clone();
        let status = status.clone();
        let releases = releases.clone();
        let download = download.clone();
        let cancelled = cancelled.clone();
        let cancel = cancel.clone();
        let version = version.clone();
        move |_| {
            controls.set_sensitive(false);
            family.set_sensitive(false);
            status.set_label("Loading stable releases…");
            cancelled.store(false, Ordering::Release);
            cancel.set_visible(true);
            cancel.set_sensitive(true);
            let requested_family = if family.selected() == 0 {
                acquisition::ProtonFamily::Ge
            } else {
                acquisition::ProtonFamily::Umu
            };
            let (sender, receiver) = mpsc::channel();
            let cancelled = cancelled.clone();
            std::thread::spawn(move || {
                sender
                    .send(acquisition::list_releases(requested_family, &cancelled))
                    .ok();
            });
            let controls = controls.clone();
            let family = family.clone();
            let status = status.clone();
            let releases = releases.clone();
            let versions = versions.clone();
            let download = download.clone();
            let cancel = cancel.clone();
            let version = version.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(result) => {
                        match result {
                            Ok(found) => {
                                versions.splice(
                                    0,
                                    versions.n_items(),
                                    &found
                                        .iter()
                                        .map(|release| release.name.as_str())
                                        .collect::<Vec<_>>(),
                                );
                                version.set_selected(0);
                                download.set_sensitive(!found.is_empty());
                                status.set_label(if found.is_empty() {
                                    "No stable releases were returned"
                                } else {
                                    "Choose a release to download and select"
                                });
                                *releases.borrow_mut() = found;
                            }
                            Err(error) => {
                                status.set_label(&format!("Could not load releases: {error}"))
                            }
                        }
                        controls.set_sensitive(true);
                        family.set_sensitive(true);
                        cancel.set_visible(false);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        status.set_label("Release lookup stopped unexpectedly");
                        controls.set_sensitive(true);
                        family.set_sensitive(true);
                        cancel.set_visible(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });
    let transfer: Rc<dyn Fn(Option<acquisition::Release>)> = Rc::new(move |release| {
        controls.set_sensitive(false);
        family.set_sensitive(false);
        cancelled.store(false, Ordering::Release);
        cancel.set_sensitive(true);
        cancel.set_visible(true);
        progress.set_visible(true);
        progress.set_fraction(0.0);
        status.set_label("Preparing download…");
        let (sender, receiver) = mpsc::channel();
        let (updates, progress_receiver) = mpsc::sync_channel(32);
        let cancelled = cancelled.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let report = |update| {
                    updates.try_send(update).ok();
                };
                if let Some(release) = release {
                    let path = acquisition::download_proton(&release, &cancelled, report)?;
                    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "Download cancelled");
                    match product_id {
                        Some(id) => compatibility::set_game_proton(id, Some(&path))?,
                        None => compatibility::set_default_proton(&path)?,
                    }
                } else {
                    let proton = compatibility::select_proton(product_id)?;
                    if let Some(runtime) = acquisition::runtime_requirement(&proton.path)?
                        && !acquisition::runtime_ready(&runtime)
                    {
                        acquisition::download_runtime(&runtime, &cancelled, report)?;
                    }
                }
                Ok(())
            })();
            sender.send(result).ok();
        });
        let controls = controls.clone();
        let family = family.clone();
        let status = status.clone();
        let progress = progress.clone();
        let cancel = cancel.clone();
        let refresh = refresh.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            for update in progress_receiver.try_iter().take(32) {
                let update: acquisition::DownloadProgress = update;
                status.set_label(&format!(
                    "{} — {}",
                    update.phase,
                    human_size(update.completed)
                ));
                if let Some(total) = update.total.filter(|total| *total > 0) {
                    progress.set_fraction((update.completed as f64 / total as f64).min(1.0));
                } else {
                    progress.pulse();
                }
            }
            match receiver.try_recv() {
                Ok(result) => {
                    if result.is_ok() {
                        refresh();
                    }
                    status.set_label(&result.map_or_else(
                        |error| format!("Download stopped: {error}"),
                        |_| "Component ready. Check requirements again before continuing.".into(),
                    ));
                    controls.set_sensitive(true);
                    family.set_sensitive(true);
                    progress.set_visible(false);
                    cancel.set_visible(false);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.set_label("Download worker stopped unexpectedly");
                    controls.set_sensitive(true);
                    family.set_sensitive(true);
                    progress.set_visible(false);
                    cancel.set_visible(false);
                    glib::ControlFlow::Break
                }
            }
        });
    });
    download.connect_clicked({
        let transfer = transfer.clone();
        move |_| {
            if let Some(release) = releases.borrow().get(version.selected() as usize).cloned() {
                transfer(Some(release));
            }
        }
    });
    runtime.connect_clicked(move |_| transfer(None));
    group
}

pub(super) fn compatibility_message(error: &compatibility::CompatibilityFailure) -> String {
    if matches!(error, compatibility::CompatibilityFailure::UmuUnavailable) {
        "Bundled UMU is unavailable. For a source checkout, run python3 tools/prepare-helpers.py --destination target/helpers, or set an absolute LUDOMERE_UMU_RUN. Installed packages can be repaired or reinstalled.".into()
    } else {
        format!("{error}. Choose the required component or a replacement in Finish setup.")
    }
}

pub(super) fn with_windows_components(
    window: &adw::ApplicationWindow,
    product_id: i64,
    _terminal_action: bool,
    uninstall_directory: Option<PathBuf>,
    ready: impl FnOnce() + 'static,
) -> Rc<std::cell::Cell<bool>> {
    let pending = Rc::new(std::cell::Cell::new(true));
    if let Some(status) = find_named_descendant(window.upcast_ref(), "application-status-message")
        .and_downcast::<gtk::Label>()
    {
        status.set_label("Checking Windows requirements…");
    }
    let (sender, receiver) = mpsc::channel();
    let session = online::account_session();
    std::thread::spawn(move || {
        let result = if uninstall_directory.as_ref().is_some_and(|path| {
            crate::installation::load_installation_marker(path)
                .ok()
                .flatten()
                .is_some_and(|marker| {
                    marker.source == crate::domain::InstallationSource::GalaxyDepot
                })
        }) {
            Ok(())
        } else {
            compatibility::preflight_windows(Some(product_id))
        };
        sender.send(result).ok();
    });
    let window = window.downgrade();
    let waiting = pending.clone();
    let mut ready = Some(ready);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if online::account_session() != session {
            waiting.set(false);
            return glib::ControlFlow::Break;
        }
        let Some(window) = window.upgrade() else {
            waiting.set(false);
            return glib::ControlFlow::Break;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result.map_err(|error| compatibility_message(&error)),
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err("Windows check stopped. Use Finish setup to check again.".into()),
        };
        waiting.set(false);
        match result {
            Ok(()) => {
                if let Some(status) =
                    find_named_descendant(window.upcast_ref(), "application-status-message")
                        .and_downcast::<gtk::Label>()
                    && status.label() == "Checking Windows requirements…"
                {
                    status.set_label("Ready");
                }
                if let Some(action) = ready.take() {
                    action();
                }
            }
            Err(error) => {
                if let Some(status) =
                    find_named_descendant(window.upcast_ref(), "application-status-message")
                        .and_downcast::<gtk::Label>()
                {
                    status.set_label(&error);
                    status.set_tooltip_text(Some(&error));
                }
                if let Some(button) = find_named_descendant(window.upcast_ref(), "finish-setup")
                    .and_downcast::<gtk::Button>()
                {
                    button.set_action_target_value(Some(&product_id.to_variant()));
                    button.set_visible(true);
                }
            }
        }
        glib::ControlFlow::Break
    });
    pending
}

pub(super) fn launch_with_components(
    window: &adw::ApplicationWindow,
    game: crate::domain::InstalledGame,
) -> mpsc::Receiver<crate::installation::LaunchEvent> {
    if game.installer_operating_system.as_deref() == Some("linux") {
        return crate::installation::launch_game(game);
    }
    let (sender, receiver) = mpsc::channel();
    with_windows_components(window, game.product_id, true, None, move || {
        let events = crate::installation::launch_game(game);
        std::thread::spawn(move || {
            for event in events {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
    });
    receiver
}

pub(super) fn connect_windows_action(
    button: &gtk::Button,
    window: &adw::ApplicationWindow,
    terminal_action: bool,
    product: impl Fn() -> Option<i64> + 'static,
    action: impl Fn(&gtk::Button) + 'static,
) {
    let window = window.clone();
    let action = Rc::new(action);
    let pending = Rc::new(RefCell::new(None::<Rc<std::cell::Cell<bool>>>));
    button.connect_clicked(move |button| {
        if pending
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.get())
        {
            return;
        }
        if let Some(product_id) = product() {
            let action = action.clone();
            let button = button.downgrade();
            let checking =
                with_windows_components(&window, product_id, terminal_action, None, move || {
                    if let Some(button) = button.upgrade()
                        && button.root().is_some()
                    {
                        action(&button);
                    }
                });
            *pending.borrow_mut() = Some(checking);
        } else {
            action(button);
        }
    });
}

pub(super) fn patch_with_components(
    window: &adw::ApplicationWindow,
    game: crate::domain::InstalledGame,
    patch: PathBuf,
    target: Option<String>,
) -> mpsc::Receiver<crate::installation::PatchEvent> {
    let (sender, receiver) = mpsc::channel();
    with_windows_components(window, game.product_id, true, None, move || {
        let events = crate::installation::run_patch(game, patch, target);
        std::thread::spawn(move || {
            for event in events {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    #[ignore = "requires private HOME/all XDG, private D-Bus, Xvfb and an inert LUDOMERE_UMU_RUN fixture"]
    fn ready_windows_action_runs_once_without_dialog_and_missing_selection_stays_actionable() {
        let helper =
            std::env::var("LUDOMERE_UMU_RUN").expect("supply an inert private helper fixture");
        assert_eq!(
            std::fs::read_to_string(helper).unwrap(),
            "inert test fixture; never execute\n"
        );
        let root = tempfile::tempdir().unwrap();
        let proton = root.path().join("GE-Proton-test");
        std::fs::create_dir_all(proton.join("files/bin")).unwrap();
        for name in ["proton", "files/bin/wine"] {
            std::fs::write(proton.join(name), "inert test fixture; never execute\n").unwrap();
            std::fs::set_permissions(proton.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        std::fs::write(proton.join("toolmanifest.vdf"), "manifest {}").unwrap();
        compatibility::set_default_proton(&proton).unwrap();
        adw::init().expect("requires an isolated GTK display");
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.ReadyActionTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let status = gtk::Label::new(None);
        status.set_widget_name("application-status-message");
        let setup = gtk::Button::with_label("Finish setup");
        setup.set_widget_name("finish-setup");
        setup.set_visible(false);
        let action = gtk::Button::with_label("Install");
        body.append(&status);
        body.append(&setup);
        body.append(&action);
        window.set_content(Some(&body));
        window.present();
        let calls = Rc::new(std::cell::Cell::new(0));
        let called = calls.clone();
        connect_windows_action(
            &action,
            &window,
            false,
            || Some(7),
            move |_| called.set(called.get() + 1),
        );
        action.emit_clicked();
        action.emit_clicked();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while calls.get() == 0 && std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(true);
        }
        assert_eq!(calls.get(), 1);
        assert!(window.visible_dialog().is_none());
        std::fs::rename(&proton, root.path().join("removed-proton")).unwrap();
        action.emit_clicked();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !setup.is_visible() && std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(true);
        }
        assert!(setup.is_visible());
        assert!(status.label().contains("replacement"));
        assert_eq!(calls.get(), 1);
        assert!(window.visible_dialog().is_none());
        window.close();
    }
}
