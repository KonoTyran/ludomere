use super::super::*;
use crate::config::GameLibrary;
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

#[derive(Clone)]
struct StorageListView {
    list: gtk::ListBox,
    checks: Rc<RefCell<HashMap<i64, gtk::CheckButton>>>,
    count: gtk::Label,
    selected: gtk::Label,
    move_button: gtk::Button,
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
            while let Some(child) = choices.first_child() {
                choices.remove(&child);
            }
            let mut labels = Vec::new();
            for (index, entry) in libraries.borrow().iter().cloned().enumerate() {
                let row = gtk::Button::new();
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
            let config = model.borrow().config.clone();
            let epoch = model.borrow().account_epoch;
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(crate::storage::inspect_libraries(&config));
            });
            let model = model.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if model.borrow().account_epoch != epoch {
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
    path_label.set_xalign(0.0);
    path_label.add_css_class("storage-path-label");
    root.append(&path_label);
    path_label.set_wrap(true);
    path_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    path_label.set_selectable(true);
    let recheck = gtk::Button::with_label("Recheck library");
    recheck.set_halign(gtk::Align::Start);
    root.append(&recheck);
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
    let (others_legend, others_size) = storage_legend_item("Others", "storage-others");
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

    let items = Rc::new(RefCell::new(Vec::<InstalledStorageItem>::new()));
    let checks = Rc::new(RefCell::new(HashMap::<i64, gtk::CheckButton>::new()));
    let list_view = StorageListView {
        list: list.clone(),
        checks: checks.clone(),
        count: item_count.clone(),
        selected: selected.clone(),
        move_button: move_button.clone(),
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
        let request = Rc::new(std::cell::Cell::new(0u64));
        Rc::new(move || {
            request.set(request.get().wrapping_add(1));
            footer.set_sensitive(false);
            *usage_values.borrow_mut() = StorageBreakdown::default();
            usage.queue_draw();
            let Some(selected_library) =
                libraries.borrow().get(library.selected() as usize).cloned()
            else {
                selected_mount.set_label("Choose or add a directory");
                path_label
                    .set_label("No library configured. Add a directory to use this library type.");
                capacity.set_label("");
                items.borrow_mut().clear();
                refresh();
                footer.set_sensitive(false);
                return;
            };
            let mount = selected_library.path.to_string_lossy().into_owned();
            selected_mount.set_label(&mount);
            path_label.set_label(&selected_library.path.display().to_string());
            capacity.set_label("Calculating storage…");
            items.borrow_mut().clear();
            refresh();
            let all_libraries = vec![selected_library.clone()];
            let titles = model_game_display_data(&model.borrow());
            let config = model.borrow().config.clone();
            let epoch = model.borrow().account_epoch;
            let generation = request.get();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<_> {
                    crate::storage::validate_library(&config, kind, &selected_library.id)?;
                    let storage = filesystem_storage(&selected_library.path);
                    let games = if kind == crate::config::LibraryKind::GameFiles {
                        StateStore::open()
                            .and_then(|store| {
                                crate::installation::reconcile_installed_games(
                                    &store,
                                    &all_libraries,
                                )
                            })
                            .unwrap_or_default()
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
                                    titles.get(&game.product_id).cloned().unwrap_or_else(|| {
                                        (format!("Product {}", game.product_id), None)
                                    });
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
                    let managed = StateStore::open()
                        .and_then(|store| store.managed_files())
                        .unwrap_or_default();
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
                    Ok((storage, games, installers, extras))
                })();
                let _ = sender.send(result);
            });
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
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if model.borrow().account_epoch != epoch || request.get() != generation {
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok((storage, games, installers, extras))) => {
                        path_label.set_label("Compatible");
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
                    Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
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
            save_storage_config(&root, &status, &model.clone(), config, move |config| {
                *libraries.borrow_mut() = config.libraries(kind).to_vec();
                model.borrow_mut().config = config;
                rebuild();
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
                let Ok(folder) = result else { return };
                let Some(path) = folder.path() else { return };
                if libraries.borrow().iter().any(|entry| entry.path == path) {
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
        let model = model.clone();
        let window = window.clone();
        let items = items.clone();
        let checks = checks.clone();
        let libraries = libraries.clone();
        let target = target.clone();
        let update_library = update_library.clone();
        move |button| {
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
            let Some(target_library) = libraries.borrow().get(target.selected() as usize).cloned()
            else {
                return;
            };
            if selected_games.is_empty()
                || selected_games
                    .iter()
                    .all(|item| item.game.library_id == target_library.id)
            {
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
            let button = button.clone();
            let update_library = update_library.clone();
            let model = model.clone();
            let epoch = model.borrow().account_epoch;
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "move" || model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return;
                }
                button.set_sensitive(false);
                let (sender, receiver) = mpsc::channel();
                let config = model.borrow().config.clone();
                std::thread::spawn(move || {
                    let result = (|| {
                        let _permit = crate::operation_gate::try_acquire()?;
                        crate::storage::validate_library(&config, crate::config::LibraryKind::GameFiles, &target_library.id)?;
                        for game in &selected_games { crate::storage::validate_library(&config, crate::config::LibraryKind::GameFiles, &game.game.library_id)?; }
                        move_installed_games(&selected_games, &target_library)
                    })();
                    let _ = sender.send(result);
                });
                let button = button.clone();
                let update_library = update_library.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match receiver.try_recv() {
                        Ok(Ok(())) => {
                            button.set_sensitive(true);
                            update_library();
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "could not move installed games");
                            button.set_tooltip_text(Some(&format!("Move failed: {error:#}")));
                            button.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
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
        let checks = view.checks.clone();
        let selected = view.selected.clone();
        let move_button = view.move_button.clone();
        check.connect_toggled(move |_| {
            let count = checks
                .borrow()
                .values()
                .filter(|check| check.is_active())
                .count();
            let text = if count == 0 {
                "No games selected".to_owned()
            } else {
                format!("{count} game{} selected", if count == 1 { "" } else { "s" })
            };
            selected.set_label(&text);
            move_button.set_sensitive(count > 0);
        });
        view.list.append(&row);
    }
    view.selected.set_label("No games selected");
    view.move_button.set_sensitive(false);
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

pub(in crate::ui) fn filesystem_mount_point(path: &Path) -> String {
    #[cfg(target_os = "linux")]
    {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        if let Ok(mounts) = fs::read_to_string("/proc/self/mountinfo")
            && let Some(mount) = mounts
                .lines()
                .filter_map(|line| line.split(" - ").next())
                .filter_map(|prefix| prefix.split_whitespace().nth(4))
                .map(decode_mount_path)
                .filter(|mount| canonical.starts_with(mount))
                .max_by_key(|mount| mount.as_os_str().len())
        {
            return mount.display().to_string();
        }
    }
    path.display().to_string()
}

#[cfg(target_os = "linux")]
fn decode_mount_path(value: &str) -> PathBuf {
    PathBuf::from(
        value
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\"),
    )
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
        anyhow::ensure!(
            !destination.exists(),
            "destination already exists: {}",
            destination.display()
        );
        move_directory(&item.game.installation_directory, &destination)?;
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
        .body("Choose the name displayed for this library. Its directory will not change.")
        .build();
    let entry = gtk::Entry::new();
    entry.set_text(&selected.name);
    entry.set_activates_default(true);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");
    let id = selected.id.clone();
    let page = feedback.0.clone();
    let status = feedback.1.clone();
    let epoch = model.borrow().account_epoch;
    dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if response != "rename"
            || model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
        {
            return;
        }
        let name = entry.text().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let mut config = model.borrow().config.clone();
        if let Some(library) = config
            .libraries_mut(kind)
            .iter_mut()
            .find(|library| library.id == id)
        {
            library.name = name;
        }
        save_storage_config(&page, &status, &model.clone(), config, move |config| {
            *libraries.borrow_mut() = config.libraries(kind).to_vec();
            model.borrow_mut().config = config;
            rebuild();
        });
    });
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
}
