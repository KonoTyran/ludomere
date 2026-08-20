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

    let account_page = settings_account_page(w, model);
    let downloads_page = adw::PreferencesPage::new();
    downloads_page.set_title("Downloads");
    downloads_page.set_icon_name(Some("folder-download-symbolic"));
    let library_page = adw::PreferencesPage::new();
    library_page.set_title("Library");
    library_page.set_icon_name(Some("view-grid-symbolic"));
    let library_display = adw::PreferencesGroup::new();
    library_display.set_title("Library display");
    let storage_page = build_storage_page(&settings_window, w, model);
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

    let bandwidth_row = adw::ActionRow::new();
    bandwidth_row.set_title("Download speed limit");
    bandwidth_row.set_subtitle("Shared by installer, Galaxy depot, and cloud-save downloads");
    let bandwidth_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bandwidth_controls.set_valign(gtk::Align::Center);
    let bandwidth = gtk::DropDown::from_strings(&[
        "Unlimited",
        "512 KiB/s",
        "1 MiB/s",
        "2 MiB/s",
        "5 MiB/s",
        "10 MiB/s",
        "Custom",
    ]);
    let configured_bandwidth = model.borrow().config.download_bandwidth_limit_bps;
    bandwidth.set_selected(bandwidth_preset_index(configured_bandwidth));
    let custom_bandwidth = gtk::SpinButton::with_range(64.0, 1_048_576.0, 64.0);
    custom_bandwidth.set_tooltip_text(Some("Custom limit in KiB/s"));
    custom_bandwidth.set_value(configured_bandwidth.unwrap_or(1024 * 1024).div_ceil(1024) as f64);
    custom_bandwidth.set_visible(bandwidth.selected() == 6);
    bandwidth_controls.append(&bandwidth);
    bandwidth_controls.append(&custom_bandwidth);
    bandwidth_row.add_suffix(&bandwidth_controls);
    downloads.add(&bandwidth_row);

    let galaxy_updates = adw::SwitchRow::new();
    galaxy_updates.set_title("Automatically update Galaxy installations");
    galaxy_updates.set_subtitle("Check after synchronization and every six hours while online");
    galaxy_updates.set_active(model.borrow().config.auto_update_galaxy_installations);
    downloads.add(&galaxy_updates);

    let offline_updates = adw::SwitchRow::new();
    offline_updates.set_title("Automatically download offline installer updates");
    offline_updates
        .set_subtitle("Download the newest complete primary installer without running it");
    offline_updates.set_active(model.borrow().config.auto_download_offline_installers);
    downloads.add(&offline_updates);

    let prune_installers = adw::SwitchRow::new();
    prune_installers.set_title("Move superseded installers to Trash");
    prune_installers.set_subtitle("Only after the newest complete replacement has been verified");
    prune_installers.set_active(model.borrow().config.prune_superseded_offline_installers);
    downloads.add(&prune_installers);

    let rebuild_row = adw::ActionRow::new();
    rebuild_row.set_title("Rebuild downloaded-file index");
    rebuild_row.set_subtitle("Inspect the managed download directory without changing any files");
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

    let source_order = installation_source_order_group(model);
    downloads_page.add(&source_order);

    let maintenance = adw::PreferencesGroup::new();
    maintenance.set_title("Storage and synchronization");
    let open_downloads = adw::ActionRow::new();
    open_downloads.set_title("Open download directory");
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
    refresh_metadata.set_subtitle("Synchronize owned games, manifests, and artwork");
    let refresh_metadata_button = gtk::Button::with_label("Refresh");
    refresh_metadata_button.set_valign(gtk::Align::Center);
    refresh_metadata.add_suffix(&refresh_metadata_button);
    maintenance.add(&refresh_metadata);
    let check_updates = adw::ActionRow::new();
    check_updates.set_title("Check for game updates");
    check_updates.set_subtitle("Queue available Galaxy updates and current offline installers");
    let check_updates_button = gtk::Button::with_label("Check Now");
    check_updates_button.set_valign(gtk::Align::Center);
    check_updates.add_suffix(&check_updates_button);
    maintenance.add(&check_updates);
    let storage_maintenance_page = adw::PreferencesPage::new();
    storage_maintenance_page.set_title("Maintenance");
    storage_maintenance_page.set_icon_name(Some("emblem-system-symbolic"));
    storage_maintenance_page.add(&maintenance);

    {
        let widgets = w.clone();
        let model = model.clone();
        check_updates_button.connect_clicked(move |_| {
            super::window::start_update_check(
                &widgets,
                &model,
                crate::updates::CheckMode::Manual,
                true,
            );
        });
    }

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
            "Library",
            "view-grid-symbolic",
            library_page.upcast::<gtk::Widget>(),
        ),
        (
            "storage",
            "Storage",
            "drive-harddisk-symbolic",
            storage_page.upcast::<gtk::Widget>(),
        ),
        (
            "maintenance",
            "Maintenance",
            "emblem-system-symbolic",
            storage_maintenance_page.upcast::<gtk::Widget>(),
        ),
        (
            "appearance",
            "Appearance",
            "applications-graphics-symbolic",
            appearance_page.upcast::<gtk::Widget>(),
        ),
    ] {
        let row = settings_navigation_row(name, title, icon);
        navigation.append(&row);
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
    navigation.connect_row_selected({
        let stack = stack.clone();
        move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(row.widget_name().as_str());
            }
        }
    });
    let initial_index = [
        "account",
        "downloads",
        "library",
        "storage",
        "maintenance",
        "appearance",
    ]
    .iter()
    .position(|page| *page == initial_page)
    .unwrap_or(0) as i32;
    navigation.select_row(navigation.row_at_index(initial_index).as_ref());
    settings_window.set_content(Some(&root));

    {
        let window = settings_window.clone();
        let model = model.clone();
        open_downloads_button.connect_clicked(move |_| {
            let path = model.borrow().config.download_directory.clone();
            super::widgets::file_open::open_directory(&path, &window, "download directory");
        });
    }
    {
        let row = clear_images.clone();
        clear_images_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            row.set_subtitle("Clearing replaceable images…");
            let cache = crate::identity::cache_root();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = clear_replaceable_images_at(&cache);
                let _ = sender.send(result);
            });
            let button = button.clone();
            let row = row.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(Ok(())) => {
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
                    Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                }
            });
        });
    }
    {
        let window = w.window.clone();
        refresh_metadata_button.connect_clicked(move |_| {
            if let Err(error) =
                gtk::prelude::WidgetExt::activate_action(&window, "win.refresh", None)
            {
                tracing::warn!(%error, "could not activate metadata refresh");
            }
        });
    }
    for (index, button) in tile_size_buttons.into_iter().enumerate() {
        let w = w.clone();
        let model = model.clone();
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
                if let Err(error) = state.config.save() {
                    tracing::warn!(%error, "could not save library card size");
                }
            }
            rebuild_home_grid(&w, &model);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        sidebar_icons.connect_active_notify(move |row| {
            let visible = row.is_active();
            {
                let mut state = model.borrow_mut();
                state.config.show_sidebar_game_icons = visible;
                if let Err(error) = state.config.save() {
                    tracing::warn!(%error, "could not save sidebar icon preference");
                }
            }
            set_sidebar_icons_visible(&w, visible);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        backup_status.connect_active_notify(move |row| {
            {
                let mut state = model.borrow_mut();
                state.config.show_backup_status = row.is_active();
                if let Err(error) = state.config.save() {
                    tracing::warn!(%error, "could not save backup status preference");
                }
            }
            update_sidebar_download_styles(&w, &model.borrow());
        });
    }

    {
        let model = model.clone();
        let rebuild_row = rebuild_row.clone();
        rebuild_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            rebuild_row.set_subtitle("Inspecting managed files…");
            let summary = reconcile_managed_directory(&mut model.borrow_mut());
            rebuild_row.set_subtitle(&summary);
            button.set_sensitive(true);
        });
    }
    {
        let model = model.clone();
        let language_list = language_list.clone();
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
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save default installer language");
            }
        });
    }
    for (button, update) in [(windows, 0_u8), (linux, 1_u8), (macos, 2_u8)] {
        let model = model.clone();
        button.connect_toggled(move |button| {
            let mut state = model.borrow_mut();
            match update {
                0 => state.config.installer_windows = button.is_active(),
                1 => state.config.installer_linux = button.is_active(),
                _ => state.config.installer_macos = button.is_active(),
            }
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save default installer platforms");
            }
        });
    }
    {
        let model = model.clone();
        concurrency.connect_value_changed(move |selector| {
            let limit = selector.value_as_int().clamp(1, 4) as usize;
            let mut state = model.borrow_mut();
            state.config.max_concurrent_downloads = limit;
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save simultaneous download limit");
            }
            download::set_concurrency(limit);
        });
    }
    {
        let model = model.clone();
        let custom_bandwidth = custom_bandwidth.clone();
        bandwidth.connect_selected_notify(move |selector| {
            let selected = selector.selected();
            custom_bandwidth.set_visible(selected == 6);
            let limit = if selected == 6 {
                Some(custom_bandwidth.value_as_int().max(64) as u64 * 1024)
            } else {
                bandwidth_preset(selected)
            };
            let mut state = model.borrow_mut();
            state.config.download_bandwidth_limit_bps = limit;
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save download speed limit");
            }
            download::set_bandwidth_limit(limit);
        });
    }
    {
        let model = model.clone();
        let bandwidth = bandwidth.clone();
        custom_bandwidth.connect_value_changed(move |selector| {
            if bandwidth.selected() != 6 {
                return;
            }
            let limit = Some(selector.value_as_int().max(64) as u64 * 1024);
            let mut state = model.borrow_mut();
            state.config.download_bandwidth_limit_bps = limit;
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save custom download speed limit");
            }
            download::set_bandwidth_limit(limit);
        });
    }
    {
        let model = model.clone();
        extras_default.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.download_extras_by_default = row.is_active();
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save default extras preference");
            }
        });
    }
    for (row, setting) in [
        (galaxy_updates, 0_u8),
        (offline_updates, 1_u8),
        (prune_installers, 2_u8),
    ] {
        let model = model.clone();
        row.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            match setting {
                0 => state.config.auto_update_galaxy_installations = row.is_active(),
                1 => state.config.auto_download_offline_installers = row.is_active(),
                _ => state.config.prune_superseded_offline_installers = row.is_active(),
            }
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save automatic update preference");
            }
        });
    }
    {
        let model = model.clone();
        patches_default.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.download_patches_by_default = row.is_active();
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save default patches preference");
            }
        });
    }
    {
        let model = model.clone();
        prefer_patch_updates.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.prefer_patch_updates = row.is_active();
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save patch update preference");
            }
        });
    }
    {
        let model = model.clone();
        interactive_prompts.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.interactive_installer_prompts = row.is_active();
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save interactive installer preference");
            }
        });
    }
    {
        let model = model.clone();
        retired_artifacts.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            state.config.show_retired_artifacts = row.is_active();
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save previous-version visibility preference");
            }
        });
    }
    {
        let model = model.clone();
        theme.connect_selected_notify(move |selector| {
            let selected = match selector.selected() {
                1 => crate::config::Theme::Light,
                2 => crate::config::Theme::Dark,
                _ => crate::config::Theme::System,
            };
            apply_theme(selected);
            let mut state = model.borrow_mut();
            state.config.theme = selected;
            if let Err(error) = state.config.save() {
                tracing::warn!(%error, "could not save color scheme");
            }
        });
    }
    settings_window.present();
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

fn bandwidth_preset(index: u32) -> Option<u64> {
    match index {
        1 => Some(512 * 1024),
        2 => Some(1024 * 1024),
        3 => Some(2 * 1024 * 1024),
        4 => Some(5 * 1024 * 1024),
        5 => Some(10 * 1024 * 1024),
        _ => None,
    }
}

fn bandwidth_preset_index(limit: Option<u64>) -> u32 {
    (0..=5)
        .find(|index| bandwidth_preset(*index) == limit)
        .unwrap_or(6)
}

fn installation_source_order_group(model: &Rc<RefCell<AppModel>>) -> adw::PreferencesGroup {
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
            drop_target.connect_drop(move |_, value, _, _| {
                let Ok(from) = value.get::<u32>() else {
                    return false;
                };
                reorder_installation_sources(&model, from as usize, index);
                refresh_installation_source_labels(&model, &labels.borrow());
                true
            });
        }
        row.add_controller(drop_target);
        {
            let model = model.clone();
            let labels = labels.clone();
            up.connect_clicked(move |_| {
                reorder_installation_sources(&model, index, index - 1);
                refresh_installation_source_labels(&model, &labels.borrow());
            });
        }
        {
            let model = model.clone();
            let labels = labels.clone();
            down.connect_clicked(move |_| {
                reorder_installation_sources(&model, index, index + 1);
                refresh_installation_source_labels(&model, &labels.borrow());
            });
        }
        labels.borrow_mut().push(row.clone());
        group.add(&row);
    }
    refresh_installation_source_labels(model, &labels.borrow());
    group
}

fn reorder_installation_sources(model: &Rc<RefCell<AppModel>>, from: usize, to: usize) {
    if from >= 3 || to >= 3 || from == to {
        return;
    }
    let mut state = model.borrow_mut();
    let source = state.config.installation_source_order.remove(from);
    state.config.installation_source_order.insert(to, source);
    if let Err(error) = state.config.save() {
        tracing::warn!(%error, "could not save preferred installation order");
    }
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
    let libraries = model.borrow().config.game_libraries.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = StateStore::open()
            .and_then(|store| crate::installation::reconcile_installed_games(&store, &libraries));
        let _ = sender.send(result);
    });
    let w = w.clone_refs();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let installed = match receiver.try_recv() {
            Ok(Ok(installed)) => installed,
            Ok(Err(error)) => {
                tracing::warn!(%error, "could not scan configured game libraries");
                return glib::ControlFlow::Break;
            }
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
        };
        let selected = {
            let mut state = model.borrow_mut();
            state.installed_products = installed.into_iter().map(|game| game.product_id).collect();
            state.selected
        };
        {
            let state = model.borrow();
            update_sidebar_download_styles(&w, &state);
            refresh_filters(&w, &state);
        }
        if let Some(selected) = selected {
            render_product_details(&w, &model, selected);
        }
        glib::ControlFlow::Break
    });
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
