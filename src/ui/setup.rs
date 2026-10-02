use super::*;
use std::cell::Cell;

const STEPS: [&str; 6] = [
    "Welcome",
    "Game Files",
    "Offline Installers",
    "Goodies & Extras",
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

fn set_setup_library(
    config: &mut Config,
    kind: crate::config::LibraryKind,
    path: &std::path::Path,
) {
    let libraries = config.libraries_mut(kind);
    if let Some(index) = libraries
        .iter()
        .position(|item| item.default)
        .or_else(|| (!libraries.is_empty()).then_some(0))
    {
        libraries[index].path = path.to_owned();
    } else {
        libraries.push(crate::config::GameLibrary {
            id: crate::config::game_library_id(path),
            name: kind.label().into(),
            path: path.to_owned(),
            default: true,
        });
    }
}

fn prepare_setup_library(
    original: &Config,
    draft: &mut Config,
    kind: crate::config::LibraryKind,
    path: &std::path::Path,
) -> anyhow::Result<bool> {
    *draft.libraries_mut(kind) = original.libraries(kind).to_vec();
    if path.as_os_str().is_empty() {
        anyhow::ensure!(
            kind != crate::config::LibraryKind::GameFiles,
            "Choose a Game Files directory."
        );
        match kind {
            crate::config::LibraryKind::OfflineInstallers => {
                draft.auto_download_offline_installers = original.auto_download_offline_installers
            }
            crate::config::LibraryKind::Extras => {
                draft.auto_download_extras = original.auto_download_extras
            }
            crate::config::LibraryKind::GameFiles => unreachable!(),
        }
        return Ok(false);
    }
    set_setup_library(draft, kind, path);
    validate_setup_library(draft, kind, false)?;
    Ok(true)
}

fn validate_setup_library(
    config: &Config,
    kind: crate::config::LibraryKind,
    create: bool,
) -> anyhow::Result<()> {
    let Some(library) = config.default_library(kind) else {
        anyhow::ensure!(
            kind != crate::config::LibraryKind::GameFiles,
            "Choose a Game Files directory."
        );
        return Ok(());
    };
    validate_folder(&library.path)?;
    anyhow::ensure!(
        library.path.components().all(|part| matches!(
            part,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )),
        "Choose an absolute directory without parent-directory components."
    );
    for other in crate::config::LibraryKind::ALL
        .into_iter()
        .flat_map(|kind| config.libraries(kind))
    {
        anyhow::ensure!(
            std::ptr::eq(other, library)
                || (other.id != library.id
                    && !other.path.starts_with(&library.path)
                    && !library.path.starts_with(&other.path)),
            "Library directories must be separate and cannot contain one another. Choose another directory, or manage existing libraries in Settings."
        );
    }
    if create {
        std::fs::create_dir_all(&library.path)?;
    }
    if library.path.exists() {
        crate::storage::validate_library(config, kind, &library.id)?;
    }
    Ok(())
}

struct SetupLibrary {
    group: adw::PreferencesGroup,
    entry: gtk::Entry,
    picking: Rc<Cell<bool>>,
    update: gtk::CheckButton,
    status: gtk::Label,
}

fn save_setup_library(
    kind: crate::config::LibraryKind,
    path: &std::path::Path,
    update: bool,
    session: u64,
) -> anyhow::Result<Config> {
    let _activity = crate::profile_reset::begin_activity("saving setup library")?;
    let _permit = crate::operation_gate::try_acquire().map_err(|_| {
        anyhow::anyhow!("Finish or pause downloads and installations before changing libraries.")
    })?;
    anyhow::ensure!(
        online::account_session() == session,
        "The account changed. Reopen setup."
    );
    let mut config = Config::load_or_create()?;
    let original = config.clone();
    anyhow::ensure!(
        !path.as_os_str().is_empty(),
        "Choose a directory, or skip this optional step to keep the previous setting."
    );
    prepare_setup_library(&original, &mut config, kind, path)?;
    validate_setup_library(&config, kind, true)?;
    let mut latest = Config::load_or_create()?;
    anyhow::ensure!(
        crate::config::LibraryKind::ALL
            .into_iter()
            .all(|kind| latest.libraries(kind) == original.libraries(kind)),
        "Library settings changed while this directory was being checked. Edit the field to retry."
    );
    // Validation can inspect many files; preserve preferences changed during that work.
    *latest.libraries_mut(kind) = config.libraries(kind).to_vec();
    config = latest;
    match kind {
        crate::config::LibraryKind::GameFiles => config.auto_update_galaxy_installations = update,
        crate::config::LibraryKind::OfflineInstallers => {
            config.auto_download_offline_installers = update
        }
        crate::config::LibraryKind::Extras => config.auto_download_extras = update,
    }
    online::with_account_session(session, || config.save())?;
    Ok(config)
}

fn merge_setup_library(target: &mut Config, source: &Config, kind: crate::config::LibraryKind) {
    *target.libraries_mut(kind) = source.libraries(kind).to_vec();
    match kind {
        crate::config::LibraryKind::GameFiles => {
            target.auto_update_galaxy_installations = source.auto_update_galaxy_installations
        }
        crate::config::LibraryKind::OfflineInstallers => {
            target.auto_download_offline_installers = source.auto_download_offline_installers
        }
        crate::config::LibraryKind::Extras => {
            target.auto_download_extras = source.auto_download_extras
        }
    }
}

fn connect_library_autosave(
    controls: &SetupLibrary,
    kind: crate::config::LibraryKind,
    model: &Rc<RefCell<AppModel>>,
    w: &Rc<Widgets>,
    draft: &Rc<RefCell<Config>>,
    saving: &Rc<Cell<usize>>,
) -> Rc<dyn Fn()> {
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let revision = Rc::new(Cell::new(0u64));
    let session = online::account_session();
    let epoch = model.borrow().account_epoch;
    let save: Rc<dyn Fn()> = Rc::new({
        let entry = controls.entry.clone();
        let update = controls.update.clone();
        let status = controls.status.clone();
        let model = model.clone();
        let draft = draft.clone();
        let w = w.clone();
        let saving = saving.clone();
        let revision = revision.clone();
        move || {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                return;
            }
            let path = std::path::PathBuf::from(entry.text().as_str());
            let submitted_path = path.clone();
            let update = update.is_active();
            revision.set(revision.get().wrapping_add(1));
            let current = revision.get();
            saving.set(saving.get() + 1);
            status.set_label("Checking and saving this library…");
            let receiver = update_policies::policy_request(move || {
                save_setup_library(kind, &path, update, session)
            });
            let model = model.clone();
            let draft = draft.clone();
            let status = status.clone();
            let w = w.clone();
            let saving = saving.clone();
            let revision = revision.clone();
            let entry = entry.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err(anyhow::anyhow!(
                        "The settings worker stopped. Edit the field to retry."
                    )),
                };
                saving.set(saving.get().saturating_sub(1));
                if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return glib::ControlFlow::Break;
                }
                match result {
                    Ok(config) => {
                        merge_setup_library(&mut model.borrow_mut().config, &config, kind);
                        merge_setup_library(&mut draft.borrow_mut(), &config, kind);
                        if revision.get() == current
                            && entry.text().as_str() == submitted_path.to_string_lossy()
                        {
                            status.set_label("Saved automatically.");
                        }
                    }
                    Err(error) => {
                        let message = format!(
                            "{} was not saved: {error}. The previous setting is unchanged.",
                            kind.label()
                        );
                        if revision.get() == current
                            && entry.text().as_str() == submitted_path.to_string_lossy()
                        {
                            status.set_label(&message);
                        }
                        if !status.is_mapped() {
                            show_status(&w, &message);
                        }
                    }
                }
                glib::ControlFlow::Break
            });
        }
    });
    let flush: Rc<dyn Fn()> = Rc::new({
        let pending = pending.clone();
        let save = save.clone();
        move || {
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
                save();
            }
        }
    });
    controls.entry.connect_changed({
        let pending = pending.clone();
        let save = save.clone();
        let status = controls.status.clone();
        move |_| {
            status.set_label("Waiting to check and save this directory…");
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let pending_inner = pending.clone();
            let save = save.clone();
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(
                Duration::from_millis(400),
                move || {
                    pending_inner.borrow_mut().take();
                    save();
                },
            ));
        }
    });
    controls.entry.connect_activate({
        let flush = flush.clone();
        move |_| flush()
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let flush = flush.clone();
        move |_| flush()
    });
    controls.entry.add_controller(focus);
    controls.update.connect_toggled(move |_| {
        if let Some(source) = pending.borrow_mut().take() {
            source.remove();
        }
        save();
    });
    flush
}

fn setup_library(
    kind: crate::config::LibraryKind,
    draft: &Rc<RefCell<Config>>,
    window: &adw::ApplicationWindow,
    active: Rc<dyn Fn() -> bool>,
) -> SetupLibrary {
    let group = adw::PreferencesGroup::new();
    group.set_title(&glib::markup_escape_text(kind.label()));
    group.set_description(Some(match kind {
        crate::config::LibraryKind::GameFiles => "Choose the directory for installed game files. This directory is required. You can add more libraries afterward in Settings.",
        crate::config::LibraryKind::OfflineInstallers => "Choose a directory for offline installers, or skip this step. You can add more libraries afterward in Settings.",
        crate::config::LibraryKind::Extras => "Choose a directory for goodies and extras, or skip this step. You can add more libraries afterward in Settings.",
    }));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Directory (absolute path)"));
    entry.set_widget_name(match kind {
        crate::config::LibraryKind::GameFiles => "setup-game-directory",
        crate::config::LibraryKind::OfflineInstallers => "setup-installer-directory",
        crate::config::LibraryKind::Extras => "setup-extras-directory",
    });
    let path = draft
        .borrow()
        .default_library(kind)
        .map(|library| library.path.clone())
        .unwrap_or_else(|| {
            crate::config::default_game_directory()
                .parent()
                .unwrap()
                .join(match kind {
                    crate::config::LibraryKind::GameFiles => "games",
                    crate::config::LibraryKind::OfflineInstallers => "installers",
                    crate::config::LibraryKind::Extras => "extras",
                })
        });
    entry.set_text(&path.to_string_lossy());
    let choose = gtk::Button::with_label("Browse…");
    row.append(&entry);
    row.append(&choose);
    group.add(&row);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_xalign(0.0);
    group.add(&status);
    let picking = Rc::new(Cell::new(false));
    choose.connect_clicked({
        let window = window.clone();
        let entry = entry.clone();
        let picking = picking.clone();
        let status = status.clone();
        move |button| {
            if !active() || picking.replace(true) {
                return;
            }
            button.set_sensitive(false);
            let picker = gtk::FileDialog::builder()
                .title("Choose library directory")
                .build();
            let entry = entry.clone();
            let active = active.clone();
            let status = status.clone();
            let button = button.clone();
            let picking = picking.clone();
            picker.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                picking.set(false);
                if !active() {
                    return;
                }
                button.set_sensitive(true);
                match result {
                    Ok(file) => {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                            status.set_label("");
                        } else {
                            status
                                .set_label("Choose a local directory, or enter its absolute path.");
                        }
                    }
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled) => {}
                    Err(_) => status.set_label(
                        "The folder picker could not open. Enter an absolute path or try again.",
                    ),
                }
            });
        }
    });
    let update = gtk::CheckButton::with_label(if kind == crate::config::LibraryKind::GameFiles {
        "Automatically update Depot builds"
    } else {
        "Keep downloaded files up to date"
    });
    update.set_tooltip_text(Some(if kind == crate::config::LibraryKind::GameFiles {
        "Download and apply available updates to installed Depot games during scheduled checks."
    } else {
        "Update existing downloads in these libraries only. This does not download games you have never backed up."
    }));
    update.set_active(match kind {
        crate::config::LibraryKind::GameFiles => draft.borrow().auto_update_galaxy_installations,
        crate::config::LibraryKind::OfflineInstallers => {
            draft.borrow().auto_download_offline_installers
        }
        crate::config::LibraryKind::Extras => draft.borrow().auto_download_extras,
    });
    group.add(&update);
    {
        let draft = draft.clone();
        update.connect_toggled(move |button| match kind {
            crate::config::LibraryKind::GameFiles => {
                draft.borrow_mut().auto_update_galaxy_installations = button.is_active()
            }
            crate::config::LibraryKind::OfflineInstallers => {
                draft.borrow_mut().auto_download_offline_installers = button.is_active()
            }
            crate::config::LibraryKind::Extras => {
                draft.borrow_mut().auto_download_extras = button.is_active()
            }
        });
    }
    SetupLibrary {
        group,
        entry,
        picking,
        update,
        status,
    }
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
    let proton_page = adw::PreferencesPage::new();
    pages.add_named(&proton_page, Some("4"));
    let runtime_page = adw::PreferencesPage::new();
    pages.add_named(&runtime_page, Some("5"));
    let original_libraries = Rc::new(model.borrow().config.clone());
    let library_draft = Rc::new(RefCell::new(original_libraries.as_ref().clone()));
    let accepted_libraries = Rc::new(Cell::new([false; 3]));
    let active: Rc<dyn Fn() -> bool> = Rc::new({
        let model = model.clone();
        let closed = closed.clone();
        move || {
            !closed.get() && model.borrow().account_epoch == epoch && !model.borrow().logout_pending
        }
    });
    let mut library_steps = Vec::new();
    let mut library_flushes = Vec::new();
    let library_saving = Rc::new(Cell::new(0usize));
    for (index, kind) in crate::config::LibraryKind::ALL.into_iter().enumerate() {
        let page = adw::PreferencesPage::new();
        let controls = setup_library(kind, &library_draft, &w.window, active.clone());
        library_flushes.push(connect_library_autosave(
            &controls,
            kind,
            model,
            w,
            &library_draft,
            &library_saving,
        ));
        page.add(&controls.group);
        pages.add_named(&page, Some(&(index + 1).to_string()));
        library_steps.push(controls);
    }
    let library_steps = Rc::new(library_steps);
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
    root.append(&pages);
    let status_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(110)
        .propagate_natural_height(true)
        .child(&status)
        .build();
    root.append(&status_scroll);
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    navigation.set_margin_top(12);
    navigation.set_margin_start(18);
    navigation.set_margin_end(18);
    navigation.set_margin_bottom(18);
    let skip = gtk::Button::with_label("Skip for now");
    skip.set_widget_name("setup-skip");
    let back = gtk::Button::with_label("Back");
    back.set_widget_name("setup-back");
    let skip_library = gtk::Button::with_label("Skip this step");
    skip_library.set_widget_name("setup-skip-library");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let finish = gtk::Button::with_label("Finish setup");
    finish.set_widget_name("setup-next");
    finish.add_css_class("suggested-action");
    navigation.append(&skip);
    navigation.append(&spacer);
    navigation.append(&back);
    navigation.append(&skip_library);
    navigation.append(&finish);
    root.append(&navigation);
    dialog.set_child(Some(&root));
    let render: Rc<dyn Fn()> = Rc::new({
        let step = step.clone();
        let pages = pages.clone();
        let position = position.clone();
        let finish = finish.clone();
        let back = back.clone();
        let skip_library = skip_library.clone();
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
            skip_library.set_visible(matches!(step.get(), 2 | 3));
            finish.set_label(if step.get() == 0 {
                "Let's get started"
            } else if step.get() == STEPS.len() - 1 {
                "Finish setup"
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
            move || active() && step.get() == 5
        }),
    );
    let components_busy: Rc<dyn Fn() -> bool> = Rc::new({
        let proton_busy = proton_download.busy.clone();
        let runtime_busy = runtime_download.busy.clone();
        let step = step.clone();
        let checking = checking_runtime.clone();
        let selection_busy = selection_busy.clone();
        let library_steps = library_steps.clone();
        let library_saving = library_saving.clone();
        move || {
            proton_busy.get()
                || runtime_busy.get()
                || checking.get()
                || (step.get() == 4 && selection_busy.get())
                || library_steps.iter().any(|controls| controls.picking.get())
                || library_saving.get() != 0
        }
    });
    skip.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    skip_library.connect_clicked({
        let accepted = accepted_libraries.clone();
        let step = step.clone();
        let busy = busy.clone();
        let active = active.clone();
        let components_busy = components_busy.clone();
        let render = render.clone();
        move |_| {
            if !active() || busy.get() || components_busy() || !matches!(step.get(), 2 | 3) {
                return;
            }
            let index = step.get() - 1;
            // Skipping does not undo choices already saved by editing this page.
            let mut choices = accepted.get();
            choices[index] = false;
            accepted.set(choices);
            step.set(step.get() + 1);
            render();
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
        let skip_library = skip_library.clone();
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
            if step.get() == 5
                && (last_step != 5
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
            skip_library.set_sensitive(enabled);
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
                let path = (current <= 3).then(|| {
                    std::path::PathBuf::from(library_steps[current - 1].entry.text().as_str())
                });
                let proton_path = selected_path();
                let update = (current <= 3).then(|| library_steps[current - 1].update.is_active());
                let session = online::account_session();
                busy.set(true);
                pages.set_sensitive(false);
                button.set_sensitive(false);
                status.set_label("Checking your choice…");
                let receiver = update_policies::policy_request(move || {
                    if let Some(path) = path {
                        save_setup_library(
                            crate::config::LibraryKind::ALL[current - 1],
                            &path,
                            update.unwrap(),
                            session,
                        )
                        .map(|config| Some((config, true)))
                    } else {
                        (|| -> anyhow::Result<()> {
                            anyhow::ensure!(
                                online::account_session() == session,
                                "The account changed. Reopen setup."
                            );
                            if let Some(path) = proton_path? {
                                let preferences = crate::compatibility::proton_preferences()?;
                                let saved = product_id
                                    .and_then(|id| preferences.overrides.get(&id.to_string()))
                                    .or(preferences.default.as_ref());
                                anyhow::ensure!(
                                    saved == Some(&path),
                                    "Wait for the Proton selection to save, or select it again."
                                );
                            }
                            proton::saved_proton(product_id).map(|_| ())
                        })()
                        .map(|()| None)
                    }
                });
                let active = active.clone();
                let busy = busy.clone();
                let pages = pages.clone();
                let status = status.clone();
                let step = step.clone();
                let render = render.clone();
                let library_draft = library_draft.clone();
                let accepted_libraries = accepted_libraries.clone();
                let model = model.clone();
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    if !active() {
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(result) => {
                            busy.set(false);
                            pages.set_sensitive(true);
                            match result {
                                Ok(library) => {
                                    if let Some((draft, accepted)) = library {
                                        let mut choices = accepted_libraries.get();
                                        choices[current - 1] = accepted;
                                        accepted_libraries.set(choices);
                                        let kind = crate::config::LibraryKind::ALL[current - 1];
                                        merge_setup_library(
                                            &mut model.borrow_mut().config,
                                            &draft,
                                            kind,
                                        );
                                        merge_setup_library(
                                            &mut library_draft.borrow_mut(),
                                            &draft,
                                            kind,
                                        );
                                    }
                                    step.set(current + 1);
                                    render();
                                }
                                Err(error) => {
                                    status.set_label(&format!("Please check this step: {error}"))
                                }
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            busy.set(false);
                            pages.set_sensitive(true);
                            status.set_label(
                                "The check stopped. Your choices are still here; try again.",
                            );
                            glib::ControlFlow::Break
                        }
                    }
                });
                return;
            }
            let session = online::account_session();
            let accepted = accepted_libraries.get();
            button.set_sensitive(false);
            busy.set(true);
            pages.set_sensitive(false);
            dialog.set_can_close(false);
            status.set_label("Verifying setup…");
            let receiver = update_policies::policy_request(move || {
                let _activity = crate::profile_reset::begin_activity("saving setup libraries")?;
                let _permit = crate::operation_gate::try_acquire().map_err(|_| {
                    anyhow::anyhow!(
                        "Finish or pause downloads and installations before changing libraries."
                    )
                })?;
                anyhow::ensure!(
                    online::account_session() == session,
                    "The account changed. Reopen setup."
                );
                let config = Config::load_or_create()?;
                for (index, kind) in crate::config::LibraryKind::ALL.into_iter().enumerate() {
                    if accepted[index] {
                        validate_setup_library(&config, kind, true)?;
                    }
                }
                crate::compatibility::preflight_windows(product_id)
                    .map_err(|error| anyhow::anyhow!(proton::compatibility_message(&error)))?;
                let mut config = Config::load_or_create()?;
                config.setup_seen = true;
                config.setup_completed = true;
                config.windows_setup_deferred = false;
                online::with_account_session(session, || config.save())?;
                Ok(config)
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
                if closed.get()
                    || model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                {
                    dialog.set_can_close(true);
                    dialog.close();
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok(config)) => {
                        let mut state = model.borrow_mut();
                        state.config.setup_seen = config.setup_seen;
                        state.config.setup_completed = config.setup_completed;
                        state.config.windows_setup_deferred = config.windows_setup_deferred;
                        drop(state);
                        completed.set(true);
                        w.finish_setup.set_visible(false);
                        dialog.set_can_close(true);
                        dialog.close();
                        let signed_in =
                            model.borrow().account_token.as_ref().is_some_and(|token| {
                                token.expires_at > chrono::Utc::now().timestamp()
                            });
                        if !signed_in {
                            show_gog_login(&w, &model);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        status.set_label(&format!(
                            "Setup is not complete: {error}. Your saved choices are retained."
                        ));
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
        for flush in &library_flushes {
            flush();
        }
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
        let session = online::account_session();
        let receiver = update_policies::policy_request(move || {
            online::with_account_session(session, || {
                let mut config = Config::load_or_create()?;
                config.setup_seen = true;
                config.save()
            })
        });
        let model = model.clone();
        let w = w.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(())) => {}
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                _ => {
                    show_status(
                        &w,
                        "Setup preferences could not be saved. Open Finish setup and try again.",
                    );
                    w.finish_setup.set_visible(true);
                }
            }
            glib::ControlFlow::Break
        });
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

    #[test]
    fn typed_library_drafts_require_games_but_not_optional_roots() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.game_libraries.clear();
        assert!(
            validate_setup_library(&config, crate::config::LibraryKind::GameFiles, false).is_err()
        );
        config.game_libraries.push(crate::config::GameLibrary {
            id: "draft-game".into(),
            name: "Games".into(),
            path: root.path().join("new-games"),
            default: true,
        });
        validate_setup_library(&config, crate::config::LibraryKind::GameFiles, false).unwrap();
        assert!(!config.game_libraries[0].path.exists());
        assert!(config.offline_libraries.is_empty() && config.extras_libraries.is_empty());
        config.extras_libraries.push(crate::config::GameLibrary {
            id: "draft-extra".into(),
            name: "Extras".into(),
            path: "relative-extras".into(),
            default: true,
        });
        assert!(
            validate_setup_library(&config, crate::config::LibraryKind::Extras, false).is_err()
        );
        assert!(!config.game_libraries[0].path.exists());
    }

    #[test]
    fn library_steps_preserve_unseen_libraries_and_restore_skipped_drafts() {
        use crate::config::{GameLibrary, LibraryKind};
        let root = tempfile::tempdir().unwrap();
        let original = Config {
            game_libraries: vec![
                GameLibrary {
                    id: "extra-game".into(),
                    name: "Other games".into(),
                    path: root.path().join("other"),
                    default: false,
                },
                GameLibrary {
                    id: "selected-game".into(),
                    name: "Chosen games".into(),
                    path: root.path().join("games"),
                    default: true,
                },
            ],
            offline_libraries: vec![
                GameLibrary {
                    id: "selected-installer".into(),
                    name: "Installer default".into(),
                    path: root.path().join("installers"),
                    default: true,
                },
                GameLibrary {
                    id: "extra-installer".into(),
                    name: "Other installers".into(),
                    path: root.path().join("other-installers"),
                    default: false,
                },
            ],
            ..Config::default()
        };
        let mut draft = original.clone();
        assert!(
            prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::GameFiles,
                &root.path().join("changed-games")
            )
            .unwrap()
        );
        assert_eq!(draft.game_libraries[0], original.game_libraries[0]);
        assert_eq!(draft.game_libraries[1].id, "selected-game");
        assert_eq!(draft.game_libraries[1].name, "Chosen games");
        assert!(draft.game_libraries[1].default);
        // Next accepts one optional suggestion. Back and Skip must undo only that draft.
        assert!(
            prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::OfflineInstallers,
                &root.path().join("changed-installers")
            )
            .unwrap()
        );
        draft.auto_download_offline_installers = true;
        assert!(
            !prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::OfflineInstallers,
                std::path::Path::new("")
            )
            .unwrap()
        );
        assert_eq!(draft.offline_libraries, original.offline_libraries);
        assert!(!draft.auto_download_offline_installers);
        assert_eq!(
            draft.game_libraries[1].path,
            root.path().join("changed-games")
        );
        // A later explicit Next can accept the retained entry draft without dropping other copies.
        assert!(
            prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::OfflineInstallers,
                &root.path().join("changed-installers")
            )
            .unwrap()
        );
        assert_eq!(draft.offline_libraries[1], original.offline_libraries[1]);
        assert_eq!(draft.offline_libraries[0].id, "selected-installer");
        assert!(
            !prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::Extras,
                std::path::Path::new("")
            )
            .unwrap()
        );
        assert!(draft.extras_libraries.is_empty());
        assert!(
            prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::GameFiles,
                std::path::Path::new("")
            )
            .is_err()
        );
        assert!(!root.path().join("changed-games").exists());
        assert!(!root.path().join("changed-installers").exists());
    }

    #[test]
    fn missing_library_paths_reject_cross_type_overlap_before_creation() {
        use crate::config::LibraryKind;
        let root = tempfile::tempdir().unwrap();
        let mut original = Config::default();
        original.game_libraries[0].path = root.path().join("games");
        for path in [
            root.path().join("games"),
            root.path().join("games/extras"),
            root.path().to_owned(),
        ] {
            let mut draft = original.clone();
            assert!(
                prepare_setup_library(&original, &mut draft, LibraryKind::Extras, &path).is_err()
            );
        }
        assert!(!original.game_libraries[0].path.exists());
        let mut draft = original.clone();
        assert!(
            prepare_setup_library(
                &original,
                &mut draft,
                LibraryKind::Extras,
                &root.path().join("extras")
            )
            .unwrap()
        );
        assert!(!root.path().join("extras").exists());
    }
}
