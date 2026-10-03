use super::*;

pub(super) mod storage;
use storage::*;

pub(super) fn show_settings(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    show_settings_page(w, model, "account");
}

pub(super) fn show_settings_page(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    initial_page: &str,
) {
    let Some(application) = w.window.application() else {
        return;
    };
    if let Some(existing) = application
        .windows()
        .into_iter()
        .find(|window| window.widget_name() == "ludomere-settings-window")
    {
        if let Some(stack) = find_settings_stack(&existing.clone().upcast()) {
            stack.set_visible_child_name(initial_page);
            stack.notify("visible-child-name");
        }
        existing.present();
        return;
    }
    let settings_window = adw::ApplicationWindow::builder()
        .application(&application)
        .title("Ludomere Settings")
        .default_width(940)
        .default_height(650)
        .transient_for(&w.window)
        .build();
    settings_window.set_widget_name("ludomere-settings-window");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Ludomere Settings",
        "Account and application preferences",
    )));
    root.append(&header);
    let preference_status = gtk::Label::new(None);
    preference_status.set_wrap(true);
    preference_status.set_selectable(true);
    preference_status.set_margin_start(12);
    preference_status.set_margin_end(12);
    preference_status.add_css_class("error");
    preference_status.set_visible(false);
    root.append(&preference_status);

    let account_page = settings_account_page(w, model);
    let downloads_page = adw::PreferencesPage::new();
    downloads_page.set_title("Downloads");
    downloads_page.set_icon_name(Some("folder-download-symbolic"));
    let library_page = adw::PreferencesPage::new();
    library_page.set_title("Game Display");
    library_page.set_icon_name(Some("view-grid-symbolic"));
    let library_display = adw::PreferencesGroup::new();
    library_display.set_title("Library display");
    let appearance_page = adw::PreferencesPage::new();
    appearance_page.set_title("Appearance");
    appearance_page.set_icon_name(Some("applications-graphics-symbolic"));

    let downloads = adw::PreferencesGroup::new();
    downloads.set_title("Downloads");

    let concurrency_row = adw::ActionRow::new();
    concurrency_row.set_title("Simultaneous file parts");
    concurrency_row
        .set_subtitle("Maximum number of multipart files downloaded for one game at once");
    let concurrency = gtk::SpinButton::with_range(1.0, 4.0, 1.0);
    concurrency.set_value(model.borrow().config.max_concurrent_downloads as f64);
    concurrency.set_valign(gtk::Align::Center);
    concurrency_row.add_suffix(&concurrency);
    downloads.add(&concurrency_row);

    let rebuild_row = adw::ActionRow::new();
    rebuild_row.set_use_markup(false);
    rebuild_row.set_title("Rebuild downloaded-file index");
    rebuild_row.set_subtitle("Inspect compatible Offline Installers and Goodies & Extras libraries without changing files");
    let rebuild_button = gtk::Button::with_label("Rebuild");
    rebuild_button.set_valign(gtk::Align::Center);
    rebuild_row.add_suffix(&rebuild_button);
    downloads.add(&rebuild_row);

    let language_row = adw::ActionRow::new();
    language_row.set_title("Default installer language");
    let language_values = installer_language_options(&model.borrow());
    let language_list = gtk::StringList::new(
        &language_values
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let language = gtk::DropDown::new(Some(language_list.clone()), gtk::Expression::NONE);
    language.set_valign(gtk::Align::Center);
    let configured_language = model.borrow().config.installer_language.clone();
    let selected_language = configured_language
        .as_ref()
        .and_then(|configured| {
            language_values
                .iter()
                .position(|value| value.eq_ignore_ascii_case(configured))
        })
        .unwrap_or(0);
    language.set_selected(selected_language as u32);
    language_row.add_suffix(&language);
    downloads.add(&language_row);

    let platform_row = adw::ActionRow::new();
    platform_row.set_title("Default installer platforms");
    let platform_choices = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let windows = gtk::CheckButton::with_label("Windows");
    let linux = gtk::CheckButton::with_label("Linux");
    let macos = gtk::CheckButton::with_label("macOS");
    windows.set_active(model.borrow().config.installer_windows);
    linux.set_active(model.borrow().config.installer_linux);
    macos.set_active(model.borrow().config.installer_macos);
    platform_choices.append(&windows);
    platform_choices.append(&linux);
    platform_choices.append(&macos);
    platform_row.add_suffix(&platform_choices);
    downloads.add(&platform_row);

    let extras_default = adw::SwitchRow::new();
    extras_default.set_title("Include extras by default");
    extras_default
        .set_subtitle("Automatically include missing extras when creating a game download plan");
    extras_default.set_active(model.borrow().config.download_extras_by_default);
    downloads.add(&extras_default);

    let patches_default = adw::SwitchRow::new();
    patches_default.set_title("Include compatible patches by default");
    patches_default.set_subtitle(
        "Automatically include missing compatible patches when creating a game download plan",
    );
    patches_default.set_active(model.borrow().config.download_patches_by_default);
    downloads.add(&patches_default);

    let prefer_patch_updates = adw::SwitchRow::new();
    prefer_patch_updates.set_title("Prefer patch updates");
    prefer_patch_updates.set_subtitle(
        "Use a compatible patch when available; use Repair if the updated game does not work",
    );
    prefer_patch_updates.set_active(model.borrow().config.prefer_patch_updates);
    downloads.add(&prefer_patch_updates);

    let interactive_prompts = adw::SwitchRow::new();
    interactive_prompts.set_title("Interactive install");
    interactive_prompts.set_subtitle(
        "Show Windows installers for optional choices while retaining Ludomere’s install directory",
    );
    interactive_prompts.set_active(model.borrow().config.interactive_installer_prompts);
    downloads.add(&interactive_prompts);

    let retired_artifacts = adw::SwitchRow::new();
    retired_artifacts.set_title("Show unavailable previous versions");
    retired_artifacts.set_subtitle(
        "Show known files no longer offered by GOG, even when they are not downloaded",
    );
    retired_artifacts.set_active(model.borrow().config.show_retired_artifacts);
    downloads.add(&retired_artifacts);
    downloads_page.add(&downloads);

    let source_order = installation_source_order_group(model, &preference_status);
    downloads_page.add(&source_order);
    downloads_page.add(&update_policies::global_group(w, model));

    let maintenance = adw::PreferencesGroup::new();
    maintenance.set_title("Storage and synchronization");
    let open_downloads = adw::ActionRow::new();
    open_downloads.set_use_markup(false);
    open_downloads.set_title("Open a downloaded-file library");
    let open_downloads_button = gtk::Button::with_label("Open");
    open_downloads_button.set_valign(gtk::Align::Center);
    open_downloads.add_suffix(&open_downloads_button);
    maintenance.add(&open_downloads);
    let clear_images = adw::ActionRow::new();
    clear_images.set_title("Clear image cache");
    clear_images.set_subtitle("Remove replaceable artwork and screenshots only");
    let clear_images_button = gtk::Button::with_label("Clear");
    clear_images_button.set_valign(gtk::Align::Center);
    clear_images.add_suffix(&clear_images_button);
    maintenance.add(&clear_images);
    let refresh_metadata = adw::ActionRow::new();
    refresh_metadata.set_title("Refresh all online metadata");
    refresh_metadata
        .set_subtitle("Refresh names and covers; reload other metadata when next opened");
    let refresh_metadata_button = gtk::Button::with_label("Refresh");
    refresh_metadata_button.set_valign(gtk::Align::Center);
    refresh_metadata.add_suffix(&refresh_metadata_button);
    maintenance.add(&refresh_metadata);
    let storage_maintenance_page = adw::PreferencesPage::new();
    storage_maintenance_page.set_title("Maintenance");
    storage_maintenance_page.set_icon_name(Some("emblem-system-symbolic"));
    storage_maintenance_page.add(&maintenance);

    let appearance = adw::PreferencesGroup::new();
    appearance.set_title("Appearance");
    let theme_row = adw::ActionRow::new();
    theme_row.set_title("Color scheme");
    let theme = gtk::DropDown::from_strings(&["System", "Light", "Dark"]);
    theme.set_valign(gtk::Align::Center);
    theme.set_selected(match model.borrow().config.theme {
        crate::config::Theme::System => 0,
        crate::config::Theme::Light => 1,
        crate::config::Theme::Dark => 2,
    });
    theme_row.add_suffix(&theme);
    appearance.add(&theme_row);
    let tile_size_row = adw::PreferencesRow::new();
    let tile_size_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    tile_size_content.set_hexpand(true);
    tile_size_content.set_margin_start(14);
    tile_size_content.set_margin_end(14);
    tile_size_content.set_margin_top(10);
    tile_size_content.set_margin_bottom(10);
    let tile_size_title = gtk::Label::new(Some("Library tile size"));
    tile_size_title.set_xalign(0.0);
    let tile_size_description =
        gtk::Label::new(Some("Adjust the size of game cards on the Home page"));
    tile_size_description.set_xalign(0.0);
    tile_size_description.add_css_class("dim-label");
    tile_size_content.append(&tile_size_title);
    tile_size_content.append(&tile_size_description);
    let tile_size = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile_size.add_css_class("linked");
    tile_size.set_halign(gtk::Align::Fill);
    tile_size.set_hexpand(true);
    let selected_tile_size = model.borrow().config.library_card_size.min(3);
    let mut tile_size_buttons = Vec::new();
    for (index, label) in ["Small", "Medium", "Large", "Extra Large"]
        .into_iter()
        .enumerate()
    {
        let button = gtk::ToggleButton::with_label(label);
        button.set_hexpand(true);
        if let Some(first) = tile_size_buttons.first() {
            button.set_group(Some(first));
        }
        button.set_active(index as u8 == selected_tile_size);
        tile_size.append(&button);
        tile_size_buttons.push(button);
    }
    tile_size_content.append(&tile_size);
    tile_size_row.set_child(Some(&tile_size_content));
    library_display.add(&tile_size_row);
    let sidebar_icons = adw::SwitchRow::new();
    sidebar_icons.set_title("Show game icons in sidebar");
    sidebar_icons.set_subtitle("Display each game's icon beside its name in the game list");
    sidebar_icons.set_active(model.borrow().config.show_sidebar_game_icons);
    library_display.add(&sidebar_icons);
    let backup_status = adw::SwitchRow::new();
    backup_status.set_title("Show backup status");
    backup_status.set_subtitle(
        "Use amber and gold states to distinguish downloaded installers from installed games",
    );
    backup_status.set_active(model.borrow().config.show_backup_status);
    library_display.add(&backup_status);
    library_page.add(&library_display);
    appearance_page.add(&appearance);

    let navigation = gtk::ListBox::new();
    navigation.set_selection_mode(gtk::SelectionMode::Single);
    navigation.add_css_class("settings-navigation");
    let navigation_shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    navigation_shell.set_width_request(210);
    navigation_shell.add_css_class("settings-sidebar");
    let navigation_title = gtk::Label::new(Some("LUDOMERE SETTINGS"));
    navigation_title.set_xalign(0.0);
    navigation_title.add_css_class("settings-sidebar-title");
    navigation_shell.append(&navigation_title);
    let stack = gtk::Stack::new();
    stack.set_widget_name("ludomere-settings-stack");
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let storage_header =
        settings_navigation_row("storage-section", "Storage", "drive-harddisk-symbolic");
    let storage_expanded = gtk::ToggleButton::new();
    storage_expanded.set_icon_name("pan-end-symbolic");
    storage_expanded.set_tooltip_text(Some("Expand Storage sections"));
    storage_expanded.add_css_class("flat");
    storage_header
        .child()
        .and_downcast::<gtk::Box>()
        .expect("navigation row content")
        .append(&storage_expanded);
    let mut storage_rows = Vec::new();
    let mut navigation_rows = Vec::new();
    for (name, title, icon, page) in [
        (
            "account",
            "Account",
            "avatar-default-symbolic",
            account_page.upcast::<gtk::Widget>(),
        ),
        (
            "downloads",
            "Downloads",
            "folder-download-symbolic",
            downloads_page.upcast::<gtk::Widget>(),
        ),
        (
            "library",
            "Game Display",
            "view-grid-symbolic",
            library_page.upcast::<gtk::Widget>(),
        ),
        (
            "storage",
            "Game Library",
            "drive-harddisk-symbolic",
            build_storage_page(
                &settings_window,
                w,
                model,
                crate::config::LibraryKind::GameFiles,
                "Game Library",
            )
            .upcast::<gtk::Widget>(),
        ),
        (
            "storage-offline",
            "Offline Installers",
            "folder-download-symbolic",
            build_storage_page(
                &settings_window,
                w,
                model,
                crate::config::LibraryKind::OfflineInstallers,
                "Offline Installers",
            )
            .upcast::<gtk::Widget>(),
        ),
        (
            "storage-extras",
            "Goodies & Extras",
            "folder-symbolic",
            build_storage_page(
                &settings_window,
                w,
                model,
                crate::config::LibraryKind::Extras,
                "Goodies & Extras",
            )
            .upcast::<gtk::Widget>(),
        ),
        (
            "maintenance",
            "Maintenance",
            "emblem-system-symbolic",
            storage_maintenance_page.upcast::<gtk::Widget>(),
        ),
        (
            "proton",
            "Proton",
            "applications-games-symbolic",
            proton_page(&settings_window).upcast::<gtk::Widget>(),
        ),
        (
            "appearance",
            "Appearance",
            "applications-graphics-symbolic",
            appearance_page.upcast::<gtk::Widget>(),
        ),
        (
            "comet",
            "GOG Online Services",
            "network-server-symbolic",
            comet::comet_page(&settings_window).upcast::<gtk::Widget>(),
        ),
    ] {
        if name == "storage" {
            navigation.append(&storage_header);
        }
        let row = settings_navigation_row(name, title, icon);
        if name == "storage" || name.starts_with("storage-") {
            row.child()
                .expect("navigation row content")
                .set_margin_start(24);
            row.set_visible(false);
            storage_rows.push(row.clone());
        }
        navigation.append(&row);
        navigation_rows.push(row);
        stack.add_named(&page, Some(name));
    }
    navigation_shell.append(&navigation);
    let navigation_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&navigation_shell)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&navigation_scroll);
    content.append(&stack);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    content.set_vexpand(true);
    root.append(&content);
    let syncing_navigation = Rc::new(std::cell::Cell::new(false));
    storage_expanded.connect_toggled({
        let navigation = navigation.clone();
        let storage_header = storage_header.clone();
        let storage_rows = storage_rows.clone();
        let stack = stack.clone();
        let syncing = syncing_navigation.clone();
        move |expanded| {
            for row in &storage_rows {
                row.set_visible(expanded.is_active());
            }
            expanded.set_icon_name(if expanded.is_active() {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
            expanded.set_tooltip_text(Some(if expanded.is_active() {
                "Collapse Storage sections"
            } else {
                "Expand Storage sections"
            }));
            if expanded.is_active() && !syncing.get() {
                navigation.select_row(Some(&storage_rows[0]));
            }
            if !expanded.is_active()
                && navigation
                    .selected_row()
                    .is_some_and(|row| storage_rows.contains(&row))
            {
                syncing.set(true);
                navigation.select_row(Some(&storage_header));
                stack.set_visible_child_name("storage");
                syncing.set(false);
            }
        }
    });
    navigation.connect_row_selected({
        let stack = stack.clone();
        let storage_expanded = storage_expanded.clone();
        let storage_header = storage_header.clone();
        let storage_default = storage_rows[0].clone();
        let syncing = syncing_navigation.clone();
        move |navigation, row| {
            if syncing.get() {
                return;
            }
            if let Some(row) = row {
                syncing.set(true);
                if row == &storage_header {
                    storage_expanded.set_active(true);
                    stack.set_visible_child_name("storage");
                    navigation.select_row(Some(&storage_default));
                } else {
                    stack.set_visible_child_name(row.widget_name().as_str());
                }
                syncing.set(false);
            }
        }
    });
    navigation.connect_row_activated({
        let storage_header = storage_header.clone();
        let storage_default = storage_rows[0].clone();
        let storage_expanded = storage_expanded.clone();
        move |navigation, row| {
            if row == &storage_header {
                storage_expanded.set_active(true);
                navigation.select_row(Some(&storage_default));
            }
        }
    });
    stack.connect_visible_child_name_notify({
        let navigation = navigation.clone();
        let storage_expanded = storage_expanded.clone();
        let syncing = syncing_navigation.clone();
        move |stack| {
            if syncing.get() {
                return;
            }
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let Some(row) = navigation_rows.iter().find(|row| row.widget_name() == name) else {
                return;
            };
            syncing.set(true);
            if storage_rows.contains(row) {
                storage_expanded.set_active(true);
            }
            navigation.select_row(Some(row));
            syncing.set(false);
        }
    });
    stack.set_visible_child_name(if stack.child_by_name(initial_page).is_some() {
        initial_page
    } else {
        "account"
    });
    stack.notify("visible-child-name");
    settings_window.set_content(Some(&root));

    {
        let window = settings_window.clone();
        let model = model.clone();
        let row = open_downloads.clone();
        open_downloads_button.connect_clicked(move |button| {
            open_download_library(&window, &model, &row, button);
        });
    }
    {
        let row = clear_images.clone();
        let model = model.clone();
        clear_images_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            row.set_subtitle("Clearing replaceable images…");
            let cache = crate::identity::cache_root();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = clear_replaceable_images_at(&cache)
                    .map_err(anyhow::Error::from)
                    .and_then(|()| {
                        online::invalidate_all_section_cache(online::DetailSection::Artwork)
                    });
                let _ = sender.send(result);
            });
            let button = button.clone();
            let row = row.clone();
            let model = model.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(Ok(())) => {
                        model
                            .borrow_mut()
                            .section_states
                            .retain(|(_, scope), _| *scope != online::DetailSection::Artwork);
                        widgets::media::clear_card_texture_cache();
                        row.set_subtitle(
                            "Image cache cleared; artwork will return on the next refresh",
                        );
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        row.set_subtitle(&format!("Could not clear image cache: {error}"));
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        row.set_subtitle("Image cache cleanup stopped unexpectedly. Try again.");
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }
    {
        let window = w.window.clone();
        let status = preference_status.clone();
        refresh_metadata_button.connect_clicked(move |_| {
            if let Err(error) =
                gtk::prelude::WidgetExt::activate_action(&window, "win.refresh", None)
            {
                tracing::warn!(%error, "could not activate metadata refresh");
                status.set_label(&format!(
                    "Could not start synchronization: {error}. Close Settings and try again."
                ));
                status.set_visible(true);
            }
        });
    }
    for (index, button) in tile_size_buttons.into_iter().enumerate() {
        let w = w.clone();
        let model = model.clone();
        let status = preference_status.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            const WIDTHS: [i32; 4] = [140, 180, 220, 260];
            let width = WIDTHS[index];
            if model.borrow().card_width == width {
                return;
            }
            {
                let mut state = model.borrow_mut();
                state.card_width = width;
                state.config.library_card_size = index as u8;
                save_preferences(&state.config, &status);
            }
            rebuild_home_grid(&w, &model);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let status = preference_status.clone();
        sidebar_icons.connect_active_notify(move |row| {
            let visible = row.is_active();
            {
                let mut state = model.borrow_mut();
                state.config.show_sidebar_game_icons = visible;
                save_preferences(&state.config, &status);
            }
            set_sidebar_icons_visible(&w, visible);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let status = preference_status.clone();
        backup_status.connect_active_notify(move |row| {
            {
                let mut state = model.borrow_mut();
                state.config.show_backup_status = row.is_active();
                save_preferences(&state.config, &status);
            }
            update_sidebar_download_styles(&w, &model.borrow());
        });
    }

    {
        let model = model.clone();
        let rebuild_row = rebuild_row.clone();
        let w = w.clone();
        rebuild_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            rebuild_row.set_subtitle("Inspecting managed files…");
            let (config, games, epoch) = {
                let state = model.borrow();
                (
                    state.config.clone(),
                    state.games.clone(),
                    state.account_epoch,
                )
            };
            let (sender, receiver) = mpsc::channel();
            let session = online::account_session();
            let scanned_roots = [config.offline_libraries.clone(), config.extras_libraries.clone()];
            std::thread::spawn(move || {
                let result = StateStore::open().and_then(|mut store| {
                    let mut summary = managed::RebuildSummary::default();
                    let statuses = crate::storage::inspect_libraries(&config)?;
                    let mut skipped = 0usize;
                    for kind in [crate::config::LibraryKind::OfflineInstallers, crate::config::LibraryKind::Extras] {
                        for library in config.libraries(kind) {
                            if !statuses.iter().any(|status| status.kind == kind && status.library_id == library.id && status.compatibility == crate::storage::LibraryCompatibility::Compatible) { skipped += 1; continue; }
                            let found = managed::rebuild_for_session(&mut store, &library.path, &games, session)?;
                            summary.files += found.files; summary.matched += found.matched; summary.unmatched += found.unmatched;
                            summary.partials += found.partials; summary.ignored += found.ignored;
                        }
                    }
                    Ok((summary, skipped, store.managed_files()?, store.download_jobs()?))
                });
                let _ = sender.send(result);
            });
            let model = model.clone();
            let w = w.clone();
            let row = rebuild_row.clone();
            let button = button.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if model.borrow().account_epoch != epoch
                    || [model.borrow().config.offline_libraries.clone(), model.borrow().config.extras_libraries.clone()] != scanned_roots
                {
                    button.set_sensitive(true);
                    row.set_subtitle("Account or library folders changed. Click Rebuild to inspect the current libraries.");
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok((summary, skipped, files, jobs))) => {
                        let mut state = model.borrow_mut();
                        managed::apply_to_games(&mut state.games, &files);
                        state.download_jobs = jobs;
                        drop(state);
                        row.set_subtitle(&format!(
                            "Indexed {} files ({} matched, {} unmatched); {} incompatible or unavailable libraries skipped",
                            summary.files, summary.matched, summary.unmatched, skipped
                        ));
                        refresh_local_action_state(&w, &model);
                    }
                    Ok(Err(error)) => {
                        row.set_subtitle(&format!("Could not inspect files: {error}. Try again."))
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => row.set_subtitle("File inspection stopped. Try again."),
                }
                button.set_sensitive(true);
                glib::ControlFlow::Break
            });
        });
    }
    {
        let model = model.clone();
        let language_list = language_list.clone();
        let status = preference_status.clone();
        language.connect_selected_notify(move |selector| {
            let selected = selector.selected();
            let mut state = model.borrow_mut();
            state.config.installer_language = if selected == 0 {
                None
            } else {
                language_list
                    .string(selected)
                    .map(|value| value.to_string())
            };
            save_preferences(&state.config, &status);
        });
    }
    for (button, update) in [(windows, 0_u8), (linux, 1_u8), (macos, 2_u8)] {
        let model = model.clone();
        let status = preference_status.clone();
        button.connect_toggled(move |button| {
            let mut state = model.borrow_mut();
            match update {
                0 => state.config.installer_windows = button.is_active(),
                1 => state.config.installer_linux = button.is_active(),
                _ => state.config.installer_macos = button.is_active(),
            }
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        concurrency.connect_value_changed(move |selector| {
            let limit = selector.value_as_int().clamp(1, 4) as usize;
            let mut state = model.borrow_mut();
            state.config.max_concurrent_downloads = limit;
            save_preferences(&state.config, &status);
            download::set_concurrency(limit);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        extras_default.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.download_extras_by_default = row.is_active();
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        patches_default.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.download_patches_by_default = row.is_active();
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        prefer_patch_updates.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.prefer_patch_updates = row.is_active();
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        interactive_prompts.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.interactive_installer_prompts = row.is_active();
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        retired_artifacts.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.show_retired_artifacts = row.is_active();
            save_preferences(&state.config, &status);
        });
    }
    {
        let model = model.clone();
        let status = preference_status.clone();
        theme.connect_selected_notify(move |selector| {
            let selected = match selector.selected() {
                1 => crate::config::Theme::Light,
                2 => crate::config::Theme::Dark,
                _ => crate::config::Theme::System,
            };
            apply_theme(selected);
            let mut state = model.borrow_mut();
            state.config.theme = selected;
            save_preferences(&state.config, &status);
        });
    }
    settings_window.present();
}

fn save_preferences(config: &Config, status: &gtk::Label) {
    match config.save() {
        Ok(()) => status.set_visible(false),
        Err(error) => {
            status.set_label(&format!("These changes apply for this session, but could not be saved: {error}. Change the option again to retry."));
            status.set_visible(true);
        }
    }
}

fn open_download_library(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    row: &adw::ActionRow,
    button: &gtk::Button,
) {
    if !window.is_visible() || !row.is_mapped() || model.borrow().logout_pending {
        return;
    }
    let config = model.borrow().config.clone();
    let mut libraries = [
        crate::config::LibraryKind::OfflineInstallers,
        crate::config::LibraryKind::Extras,
    ]
    .into_iter()
    .flat_map(|kind| {
        config
            .libraries(kind)
            .iter()
            .cloned()
            .map(move |library| (kind, library))
    })
    .collect::<Vec<_>>();
    if libraries.is_empty() {
        row.set_subtitle("No Offline Installers or Goodies & Extras directory is configured. Add a directory in Settings → Storage first.");
        return;
    }
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let open = {
        let window = window.clone();
        let model = model.clone();
        let row = row.clone();
        let button = button.clone();
        move |kind, library: crate::config::GameLibrary| {
            if !window.is_visible()
                || !row.is_mapped()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
                || online::account_session() != session
            {
                return;
            }
            row.set_subtitle("Checking library access…");
            button.set_sensitive(false);
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<_> {
                    let _activity = crate::profile_reset::begin_activity("library inspection")?;
                    anyhow::ensure!(
                        online::account_session() == session,
                        "Account changed; open the library again."
                    );
                    let current = crate::storage::read_config()?;
                    let found = crate::storage::validate_library(&current, kind, &library.id)?;
                    anyhow::ensure!(
                        found.path == library.path,
                        "Library changed; choose it again."
                    );
                    Ok(found)
                })();
                let _ = sender.send(result);
            });
            let window = window.clone();
            let model = model.clone();
            let row = row.clone();
            let button = button.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if !window.is_visible()
                    || !row.is_mapped()
                    || model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                    || online::account_session() != session
                {
                    row.set_subtitle(
                        "Opening canceled because the window or account changed. Try again.",
                    );
                    button.set_sensitive(true);
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok(library)) => {
                        row.set_subtitle("Opening library in your file manager…");
                        let model = Rc::downgrade(&model);
                        let row = row.downgrade();
                        super::widgets::file_open::launch_validated_directory(
                            &library.path,
                            &window,
                            "library directory",
                            move || {
                                online::account_session() == session
                                    && row.upgrade().is_some_and(|row| row.is_mapped())
                                    && model.upgrade().is_some_and(|model| {
                                        let model = model.borrow();
                                        model.account_epoch == epoch && !model.logout_pending
                                    })
                            },
                        );
                    }
                    Ok(Err(error)) => {
                        row.set_subtitle(&format!("Could not open library: {error:#}"))
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => {
                        row.set_subtitle("Library inspection stopped unexpectedly. Try again.")
                    }
                }
                button.set_sensitive(true);
                glib::ControlFlow::Break
            });
        }
    };
    if libraries.len() == 1 {
        let (kind, library) = libraries.remove(0);
        open(kind, library);
        return;
    }
    let dialog = adw::AlertDialog::builder().heading("Open a library").body("Choose a configured Offline Installers or Goodies & Extras directory. Incompatible libraries cannot be opened here.").build();
    dialog.add_response("cancel", "Cancel");
    dialog.set_close_response("cancel");
    for (index, (kind, library)) in libraries.iter().enumerate() {
        dialog.add_response(
            &index.to_string(),
            &format!("{} — {}", kind.label(), library.path.display()),
        );
    }
    dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if let Some((kind, library)) = response
            .parse::<usize>()
            .ok()
            .and_then(|index| libraries.get(index))
            .cloned()
        {
            open(kind, library);
        }
    });
}

fn find_settings_stack(widget: &gtk::Widget) -> Option<gtk::Stack> {
    if widget.widget_name() == "ludomere-settings-stack" {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(stack) = find_settings_stack(&current) {
            return Some(stack);
        }
        child = current.next_sibling();
    }
    None
}

fn installation_source_order_group(
    model: &Rc<RefCell<AppModel>>,
    status: &gtk::Label,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Preferred installation order");
    group.set_description(Some(
        "Fresh installs use the first available source. Drag rows or use the arrow buttons.",
    ));
    let labels = Rc::new(RefCell::new(Vec::<adw::ActionRow>::new()));
    for index in 0..3 {
        let row = adw::ActionRow::new();
        row.set_activatable(false);
        let drag = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        drag.set_tooltip_text(Some("Drag to reorder"));
        row.add_prefix(&drag);
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_tooltip_text(Some("Move up"));
        up.set_valign(gtk::Align::Center);
        up.set_sensitive(index > 0);
        let down = gtk::Button::from_icon_name("go-down-symbolic");
        down.set_tooltip_text(Some("Move down"));
        down.set_valign(gtk::Align::Center);
        down.set_sensitive(index < 2);
        row.add_suffix(&up);
        row.add_suffix(&down);

        let drag_source = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .build();
        drag_source.connect_prepare(move |_, _, _| {
            Some(gdk::ContentProvider::for_value(&(index as u32).to_value()))
        });
        row.add_controller(drag_source);
        let drop_target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        {
            let model = model.clone();
            let labels = labels.clone();
            let status = status.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                let Ok(from) = value.get::<u32>() else {
                    return false;
                };
                reorder_installation_sources(&model, from as usize, index, &status);
                refresh_installation_source_labels(&model, &labels.borrow());
                true
            });
        }
        row.add_controller(drop_target);
        {
            let model = model.clone();
            let labels = labels.clone();
            let status = status.clone();
            up.connect_clicked(move |_| {
                reorder_installation_sources(&model, index, index - 1, &status);
                refresh_installation_source_labels(&model, &labels.borrow());
            });
        }
        {
            let model = model.clone();
            let labels = labels.clone();
            let status = status.clone();
            down.connect_clicked(move |_| {
                reorder_installation_sources(&model, index, index + 1, &status);
                refresh_installation_source_labels(&model, &labels.borrow());
            });
        }
        labels.borrow_mut().push(row.clone());
        group.add(&row);
    }
    refresh_installation_source_labels(model, &labels.borrow());
    group
}

fn reorder_installation_sources(
    model: &Rc<RefCell<AppModel>>,
    from: usize,
    to: usize,
    status: &gtk::Label,
) {
    if from >= 3 || to >= 3 || from == to {
        return;
    }
    let mut state = model.borrow_mut();
    let source = state.config.installation_source_order.remove(from);
    state.config.installation_source_order.insert(to, source);
    save_preferences(&state.config, status);
}

fn refresh_installation_source_labels(model: &Rc<RefCell<AppModel>>, rows: &[adw::ActionRow]) {
    use crate::config::PreferredInstallationSource::*;
    for (row, source) in rows
        .iter()
        .zip(&model.borrow().config.installation_source_order)
    {
        let (title, subtitle) = match source {
            LinuxOffline => ("Linux", "Native offline installer"),
            WindowsGalaxy => ("Windows", "Galaxy build"),
            WindowsOffline => ("Windows", "Offline installer"),
        };
        row.set_title(title);
        row.set_subtitle(subtitle);
    }
}

pub(super) fn settings_navigation_row(name: &str, title: &str, icon: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(name);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    content.append(&label);
    row.set_child(Some(&content));
    row
}

fn settings_account_page(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Account");
    page.set_icon_name(Some("avatar-default-symbolic"));
    let profile_group = adw::PreferencesGroup::new();
    profile_group.set_title("GOG account");
    let profile = model.borrow().account_profile.clone();
    let identity = adw::ActionRow::new();
    identity.set_title(
        profile
            .as_ref()
            .map_or("Not signed in", |profile| profile.username.as_str()),
    );
    identity.set_subtitle(
        profile
            .as_ref()
            .map_or("Sign in to synchronize your GOG library", |profile| {
                profile.email.as_str()
            }),
    );
    let avatar = gtk::Picture::new();
    avatar.set_width_request(72);
    avatar.set_height_request(72);
    avatar.set_content_fit(gtk::ContentFit::Cover);
    avatar.add_css_class("settings-account-avatar");
    if let Some(path) = profile
        .as_ref()
        .and_then(|profile| profile.avatar_path.as_ref())
    {
        avatar.set_filename(Some(path));
    }
    identity.add_prefix(&avatar);
    let account_action = gtk::Button::with_label(if profile.is_some() {
        "Sign in again…"
    } else {
        "Sign in…"
    });
    account_action.set_valign(gtk::Align::Center);
    identity.add_suffix(&account_action);
    profile_group.add(&identity);
    if let Some(profile) = &profile {
        for (title, value) in [
            ("GOG ID", profile.user_id.as_str()),
            ("Country", profile.country.as_str()),
            ("Language", profile.preferred_language.as_str()),
            ("Currency", profile.selected_currency.as_str()),
        ] {
            let row = adw::ActionRow::new();
            row.set_title(title);
            row.set_subtitle(if value.is_empty() {
                "Not provided"
            } else {
                value
            });
            profile_group.add(&row);
        }
        if let Some(member_since) = profile.member_since
            && let Some(date) = chrono::DateTime::from_timestamp(member_since, 0)
        {
            let row = adw::ActionRow::new();
            row.set_title("Member since");
            row.set_subtitle(&date.format("%B %-d, %Y").to_string());
            profile_group.add(&row);
        }
    }
    page.add(&profile_group);

    let privacy = adw::PreferencesGroup::new();
    privacy.set_title("Factory Reset");
    let reset_row = adw::ActionRow::builder()
        .title("Reset Ludomere")
        .subtitle("Delete this application's profile and cache, then close Ludomere. Library files are kept.")
        .build();
    let reset = gtk::Button::with_label("Factory Reset…");
    reset.add_css_class("destructive-action");
    reset.set_valign(gtk::Align::Center);
    let reset_status = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .visible(false)
        .build();
    reset_status.set_widget_name("factory-reset-status");
    let reset_status_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .max_content_height(160)
        .propagate_natural_height(true)
        .child(&reset_status)
        .build();
    reset.connect_clicked({
        let w = w.clone();
        let model = model.clone();
        let reset_status = reset_status.clone();
        move |button| {
            if model.borrow().logout_pending { return; }
            let Some(parent) = button.root().and_downcast::<gtk::Window>() else { return; };
            let epoch = model.borrow().account_epoch;
            let confirmation = adw::AlertDialog::builder()
                .heading("Factory reset Ludomere?")
                .body("This deletes Ludomere's database, settings, favorites, tags, playtime, queue and automatic-resume records, local login data, cached metadata, images and logs. Ludomere will close. If the system credential store is unavailable, its saved entry may remain; automatic login stays disabled.\n\nGames, downloaded files, game prefixes and their saves, Proton versions, runtimes and cloud recovery copies are kept. Running games continue; background downloads and setup are stopped safely. Custom library folders must be added again afterward.")
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("reset", "Factory Reset")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            let w = w.clone();
            let model = model.clone();
            let reset_status = reset_status.clone();
            let parent_lifetime = parent.downgrade();
            confirmation.choose(Some(&parent), gio::Cancellable::NONE, move |response| {
                if response == "reset" && parent_lifetime.upgrade().is_some_and(|window| window.is_visible())
                    && model.borrow().account_epoch == epoch && !model.borrow().logout_pending {
                    super::window::sign_out(&w, &model, true, Some(reset_status));
                }
            });
        }
    });
    reset_row.add_suffix(&reset);
    reset_row.set_activatable_widget(Some(&reset));
    privacy.add(&reset_row);
    privacy.add(&reset_status_scroll);
    page.add(&privacy);

    let connection = adw::PreferencesGroup::new();
    connection.set_title("Connection");
    let status = adw::ActionRow::new();
    let authenticated = model
        .borrow()
        .account_token
        .as_ref()
        .is_some_and(|token| token.expires_at > chrono::Utc::now().timestamp());
    status.set_title("GOG session");
    status.set_subtitle(if !model.borrow().network_available {
        "Offline"
    } else if authenticated {
        "Online and authenticated"
    } else {
        "Authentication required"
    });
    connection.add(&status);
    page.add(&connection);
    {
        let reconnect = w.reconnect.clone();
        account_action.connect_clicked(move |_| reconnect.emit_clicked());
    }
    page
}

pub(super) fn refresh_installed_state_after_library_change(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
) {
    refresh_local_action_state(w, model);
}

fn clear_replaceable_images_at(cache_root: &std::path::Path) -> std::io::Result<()> {
    [
        cache_root.join("products"),
        cache_root.join("media"),
        cache_root.join("screenshots"),
    ]
    .into_iter()
    .try_for_each(|path| match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[ignore = "requires private HOME/all XDG, D-Bus and display; never activates a file manager"]
    fn downloaded_library_open_skips_redundant_choices_and_stops_stale_requests() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        assert!(home.starts_with("/tmp") && home.to_string_lossy().contains("ludomere-p286-"));
        super::super::widgets::file_open::DIRECTORY_LAUNCHES
            .with(|paths| *paths.borrow_mut() = Some(Vec::new()));
        adw::init().unwrap();
        let application = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.LibraryOpenTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        application.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&application);
        let group = adw::PreferencesGroup::new();
        let row = adw::ActionRow::builder()
            .title("Open library")
            .use_markup(false)
            .build();
        let button = gtk::Button::with_label("Open");
        row.add_suffix(&button);
        group.add(&row);
        window.set_content(Some(&group));
        window.present();
        let model = Rc::new(RefCell::new(AppModel::default()));
        model.borrow_mut().config.offline_libraries.clear();
        model.borrow_mut().config.extras_libraries.clear();
        button.connect_clicked({
            let window = window.clone();
            let model = model.clone();
            let row = row.clone();
            move |button| open_download_library(&window, &model, &row, button)
        });
        let wait_until = |condition: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !condition() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "settings response timed out"
                );
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait_until(&|| row.is_mapped());
        button.emit_clicked();
        assert!(row.subtitle().unwrap().contains("Settings → Storage"));
        assert!(window.visible_dialog().is_none());

        // An ordinary file cannot be used as a library. Exercising the real worker
        // therefore proves direct dispatch without ever launching a desktop app.
        let invalid = home.join("not-a-directory");
        std::fs::write(&invalid, b"synthetic").unwrap();
        let library = crate::config::GameLibrary {
            id: "single".into(),
            name: "Single".into(),
            path: invalid.clone(),
            default: true,
        };
        model.borrow_mut().config.offline_libraries = vec![library.clone()];
        model.borrow().config.save().unwrap();
        button.emit_clicked();
        assert!(window.visible_dialog().is_none());
        assert!(!button.is_sensitive());
        assert_eq!(row.subtitle().as_deref(), Some("Checking library access…"));
        wait_until(&|| button.is_sensitive());
        assert!(row.subtitle().unwrap().contains("Could not open library"));
        assert!(window.visible_dialog().is_none());

        std::fs::remove_file(&invalid).unwrap();
        std::fs::create_dir(&invalid).unwrap();
        button.emit_clicked();
        wait_until(&|| button.is_sensitive());
        assert!(window.visible_dialog().is_none());
        super::super::widgets::file_open::DIRECTORY_LAUNCHES
            .with(|paths| assert_eq!(paths.borrow().as_ref().unwrap(), &[invalid]));

        model.borrow_mut().config.extras_libraries = vec![crate::config::GameLibrary {
            id: "second".into(),
            path: home.join("other"),
            ..library
        }];
        let extra = home.join("other");
        std::fs::create_dir(&extra).unwrap();
        model.borrow().config.save().unwrap();
        button.emit_clicked();
        let dialog = window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>()
            .unwrap();
        assert_eq!(dialog.heading().as_deref(), Some("Open a library"));
        assert!(button.is_sensitive());
        dialog.close();
        wait_until(&|| window.visible_dialog().is_none());

        fn click(widget: &gtk::Widget, label: &str) -> bool {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.label().as_deref() == Some(label)
            {
                button.emit_clicked();
                return true;
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if click(&widget, label) {
                    return true;
                }
                child = widget.next_sibling();
            }
            false
        }
        button.emit_clicked();
        let dialog = window.visible_dialog().unwrap();
        assert!(click(
            dialog.upcast_ref(),
            &format!(
                "{} — {}",
                crate::config::LibraryKind::Extras.label(),
                extra.display()
            )
        ));
        wait_until(&|| {
            let status = row.subtitle().unwrap_or_default();
            assert!(
                !status.contains("Could not open") && !status.contains("canceled"),
                "{status}"
            );
            super::super::widgets::file_open::DIRECTORY_LAUNCHES
                .with(|paths| paths.borrow().as_ref().unwrap().len() == 2)
        });
        super::super::widgets::file_open::DIRECTORY_LAUNCHES
            .with(|paths| assert_eq!(paths.borrow().as_ref().unwrap().last(), Some(&extra)));

        model.borrow_mut().config.extras_libraries.clear();
        button.emit_clicked();
        model.borrow_mut().account_epoch += 1;
        wait_until(&|| button.is_sensitive());
        assert!(row.subtitle().unwrap().contains("canceled"));
        assert!(window.visible_dialog().is_none());
        button.emit_clicked();
        group.set_visible(false);
        wait_until(&|| button.is_sensitive());
        assert!(row.subtitle().unwrap().contains("canceled"));
        super::super::widgets::file_open::DIRECTORY_LAUNCHES
            .with(|paths| assert_eq!(paths.borrow_mut().take().unwrap().len(), 2));
        window.close();
    }

    #[test]
    fn image_cache_clear_preserves_unrelated_cached_state() {
        let root = std::env::temp_dir().join(format!(
            "gog-image-cache-clear-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for directory in ["products", "media", "screenshots", "account"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
            std::fs::write(root.join(directory).join("cached"), b"data").unwrap();
        }
        std::fs::write(root.join("persistent-marker"), b"keep").unwrap();

        clear_replaceable_images_at(&root).unwrap();

        assert!(!root.join("products").exists());
        assert!(!root.join("media").exists());
        assert!(!root.join("screenshots").exists());
        assert!(root.join("account/cached").exists());
        assert!(root.join("persistent-marker").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
