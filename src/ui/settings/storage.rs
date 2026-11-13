use super::super::*;
use crate::config::GameLibrary;
use crate::installation;
use anyhow::Context;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct InstalledStorageItem {
    game: crate::domain::InstalledGame,
    title: String,
    artwork: Option<PathBuf>,
    size: u64,
}

#[cfg(test)]
type StorageInspectionResult = anyhow::Result<(
    Option<(u64, u64)>,
    Vec<InstalledStorageItem>,
    u64,
    u64,
    Vec<crate::storage::GameDirectoryIssue>,
)>;

#[cfg(test)]
thread_local! {
    static STORAGE_INSPECTIONS: RefCell<Option<Vec<mpsc::Sender<StorageInspectionResult>>>> = const { RefCell::new(None) };
    static STORAGE_INSPECTION_DELAYS: RefCell<std::collections::VecDeque<mpsc::Receiver<()>>> = const { RefCell::new(std::collections::VecDeque::new()) };
}

#[derive(Clone)]
struct StorageListView {
    list: gtk::ListBox,
    checks: Rc<RefCell<HashMap<i64, gtk::CheckButton>>>,
    count: gtk::Label,
    update_selection: Rc<dyn Fn()>,
}

fn connect_move_selection(
    items: &Rc<RefCell<Vec<InstalledStorageItem>>>,
    checks: &Rc<RefCell<HashMap<i64, gtk::CheckButton>>>,
    libraries: &Rc<RefCell<Vec<GameLibrary>>>,
    target: &gtk::DropDown,
    selected: &gtk::Label,
    button: &gtk::Button,
    moving: &Rc<std::cell::Cell<bool>>,
) -> Rc<dyn Fn()> {
    let update: Rc<dyn Fn()> = {
        let items = items.clone();
        let checks = Rc::downgrade(checks);
        let libraries = libraries.clone();
        let target = target.downgrade();
        let selected = selected.downgrade();
        let button = button.downgrade();
        let moving = moving.clone();
        Rc::new(move || {
            let (Some(checks), Some(target), Some(selected), Some(button)) = (
                checks.upgrade(),
                target.upgrade(),
                selected.upgrade(),
                button.upgrade(),
            ) else {
                return;
            };
            let items = items.borrow();
            let checks = checks.borrow();
            let chosen = items
                .iter()
                .filter(|item| {
                    checks
                        .get(&item.game.product_id)
                        .is_some_and(gtk::CheckButton::is_active)
                })
                .collect::<Vec<_>>();
            let count = chosen.len();
            selected.set_label(&if count == 0 {
                "No games selected".to_owned()
            } else {
                format!("{count} game{} selected", if count == 1 { "" } else { "s" })
            });
            let libraries = libraries.borrow();
            let destination = libraries.get(target.selected() as usize);
            let reason = if moving.get() {
                Some("Wait for the current move to finish.")
            } else if chosen.is_empty() {
                Some("Select at least one game to move.")
            } else if chosen.iter().any(|item| {
                item.game.compatibility.is_some()
                    || item
                        .game
                        .installer_operating_system
                        .as_deref()
                        .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
            }) {
                Some(
                    "Windows games cannot be moved here because their compatibility prefix would also need to move. Uninstall and reinstall into the other library.",
                )
            } else if let Some(destination) = destination {
                chosen
                    .iter()
                    .all(|item| item.game.library_id == destination.id)
                    .then_some("Choose a different destination library.")
            } else {
                Some("Choose a destination library.")
            };
            button.set_sensitive(reason.is_none());
            button.set_tooltip_text(reason);
        })
    };
    target.connect_selected_notify({
        let update = update.clone();
        move |_| update()
    });
    update();
    update
}

#[derive(Debug, Clone, Copy, Default)]
struct StorageBreakdown {
    total: u64,
    free: u64,
    games: u64,
    installers: u64,
    extras: u64,
}

impl StorageBreakdown {
    fn others(self) -> u64 {
        self.total
            .saturating_sub(self.free)
            .saturating_sub(self.games)
            .saturating_sub(self.installers)
            .saturating_sub(self.extras)
    }
}

fn save_storage_config(
    page: &gtk::Box,
    status: &gtk::Label,
    model: &Rc<RefCell<AppModel>>,
    config: Config,
    saved: impl FnOnce(Config) + 'static,
) {
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let previous = status.label();
    status.set_label("Saving library settings…");
    let window = page.root().and_downcast::<gtk::Window>();
    if let Some(window) = &window {
        window.set_sensitive(false);
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let _activity = crate::profile_reset::begin_activity("saving library settings")?;
            let current = crate::storage::read_config()?;
            let changed = crate::config::LibraryKind::ALL.into_iter().any(|kind| {
                current
                    .libraries(kind)
                    .iter()
                    .map(|item| (&item.id, &item.path))
                    .collect::<std::collections::BTreeSet<_>>()
                    != config
                        .libraries(kind)
                        .iter()
                        .map(|item| (&item.id, &item.path))
                        .collect()
            });
            let _permit = if changed {
                Some(crate::operation_gate::try_acquire().map_err(|_| {
                    anyhow::anyhow!(
                        "Finish or pause downloads and installations before changing libraries."
                    )
                })?)
            } else {
                None
            };
            online::with_account_session(session, || config.save())?;
            Ok::<_, anyhow::Error>(config)
        })();
        let _ = sender.send(result);
    });
    let status = status.clone();
    let model = model.clone();
    let mut saved = Some(saved);
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if model.borrow().account_epoch != epoch {
            if let Some(window) = &window {
                window.set_sensitive(true);
            }
            status.set_label("Account changed; reopen Settings to review library folders.");
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!("Saving stopped; try again")),
        };
        if let Some(window) = &window {
            window.set_sensitive(true);
        }
        match result {
            Ok(config) => {
                status.set_label(&previous);
                saved.take().unwrap()(config);
            }
            Err(error) => status.set_label(&format!("Could not save library settings: {error}")),
        }
        glib::ControlFlow::Break
    });
}

pub(super) fn build_storage_page(
    window: &adw::ApplicationWindow,
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    kind: crate::config::LibraryKind,
    title: &str,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&build_storage_section(window, w, model, kind, title))
        .build();
    root.append(&scroll);
    root
}

fn build_storage_section(
    window: &adw::ApplicationWindow,
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    kind: crate::config::LibraryKind,
    title: &str,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.add_css_class("storage-settings-page");
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("title-1");
    root.append(&title);

    let libraries = Rc::new(RefCell::new(model.borrow().config.libraries(kind).to_vec()));
    let library_names = gtk::StringList::new(
        &libraries
            .borrow()
            .iter()
            .map(|library| library.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let library = gtk::DropDown::new(Some(library_names.clone()), gtk::Expression::NONE);
    library.set_selected(
        libraries
            .borrow()
            .iter()
            .position(|library| library.default)
            .unwrap_or(0) as u32,
    );
    let library_menu = gtk::MenuButton::new();
    library_menu.set_widget_name("storage-library-menu");
    library_menu.set_hexpand(true);
    library_menu.add_css_class("storage-library-menu");
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.append(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
    let selected_mount = gtk::Label::new(None);
    selected_mount.set_xalign(0.0);
    selected_mount.set_hexpand(true);
    selected_mount.add_css_class("storage-library-path");
    toolbar.append(&selected_mount);
    let capacity = gtk::Label::new(Some("Calculating storage…"));
    capacity.set_widget_name("storage-capacity");
    capacity.add_css_class("storage-capacity-label");
    toolbar.append(&capacity);
    toolbar.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    library_menu.set_child(Some(&toolbar));
    let library_popover = gtk::Popover::new();
    let library_choices = gtk::Box::new(gtk::Orientation::Vertical, 2);
    library_choices.add_css_class("storage-library-choices");
    library_popover.set_child(Some(&library_choices));
    library_menu.set_popover(Some(&library_popover));
    root.append(&library_menu);

    let add = gtk::Button::with_label("Add Directory");
    add.add_css_class("flat");
    let rebuild_library_choices = {
        let choices = library_choices.clone();
        let libraries = libraries.clone();
        let model = model.clone();
        let library = library.clone();
        let popover = library_popover.clone();
        let add = add.clone();
        Rc::new(move || {
            if model.borrow().logout_pending {
                return;
            }
            let epoch = model.borrow().account_epoch;
            let session = (online::account_session(), auth::session());
            while let Some(child) = choices.first_child() {
                choices.remove(&child);
            }
            let mut labels = Vec::new();
            for (index, entry) in libraries.borrow().iter().cloned().enumerate() {
                let row = gtk::Button::new();
                row.set_widget_name(&format!("storage-library-choice-{index}"));
                row.add_css_class("flat");
                row.add_css_class("storage-library-choice");
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                content.append(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
                let mount = entry.path.to_string_lossy().into_owned();
                let name = gtk::Label::new(Some(&mount));
                name.set_xalign(0.0);
                name.set_hexpand(true);
                name.add_css_class("storage-library-path");
                content.append(&name);
                if entry.default {
                    let default = gtk::Image::from_icon_name("starred-symbolic");
                    default.add_css_class("storage-default-library");
                    default.set_tooltip_text(Some("Default library for this type"));
                    content.append(&default);
                }
                let status = gtk::Label::new(Some("Checking…"));
                status.set_wrap(true);
                status.set_max_width_chars(22);
                content.append(&status);
                labels.push((entry.id.clone(), status));
                row.set_child(Some(&content));
                row.connect_clicked({
                    let library = library.clone();
                    let popover = popover.clone();
                    move |_| {
                        library.set_selected(index as u32);
                        popover.popdown();
                    }
                });
                choices.append(&row);
            }
            choices.append(&add);
            #[cfg(test)]
            if STORAGE_INSPECTIONS.with_borrow(Option::is_some) {
                return;
            }
            let config = model.borrow().config.clone();
            let activity = match crate::profile_reset::begin_activity("checking library choices") {
                Ok(activity) => activity,
                Err(error) => {
                    for (_, label) in &labels {
                        label.set_label("Inspection stopped");
                        label.set_tooltip_text(Some(&error.to_string()));
                    }
                    return;
                }
            };
            #[cfg(test)]
            let delay = STORAGE_INSPECTION_DELAYS.with_borrow_mut(|delays| delays.pop_front());
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = (|| {
                    let _activity = activity;
                    #[cfg(test)]
                    if let Some(delay) = delay {
                        delay.recv()?;
                    }
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed; reopen Storage settings."
                    );
                    crate::storage::inspect_libraries(&config)
                })();
                let _ = sender.send(result);
            });
            let model = model.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                    || session != (online::account_session(), auth::session())
                {
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(result) => {
                        for (id, label) in &labels {
                            let status = result.as_ref().ok().and_then(|all| {
                                all.iter()
                                    .find(|status| status.kind == kind && &status.library_id == id)
                            });
                            let (text, reason) = match status.map(|status| &status.compatibility) {
                                Some(crate::storage::LibraryCompatibility::Compatible) => {
                                    ("Compatible", None)
                                }
                                Some(crate::storage::LibraryCompatibility::Incompatible(
                                    reason,
                                )) => ("Incompatible", Some(reason.as_str())),
                                Some(crate::storage::LibraryCompatibility::Unavailable(reason)) => {
                                    ("Unavailable", Some(reason.as_str()))
                                }
                                None => ("Inspection failed", None),
                            };
                            label.set_label(text);
                            label.set_tooltip_text(reason);
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => {
                        for (_, label) in &labels {
                            label.set_label("Inspection stopped");
                        }
                    }
                }
                glib::ControlFlow::Break
            });
        })
    };
    rebuild_library_choices();

    let path_label = gtk::Label::new(None);
    path_label.set_widget_name("storage-inspection-status");
    path_label.set_xalign(0.0);
    path_label.add_css_class("storage-path-label");
    root.append(&path_label);
    path_label.set_wrap(true);
    path_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    path_label.set_selectable(true);
    let recheck = gtk::Button::with_label("Recheck library");
    recheck.set_widget_name("storage-recheck");
    recheck.set_halign(gtk::Align::Start);
    root.append(&recheck);
    let game_issues = gtk::Box::new(gtk::Orientation::Vertical, 10);
    game_issues.set_visible(false);
    root.append(&game_issues);
    if kind != crate::config::LibraryKind::GameFiles {
        let updates = gtk::CheckButton::with_label("Keep downloaded files up to date");
        updates.set_tooltip_text(Some("Only update files already downloaded into this library type. Does not download other owned games."));
        updates.set_active(if kind == crate::config::LibraryKind::OfflineInstallers {
            model.borrow().config.auto_download_offline_installers
        } else {
            model.borrow().config.auto_download_extras
        });
        updates.connect_toggled({
            let model = model.clone();
            let root = root.clone();
            let status = path_label.clone();
            move |button| {
                let previous = if kind == crate::config::LibraryKind::OfflineInstallers {
                    model.borrow().config.auto_download_offline_installers
                } else {
                    model.borrow().config.auto_download_extras
                };
                if previous == button.is_active() {
                    return;
                }
                let mut config = model.borrow().config.clone();
                let requested = button.is_active();
                if kind == crate::config::LibraryKind::OfflineInstallers {
                    config.auto_download_offline_installers = requested;
                } else {
                    config.auto_download_extras = requested;
                }
                button.set_active(previous);
                let model = model.clone();
                let button = button.clone();
                save_storage_config(&root, &status, &model.clone(), config, move |config| {
                    model.borrow_mut().config = config;
                    button.set_active(requested);
                });
            }
        });
        root.append(&updates);
    }
    let usage = gtk::DrawingArea::new();
    usage.set_height_request(12);
    usage.add_css_class("storage-usage-bar");
    let usage_values = Rc::new(RefCell::new(StorageBreakdown::default()));
    usage.set_draw_func({
        let values = usage_values.clone();
        move |_, context, width, height| {
            let breakdown = *values.borrow();
            let total = breakdown.total.max(1) as f64;
            let segments = [
                (breakdown.games, (0.10, 0.62, 0.94)),
                (breakdown.installers, (0.73, 0.38, 0.82)),
                (breakdown.extras, (0.25, 0.69, 0.39)),
                (breakdown.others(), (0.95, 0.72, 0.18)),
                (breakdown.free, (0.30, 0.33, 0.38)),
            ];
            let mut offset = 0.0;
            for (bytes, (red, green, blue)) in segments {
                let segment_width =
                    (bytes as f64 / total * width as f64).clamp(0.0, width as f64 - offset);
                context.set_source_rgb(red, green, blue);
                context.rectangle(offset, 0.0, segment_width, height as f64);
                let _ = context.fill();
                offset += segment_width;
            }
        }
    });
    let usage_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    usage_row.append(&gtk::Label::new(Some("Drive usage")));
    usage.set_hexpand(true);
    usage_row.append(&usage);
    let manage_library = gtk::MenuButton::new();
    manage_library.set_icon_name("view-more-symbolic");
    manage_library.set_tooltip_text(Some("Manage selected library"));
    manage_library.add_css_class("flat");
    let manage_popover = gtk::Popover::new();
    let manage_actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
    manage_actions.set_margin_top(6);
    manage_actions.set_margin_bottom(6);
    manage_actions.set_margin_start(6);
    manage_actions.set_margin_end(6);
    let default_games = gtk::Button::with_label("Make default");
    let rename_library = gtk::Button::with_label("Rename library");
    let remove_library = gtk::Button::with_label("Remove library");
    for action in [&default_games, &rename_library, &remove_library] {
        action.add_css_class("flat");
        action.set_halign(gtk::Align::Fill);
        manage_actions.append(action);
    }
    manage_popover.set_child(Some(&manage_actions));
    manage_library.set_popover(Some(&manage_popover));
    usage_row.append(&manage_library);
    root.append(&usage_row);
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    legend.add_css_class("storage-legend");
    let (games_legend, games_size) = storage_legend_item("Games", "storage-games");
    let (installers_legend, installers_size) =
        storage_legend_item("Installers", "storage-installers");
    let (extras_legend, extras_size) = storage_legend_item("Extras", "storage-extras");
    let (others_legend, others_size) = storage_legend_item("Other", "storage-others");
    let (free_legend, free_size) = storage_legend_item("Free", "storage-free");
    for item in [
        games_legend,
        installers_legend,
        extras_legend,
        others_legend,
        free_legend,
    ] {
        legend.append(&item);
    }
    root.append(&legend);
    let usage_explanation = gtk::Label::new(Some(
        "Capacity and free space cover the drive. Games, Installers and Extras count this library; Other is the remaining used or unavailable space.",
    ));
    usage_explanation.set_xalign(0.0);
    usage_explanation.set_wrap(true);
    usage_explanation.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    usage_explanation.add_css_class("dim-label");
    root.append(&usage_explanation);

    let list_header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let item_count = gtk::Label::new(Some("Installed games"));
    item_count.set_xalign(0.0);
    item_count.set_hexpand(true);
    item_count.add_css_class("title-3");
    list_header.append(&item_count);
    let sort = gtk::DropDown::from_strings(&["Size on disk", "Alphabetical", "Last played"]);
    sort.set_tooltip_text(Some("Sort installed games"));
    list_header.append(&sort);
    list_header.set_visible(kind == crate::config::LibraryKind::GameFiles);
    root.append(&list_header);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("storage-game-list");
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .max_content_height(320)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    scroll.set_visible(kind == crate::config::LibraryKind::GameFiles);
    root.append(&scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let selected = gtk::Label::new(Some("No games selected"));
    selected.set_hexpand(true);
    selected.set_xalign(0.0);
    footer.append(&selected);
    let target = gtk::DropDown::new(Some(library_names.clone()), gtk::Expression::NONE);
    target.set_tooltip_text(Some("Destination library"));
    footer.append(&target);
    let move_button = gtk::Button::with_label("Move");
    move_button.set_sensitive(false);
    footer.append(&move_button);
    footer.set_visible(kind == crate::config::LibraryKind::GameFiles);
    root.append(&footer);
    let move_progress = gtk::ProgressBar::new();
    move_progress.set_visible(false);
    root.append(&move_progress);

    let items = Rc::new(RefCell::new(Vec::<InstalledStorageItem>::new()));
    let checks = Rc::new(RefCell::new(HashMap::<i64, gtk::CheckButton>::new()));
    let moving = Rc::new(std::cell::Cell::new(false));
    let update_selection = connect_move_selection(
        &items,
        &checks,
        &libraries,
        &target,
        &selected,
        &move_button,
        &moving,
    );
    let list_view = StorageListView {
        list: list.clone(),
        checks: checks.clone(),
        count: item_count.clone(),
        update_selection: update_selection.clone(),
    };
    let refresh = {
        let list_view = list_view.clone();
        let items = items.clone();
        let sort = sort.clone();
        Rc::new(move || {
            render_storage_items(&list_view, &items.borrow(), sort.selected());
        })
    };
    let update_library = {
        let model = model.clone();
        let libraries = libraries.clone();
        let library = library.clone();
        let selected_mount = selected_mount.clone();
        let path_label = path_label.clone();
        let capacity = capacity.clone();
        let usage = usage.clone();
        let usage_values = usage_values.clone();
        let games_size = games_size.clone();
        let installers_size = installers_size.clone();
        let extras_size = extras_size.clone();
        let others_size = others_size.clone();
        let free_size = free_size.clone();
        let items = items.clone();
        let refresh = refresh.clone();
        let footer = footer.clone();
        let manage_library = manage_library.clone();
        let remove_library = remove_library.clone();
        let default_games = default_games.clone();
        let request = Rc::new(std::cell::Cell::new(0u64));
        let game_issues = game_issues.clone();
        let window = window.clone();
        let w = w.clone();
        let recheck = recheck.downgrade();
        Rc::new(move || {
            if model.borrow().logout_pending {
                path_label
                    .set_label("Account change in progress. Recheck library when it finishes.");
                if let Some(recheck) = recheck.upgrade() {
                    recheck.set_label("Recheck library");
                    recheck.set_sensitive(true);
                }
                return;
            }
            let epoch = model.borrow().account_epoch;
            let session = (online::account_session(), auth::session());
            request.set(request.get().wrapping_add(1));
            if let Some(recheck) = recheck.upgrade() {
                recheck.set_sensitive(false);
                recheck.set_label("Checking library…");
            }
            footer.set_sensitive(false);
            *usage_values.borrow_mut() = StorageBreakdown::default();
            for label in [
                &games_size,
                &installers_size,
                &extras_size,
                &others_size,
                &free_size,
            ] {
                label.set_label("—");
            }
            usage.queue_draw();
            while let Some(child) = game_issues.first_child() {
                game_issues.remove(&child);
            }
            game_issues.set_visible(false);
            let Some(selected_library) =
                libraries.borrow().get(library.selected() as usize).cloned()
            else {
                if let Some(recheck) = recheck.upgrade() {
                    recheck.set_label("Recheck library");
                }
                manage_library.set_sensitive(false);
                selected_mount.set_label("Choose or add a directory");
                path_label
                    .set_label("No library configured. Add a directory to use this library type.");
                capacity.set_label("");
                items.borrow_mut().clear();
                refresh();
                footer.set_sensitive(false);
                return;
            };
            manage_library.set_sensitive(true);
            let removable =
                kind != crate::config::LibraryKind::GameFiles || libraries.borrow().len() > 1;
            remove_library.set_sensitive(removable);
            remove_library.set_tooltip_text(
                (!removable).then_some("Add another Game Files library before removing this one."),
            );
            default_games.set_sensitive(!selected_library.default);
            default_games.set_tooltip_text(
                selected_library
                    .default
                    .then_some("This is already the default library."),
            );
            let mount = selected_library.path.to_string_lossy().into_owned();
            selected_mount.set_label(&mount);
            path_label.set_label(&selected_library.path.display().to_string());
            capacity.set_label("Calculating storage…");
            items.borrow_mut().clear();
            refresh();
            let all_libraries = vec![selected_library.clone()];
            let recovery_library_id = selected_library.id.clone();
            let titles = model_game_display_data(&model.borrow());
            let config = model.borrow().config.clone();
            let activity = match crate::profile_reset::begin_activity("inspecting selected library")
            {
                Ok(activity) => activity,
                Err(error) => {
                    path_label.set_label(&format!("Could not inspect library: {error}"));
                    capacity.set_label("Storage information unavailable");
                    if let Some(recheck) = recheck.upgrade() {
                        recheck.set_label("Recheck library");
                        recheck.set_sensitive(true);
                    }
                    return;
                }
            };
            let generation = request.get();
            let (sender, receiver) = mpsc::channel();
            #[cfg(test)]
            let captured = STORAGE_INSPECTIONS.with_borrow_mut(|requests| {
                if let Some(requests) = requests {
                    requests.push(sender.clone());
                    true
                } else {
                    false
                }
            });
            #[cfg(not(test))]
            let captured = false;
            if !captured {
                #[cfg(test)]
                let delay = STORAGE_INSPECTION_DELAYS.with_borrow_mut(|delays| delays.pop_front());
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<_> {
                        let _activity = activity;
                        #[cfg(test)]
                        if let Some(delay) = delay {
                            delay.recv()?;
                        }
                        anyhow::ensure!(
                            session == (online::account_session(), auth::session()),
                            "Account changed; reopen Storage settings."
                        );
                        let inspection = crate::storage::inspect_library_status(
                            &config,
                            kind,
                            &selected_library.id,
                        )?;
                        match inspection.compatibility {
                            crate::storage::LibraryCompatibility::Compatible => {}
                            crate::storage::LibraryCompatibility::Incompatible(reason)
                            | crate::storage::LibraryCompatibility::Unavailable(reason) => {
                                anyhow::bail!(reason)
                            }
                        }
                        let storage = filesystem_storage(&selected_library.path);
                        anyhow::ensure!(
                            session == (online::account_session(), auth::session()),
                            "Account changed; reopen Storage settings."
                        );
                        let store = StateStore::open()?;
                        let games = if kind == crate::config::LibraryKind::GameFiles {
                            crate::installation::reconcile_installed_games(&store, &all_libraries)?
                                .into_iter()
                                .filter_map(|mut game| {
                                    let (library_id, directory) =
                                        crate::installation::resolve_installation_directory(
                                            &game,
                                            &all_libraries,
                                        )?;
                                    if library_id != selected_library.id {
                                        return None;
                                    }
                                    game.library_id = library_id;
                                    game.installation_directory = directory;
                                    let (title, artwork) =
                                        titles.get(&game.product_id).cloned().unwrap_or_else(
                                            || (format!("Product {}", game.product_id), None),
                                        );
                                    let size = directory_size(&game.installation_directory);
                                    Some(InstalledStorageItem {
                                        game,
                                        title,
                                        artwork,
                                        size,
                                    })
                                })
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };
                        let managed = store.managed_files()?;
                        let installers = managed
                            .iter()
                            .filter(|file| {
                                file.present
                                    && matches!(
                                        file.kind,
                                        ArtifactKind::Installer | ArtifactKind::Patch
                                    )
                                    && file.path.starts_with(&selected_library.path)
                            })
                            .map(|file| file.size)
                            .sum::<u64>();
                        let extras = managed
                            .iter()
                            .filter(|file| {
                                file.present
                                    && file.kind == ArtifactKind::Extra
                                    && file.path.starts_with(&selected_library.path)
                            })
                            .map(|file| file.size)
                            .sum::<u64>();
                        Ok((storage, games, installers, extras, inspection.game_issues))
                    })();
                    let _ = sender.send(result);
                });
            }
            let capacity = capacity.clone();
            let usage = usage.clone();
            let usage_values = usage_values.clone();
            let items = items.clone();
            let refresh = refresh.clone();
            let games_size = games_size.clone();
            let installers_size = installers_size.clone();
            let extras_size = extras_size.clone();
            let others_size = others_size.clone();
            let free_size = free_size.clone();
            let model = model.clone();
            let request = request.clone();
            let path_label = path_label.clone();
            let footer = footer.clone();
            let game_issues = game_issues.clone();
            let window = window.clone();
            let w = w.clone();
            let recheck = recheck.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if request.get() != generation {
                    return glib::ControlFlow::Break;
                }
                if model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                    || session != (online::account_session(), auth::session())
                {
                    if let Some(recheck) = recheck.upgrade() {
                        recheck.set_label("Recheck library");
                        recheck.set_sensitive(true);
                    }
                    path_label.set_label(
                        "Account changed. Recheck library to inspect the current account.",
                    );
                    capacity.set_label("Storage information unavailable");
                    return glib::ControlFlow::Break;
                }
                let result = receiver.try_recv();
                if !matches!(&result, Err(mpsc::TryRecvError::Empty))
                    && let Some(recheck) = recheck.upgrade()
                {
                    recheck.set_label("Recheck library");
                    recheck.set_sensitive(true);
                }
                match result {
                    Ok(Ok((storage, games, installers, extras, issues))) => {
                        path_label.set_label(if issues.is_empty() { "Compatible" } else { "Library available. Some game folders need attention; other games remain usable." });
                        game_issues.set_visible(!issues.is_empty());
                        for issue in issues {
                            let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
                            let message = gtk::Label::new(Some(&format!(
                                "{}\n{}",
                                issue.path.display(),
                                issue.reason
                            )));
                            message.set_wrap(true);
                            message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                            message.set_selectable(true);
                            message.set_xalign(0.0);
                            row.append(&message);
                            let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            let browse = gtk::Button::with_label("Browse Files");
                            browse.connect_clicked({
                                let window = window.clone();
                                let model = model.clone();
                                let path = issue.path.clone();
                                move |button| {
                                    if model.borrow().account_epoch == epoch
                                        && !model.borrow().logout_pending
                                    {
                                        browse_recovery_directory(
                                            &window,
                                            &model,
                                            path.clone(),
                                            button,
                                        );
                                    }
                                }
                            });
                            buttons.append(&browse);
                            let known = {
                                let state = model.borrow();
                                let mut matches = state.games.iter().filter(|game| {
                                    issue.path.file_name().and_then(|name| name.to_str())
                                        == Some(game.slug.as_str())
                                });
                                matches
                                    .next()
                                    .filter(|_| matches.next().is_none())
                                    .map(|game| {
                                        DetailPageModel::game(
                                            game.clone(),
                                            state.favorites.contains(&game.product_id),
                                        )
                                    })
                            };
                            if let Some(game) = known {
                                let repair = gtk::Button::with_label("Review Repair…");
                                repair.connect_clicked({
                                    let window = window.clone();
                                    let model = model.clone();
                                    let game = game.clone();
                                    let directory = issue.path.clone();
                                    move |_| {
                                        if model.borrow().account_epoch == epoch
                                            && !model.borrow().logout_pending
                                        {
                                            show_directory_repair_dialog(
                                                &window,
                                                &model,
                                                &game,
                                                directory.clone(),
                                            );
                                        }
                                    }
                                });
                                buttons.append(&repair);
                                let reset = gtk::Button::with_label("Review File Reset…");
                                reset.connect_clicked({
                                    let window = window.clone();
                                    let model = model.clone();
                                    let w = w.clone();
                                    let recheck = recheck.clone();
                                    let recovery_library_id = recovery_library_id.clone();
                                    move |_| {
                                        if model.borrow().account_epoch != epoch
                                            || model.borrow().logout_pending
                                        {
                                            return;
                                        }
                                        let refresh: Rc<dyn Fn()> = Rc::new({
                                            let w = w.clone();
                                            let model = model.clone();
                                            let recheck = recheck.clone();
                                            move || {
                                                super::refresh_installed_state_after_library_change(
                                                    &w, &model,
                                                );
                                                if let Some(recheck) = recheck.upgrade() {
                                                    recheck.emit_clicked();
                                                }
                                            }
                                        });
                                        super::super::uninstall::show_game_directory_reset_dialog(
                                            &window,
                                            &model,
                                            &game,
                                            recovery_library_id.clone(),
                                            refresh,
                                        );
                                    }
                                });
                                buttons.append(&reset);
                            } else {
                                let explanation = gtk::Label::new(Some(
                                    "This folder is not linked to a game in this profile. Browse it before deciding what to keep; Ludomere will not delete unidentified contents.",
                                ));
                                explanation.set_wrap(true);
                                row.append(&explanation);
                            }
                            row.append(&buttons);
                            game_issues.append(&row);
                        }
                        footer.set_sensitive(true);
                        let games_bytes = games.iter().map(|game| game.size).sum::<u64>();
                        if let Some((total, free)) = storage {
                            capacity.set_label(&format!(
                                "{} free of {}",
                                human_size(free),
                                human_size(total)
                            ));
                            let breakdown = StorageBreakdown {
                                total,
                                free,
                                games: games_bytes,
                                installers,
                                extras,
                            };
                            *usage_values.borrow_mut() = breakdown;
                            games_size.set_label(&human_size(breakdown.games));
                            installers_size.set_label(&human_size(breakdown.installers));
                            extras_size.set_label(&human_size(breakdown.extras));
                            others_size.set_label(&human_size(breakdown.others()));
                            free_size.set_label(&human_size(breakdown.free));
                            usage.queue_draw();
                        } else {
                            capacity.set_label("Storage information unavailable");
                        }
                        *items.borrow_mut() = games;
                        refresh();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        path_label.set_label(&format!("Incompatible or unavailable: {error:#}\nCorrect the directory contents, then Recheck library. Files are not moved or deleted."));
                        capacity.set_label("Content actions disabled");
                        footer.set_sensitive(false);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        path_label.set_label("Library inspection stopped unexpectedly. Click Recheck library to try again.");
                        capacity.set_label("Storage information unavailable");
                        footer.set_sensitive(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        })
    };
    recheck.connect_clicked({
        let update = update_library.clone();
        let rebuild = rebuild_library_choices.clone();
        move |_| {
            rebuild();
            update();
        }
    });
    default_games.connect_clicked({
        let root = root.clone();
        let status = path_label.clone();
        let model = model.clone();
        let libraries = libraries.clone();
        let library = library.clone();
        let rebuild = rebuild_library_choices.clone();
        let update = update_library.clone();
        let popover = manage_popover.clone();
        move |_| {
            let Some(selected) = libraries.borrow().get(library.selected() as usize).cloned()
            else {
                return;
            };
            let mut config = model.borrow().config.clone();
            for entry in config.libraries_mut(kind) {
                entry.default = entry.id == selected.id;
            }
            config.normalize_libraries();
            let model = model.clone();
            let libraries = libraries.clone();
            let rebuild = rebuild.clone();
            let update = update.clone();
            save_storage_config(&root, &status, &model.clone(), config, move |config| {
                *libraries.borrow_mut() = config.libraries(kind).to_vec();
                model.borrow_mut().config = config;
                rebuild();
                update();
            });
            popover.popdown();
        }
    });
    rename_library.connect_clicked({
        let root = root.clone();
        let status = path_label.clone();
        let window = window.clone();
        let model = model.clone();
        let libraries = libraries.clone();
        let library = library.clone();
        let rebuild = rebuild_library_choices.clone();
        let popover = manage_popover.clone();
        move |_| {
            popover.popdown();
            let Some(selected) = libraries.borrow().get(library.selected() as usize).cloned()
            else {
                return;
            };
            present_library_rename_dialog(
                &window,
                &selected,
                kind,
                (&root, &status),
                model.clone(),
                libraries.clone(),
                rebuild.clone(),
            );
        }
    });
    remove_library.connect_clicked({
        let root = root.clone();
        let status = path_label.clone();
        let window = window.clone();
        let w = w.clone();
        let model = model.clone();
        let libraries = libraries.clone();
        let names = library_names.clone();
        let library = library.clone();
        let rebuild = rebuild_library_choices.clone();
        let update = update_library.clone();
        let popover = manage_popover.clone();
        move |_| {
            popover.popdown();
            let index = library.selected() as usize;
            let Some(selected) = libraries.borrow().get(index).cloned() else {
                return;
            };
            if kind == crate::config::LibraryKind::GameFiles && libraries.borrow().len() <= 1 {
                return;
            }
            let confirmation = adw::AlertDialog::builder()
                .heading("Remove this library?")
                .body(format!(
                    "{} will no longer be scanned. No files will be deleted.",
                    selected.name
                ))
                .build();
            confirmation.add_response("cancel", "Cancel");
            confirmation.add_response("remove", "Remove");
            confirmation.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            let w = w.clone();
            let model = model.clone();
            let libraries = libraries.clone();
            let names = names.clone();
            let library = library.clone();
            let rebuild = rebuild.clone();
            let update = update.clone();
            let root = root.clone();
            let status = status.clone();
            let epoch = model.borrow().account_epoch;
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "remove"
                    || model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                {
                    return;
                }
                let mut config = model.borrow().config.clone();
                config
                    .libraries_mut(kind)
                    .retain(|entry| entry.id != selected.id);
                config.normalize_libraries();
                save_storage_config(&root, &status, &model.clone(), config, move |config| {
                    model.borrow_mut().config = config.clone();
                    *libraries.borrow_mut() = config.libraries(kind).to_vec();
                    names.splice(
                        0,
                        names.n_items(),
                        &config
                            .libraries(kind)
                            .iter()
                            .map(|entry| entry.path.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>(),
                    );
                    library.set_selected(
                        config
                            .libraries(kind)
                            .len()
                            .checked_sub(1)
                            .map_or(gtk::INVALID_LIST_POSITION, |last| index.min(last) as u32),
                    );
                    rebuild();
                    update();
                    super::refresh_installed_state_after_library_change(&w, &model);
                });
            });
        }
    });
    library.connect_selected_notify({
        let update_library = update_library.clone();
        move |_| update_library()
    });
    sort.connect_selected_notify({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    add.connect_clicked({
        let window = window.clone();
        let model = model.clone();
        let libraries = libraries.clone();
        let library_names = library_names.clone();
        let library = library.clone();
        let rebuild_library_choices = rebuild_library_choices.clone();
        let root = root.clone();
        let status = path_label.clone();
        let w = w.clone();
        move |_| {
            let picker = gtk::FileDialog::builder()
                .title("Add library directory")
                .modal(true)
                .build();
            let model = model.clone();
            let libraries = libraries.clone();
            let library_names = library_names.clone();
            let library = library.clone();
            let rebuild_library_choices = rebuild_library_choices.clone();
            let root = root.clone();
            let status = status.clone();
            let w = w.clone();
            let epoch = model.borrow().account_epoch;
            picker.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return;
                }
                let folder = match result {
                    Ok(folder) => folder,
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled) =>
                    {
                        return;
                    }
                    Err(error) => {
                        status.set_label(&format!("Could not choose a library directory: {error}"));
                        return;
                    }
                };
                let Some(path) = folder.path() else {
                    status.set_label("Choose a local directory for this library.");
                    return;
                };
                let existing = libraries
                    .borrow()
                    .iter()
                    .position(|entry| entry.path == path);
                if let Some(index) = existing {
                    library.set_selected(index as u32);
                    status.set_label("This directory is already configured and has been selected.");
                    return;
                }
                let entry = GameLibrary {
                    id: crate::config::game_library_id(&path),
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Games".into()),
                    path,
                    default: false,
                };
                let mut config = model.borrow().config.clone();
                config.libraries_mut(kind).push(entry.clone());
                config.normalize_libraries();
                save_storage_config(&root, &status, &model.clone(), config, move |config| {
                    model.borrow_mut().config = config.clone();
                    *libraries.borrow_mut() = config.libraries(kind).to_vec();
                    library_names.append(&entry.path.to_string_lossy());
                    rebuild_library_choices();
                    library.set_selected((libraries.borrow().len() - 1) as u32);
                    super::refresh_installed_state_after_library_change(&w, &model);
                });
            });
        }
    });
    move_button.connect_clicked({
        let w = w.clone();
        let model = model.clone();
        let window = window.clone();
        let items = items.clone();
        let checks = checks.clone();
        let libraries = libraries.clone();
        let target = target.clone();
        let update_library = update_library.clone();
        let status = path_label.clone();
        let progress = move_progress.clone();
        let moving = moving.clone();
        move |_| {
            if moving.get() {
                status.set_label("Moving game files… Please wait for the current move to finish.");
                return;
            }
            let selected_games = items
                .borrow()
                .iter()
                .filter(|item| {
                    checks
                        .borrow()
                        .get(&item.game.product_id)
                        .is_some_and(gtk::CheckButton::is_active)
                })
                .cloned()
                .collect::<Vec<_>>();
            if let Some(item) = selected_games.iter().find(|item|
                item.game.compatibility.is_some() || item.game.installer_operating_system.as_deref()
                    .is_some_and(|os| os.eq_ignore_ascii_case("windows"))) {
                status.set_label(&format!("{} cannot be moved here: moving Windows games also requires migrating their Proton prefix. Keep this installation in its current library, or uninstall and reinstall into the other library.", item.title));
                return;
            }
            if selected_games.iter().any(|item| installation::is_game_running(item.game.product_id)) {
                status.set_label("Stop the selected games before moving their files.");
                return;
            }
            let Some(target_library) = libraries.borrow().get(target.selected() as usize).cloned()
            else {
                status.set_label("Choose a destination library before moving games.");
                return;
            };
            if selected_games.is_empty()
                || selected_games
                    .iter()
                    .all(|item| item.game.library_id == target_library.id)
            {
                status.set_label("Select games and choose a different destination library to move them.");
                return;
            }
            let confirmation = adw::AlertDialog::builder()
                .heading("Move installed games?")
                .body(format!(
                    "Move {} selected game{} to {}? Games cannot be launched while files are moving.",
                    selected_games.len(),
                    if selected_games.len() == 1 { "" } else { "s" },
                    target_library.path.display()
                ))
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("move", "Move")]);
            confirmation.set_response_appearance("move", adw::ResponseAppearance::Suggested);
            confirmation.set_default_response(Some("move"));
            let update_selection = update_selection.clone();
            let update_library = update_library.clone();
            let model = model.clone();
            let epoch = model.borrow().account_epoch;
            let status = status.clone();
            let progress = progress.clone();
            let moving = moving.clone();
            let w = w.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "move" || model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return;
                }
                moving.set(true);
                update_selection();
                status.set_label("Moving game files… This may take several minutes.");
                progress.set_visible(true);
                progress.pulse();
                let (sender, receiver) = mpsc::channel();
                let config = model.borrow().config.clone();
                std::thread::spawn(move || {
                    let result = (|| {
                        let _activity = crate::profile_reset::begin_activity("moving installed games")?;
                        let _permit = crate::operation_gate::try_acquire()?;
                        let _reservation = crate::installation::recovery::Reservation::reserve(
                            &selected_games.iter().map(|game| game.game.product_id).collect::<Vec<_>>())?;
                        for item in &selected_games {
                            let directory = &item.game.installation_directory;
                            let journal = installation::operation_journal::path(
                                directory.parent().context("Game has no library directory")?,
                                directory.file_name().and_then(|name| name.to_str()).context("Invalid game directory name")?)?;
                            anyhow::ensure!(!journal.try_exists()?,
                                "Finish or discard the saved installation operation for {} before moving it.", item.title);
                            anyhow::ensure!(!installation::installation_operation_snapshot(item.game.product_id)
                                .is_some_and(|snapshot| snapshot.queued || matches!(snapshot.state,
                                    crate::domain::InstallationState::Pending | crate::domain::InstallationState::Installing | crate::domain::InstallationState::Uninstalling)),
                                "Finish or cancel installation work for {} before moving it.", item.title);
                            anyhow::ensure!(installation::depot_operation_snapshot_for_product(item.game.product_id)
                                .is_none_or(|snapshot| matches!(snapshot.state.as_str(), "complete" | "abandoned")),
                                "Finish or discard the saved Depot operation for {} before moving it.", item.title);
                        }
                        crate::storage::validate_library(&config, crate::config::LibraryKind::GameFiles, &target_library.id)?;
                        for game in &selected_games { crate::storage::validate_library(&config, crate::config::LibraryKind::GameFiles, &game.game.library_id)?; }
                        move_installed_games(&selected_games, &target_library)
                    })();
                    let _ = sender.send(result);
                });
                let update_selection = update_selection.clone();
                let update_library = update_library.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                        moving.set(false); progress.set_visible(false); update_selection();
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(Ok(())) => {
                            moving.set(false); progress.set_visible(false);
                            update_selection();
                            update_library();
                            super::refresh_installed_state_after_library_change(&w, &model);
                            status.set_label("Games moved successfully. Refreshing the library…");
                            show_status(&w, "Selected games moved successfully.");
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            moving.set(false); progress.set_visible(false);
                            tracing::warn!(%error, "could not move installed games");
                            status.set_label(&format!("Could not move games: {error:#}"));
                            update_library();
                            super::refresh_installed_state_after_library_change(&w, &model);
                            show_status(&w, &format!("Could not move all selected games: {error:#}. Some files may have moved; both libraries have been refreshed."));
                            update_selection();
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => { progress.pulse(); glib::ControlFlow::Continue },
                        Err(mpsc::TryRecvError::Disconnected) => {
                            moving.set(false); progress.set_visible(false); update_selection();
                            status.set_label("Moving stopped unexpectedly. Recheck both libraries before trying again.");
                            update_library();
                            super::refresh_installed_state_after_library_change(&w, &model);
                            show_status(&w, "Moving stopped unexpectedly. Recheck both libraries before trying again.");
                            glib::ControlFlow::Break
                        },
                    }
                });
            });
        }
    });
    update_library();
    // A change to another type can make this root overlap it. Recheck all
    // sections from the model only when configuration changes, never by polling disk.
    glib::timeout_add_local(Duration::from_millis(200), {
        let root = root.downgrade();
        let model = model.clone();
        let update = update_library;
        let rebuild = rebuild_library_choices;
        let mut previous = crate::config::LibraryKind::ALL
            .map(|kind| model.borrow().config.libraries(kind).to_vec());
        move || {
            if root.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            let current = crate::config::LibraryKind::ALL
                .map(|kind| model.borrow().config.libraries(kind).to_vec());
            if current != previous {
                previous = current;
                rebuild();
                update();
            }
            glib::ControlFlow::Continue
        }
    });
    root
}

fn storage_legend_item(title: &str, color_class: &str) -> (gtk::Box, gtk::Label) {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.set_width_request(8);
    dot.set_height_request(8);
    dot.set_valign(gtk::Align::Center);
    dot.add_css_class("storage-legend-dot");
    dot.add_css_class(color_class);
    item.append(&dot);
    let title = gtk::Label::new(Some(title));
    title.add_css_class("storage-legend-title");
    item.append(&title);
    let size = gtk::Label::new(Some("0 B"));
    size.set_widget_name(color_class);
    size.add_css_class("storage-legend-size");
    item.append(&size);
    (item, size)
}

fn render_storage_items(view: &StorageListView, items: &[InstalledStorageItem], sort: u32) {
    while let Some(child) = view.list.first_child() {
        view.list.remove(&child);
    }
    view.checks.borrow_mut().clear();
    let mut visible = items.to_vec();
    match sort {
        0 => visible.sort_by_key(|item| std::cmp::Reverse(item.size)),
        1 => visible.sort_by_key(|item| item.title.to_ascii_lowercase()),
        _ => visible.sort_by_key(|item| std::cmp::Reverse(item.game.last_played_at)),
    }
    view.count.set_label(&format!("Items  {}", visible.len()));
    for item in visible {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("storage-game-row");
        let artwork = gtk::Picture::new();
        artwork.set_width_request(112);
        artwork.set_height_request(58);
        artwork.set_content_fit(gtk::ContentFit::Cover);
        if let Some(path) = item.artwork.as_ref() {
            artwork.set_filename(Some(path));
        }
        row.append(&artwork);
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
        labels.set_hexpand(true);
        let title = gtk::Label::new(Some(&item.title));
        title.set_xalign(0.0);
        title.add_css_class("file-name");
        labels.append(&title);
        let path = gtk::Label::new(Some(
            &item.game.installation_directory.display().to_string(),
        ));
        path.set_xalign(0.0);
        path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path.add_css_class("dim-label");
        labels.append(&path);
        let activity = gtk::Label::new(Some(&format!(
            "Played {} · Last played {}",
            format_stored_playtime(item.game.playtime_seconds),
            item.game
                .last_played_at
                .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                .map_or_else(
                    || "never".to_owned(),
                    |date| date.format("%b %-d, %Y").to_string()
                )
        )));
        activity.set_xalign(0.0);
        activity.add_css_class("dim-label");
        labels.append(&activity);
        row.append(&labels);
        let size = gtk::Label::new(Some(&human_size(item.size)));
        size.add_css_class("storage-game-size");
        row.append(&size);
        let check = gtk::CheckButton::new();
        row.append(&check);
        view.checks
            .borrow_mut()
            .insert(item.game.product_id, check.clone());
        let update_selection = view.update_selection.clone();
        check.connect_toggled(move |_| update_selection());
        view.list.append(&row);
    }
    (view.update_selection)();
}

fn format_stored_playtime(seconds: u64) -> String {
    if seconds < 3_600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{:.1} hours", seconds as f64 / 3_600.0)
    }
}

fn model_game_display_data(model: &AppModel) -> HashMap<i64, (String, Option<PathBuf>)> {
    model
        .games
        .iter()
        .map(|game| (game.product_id, (game.title.clone(), game.artwork.clone())))
        .collect()
}

fn filesystem_storage(path: &Path) -> Option<(u64, u64)> {
    Some((
        fs2::total_space(path).ok()?,
        fs2::available_space(path).ok()?,
    ))
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = path.symlink_metadata() else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| directory_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

fn move_installed_games(
    items: &[InstalledStorageItem],
    target: &GameLibrary,
) -> anyhow::Result<()> {
    fs::create_dir_all(&target.path)?;
    let mut moves = Vec::new();
    for item in items {
        if item.game.library_id == target.id {
            continue;
        }
        let directory_name = item
            .game
            .installation_directory
            .file_name()
            .context("installed game has no directory name")?;
        let destination = target.path.join(directory_name);
        let source = fs::symlink_metadata(&item.game.installation_directory)?;
        anyhow::ensure!(
            source.is_dir() && !source.file_type().is_symlink(),
            "The source game directory changed; recheck its library."
        );
        anyhow::ensure!(
            match fs::symlink_metadata(&destination) {
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(error) => return Err(error.into()),
            },
            "destination already exists: {}",
            destination.display()
        );
        moves.push((&item.game.installation_directory, destination));
    }
    for (source, destination) in moves {
        move_directory(source, &destination)?;
    }
    Ok(())
}

fn present_library_rename_dialog(
    window: &adw::ApplicationWindow,
    selected: &GameLibrary,
    kind: crate::config::LibraryKind,
    feedback: (&gtk::Box, &gtk::Label),
    model: Rc<RefCell<AppModel>>,
    libraries: Rc<RefCell<Vec<GameLibrary>>>,
    rebuild: Rc<dyn Fn()>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading("Rename library")
        .body("Choose the name displayed for this library. Valid names save automatically; its directory will not change.")
        .build();
    let entry = gtk::Entry::new();
    entry.set_text(&selected.name);
    entry.set_activates_default(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let local_status = gtk::Label::new(None);
    local_status.set_wrap(true);
    content.append(&entry);
    content.append(&local_status);
    dialog.set_extra_child(Some(&content));
    dialog.add_response("done", "Done");
    dialog.set_default_response(Some("done"));
    dialog.set_close_response("done");
    let id = selected.id.clone();
    let status = feedback.1.clone();
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    let draft_status = local_status.clone();
    let save: Rc<dyn Fn()> = Rc::new({
        let entry = entry.clone();
        move || {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                return;
            }
            let name = entry.text().trim().to_owned();
            if name.is_empty() {
                local_status.set_label("Enter a name. The previous name is unchanged.");
                return;
            }
            local_status.set_label("Saving name…");
            let id_worker = id.clone();
            let name_worker = name.clone();
            let receiver = update_policies::policy_request(move || {
                let _activity = crate::profile_reset::begin_activity("renaming library")?;
                online::with_account_session(session, || {
                    let mut config = Config::load_or_create()?;
                    let library = config
                        .libraries_mut(kind)
                        .iter_mut()
                        .find(|library| library.id == id_worker)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "This library is no longer configured. Reopen Settings."
                            )
                        })?;
                    library.name = name_worker;
                    config.save()
                })
            });
            let model = model.clone();
            let libraries = libraries.clone();
            let rebuild = rebuild.clone();
            let local_status = local_status.clone();
            let entry = entry.clone();
            let status = status.clone();
            let id = id.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err(anyhow::anyhow!("The settings worker stopped.")),
                };
                if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return glib::ControlFlow::Break;
                }
                match result {
                    Ok(()) => {
                        if let Some(library) = model
                            .borrow_mut()
                            .config
                            .libraries_mut(kind)
                            .iter_mut()
                            .find(|library| library.id == id)
                        {
                            library.name = name.clone();
                        }
                        if let Some(library) = libraries
                            .borrow_mut()
                            .iter_mut()
                            .find(|library| library.id == id)
                        {
                            library.name = name.clone();
                        }
                        if entry.text().trim() == name {
                            local_status.set_label("Saved automatically.");
                        }
                        rebuild();
                    }
                    Err(error) => {
                        let message =
                            format!("Library name was not saved: {error}. Edit the name to retry.");
                        if entry.text().trim() == name {
                            local_status.set_label(&message);
                        }
                        status.set_label(&message);
                    }
                }
                glib::ControlFlow::Break
            });
        }
    });
    entry.connect_changed({
        let pending = pending.clone();
        let save = save.clone();
        move |entry| {
            draft_status.set_label(if entry.text().trim().is_empty() {
                "Enter a name. The previous name is unchanged."
            } else {
                "Waiting to save the latest name…"
            });
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let inner = pending.clone();
            let save = save.clone();
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(
                Duration::from_millis(400),
                move || {
                    inner.borrow_mut().take();
                    save();
                },
            ));
        }
    });
    let flush: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(source) = pending.borrow_mut().take() {
            source.remove();
            save();
        }
    });
    entry.connect_activate({
        let flush = flush.clone();
        move |_| flush()
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let flush = flush.clone();
        move |_| flush()
    });
    entry.add_controller(focus);
    dialog.choose(Some(window), gio::Cancellable::NONE, move |_| flush());
}

fn move_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    let temporary = destination.with_extension("moving");
    anyhow::ensure!(!temporary.exists(), "temporary move path already exists");
    copy_directory(source, &temporary)?;
    fs::rename(&temporary, destination)?;
    fs::remove_dir_all(source)?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        if source_path.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(&source_path)?, &target_path)?;
        } else if source_path.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
            fs::set_permissions(&target_path, fs::metadata(&source_path)?.permissions())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[ignore = "requires private p361 HOME/all XDG, GTK and D-Bus; no file moves or helpers"]
    fn move_controls_require_eligible_selection_and_preserve_busy_state() {
        let home = std::env::var("HOME").unwrap();
        assert!(home.starts_with("/tmp/ludomere-p361-"));
        let root = Path::new(&home).parent().unwrap();
        for key in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(Path::new(&std::env::var_os(key).unwrap()).starts_with(root));
        }
        adw::init().unwrap();
        let libraries = Rc::new(RefCell::new(
            ["source", "target"]
                .into_iter()
                .map(|id| GameLibrary {
                    id: id.into(),
                    name: id.into(),
                    path: root.join(id),
                    default: id == "source",
                })
                .collect::<Vec<_>>(),
        ));
        let items = Rc::new(RefCell::new([(1, "linux", "source"), (2, "WiNdOwS", "source"), (3, "linux", "target")]
            .into_iter().map(|(id, os, library)| InstalledStorageItem {
                game: serde_json::from_value(serde_json::json!({
                    "product_id":id, "library_id":library,
                    "installation_directory":root.join(library).join(format!("game-{id}")),
                    "installer_files":[], "installer_complete":true, "installer_operating_system":os,
                    "launch_arguments":[], "state":"installed", "playtime_seconds":0,
                    "created_at":1, "updated_at":1
                })).unwrap(), title: format!("Synthetic game {id}"), artwork: None, size: 0,
            }).collect::<Vec<_>>()));
        let checks = Rc::new(RefCell::new(HashMap::new()));
        let target = gtk::DropDown::from_strings(&["Source", "Target"]);
        let selected = gtk::Label::new(None);
        let button = gtk::Button::with_label("Move");
        let moving = Rc::new(std::cell::Cell::new(false));
        let update = connect_move_selection(
            &items, &checks, &libraries, &target, &selected, &button, &moving,
        );
        let view = StorageListView {
            list: gtk::ListBox::new(),
            checks: checks.clone(),
            count: gtk::Label::new(None),
            update_selection: update.clone(),
        };
        render_storage_items(&view, &items.borrow(), 1);
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.MoveEligibilityTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&view.list);
        content.append(&target);
        content.append(&selected);
        content.append(&button);
        window.set_content(Some(&content));
        window.present();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !button.is_mapped() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(button.is_mapped());
        let native = checks.borrow()[&1].clone();
        let windows = checks.borrow()[&2].clone();
        let already_at_target = checks.borrow()[&3].clone();
        assert!(!button.is_sensitive());
        assert_eq!(selected.text(), "No games selected");
        assert_eq!(
            target.selected(),
            0,
            "eligibility must not select a destination"
        );
        native.set_active(true);
        assert!(!button.is_sensitive());
        assert_eq!(selected.text(), "1 game selected");
        assert_eq!(
            button.tooltip_text().as_deref(),
            Some("Choose a different destination library.")
        );
        target.set_selected(1);
        assert!(button.is_sensitive());
        assert!(button.tooltip_text().is_none());
        // A nonempty DropDown autoselects; invalid set_selected does not clear it.
        let names = target
            .model()
            .unwrap()
            .downcast::<gtk::StringList>()
            .unwrap();
        names.splice(0, names.n_items(), &[]);
        assert_eq!(target.selected(), gtk::INVALID_LIST_POSITION);
        assert!(!button.is_sensitive());
        assert_eq!(
            button.tooltip_text().as_deref(),
            Some("Choose a destination library.")
        );
        names.splice(0, 0, &["Source", "Target"]);
        target.set_selected(1);
        already_at_target.set_active(true);
        assert!(
            button.is_sensitive(),
            "existing backend permits a mixed selection with a game to move"
        );
        native.set_active(false);
        assert!(!button.is_sensitive());
        already_at_target.set_active(false);
        windows.set_active(true);
        assert!(!button.is_sensitive());
        assert!(
            button
                .tooltip_text()
                .unwrap()
                .contains("compatibility prefix")
        );
        native.set_active(true);
        assert!(
            !button.is_sensitive(),
            "a Windows selection must explain the existing refusal"
        );
        windows.set_active(false);
        assert!(button.is_sensitive());
        moving.set(true);
        update();
        assert!(!button.is_sensitive());
        target.set_selected(0);
        target.set_selected(1);
        native.set_active(false);
        native.set_active(true);
        assert!(
            !button.is_sensitive(),
            "selection callbacks must not override active Move"
        );
        assert_eq!(
            button.tooltip_text().as_deref(),
            Some("Wait for the current move to finish.")
        );
        moving.set(false);
        update();
        assert!(button.is_sensitive());
        // Compatibility metadata alone is also refused by the existing click-time policy.
        items.borrow_mut()[0].game.compatibility =
            Some(crate::compatibility::GameCompatibilityPreferences {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                prefix_slug: "synthetic".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
                pending_profile: None,
            });
        native.set_active(false);
        native.set_active(true);
        assert!(!button.is_sensitive());
        assert!(button.tooltip_text().unwrap().contains("Windows games"));
        items.borrow_mut()[0].game.compatibility = None;
        moving.set(true);
        update();
        render_storage_items(&view, &items.borrow(), 0);
        assert!(!button.is_sensitive());
        assert_eq!(
            button.tooltip_text().as_deref(),
            Some("Wait for the current move to finish.")
        );
        moving.set(false);
        update();
        assert!(!button.is_sensitive());
        assert_eq!(selected.text(), "No games selected");
        assert_eq!(
            button.tooltip_text().as_deref(),
            Some("Select at least one game to move.")
        );
        assert!(
            libraries
                .borrow()
                .iter()
                .all(|library| !library.path.exists()),
            "presentation must never move or create files"
        );
        window.destroy();
    }

    #[test]
    #[ignore = "requires private HOME/all XDG under /tmp/ludomere-p357-, GTK and D-Bus"]
    fn storage_inspection_admission_tracks_activity_and_rejects_stale_workers() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p357-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn delay_inspections() -> [mpsc::Sender<()>; 2] {
            std::array::from_fn(|_| {
                let (sender, receiver) = mpsc::channel();
                STORAGE_INSPECTION_DELAYS.with_borrow_mut(|delays| delays.push_back(receiver));
                sender
            })
        }
        fn assert_both_registered() {
            let error = crate::profile_reset::reserve().err().unwrap().to_string();
            assert!(error.contains("checking library choices"), "{error}");
            assert!(error.contains("inspecting selected library"), "{error}");
            assert!(STORAGE_INSPECTION_DELAYS.with_borrow(|delays| delays.is_empty()));
        }
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("games");
        fs::create_dir(&library).unwrap();
        let config = Config {
            game_libraries: vec![GameLibrary {
                id: "test".into(),
                name: "Test".into(),
                path: library,
                default: true,
            }],
            offline_libraries: vec![],
            extras_libraries: vec![],
            ..Config::default()
        };
        let database = crate::identity::database();
        assert!(database.starts_with(std::env::var("XDG_DATA_HOME").unwrap()));
        assert!(
            !database.exists(),
            "fixture requires a fresh synthetic profile"
        );
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.StorageLifecycleTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let w = Rc::new(window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            ..AppModel::default()
        }));
        let delayed = delay_inspections();
        let page = build_storage_page(
            &w.window,
            &w,
            &model,
            crate::config::LibraryKind::GameFiles,
            "Game Library",
        );
        let recheck = find_named_descendant(page.upcast_ref(), "storage-recheck")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let status = find_named_descendant(page.upcast_ref(), "storage-inspection-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        w.window.set_content(Some(&page));
        w.window.present();
        assert_both_registered();
        assert!(!recheck.is_sensitive());
        // Dropping the test-only wait channels exercises real worker failure and release.
        drop(delayed);
        wait(|| recheck.is_sensitive());
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(status.text().starts_with("Incompatible or unavailable:"));
        assert_eq!(recheck.label().as_deref(), Some("Recheck library"));
        assert!(!database.exists());

        let delayed = delay_inspections();
        recheck.emit_clicked();
        assert_both_registered();
        auth::invalidate_session();
        for sender in delayed {
            sender.send(()).unwrap();
        }
        wait(|| crate::profile_reset::reserve().is_ok());
        wait(|| recheck.is_sensitive());
        assert!(
            !database.exists(),
            "stale workers must stop before either DB open"
        );
        assert_eq!(
            status.text(),
            "Account changed. Recheck library to inspect the current account."
        );
        // Settings is reused after authentication changes. An explicit Recheck
        // admits the current session on this same page, without automatic work.
        recheck.emit_clicked();
        wait(|| recheck.is_sensitive());
        wait(|| crate::profile_reset::reserve().is_ok());
        assert_eq!(status.text(), "Compatible");
        assert!(database.is_file());
        assert_eq!(w.window.content().as_ref(), Some(page.upcast_ref()));
        let frozen = crate::profile_reset::reserve().unwrap();
        recheck.emit_clicked();
        assert!(recheck.is_sensitive());
        assert_eq!(recheck.label().as_deref(), Some("Recheck library"));
        assert!(status.text().starts_with("Could not inspect library:"));
        drop(frozen);

        model.borrow_mut().logout_pending = true;
        recheck.emit_clicked();
        assert!(recheck.is_sensitive());
        assert_eq!(
            status.text(),
            "Account change in progress. Recheck library when it finishes."
        );
        assert!(crate::profile_reset::reserve().is_ok());
        model.borrow_mut().logout_pending = false;
        let delayed = delay_inspections();
        recheck.emit_clicked();
        assert_both_registered();
        model.borrow_mut().account_epoch += 1;
        online::invalidate_library_session();
        for sender in delayed {
            sender.send(()).unwrap();
        }
        wait(|| crate::profile_reset::reserve().is_ok());
        wait(|| recheck.is_sensitive());
        assert_eq!(
            status.text(),
            "Account changed. Recheck library to inspect the current account."
        );
        recheck.emit_clicked();
        wait(|| recheck.is_sensitive());
        wait(|| crate::profile_reset::reserve().is_ok());
        assert_eq!(status.text(), "Compatible");
        assert_eq!(w.window.content().as_ref(), Some(page.upcast_ref()));
        assert!(STORAGE_INSPECTION_DELAYS.with_borrow(|delays| delays.is_empty()));
        w.window.destroy();
    }

    #[test]
    #[ignore = "requires private HOME/all XDG, GTK and D-Bus; inspection results are synthetic"]
    fn recheck_feedback_tracks_current_request_and_clears_stale_totals() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p350-")
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.StorageFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let config = Config {
            game_libraries: (0..2)
                .map(|index| GameLibrary {
                    id: index.to_string(),
                    name: format!("Fixture {index}"),
                    path: std::env::temp_dir().join(format!("synthetic-library-{index}")),
                    default: index == 0,
                })
                .collect(),
            ..Config::default()
        };
        let w = Rc::new(window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            ..AppModel::default()
        }));
        STORAGE_INSPECTIONS.with_borrow_mut(|requests| {
            assert!(requests.is_none());
            *requests = Some(Vec::new());
        });
        let page = build_storage_page(
            &w.window,
            &w,
            &model,
            crate::config::LibraryKind::GameFiles,
            "Game Library",
        );
        w.window.set_content(Some(&page));
        w.window.present();
        let recheck = find_named_descendant(page.upcast_ref(), "storage-recheck")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let menu = find_named_descendant(page.upcast_ref(), "storage-library-menu")
            .and_downcast::<gtk::MenuButton>()
            .unwrap();
        let status = find_named_descendant(page.upcast_ref(), "storage-inspection-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let capacity = find_named_descendant(page.upcast_ref(), "storage-capacity")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let totals = [
            "storage-games",
            "storage-installers",
            "storage-extras",
            "storage-others",
            "storage-free",
        ]
        .map(|name| {
            find_named_descendant(page.upcast_ref(), name)
                .and_downcast::<gtk::Label>()
                .unwrap()
        });
        wait(|| recheck.is_mapped());
        assert!(!recheck.is_sensitive());
        assert_eq!(recheck.label().as_deref(), Some("Checking library…"));
        assert!(totals.iter().all(|label| label.text() == "—"));
        assert!(menu.is_sensitive());
        assert!(menu.grab_focus());
        let focus = gtk::prelude::GtkWindowExt::focus(&w.window);
        let generation = model.borrow().detail_generation;
        assert_eq!(
            STORAGE_INSPECTIONS.with_borrow(|requests| requests.as_ref().unwrap().len()),
            1
        );
        let stale = STORAGE_INSPECTIONS
            .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap());
        // Changing libraries remains possible while the old inspection is pending.
        find_named_descendant(
            menu.popover().unwrap().upcast_ref(),
            "storage-library-choice-1",
        )
        .and_downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
        let current = STORAGE_INSPECTIONS
            .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap());
        stale
            .send(Ok((Some((9000, 1)), vec![], 8000, 0, vec![])))
            .unwrap();
        // Receiver closure proves the old generation was rejected, without a disk/timing race.
        wait(|| {
            stale
                .send(Err(anyhow::anyhow!("stale inspection")))
                .is_err()
        });
        assert!(!recheck.is_sensitive());
        assert_eq!(recheck.label().as_deref(), Some("Checking library…"));
        assert!(totals.iter().all(|label| label.text() == "—"));
        current
            .send(Ok((Some((1000, 600)), vec![], 120, 40, vec![])))
            .unwrap();
        wait(|| recheck.is_sensitive());
        assert_eq!(recheck.label().as_deref(), Some("Recheck library"));
        assert_eq!(status.text(), "Compatible");
        assert_eq!(
            capacity.text(),
            format!("{} free of {}", human_size(600), human_size(1000))
        );
        for (label, bytes) in totals.iter().zip([0, 120, 40, 240, 600]) {
            assert_eq!(label.text(), human_size(bytes));
        }
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&w.window), focus);
        assert_eq!(model.borrow().detail_generation, generation);
        assert_eq!(w.window.content().as_ref(), Some(page.upcast_ref()));

        recheck.emit_clicked();
        assert!(!recheck.is_sensitive());
        assert!(totals.iter().all(|label| label.text() == "—"));
        STORAGE_INSPECTIONS
            .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap())
            .send(Err(anyhow::anyhow!("Synthetic unavailable library")))
            .unwrap();
        wait(|| recheck.is_sensitive());
        assert!(status.text().contains("Synthetic unavailable library"));
        assert!(totals.iter().all(|label| label.text() == "—"));

        recheck.emit_clicked();
        drop(
            STORAGE_INSPECTIONS
                .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap()),
        );
        wait(|| recheck.is_sensitive());
        assert!(status.text().contains("inspection stopped unexpectedly"));
        assert!(totals.iter().all(|label| label.text() == "—"));

        recheck.emit_clicked();
        STORAGE_INSPECTIONS
            .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap())
            .send(Ok((None, vec![], 120, 40, vec![])))
            .unwrap();
        wait(|| recheck.is_sensitive());
        assert_eq!(capacity.text(), "Storage information unavailable");
        assert!(totals.iter().all(|label| label.text() == "—"));

        recheck.emit_clicked();
        let old_account = STORAGE_INSPECTIONS
            .with_borrow_mut(|requests| requests.as_mut().unwrap().pop().unwrap());
        model.borrow_mut().account_epoch += 1;
        wait(|| {
            old_account
                .send(Err(anyhow::anyhow!("old account")))
                .is_err()
        });
        assert!(recheck.is_sensitive());
        assert_eq!(recheck.label().as_deref(), Some("Recheck library"));
        assert_eq!(
            status.text(),
            "Account changed. Recheck library to inspect the current account."
        );
        assert!(!status.text().contains("old account"));
        assert!(totals.iter().all(|label| label.text() == "—"));

        let empty_model = Rc::new(RefCell::new(AppModel {
            config: Config {
                game_libraries: vec![],
                ..Config::default()
            },
            ..AppModel::default()
        }));
        let empty = build_storage_page(
            &w.window,
            &w,
            &empty_model,
            crate::config::LibraryKind::GameFiles,
            "Game Library",
        );
        w.window.set_content(Some(&empty));
        let empty_recheck = find_named_descendant(empty.upcast_ref(), "storage-recheck")
            .and_downcast::<gtk::Button>()
            .unwrap();
        wait(|| empty_recheck.is_mapped());
        assert!(!empty_recheck.is_sensitive());
        assert_eq!(empty_recheck.label().as_deref(), Some("Recheck library"));
        assert!(
            find_named_descendant(empty.upcast_ref(), "storage-inspection-status")
                .and_downcast::<gtk::Label>()
                .unwrap()
                .text()
                .contains("No library configured")
        );
        assert!(STORAGE_INSPECTIONS.with_borrow(|requests| requests.as_ref().unwrap().is_empty()));
        STORAGE_INSPECTIONS.with_borrow_mut(|requests| *requests = None);
        w.window.destroy();
    }

    #[test]
    fn directory_size_and_copy_preserve_nested_files() {
        let root = std::env::temp_dir().join(format!(
            "gog-storage-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("one"), b"1234").unwrap();
        fs::write(source.join("nested/two"), b"12").unwrap();
        assert_eq!(directory_size(&source), 6);
        copy_directory(&source, &destination).unwrap();
        assert_eq!(directory_size(&destination), 6);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn move_preflights_all_destinations_before_moving_any_native_payload() {
        let root = tempfile::tempdir().unwrap();
        let target = GameLibrary {
            id: "target".into(),
            name: "Target".into(),
            path: root.path().join("target"),
            default: false,
        };
        fs::create_dir_all(&target.path).unwrap();
        let items = ["first", "second"].into_iter().enumerate().map(|(index, name)| {
            let directory = root.path().join("source").join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), name).unwrap();
            InstalledStorageItem {
                game: serde_json::from_value(serde_json::json!({
                    "product_id":index as i64 + 1,"library_id":"source","installation_directory":directory,
                    "installer_files":[],"installer_complete":true,"installer_operating_system":"linux",
                    "launch_arguments":[],"state":"installed","playtime_seconds":0,"created_at":1,"updated_at":1
                })).unwrap(), title:name.into(), artwork:None, size:0,
            }
        }).collect::<Vec<_>>();
        std::os::unix::fs::symlink(root.path().join("missing"), target.path.join("second"))
            .unwrap();
        assert!(move_installed_games(&items, &target).is_err());
        assert!(
            items
                .iter()
                .all(|item| item.game.installation_directory.join("payload").is_file())
        );
        assert!(!target.path.join("first").exists());
        fs::remove_file(target.path.join("second")).unwrap();
        move_installed_games(&items, &target).unwrap();
        for name in ["first", "second"] {
            assert_eq!(
                fs::read_to_string(target.path.join(name).join("payload")).unwrap(),
                name
            );
            assert!(!root.path().join("source").join(name).exists());
        }
    }
}
