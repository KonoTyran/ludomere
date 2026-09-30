use super::*;
use crate::compatibility::{self, acquisition};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) fn saved_proton(
    product_id: Option<i64>,
) -> anyhow::Result<crate::compatibility::ProtonInstallation> {
    let preferences = crate::compatibility::proton_preferences()?;
    let path = product_id
        .and_then(|id| preferences.overrides.get(&id.to_string()))
        .or(preferences.default.as_ref())
        .ok_or_else(|| anyhow::anyhow!("Select a Proton version before continuing."))?;
    Ok(crate::compatibility::validate_proton(path)?)
}

pub(super) fn runtime_readiness(product_id: Option<i64>) -> anyhow::Result<Option<bool>> {
    let proton = saved_proton(product_id)?;
    Ok(
        crate::compatibility::acquisition::runtime_requirement(&proton.path)?
            .map(|runtime| crate::compatibility::acquisition::runtime_ready(&runtime)),
    )
}

pub(super) fn runtime_check(
    product_id: Option<i64>,
    component: &ComponentGroup,
    active: Rc<dyn Fn() -> bool>,
) -> (Rc<dyn Fn()>, Rc<std::cell::Cell<bool>>) {
    let retry = gtk::Button::with_label("Retry runtime detection");
    retry.set_widget_name("runtime-retry");
    retry.set_visible(false);
    component.group.add(&retry);
    let checking_runtime = Rc::new(std::cell::Cell::new(false));
    let runtime_generation = Rc::new(std::cell::Cell::new(0u64));
    let check_runtime: Rc<dyn Fn()> = Rc::new({
        let active = active.clone();
        let checking = checking_runtime.clone();
        let generation = runtime_generation.clone();
        let button = component.runtime_button.clone();
        let status = component.status.clone();
        let retry = retry.clone();
        move || {
            if !active() {
                return;
            }
            let request = generation.get().wrapping_add(1);
            generation.set(request);
            checking.set(true);
            button.set_sensitive(false);
            retry.set_visible(false);
            status.set_label("Looking for the required Steam Linux Runtime…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(runtime_readiness(product_id));
            });
            let active = active.clone();
            let checking = checking.clone();
            let generation = generation.clone();
            let button = button.clone();
            let status = status.clone();
            let retry = retry.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if !active() || generation.get() != request {
                    if generation.get() == request {
                        checking.set(false);
                    }
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err(anyhow::anyhow!("Runtime detection stopped. Try again.")),
                };
                checking.set(false);
                match result {
                    Ok(Some(true)) => {
                        status.set_label("Steam Linux Runtime found! You're all set!")
                    }
                    Ok(Some(false)) => {
                        status.set_label("Click the button below to download the missing runtime.");
                        button.set_sensitive(true);
                    }
                    Ok(None) => status
                        .set_label("This Proton version does not require a Steam Linux Runtime."),
                    Err(error) => {
                        status.set_label(&format!(
                            "Could not check the selected Proton runtime: {error}"
                        ));
                        retry.set_visible(true);
                    }
                }
                glib::ControlFlow::Break
            });
        }
    });
    retry.connect_clicked({
        let check = check_runtime.clone();
        move |_| check()
    });
    (check_runtime, checking_runtime)
}

pub(super) fn proton_page(window: &adw::ApplicationWindow) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Proton");
    let closed = Rc::new(std::cell::Cell::new(false));
    let active: Rc<dyn Fn() -> bool> = Rc::new({
        let closed = closed.clone();
        move || !closed.get()
    });
    let acquisition_busy = Rc::new(std::cell::Cell::new(false));
    let selection = proton_selection_group_guarded(
        window,
        None,
        Some(active.clone()),
        Some(acquisition_busy.clone()),
        true,
    );
    selection.group.set_description(Some("Choose the default Proton version for Windows games. Changes are saved automatically; individual games can use their own overrides."));
    page.add(&selection.group);
    let download = acquisition_group_guarded(
        None,
        selection.refresh.clone(),
        ComponentScope::Proton,
        acquisition_busy.clone(),
        Some(selection.busy.clone()),
    );
    download.group.set_title("Download Proton");
    download.group.set_description(Some("Browse and download Proton versions. A completed download becomes your saved default; existing installations are kept."));
    page.add(&download.group);
    let runtime = acquisition_group_guarded(
        None,
        selection.refresh.clone(),
        ComponentScope::Runtime,
        acquisition_busy.clone(),
        Some(selection.busy.clone()),
    );
    runtime.group.set_title("Steam Linux Runtime");
    runtime.group.set_description(Some("Check the runtime required by your saved Proton version. Missing runtimes are downloaded only when you request them."));
    runtime.runtime_button.set_sensitive(false);
    page.add(&runtime.group);
    let (check_runtime, checking) = runtime_check(
        None,
        &runtime,
        Rc::new({
            let active = active.clone();
            let selecting = selection.busy.clone();
            move || active() && !selecting.get()
        }),
    );
    page.connect_map({
        let refresh = selection.refresh.clone();
        let busy = selection.busy.clone();
        let acquisition_busy = acquisition_busy.clone();
        move |_| {
            if !busy.get() && !acquisition_busy.get() {
                refresh();
            }
        }
    });
    window.connect_close_request({
        let closed = closed.clone();
        let proton_cancel = download.cancelled.clone();
        let runtime_cancel = runtime.cancelled.clone();
        move |_| {
            closed.set(true);
            proton_cancel.store(true, Ordering::Release);
            runtime_cancel.store(true, Ordering::Release);
            glib::Propagation::Proceed
        }
    });
    let weak_page = page.downgrade();
    let mut was_selecting = true;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if !active() || weak_page.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        selection.group.set_sensitive(!acquisition_busy.get());
        download
            .group
            .set_sensitive(!selection.busy.get() && (!checking.get() || acquisition_busy.get()));
        runtime.group.set_sensitive(!selection.busy.get());
        if was_selecting && !selection.busy.get() && !acquisition_busy.get() {
            check_runtime();
        }
        was_selecting = selection.busy.get();
        glib::ControlFlow::Continue
    });
    page
}

pub(super) fn proton_selection_group(
    window: &adw::ApplicationWindow,
    product_id: Option<i64>,
) -> (adw::PreferencesGroup, Rc<dyn Fn()>) {
    let selection = proton_selection_group_guarded(window, product_id, None, None, false);
    (selection.group, selection.refresh)
}

pub(super) struct ProtonSelection {
    pub group: adw::PreferencesGroup,
    pub refresh: Rc<dyn Fn()>,
    pub busy: Rc<std::cell::Cell<bool>>,
    pub detected: Rc<std::cell::Cell<Option<bool>>>,
    pub selected_path: Rc<dyn Fn() -> anyhow::Result<Option<PathBuf>>>,
}

pub(super) fn proton_selection_group_guarded(
    window: &adw::ApplicationWindow,
    product_id: Option<i64>,
    active: Option<Rc<dyn Fn() -> bool>>,
    acquisition_busy: Option<Rc<std::cell::Cell<bool>>>,
    prompt_without_saved: bool,
) -> ProtonSelection {
    let automatic = active.is_some();
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
    buttons.set_widget_name("proton-selection-actions");
    buttons.set_margin_top(8);
    let apply = gtk::Button::with_label("Use selected version");
    apply.set_visible(!automatic);
    let browse = gtk::Button::with_label("Choose folder…");
    browse.set_visible(!automatic);
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
    let busy = Rc::new(std::cell::Cell::new(false));
    let detected = Rc::new(std::cell::Cell::new(None));
    let custom_index = Rc::new(std::cell::Cell::new(gtk::INVALID_LIST_POSITION));
    let saved_index = Rc::new(std::cell::Cell::new(gtk::INVALID_LIST_POSITION));
    let save_error = Rc::new(RefCell::new(None::<String>));
    let reload: Rc<dyn Fn()> = Rc::new({
        let choices = choices.clone();
        let selected = selected.clone();
        let paths = paths.clone();
        let status = status.clone();
        let buttons = buttons.clone();
        let apply = apply.clone();
        let busy = busy.clone();
        let detected = detected.clone();
        let custom_index = custom_index.clone();
        let saved_index = saved_index.clone();
        let active = active.clone();
        let browse = browse.clone();
        let save_error = save_error.clone();
        move || {
            if active.as_ref().is_some_and(|active| !active()) {
                return;
            }
            busy.set(true);
            selected.set_sensitive(false);
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
            let busy = busy.clone();
            let detected = detected.clone();
            let custom_index = custom_index.clone();
            let saved_index = saved_index.clone();
            let active = active.clone();
            let browse = browse.clone();
            let save_error = save_error.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if active.as_ref().is_some_and(|active| !active()) {
                    busy.set(false);
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok((preferences, installations))) => {
                        detected.set(Some(!installations.is_empty()));
                        let saved = product_id
                            .and_then(|id| preferences.overrides.get(&id.to_string()))
                            .or(preferences.default.as_ref());
                        let mut entries = Vec::new();
                        let mut labels = Vec::new();
                        if automatic
                            && prompt_without_saved
                            && saved.is_none()
                            && product_id.is_none()
                        {
                            entries.push(None);
                            labels.push("Select a Proton version".to_string());
                        }
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
                        if automatic && entries.is_empty() {
                            entries.push(None);
                            labels.push("Select a Proton version".into());
                        }
                        let index = if (automatic
                            && prompt_without_saved
                            && saved.is_none()
                            && product_id.is_none())
                            || (product_id.is_some_and(|id| {
                                !preferences.overrides.contains_key(&id.to_string())
                            }) && (!automatic || saved.is_some()))
                        {
                            0
                        } else {
                            saved
                                .and_then(|saved| {
                                    entries.iter().position(|path| path.as_ref() == Some(saved))
                                })
                                .or_else(|| {
                                    if automatic {
                                        entries.iter().position(Option::is_some)
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0)
                        };
                        if automatic {
                            custom_index.set(entries.len() as u32);
                            labels.push("Custom Proton Directory".into());
                        }
                        choices.splice(
                            0,
                            choices.n_items(),
                            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
                        );
                        *paths.borrow_mut() = entries;
                        apply.set_sensitive(!paths.borrow().is_empty());
                        saved_index.set(index as u32);
                        selected.set_selected(index as u32);
                        browse.set_visible(!automatic || index as u32 == custom_index.get());
                        status.set_label(&save_error.borrow_mut().take().unwrap_or_else(|| if automatic { String::new() } else { preferences.default.map_or_else(
                            || "No default saved. Choose a detected version, a folder, or download one below.".into(),
                            |path| format!("Application default: {}", path.display()),
                        ) }));
                        buttons.set_sensitive(true);
                        selected.set_sensitive(true);
                        busy.set(false);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        busy.set(false);
                        status.set_label(&format!("Could not read Proton preferences: {error}"));
                        buttons.set_sensitive(true);
                        selected.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        busy.set(false);
                        status.set_label("Proton discovery stopped unexpectedly");
                        buttons.set_sensitive(true);
                        selected.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });
    let save: Rc<dyn Fn(Option<PathBuf>)> = Rc::new({
        let busy = busy.clone();
        let status = status.clone();
        let buttons = buttons.clone();
        let selected = selected.clone();
        let reload = reload.clone();
        let active = active.clone();
        let acquisition_busy = acquisition_busy.clone();
        move |path| {
            if busy.get()
                || acquisition_busy.as_ref().is_some_and(|busy| busy.get())
                || active.as_ref().is_some_and(|active| !active())
            {
                return;
            }
            busy.set(true);
            status.set_label("Validating and saving Proton selection…");
            buttons.set_sensitive(false);
            selected.set_sensitive(false);
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = match product_id {
                    Some(id) => compatibility::set_game_proton(id, path.as_deref()),
                    None => compatibility::set_default_proton(
                        path.as_deref().expect("global selection has a path"),
                    ),
                };
                let _ = sender.send(result.map_err(|error| error.to_string()));
            });
            let busy = busy.clone();
            let active = active.clone();
            let reload = reload.clone();
            let save_error = save_error.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if active.as_ref().is_some_and(|active| !active()) {
                    busy.set(false);
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err("Saving Proton selection stopped unexpectedly".into()),
                };
                *save_error.borrow_mut() = result.err();
                reload();
                glib::ControlFlow::Break
            });
        }
    });
    apply.connect_clicked({
        let paths = paths.clone();
        let selected = selected.clone();
        let save = save.clone();
        move |_| {
            let Some(path) = paths.borrow().get(selected.selected() as usize).cloned() else {
                return;
            };
            save(path);
        }
    });
    if automatic {
        selected.connect_selected_notify({
            let paths = paths.clone();
            let busy = busy.clone();
            let browse = browse.clone();
            let status = status.clone();
            let active = active.clone();
            let save = save.clone();
            let acquisition_busy = acquisition_busy.clone();
            let custom_index = custom_index.clone();
            move |selected| {
                if acquisition_busy.as_ref().is_some_and(|busy| busy.get()) {
                    if selected.selected() != saved_index.get() { selected.set_selected(saved_index.get()); }
                    return;
                }
                if busy.get() || active.as_ref().is_some_and(|active| !active()) { return; }
                let custom = selected.selected() == custom_index.get();
                browse.set_visible(custom);
                if custom {
                    status.set_label("Choose a Proton directory. Your saved selection stays unchanged until a valid folder is selected.");
                } else if let Some(path) = paths.borrow().get(selected.selected() as usize).cloned()
                    && (path.is_some() || product_id.is_some()) {
                    save(path);
                }
            }
        });
    }
    browse.connect_clicked({
        let window = window.clone();
        let active = active.clone();
        let busy = busy.clone();
        let selected = selected.clone();
        let save = save.clone();
        let acquisition_busy = acquisition_busy.clone();
        move |_| {
            if busy.get() || acquisition_busy.as_ref().is_some_and(|busy| busy.get()) {
                return;
            }
            busy.set(true);
            selected.set_sensitive(false);
            buttons.set_sensitive(false);
            let chooser = gtk::FileDialog::builder()
                .title("Choose a Proton installation folder")
                .build();
            let buttons = buttons.clone();
            let active = active.clone();
            let busy = busy.clone();
            let selected = selected.clone();
            let save = save.clone();
            chooser.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                busy.set(false);
                buttons.set_sensitive(true);
                selected.set_sensitive(true);
                if active.as_ref().is_some_and(|active| !active()) {
                    return;
                }
                if let Ok(folder) = result
                    && let Some(path) = folder.path()
                {
                    save(Some(path));
                }
            });
        }
    });
    refresh.connect_clicked({
        let reload = reload.clone();
        move |_| {
            if !acquisition_busy.as_ref().is_some_and(|busy| busy.get()) {
                reload();
            }
        }
    });
    reload();
    let selected_path = Rc::new(move || {
        let path = paths.borrow().get(selected.selected() as usize).cloned();
        match path {
            Some(Some(path)) => Ok(Some(path)),
            Some(None) if product_id.is_some() => Ok(None),
            _ => anyhow::bail!(
                "Select a Proton version or choose a valid custom directory before continuing."
            ),
        }
    });
    ProtonSelection {
        group,
        refresh: reload,
        busy,
        detected,
        selected_path,
    }
}

pub(super) enum ComponentScope {
    Proton,
    Runtime,
}

pub(super) struct ComponentGroup {
    pub group: adw::PreferencesGroup,
    pub busy: Rc<std::cell::Cell<bool>>,
    pub cancelled: Arc<AtomicBool>,
    pub runtime_button: gtk::Button,
    pub status: gtk::Label,
    pub succeeded: Rc<std::cell::Cell<bool>>,
}

pub(super) fn acquisition_group(
    product_id: Option<i64>,
    refresh: Rc<dyn Fn()>,
    scope: ComponentScope,
) -> ComponentGroup {
    acquisition_group_guarded(
        product_id,
        refresh,
        scope,
        Rc::new(std::cell::Cell::new(false)),
        None,
    )
}

pub(super) fn acquisition_group_guarded(
    product_id: Option<i64>,
    refresh: Rc<dyn Fn()>,
    scope: ComponentScope,
    busy: Rc<std::cell::Cell<bool>>,
    blocked: Option<Rc<std::cell::Cell<bool>>>,
) -> ComponentGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Download compatibility components");
    group.set_description(Some("Downloads start only when requested. Proton and the Steam Linux Runtime are stored separately from your games. External installations are never removed."));
    let family = adw::ComboRow::new();
    family.set_title("Proton family");
    family.set_model(Some(&gtk::StringList::new(&["GE-Proton", "UMU-Proton"])));
    group.add(&family);
    family.set_visible(!matches!(scope, ComponentScope::Runtime));
    let versions = gtk::StringList::new(&[]);
    let version = adw::ComboRow::new();
    version.set_title("Stable release");
    version.set_subtitle("Load releases to include current and older versions");
    version.set_model(Some(&versions));
    group.add(&version);
    version.set_visible(!matches!(scope, ComponentScope::Runtime));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_margin_top(8);
    let catalog = gtk::Button::with_label("Load releases");
    let download = gtk::Button::with_label("Download and select");
    download.set_sensitive(false);
    let runtime = gtk::Button::with_label("Download missing runtime");
    controls.append(&catalog);
    controls.append(&download);
    controls.append(&runtime);
    catalog.set_visible(!matches!(scope, ComponentScope::Runtime));
    download.set_visible(!matches!(scope, ComponentScope::Runtime));
    runtime.set_visible(!matches!(scope, ComponentScope::Proton));
    if !matches!(scope, ComponentScope::Runtime) {
        group.add(&controls);
    }
    let progress = gtk::ProgressBar::new();
    progress.set_visible(false);
    group.add(&progress);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    group.add(&status);
    if matches!(scope, ComponentScope::Runtime) {
        group.add(&controls);
    }
    let cancel = gtk::Button::with_label("Cancel download");
    cancel.set_halign(gtk::Align::Start);
    cancel.set_visible(false);
    group.add(&cancel);
    let cancelled = Arc::new(AtomicBool::new(false));
    let alive = Rc::new(std::cell::Cell::new(true));
    let succeeded = Rc::new(std::cell::Cell::new(false));
    let component = ComponentGroup {
        group: group.clone(),
        busy: busy.clone(),
        cancelled: cancelled.clone(),
        runtime_button: runtime.clone(),
        status: status.clone(),
        succeeded: succeeded.clone(),
    };
    group.connect_unrealize({
        let cancelled = cancelled.clone();
        let alive = alive.clone();
        move |_| {
            alive.set(false);
            cancelled.store(true, Ordering::Release);
        }
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
        let busy = busy.clone();
        let blocked = blocked.clone();
        let alive = alive.clone();
        move |_| {
            if !alive.get() || busy.get() || blocked.as_ref().is_some_and(|busy| busy.get()) {
                return;
            }
            busy.set(true);
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
            let busy = busy.clone();
            let alive = alive.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if !alive.get() {
                    busy.set(false);
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(result) => {
                        busy.set(false);
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
                        busy.set(false);
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
        if !alive.get() || busy.get() || blocked.as_ref().is_some_and(|busy| busy.get()) {
            return;
        }
        succeeded.set(false);
        busy.set(true);
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
        let busy = busy.clone();
        let succeeded = succeeded.clone();
        let alive = alive.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if !alive.get() {
                busy.set(false);
                return glib::ControlFlow::Break;
            }
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
                    busy.set(false);
                    succeeded.set(result.is_ok());
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
                    busy.set(false);
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
    component
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
    #[ignore = "requires private HOME/all XDG, private D-Bus and Xvfb"]
    fn settings_autosaves_explicit_choice_and_checks_saved_runtime_without_discovery_writes() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p172-")
        );
        let root =
            PathBuf::from(std::env::var("HOME").unwrap()).join(".steam/root/compatibilitytools.d");
        let versions = [root.join("GE-Proton-first"), root.join("GE-Proton-second")];
        for path in &versions {
            std::fs::create_dir_all(path.join("files/bin")).unwrap();
            for name in ["proton", "files/bin/wine"] {
                std::fs::write(path.join(name), "inert; never execute\n").unwrap();
                std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            std::fs::write(path.join("toolmanifest.vdf"), "manifest {}").unwrap();
        }
        std::fs::write(
            versions[0].join("toolmanifest.vdf"),
            "manifest { require_tool_appid 4183110 }",
        )
        .unwrap();
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.ProtonSettingsTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let page = proton_page(&window);
        window.set_content(Some(&page));
        window.present();
        fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut result = vec![widget.clone()];
            let mut child = widget.first_child();
            while let Some(widget) = child {
                result.extend(children(&widget));
                child = widget.next_sibling();
            }
            result
        }
        fn wait_until(condition: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !condition() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(condition());
        }
        let widgets = children(page.upcast_ref());
        let row = widgets
            .iter()
            .filter_map(|w| w.clone().downcast::<adw::ComboRow>().ok())
            .find(|row| row.title() == "Proton version")
            .unwrap();
        let button = |title: &str| {
            widgets
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some(title))
                .unwrap()
        };
        let runtime_button = widgets
            .iter()
            .filter_map(|widget| widget.clone().downcast::<gtk::Button>().ok())
            .find(|button| {
                button.is_visible() && button.label().as_deref() == Some("Download missing runtime")
            })
            .unwrap();
        let has_text = |text: &str| {
            widgets
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk::Label>().ok())
                .any(|label| label.label().contains(text))
        };
        wait_until(|| has_text("Select a Proton version before continuing"));
        assert!(!crate::identity::config_root().join("proton.json").exists());
        assert_eq!(row.selected(), 0);
        assert!(!button("Choose folder…").is_visible());
        assert!(!button("Use selected version").is_visible());
        let choices = row.model().unwrap().downcast::<gtk::StringList>().unwrap();
        assert_eq!(
            choices.string(choices.n_items() - 1).unwrap(),
            "Custom Proton Directory"
        );
        row.set_selected(1);
        wait_until(|| has_text("Click the button below to download the missing runtime."));
        assert_eq!(
            compatibility::proton_preferences().unwrap().default,
            Some(versions[0].clone())
        );
        assert!(runtime_button.is_sensitive());
        assert!(
            widgets
                .iter()
                .filter_map(|w| w.clone().downcast::<adw::PreferencesGroup>().ok())
                .any(|group| group.title() == "Download Proton" && group.is_visible())
        );
        let groups: Vec<_> = widgets
            .iter()
            .filter_map(|w| w.clone().downcast::<adw::PreferencesGroup>().ok())
            .collect();
        assert_eq!(groups.last().unwrap().title(), "Steam Linux Runtime");
        let second = (0..choices.n_items())
            .find(|index| choices.string(*index).unwrap().contains("GE-Proton-second"))
            .unwrap();
        row.set_selected(second);
        wait_until(|| has_text("does not require a Steam Linux Runtime"));
        assert!(!runtime_button.is_sensitive());
        row.set_selected(choices.n_items() - 1);
        assert!(button("Choose folder…").is_visible());
        assert_eq!(
            compatibility::proton_preferences().unwrap().default,
            Some(versions[1].clone())
        );
        std::fs::write(
            versions[1].join("toolmanifest.vdf"),
            "manifest { require_tool_appid 999 }",
        )
        .unwrap();
        page.set_visible(false);
        page.set_visible(true);
        wait_until(|| has_text("unsupported Steam Linux Runtime"));
        assert!(button("Retry runtime detection").is_visible());
        assert!(!runtime_button.is_sensitive());
        std::fs::write(versions[1].join("toolmanifest.vdf"), "manifest {}").unwrap();
        button("Retry runtime detection").emit_clicked();
        wait_until(|| has_text("does not require a Steam Linux Runtime"));
        button("Refresh").emit_clicked();
        window.close();
        wait_until(|| app.windows().is_empty());
    }

    #[test]
    #[ignore = "requires private HOME/all XDG, private D-Bus and Xvfb"]
    fn wizard_selection_only_saves_user_choices_and_restores_failed_choices() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p166-")
        );
        let root = tempfile::tempdir().unwrap();
        let discovered =
            PathBuf::from(std::env::var("HOME").unwrap()).join(".steam/root/compatibilitytools.d");
        let versions = [
            discovered.join("GE-Proton-first"),
            discovered.join("GE-Proton-second"),
        ];
        for path in &versions {
            std::fs::create_dir_all(path.join("files/bin")).unwrap();
            for name in ["proton", "files/bin/wine"] {
                std::fs::write(path.join(name), "inert; never execute\n").unwrap();
                std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            std::fs::write(path.join("toolmanifest.vdf"), "manifest {}").unwrap();
        }
        compatibility::set_default_proton(&versions[0]).unwrap();
        compatibility::set_game_proton(123, Some(&versions[1])).unwrap();
        let preferences = crate::identity::config_root().join("proton.json");
        let original = std::fs::read(&preferences).unwrap();
        let modified = std::fs::metadata(&preferences).unwrap().modified().unwrap();
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.WizardSelectionTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let active = Rc::new(std::cell::Cell::new(true));
        let selection = proton_selection_group_guarded(
            &window,
            None,
            Some(Rc::new({
                let active = active.clone();
                move || active.get()
            })),
            None,
            false,
        );
        window.set_content(Some(&selection.group));
        window.present();
        fn settle(busy: &std::cell::Cell<bool>) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while busy.get() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!busy.get());
        }
        fn row(widget: &gtk::Widget) -> Option<adw::ComboRow> {
            if let Ok(row) = widget.clone().downcast() {
                return Some(row);
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(row) = row(&widget) {
                    return Some(row);
                }
                child = widget.next_sibling();
            }
            None
        }
        settle(&selection.busy);
        assert_eq!(std::fs::read(&preferences).unwrap(), original);
        assert_eq!(
            std::fs::metadata(&preferences).unwrap().modified().unwrap(),
            modified
        );
        let row = row(selection.group.upcast_ref()).unwrap();
        let choices = row.model().unwrap().downcast::<gtk::StringList>().unwrap();
        let index = |path: &std::path::Path| {
            (0..choices.n_items())
                .find(|index| {
                    choices
                        .string(*index)
                        .unwrap()
                        .contains(path.to_str().unwrap())
                })
                .unwrap()
        };
        let first = index(&versions[0]);
        let second = index(&versions[1]);
        row.set_selected(choices.n_items() - 1);
        assert!(!selection.busy.get());
        assert_eq!(
            compatibility::proton_preferences().unwrap().default,
            Some(versions[0].clone())
        );
        row.set_selected(second);
        assert!(selection.busy.get());
        assert!(!row.is_sensitive());
        row.set_selected(first); // a queued notification while saving cannot start another write
        settle(&selection.busy);
        assert_eq!(
            compatibility::proton_preferences().unwrap().default,
            Some(versions[1].clone())
        );
        assert_eq!(row.selected(), second);
        std::fs::rename(&versions[0], root.path().join("removed")).unwrap();
        row.set_selected(first);
        settle(&selection.busy);
        assert_eq!(
            compatibility::proton_preferences().unwrap().default,
            Some(versions[1].clone())
        );
        assert!(
            row.selected_item()
                .and_downcast::<gtk::StringObject>()
                .unwrap()
                .string()
                .contains("GE-Proton-second")
        );
        active.set(false);
        row.set_selected(choices.n_items() - 1);
        assert!(!selection.busy.get());
        window.close();
    }

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
