use super::*;

pub(super) fn show_game_settings(
    parent: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    installed: Option<crate::domain::InstalledGame>,
    refresh_after_change: Rc<dyn Fn()>,
) {
    let Some(application) = parent.application() else {
        return;
    };
    let window_name = format!("ludomere-game-settings-{}", game.product_id);
    let session = online::account_session();
    let cloud_session = CloudActionSession {
        auth: auth::session(),
        online: session,
        epoch: model.borrow().account_epoch,
        model: Rc::downgrade(model),
    };
    if let Some(existing) = application
        .windows()
        .into_iter()
        .find(|window| window.widget_name() == window_name)
    {
        existing.present();
        return;
    }

    let window = adw::ApplicationWindow::builder()
        .application(&application)
        .title(format!("{} Settings", game.title))
        .default_width(900)
        .default_height(620)
        .transient_for(parent)
        .build();
    window.set_widget_name(&window_name);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &format!("{} Settings", game.title),
        "Game-specific preferences",
    )));
    root.append(&header);

    let navigation = gtk::ListBox::new();
    navigation.set_selection_mode(gtk::SelectionMode::Single);
    navigation.add_css_class("settings-navigation");
    let navigation_shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    navigation_shell.set_width_request(210);
    navigation_shell.add_css_class("settings-sidebar");
    let navigation_title = gtk::Label::new(Some(&game.title.to_uppercase()));
    navigation_title.set_xalign(0.0);
    navigation_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    navigation_title.add_css_class("settings-sidebar-title");
    navigation_shell.append(&navigation_title);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let executable = adw::EntryRow::new();
    executable.set_title("Game executable");
    executable.set_tooltip_text(Some("Path relative to the game's installation directory"));
    let executable_text = installed
        .as_ref()
        .and_then(|value| {
            value.primary_executable.as_ref().and_then(|path| {
                path.strip_prefix(&value.installation_directory)
                    .ok()
                    .unwrap_or(path)
                    .to_str()
            })
        })
        .unwrap_or_default();
    executable.set_text(executable_text);
    let browse_executable = gtk::Button::from_icon_name("document-open-symbolic");
    browse_executable.set_tooltip_text(Some("Choose an executable inside the game directory"));
    browse_executable.set_valign(gtk::Align::Center);
    browse_executable.add_css_class("flat");
    executable.add_suffix(&browse_executable);

    let launch_options = adw::EntryRow::new();
    launch_options.set_title("Launch options (optional)");
    launch_options.set_tooltip_text(Some(
        "Command-line arguments passed to the game; quotes are supported",
    ));
    launch_options.set_text(
        &installed
            .as_ref()
            .map(|value| shell_words::join(value.launch_arguments.iter().map(String::as_str)))
            .unwrap_or_default(),
    );
    let general_group = adw::PreferencesGroup::new();
    general_group.set_title("Launch");
    general_group.add(&executable);
    general_group.add(&launch_options);
    let general_page = adw::PreferencesPage::new();
    general_page.set_title("General");
    general_page.add(&general_group);
    general_page.add(&files::archive_deletion_group(
        &window,
        game.product_id,
        refresh_after_change.clone(),
    ));

    let save_status = gtk::Label::new(None);
    save_status.set_xalign(0.0);
    save_status.add_css_class("dim-label");
    save_status.set_margin_top(8);
    general_group.add(&save_status);

    if installed.is_none() {
        executable.set_sensitive(false);
        browse_executable.set_sensitive(false);
        launch_options.set_sensitive(false);
        save_status.set_label("Install this game before configuring its launcher");
    }

    let compatibility_page = adw::PreferencesPage::new();
    compatibility_page.set_title("Compatibility");
    let compatibility_group = adw::PreferencesGroup::new();
    compatibility_group.set_title("Windows compatibility");
    if let Some(compatibility) = installed
        .as_ref()
        .and_then(|game| game.compatibility.as_ref())
    {
        compatibility_group.add(&info_row("Backend", "UMU"));
        compatibility_group.add(&info_row("Profile", &compatibility.profile.game_id));
        compatibility_group.add(&info_row("Store", &compatibility.profile.store));
        compatibility_group.add(&info_row(
            "Prefix",
            &format!(".ludomere/compatibility/{}", compatibility.prefix_slug),
        ));
        compatibility_group.add(&info_row("Library drive", "L:"));
    } else if installed
        .as_ref()
        .is_some_and(|game| game.installer_operating_system.as_deref() == Some("linux"))
    {
        compatibility_group.add(&info_row(
            "Native Linux",
            "No compatibility environment is required.",
        ));
    } else {
        compatibility_group.add(&info_row(
            "UMU",
            "Install a Windows game to configure its compatibility environment.",
        ));
    }
    compatibility_page.add(&compatibility_group);
    compatibility_page.add(&proton_selection_group(&window, Some(game.product_id)).0);
    compatibility_page.add(&dll_overrides::group(game.product_id));

    let fixes_group = adw::PreferencesGroup::new();
    fixes_group.set_title("Compatibility fixes");
    fixes_group.set_description(Some(
        "Recommended fixes are preselected for known games. Changes apply the next time the game launches.",
    ));
    let recommended = crate::compatibility::recommended_fix_ids(game.product_id);
    let resetting = Rc::new(std::cell::Cell::new(false));
    let fixes_status = gtk::Label::builder().wrap(true).xalign(0.0).build();
    fixes_group.add(&fixes_status);
    let mut fix_rows = Vec::new();
    for fix in crate::compatibility::available_fixes() {
        let row = adw::SwitchRow::new();
        row.set_title(&fix.title);
        let is_recommended = recommended.contains(&fix.id);
        row.set_subtitle(&if is_recommended {
            format!("{} Recommended for this game.", fix.description)
        } else {
            fix.description.clone()
        });
        row.set_active(is_recommended);
        row.set_sensitive(false);
        let product_id = game.product_id;
        let fix_id = fix.id.clone();
        let resetting = resetting.clone();
        let refresh_after_change = refresh_after_change.clone();
        let status = fixes_status.clone();
        let group = fixes_group.downgrade();
        row.connect_active_notify(move |row| {
            if resetting.get() {
                return;
            }
            let Some(group) = group.upgrade() else {
                return;
            };
            let active = row.is_active();
            let fix_id = fix_id.clone();
            let row = row.clone();
            let resetting = resetting.clone();
            let refresh = refresh_after_change.clone();
            save_game_setting(
                group.upcast(),
                &status,
                session,
                move || {
                    StateStore::open()?.set_compatibility_fix_override(product_id, &fix_id, active)
                },
                move |saved| {
                    if saved {
                        refresh();
                    } else {
                        resetting.set(true);
                        row.set_active(!active);
                        resetting.set(false);
                    }
                },
            );
        });
        fixes_group.add(&row);
        fix_rows.push((fix.id.clone(), row));
    }

    let reset_row = adw::ActionRow::new();
    reset_row.set_title("Recommended settings");
    reset_row
        .set_subtitle("Reset the compatibility-fix switches to shipped recommendations. DLL overrides are unchanged.");
    let reset = gtk::Button::with_label("Reapply Recommended");
    reset.set_valign(gtk::Align::Center);
    reset.set_sensitive(false);
    reset_row.add_suffix(&reset);
    fixes_group.add(&reset_row);
    let product_id = game.product_id;
    reset.connect_clicked({
        let resetting = resetting.clone();
        let refresh_after_change = refresh_after_change.clone();
        let group = fixes_group.downgrade();
        let status = fixes_status.clone();
        let fix_rows = fix_rows.clone();
        move |_| {
            let Some(group) = group.upgrade() else {
                return;
            };
            let resetting = resetting.clone();
            let rows = fix_rows.clone();
            let refresh = refresh_after_change.clone();
            save_game_setting(
                group.upcast(),
                &status,
                session,
                move || StateStore::open()?.clear_compatibility_fix_overrides(product_id),
                move |saved| {
                    if saved {
                        resetting.set(true);
                        let recommended = crate::compatibility::recommended_fix_ids(product_id);
                        for (fix_id, row) in &rows {
                            row.set_active(recommended.contains(fix_id));
                        }
                        resetting.set(false);
                        refresh();
                    }
                },
            );
        }
    });
    load_compatibility_fix_preferences(
        &fixes_group,
        &fixes_status,
        fix_rows,
        &reset,
        resetting,
        game.product_id,
        session,
        installed.is_some(),
    );
    compatibility_page.add(&fixes_group);
    let updates_page = adw::PreferencesPage::new();
    updates_page.set_title("Updates");
    updates_page.add(&update_policies::game_group(model, game));

    let files_page = adw::PreferencesPage::new();
    files_page.set_title("Installed Files");
    let files_group = adw::PreferencesGroup::new();
    files_group.set_title("Local installation");
    if let Some(installed) = &installed {
        files_group.add(&info_row(
            "Installation directory",
            &installed.installation_directory.to_string_lossy(),
        ));
        files_group.add(&info_row(
            "Installed version",
            installed.installed_version.as_deref().unwrap_or("Unknown"),
        ));
        files_group.add(&info_row(
            "Installer platform",
            installed
                .installer_operating_system
                .as_deref()
                .unwrap_or("Unknown"),
        ));
        files_group.add(&info_row(
            "Installer language",
            installed.installer_language.as_deref().unwrap_or("Unknown"),
        ));
        let open = gtk::Button::with_label("Browse…");
        let open_row = adw::ActionRow::new();
        open_row.set_title("Browse installed files");
        open_row.add_suffix(&open);
        let path = installed.installation_directory.clone();
        let launch_parent = window.clone();
        open.connect_clicked(move |_| {
            super::widgets::file_open::open_directory(
                &path,
                &launch_parent,
                "installed-game directory",
            );
        });
        files_group.add(&open_row);
    } else {
        files_group.set_description(Some("This game is not currently installed."));
    }
    files_page.add(&files_group);

    let cloud_page = adw::PreferencesPage::new();
    cloud_page.set_title("Cloud Saves");
    let cloud_group = adw::PreferencesGroup::new();
    cloud_group.set_title("GOG Cloud Saves");
    let cloud_status = gtk::Label::new(None);
    cloud_status.set_xalign(0.0);
    cloud_status.set_wrap(true);
    if let Some(installed_game) = &installed {
        if installed_game.compatibility.is_some()
            && installed_game
                .installer_operating_system
                .as_deref()
                .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
        {
            let record = StateStore::open()
                .and_then(|store| store.cloud_save_record(installed_game.product_id))
                .unwrap_or(crate::state::CloudSaveRecord {
                    preference: crate::domain::CloudSavePreference::Undecided,
                    availability: crate::domain::CloudSaveAvailability::Unknown,
                    locations: Vec::new(),
                    metadata_build_id: None,
                    metadata_checked_at: None,
                    metadata_error: None,
                    last_successful_sync: None,
                    status: crate::domain::CloudSaveStatus::NeverSynced,
                    error: None,
                    conflicts: Vec::new(),
                });
            let enabled = adw::SwitchRow::new();
            enabled.set_title("Synchronize saves with GOG");
            enabled.set_subtitle("Runs before launch and after the monitored game process exits");
            enabled.set_active(record.preference == crate::domain::CloudSavePreference::Enabled);
            let supported = record.availability == crate::domain::CloudSaveAvailability::Supported;
            let management =
                cloud_management::cloud_management_group(&window, model, installed_game);
            management.set_visible(supported);
            cloud_page.add(&management);
            let locations_state = Rc::new(RefCell::new(record.locations.clone()));
            enabled.set_sensitive(supported);
            let product_id = installed_game.product_id;
            let reverting = Rc::new(std::cell::Cell::new(false));
            let status = cloud_status.clone();
            enabled.connect_active_notify(move |row| {
                if reverting.get() {
                    return;
                }
                let preference = if row.is_active() {
                    crate::domain::CloudSavePreference::Enabled
                } else {
                    crate::domain::CloudSavePreference::Disabled
                };
                let active = row.is_active();
                let row = row.clone();
                let reverting = reverting.clone();
                save_game_setting(
                    row.clone().upcast(),
                    &status,
                    session,
                    move || StateStore::open()?.set_cloud_save_preference(product_id, preference),
                    move |saved| {
                        if !saved {
                            reverting.set(true);
                            row.set_active(!active);
                            reverting.set(false);
                        }
                    },
                );
            });
            cloud_group.add(&enabled);
            cloud_group.add(&info_row(
                "Last successful sync",
                &record
                    .last_successful_sync
                    .map(|time| {
                        chrono::DateTime::from_timestamp(time, 0)
                            .map(|value| value.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_else(|| "Never".into()),
            ));
            let inventory_row = adw::ActionRow::new();
            inventory_row.set_title("GOG Cloud storage");
            inventory_row.set_subtitle("Remote save files have not been checked");
            inventory_row.set_visible(supported);
            let check_inventory = gtk::Button::with_label("Check now");
            check_inventory.set_valign(gtk::Align::Start);
            check_inventory.set_margin_top(10);
            check_inventory.set_sensitive(supported);
            let inventory_game = installed_game.clone();
            let inventory_status = inventory_row.clone();
            let origin = cloud_session.clone();
            check_inventory.connect_clicked(move |button| {
                load_cloud_inventory(
                    inventory_game.clone(),
                    inventory_status.clone(),
                    button.clone(),
                    origin.clone(),
                );
            });
            inventory_row.add_suffix(&check_inventory);
            cloud_group.add(&inventory_row);
            let locations_row = adw::ActionRow::new();
            locations_row.set_title("Save locations");
            locations_row.set_subtitle(&cloud_location_summary(&record.locations));
            locations_row.set_visible(supported);
            let open_save_folder = gtk::Button::with_label("Open save folder");
            open_save_folder.set_valign(gtk::Align::Start);
            open_save_folder.set_margin_top(10);
            open_save_folder.set_sensitive(supported && !record.locations.is_empty());
            let save_parent = window.clone();
            let save_locations = locations_state.clone();
            open_save_folder.connect_clicked(move |_| {
                if let Some(location) = save_locations.borrow().first() {
                    super::widgets::file_open::open_directory(
                        &location.path,
                        &save_parent,
                        "cloud-save directory",
                    );
                }
            });
            locations_row.add_suffix(&open_save_folder);
            cloud_group.add(&locations_row);
            match record.availability {
                crate::domain::CloudSaveAvailability::Supported => {
                    cloud_status.set_label("GOG cloud saves are supported for this game.")
                }
                crate::domain::CloudSaveAvailability::Unsupported => {
                    cloud_status.set_label("GOG reports cloud saves are disabled for this game.")
                }
                crate::domain::CloudSaveAvailability::Unavailable => cloud_status.set_label(
                    record
                        .metadata_error
                        .as_deref()
                        .unwrap_or("GOG cloud-save metadata is unavailable for this game."),
                ),
                crate::domain::CloudSaveAvailability::Unknown => cloud_status.set_label(
                    record
                        .metadata_error
                        .as_deref()
                        .unwrap_or("Cloud-save support has not been checked yet."),
                ),
            }
            if supported && let Some(error) = &record.error {
                cloud_status.set_label(error);
                cloud_status.add_css_class("error");
            } else if supported && !record.conflicts.is_empty() {
                cloud_status.set_label(&format!(
                    "{} pending conflict(s) require a manual choice",
                    record.conflicts.len()
                ));
            }

            let sync_row = adw::ActionRow::new();
            sync_row.set_title("Synchronize saves");
            sync_row.set_subtitle("Compare local and cloud saves and prompt before conflicts");
            let sync_now = gtk::Button::with_label("Sync now");
            sync_now.set_valign(gtk::Align::Start);
            sync_now.set_margin_top(10);
            sync_now.set_sensitive(supported);
            let game = installed_game.clone();
            let locations = locations_state.clone();
            let status = cloud_status.clone();
            let origin = cloud_session.clone();
            sync_now.connect_clicked(move |button| {
                run_cloud_action(
                    button,
                    &status,
                    &origin,
                    crate::cloud_saves::CloudSyncRequest {
                        game: game.clone(),
                        locations: locations.borrow().clone(),
                        mode: crate::domain::CloudSyncMode::Normal,
                    },
                )
            });
            sync_row.add_suffix(&sync_now);

            let advanced = gtk::MenuButton::new();
            advanced.set_label("Advanced…");
            advanced.set_valign(gtk::Align::Start);
            advanced.set_margin_top(10);
            advanced.set_sensitive(supported);
            let popover = gtk::Popover::new();
            let advanced_content = gtk::Box::new(gtk::Orientation::Vertical, 10);
            advanced_content.set_margin_top(14);
            advanced_content.set_margin_bottom(14);
            advanced_content.set_margin_start(14);
            advanced_content.set_margin_end(14);
            advanced_content.set_width_request(340);
            let advanced_title = gtk::Label::new(Some("Force synchronization"));
            advanced_title.set_xalign(0.0);
            advanced_title.add_css_class("heading");
            advanced_content.append(&advanced_title);
            let warning = gtk::Label::new(Some(
                "Warning: force operations will overwrite save data and will most likely erase either local or cloud progress. Use them only if you know which copy must be kept.",
            ));
            warning.set_wrap(true);
            warning.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            warning.set_max_width_chars(44);
            warning.set_xalign(0.0);
            warning.add_css_class("error");
            advanced_content.append(&warning);
            let force_actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            for (index, (label, mode)) in [
                (
                    "Force download",
                    crate::domain::CloudSyncMode::ForceDownload,
                ),
                ("Force upload", crate::domain::CloudSyncMode::ForceUpload),
            ]
            .into_iter()
            .enumerate()
            {
                if index == 1 {
                    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    spacer.set_hexpand(true);
                    force_actions.append(&spacer);
                }
                let button = gtk::Button::with_label(label);
                button.add_css_class("destructive-action");
                let game = installed_game.clone();
                let locations = locations_state.clone();
                let status = cloud_status.clone();
                let parent = window.clone();
                let popover = popover.clone();
                let origin = cloud_session.clone();
                button.connect_clicked(move |button| {
                    popover.popdown();
                    confirm_force_cloud_action(
                        &parent,
                        button,
                        &status,
                        &origin,
                        crate::cloud_saves::CloudSyncRequest {
                            game: game.clone(),
                            locations: locations.borrow().clone(),
                            mode,
                        },
                    );
                });
                force_actions.append(&button);
            }
            advanced_content.append(&force_actions);
            popover.set_child(Some(&advanced_content));
            advanced.set_popover(Some(&popover));
            sync_row.add_suffix(&advanced);
            cloud_group.add(&sync_row);

            let backup_row = adw::ActionRow::new();
            backup_row.set_title("Local backups");
            backup_row.set_subtitle("Copies made before cloud downloads overwrite local files");
            let backup = gtk::Button::with_label("Open backup folder");
            backup.set_valign(gtk::Align::Start);
            backup.set_margin_top(10);
            backup.set_sensitive(supported);
            let backup_path = crate::cloud_saves::sync::backup_directory(product_id);
            let backup_parent = window.clone();
            backup.connect_clicked(move |_| {
                std::fs::create_dir_all(&backup_path).ok();
                super::widgets::file_open::open_directory(
                    &backup_path,
                    &backup_parent,
                    "cloud-save backup directory",
                );
            });
            backup_row.add_suffix(&backup);
            cloud_group.add(&backup_row);

            let override_row = adw::ActionRow::new();
            override_row.set_title("Override save directory");
            override_row.set_subtitle("Use only when GOG's configured location cannot be resolved");
            let choose = gtk::Button::with_label("Choose…");
            choose.set_valign(gtk::Align::Start);
            choose.set_margin_top(10);
            choose.set_sensitive(
                supported || record.availability == crate::domain::CloudSaveAvailability::Unknown,
            );
            let choose_parent = window.clone();
            let override_status = cloud_status.clone();
            let override_locations = locations_state.clone();
            let override_locations_row = locations_row.clone();
            let override_open_folder = open_save_folder.clone();
            let override_model = model.clone();
            choose.connect_clicked(move |_| {
                let picker = gtk::FileDialog::builder()
                    .title("Choose save directory")
                    .modal(true)
                    .build();
                let override_status = override_status.clone();
                let override_locations = override_locations.clone();
                let override_locations_row = override_locations_row.clone();
                let override_open_folder = override_open_folder.clone();
                let override_model = override_model.clone();
                let epoch = override_model.borrow().account_epoch;
                let parent = choose_parent.clone();
                picker.select_folder(
                    Some(&choose_parent),
                    gio::Cancellable::NONE,
                    move |result| {
                        if !parent.is_visible()
                            || override_model.borrow().account_epoch != epoch
                            || override_model.borrow().logout_pending
                        {
                            return;
                        }
                        let file = match result {
                            Ok(file) => file,
                            Err(error)
                                if error.matches(gtk::DialogError::Dismissed)
                                    || error.matches(gtk::DialogError::Cancelled) =>
                            {
                                return;
                            }
                            Err(error) => {
                                override_status.set_label(&format!(
                                    "Could not choose a save directory: {error}"
                                ));
                                return;
                            }
                        };
                        let Some(path) = file.path() else {
                            override_status
                                .set_label("Choose a local directory for your save files.");
                            return;
                        };
                        let location = crate::domain::CloudSaveLocation {
                            name: "override".into(),
                            path,
                            remote_namespace: "override".into(),
                            user_override: true,
                        };
                        match StateStore::open().and_then(|store| {
                            store.set_cloud_save_locations(
                                product_id,
                                std::slice::from_ref(&location),
                            )
                        }) {
                            Ok(()) => {
                                *override_locations.borrow_mut() = vec![location];
                                override_locations_row.set_subtitle(&cloud_location_summary(
                                    &override_locations.borrow(),
                                ));
                                override_locations_row.set_visible(true);
                                override_open_folder.set_sensitive(true);
                                override_status.set_label("Override save directory updated");
                            }
                            Err(error) => override_status
                                .set_label(&format!("Could not save override: {error}")),
                        }
                    },
                );
            });
            override_row.add_suffix(&choose);
            cloud_group.add(&override_row);

            if record.availability == crate::domain::CloudSaveAvailability::Unknown {
                let retry = gtk::Button::with_label("Retry metadata discovery");
                retry.set_valign(gtk::Align::Start);
                retry.set_margin_top(10);
                let game = installed_game.clone();
                let locations = locations_state.clone();
                let status = cloud_status.clone();
                let locations_row = locations_row.clone();
                let open_save_folder = open_save_folder.clone();
                let inventory_row = inventory_row.clone();
                let check_inventory = check_inventory.clone();
                let enabled = enabled.clone();
                let sync_now = sync_now.clone();
                let advanced = advanced.clone();
                let backup = backup.clone();
                let choose = choose.clone();
                let management = management.clone();
                let model = model.clone();
                let epoch = model.borrow().account_epoch;
                retry.connect_clicked(move |button| {
                    if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                        return;
                    }
                    button.set_sensitive(false);
                    status.remove_css_class("error");
                    status.set_label("Checking GOG cloud-save support…");
                    let (sender, receiver) = mpsc::channel();
                    let game = game.clone();
                    let stored_locations = locations.borrow().clone();
                    std::thread::spawn(move || {
                        let result =
                            crate::cloud_saves::discover_and_store(&game, &stored_locations)
                                .map_err(|error| format!("{error:#}"));
                        sender.send(result).ok();
                    });
                    let button = button.clone();
                    let status = status.clone();
                    let locations = locations.clone();
                    let locations_row = locations_row.clone();
                    let open_save_folder = open_save_folder.clone();
                    let inventory_row = inventory_row.clone();
                    let check_inventory = check_inventory.clone();
                    let enabled = enabled.clone();
                    let sync_now = sync_now.clone();
                    let advanced = advanced.clone();
                    let backup = backup.clone();
                    let choose = choose.clone();
                    let management = management.clone();
                    let model = model.clone();
                    glib::timeout_add_local(Duration::from_millis(100), move || {
                        if management.root().is_none()
                            || model.borrow().account_epoch != epoch
                            || model.borrow().logout_pending
                        {
                            return glib::ControlFlow::Break;
                        }
                        match receiver.try_recv() {
                            Ok(Ok(discovery)) => {
                                let supported = discovery.availability
                                    == crate::domain::CloudSaveAvailability::Supported;
                                management.set_visible(supported);
                                *locations.borrow_mut() = discovery.locations.clone();
                                locations_row
                                    .set_subtitle(&cloud_location_summary(&discovery.locations));
                                locations_row.set_visible(supported);
                                open_save_folder
                                    .set_sensitive(supported && !discovery.locations.is_empty());
                                inventory_row.set_visible(supported);
                                check_inventory.set_sensitive(supported);
                                if supported {
                                    inventory_row
                                        .set_subtitle("Remote save files have not been checked");
                                }
                                enabled.set_sensitive(supported);
                                sync_now.set_sensitive(supported);
                                advanced.set_sensitive(supported);
                                backup.set_sensitive(supported);
                                choose.set_sensitive(
                                    supported
                                        || discovery.availability
                                            == crate::domain::CloudSaveAvailability::Unknown,
                                );
                                status.set_label(match discovery.availability {
                                    crate::domain::CloudSaveAvailability::Supported => {
                                        "GOG cloud saves are supported for this game."
                                    }
                                    crate::domain::CloudSaveAvailability::Unsupported => {
                                        "GOG reports cloud saves are disabled for this game."
                                    }
                                    crate::domain::CloudSaveAvailability::Unavailable => {
                                        discovery.reason.as_deref().unwrap_or(
                                            "GOG cloud-save metadata is unavailable for this game.",
                                        )
                                    }
                                    crate::domain::CloudSaveAvailability::Unknown => {
                                        discovery.reason.as_deref().unwrap_or(
                                            "Cloud-save discovery failed; retry is available.",
                                        )
                                    }
                                });
                                button.set_visible(
                                    discovery.availability
                                        == crate::domain::CloudSaveAvailability::Unknown,
                                );
                                button.set_sensitive(true);
                                glib::ControlFlow::Break
                            }
                            Ok(Err(error)) => {
                                status.add_css_class("error");
                                status.set_label(&error);
                                button.set_sensitive(true);
                                glib::ControlFlow::Break
                            }
                            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                status
                                    .set_label("Cloud-save discovery worker stopped unexpectedly");
                                button.set_sensitive(true);
                                glib::ControlFlow::Break
                            }
                        }
                    });
                });
                let retry_row = adw::ActionRow::new();
                retry_row.set_title("Cloud-save support");
                retry_row.set_subtitle("Check GOG metadata again");
                retry_row.add_suffix(&retry);
                cloud_group.add(&retry_row);
            }
        } else {
            cloud_group.set_description(Some("Cloud saves currently support Windows games installed into a managed UMU prefix only."));
        }
    } else {
        cloud_group.set_description(Some(
            "Install the Windows version to configure cloud saves.",
        ));
    }
    cloud_group.add(&cloud_status);
    cloud_page.add(&cloud_group);

    let installation_page = adw::PreferencesPage::new();
    installation_page.set_title("Installation");
    let installation_group = adw::PreferencesGroup::new();
    installation_group.set_title("Installed source");
    let installation_marker = installed.as_ref().and_then(|game| {
        crate::installation::load_installation_marker(&game.installation_directory)
            .ok()
            .flatten()
    });
    installation_group.add(&info_row(
        "Current installation",
        installed
            .as_ref()
            .and_then(|value| value.installed_version.as_deref())
            .unwrap_or("Not installed"),
    ));
    installation_group.add(&info_row(
        "Source",
        match installation_marker.as_ref().map(|marker| marker.source) {
            Some(crate::domain::InstallationSource::OfflineInstaller) => "Offline installer",
            Some(crate::domain::InstallationSource::GalaxyDepot) => "Galaxy depot",
            None if installed.is_some() => "Unknown",
            None => "Not installed",
        },
    ));
    let reinstall_row = adw::ActionRow::new();
    reinstall_row.set_title("Reinstall using another source");
    reinstall_row.set_subtitle(
        "Back up known saves, remove this installation, and install from another source",
    );
    let reinstall = gtk::Button::with_label("Choose Source…");
    reinstall.set_valign(gtk::Align::Center);
    reinstall.set_sensitive(installed.is_some());
    if let Some(installed_game) = installed.clone() {
        let parent = window.clone();
        let model = model.clone();
        let game = game.clone();
        reinstall.connect_clicked(move |_| {
            present_source_migration(&parent, &model, &game, &installed_game)
        });
    }
    reinstall_row.add_suffix(&reinstall);
    installation_group.add(&reinstall_row);
    installation_page.add(&installation_group);

    if let Some(installed_game) = &installed
        && let Some(marker) = installation_marker
        && marker.source == crate::domain::InstallationSource::GalaxyDepot
    {
        let branch_group = adw::PreferencesGroup::new();
        branch_group.set_title("Branches");
        let mut branches = game
            .galaxy_builds
            .iter()
            .filter(|build| {
                build.generation == 2
                    && build.currently_returned
                    && marker
                        .base
                        .operating_system
                        .as_deref()
                        .is_none_or(|os| build.operating_system.eq_ignore_ascii_case(os))
            })
            .map(|build| build.branch.clone())
            .collect::<Vec<_>>();
        branches.sort_by(|left, right| match (left, right) {
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            _ => left.cmp(right),
        });
        branches.dedup();
        let labels = branches
            .iter()
            .map(|branch| branch.as_deref().unwrap_or("Master"))
            .collect::<Vec<_>>();
        let selector =
            gtk::DropDown::new(Some(gtk::StringList::new(&labels)), gtk::Expression::NONE);
        let current = marker
            .galaxy_depot
            .as_ref()
            .and_then(|provenance| provenance.branch.as_ref());
        selector.set_selected(
            branches
                .iter()
                .position(|branch| branch.as_ref() == current)
                .unwrap_or(0) as u32,
        );
        let branch_row = adw::ActionRow::new();
        branch_row.set_title("Branch");
        branch_row.add_suffix(&selector);
        branch_group.add(&branch_row);
        let password = adw::PasswordEntryRow::new();
        password.set_title("Protected branch password");
        password.set_show_apply_button(false);
        branch_group.add(&password);
        let status = gtk::Label::new(Some(
            "Valid protected-branch passwords are saved automatically.",
        ));
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        branch_group.add(&status);
        let actions = adw::ActionRow::new();
        actions.set_title("Apply branch");
        let forget = gtk::Button::with_label("Forget Password");
        let switch = gtk::Button::with_label("Switch");
        switch.add_css_class("suggested-action");
        actions.add_suffix(&forget);
        actions.add_suffix(&switch);
        branch_group.add(&actions);
        wire_branch_actions(
            model,
            game,
            installed_game,
            marker,
            branches,
            selector,
            password,
            status,
            switch,
            forget,
        );
        installation_page.add(&branch_group);
    }

    for (name, title, icon, page) in [
        (
            "cloud-saves",
            "Cloud Saves",
            "folder-remote-symbolic",
            cloud_page.upcast::<gtk::Widget>(),
        ),
        (
            "general",
            "General",
            "preferences-system-symbolic",
            general_page.upcast::<gtk::Widget>(),
        ),
        (
            "compatibility",
            "Compatibility",
            "applications-engineering-symbolic",
            compatibility_page.upcast::<gtk::Widget>(),
        ),
        (
            "updates",
            "Updates",
            "view-refresh-symbolic",
            updates_page.upcast::<gtk::Widget>(),
        ),
        (
            "files",
            "Installed Files",
            "folder-symbolic",
            files_page.upcast::<gtk::Widget>(),
        ),
        (
            "installation",
            "Installation",
            "drive-harddisk-symbolic",
            installation_page.upcast::<gtk::Widget>(),
        ),
    ] {
        let row = settings_navigation_row(name, title, icon);
        navigation.append(&row);
        stack.add_named(&page, Some(name));
    }
    navigation.connect_row_selected({
        let stack = stack.clone();
        move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(row.widget_name().as_str());
            }
        }
    });
    navigation.select_row(navigation.row_at_index(0).as_ref());
    navigation_shell.append(&navigation);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&navigation_shell);
    content.append(&stack);
    content.set_vexpand(true);
    root.append(&content);
    window.set_content(Some(&root));

    if let Some(installed) = installed.clone() {
        let window = window.clone();
        let executable = executable.clone();
        let install_directory = installed.installation_directory.clone();
        let save_status = save_status.clone();
        browse_executable.connect_clicked(move |_| {
            let picker = gtk::FileDialog::builder()
                .title("Choose game executable")
                .modal(true)
                .initial_folder(&gio::File::for_path(&install_directory))
                .build();
            let executable = executable.clone();
            let install_directory = install_directory.clone();
            let save_status = save_status.clone();
            picker.open(Some(&window), gio::Cancellable::NONE, move |result| {
                let file = match result {
                    Ok(file) => file,
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled)
                            || error.matches(gio::IOErrorEnum::Cancelled) =>
                    {
                        return;
                    }
                    Err(error) => {
                        save_status.set_label(&format!("Could not choose executable: {error}"));
                        return;
                    }
                };
                let Some(path) = file.path() else {
                    save_status.set_label("Choose an executable on the local filesystem");
                    return;
                };
                match path.strip_prefix(&install_directory) {
                    Ok(relative) if relative.components().next().is_some() => {
                        executable.set_text(&relative.to_string_lossy());
                        executable.emit_by_name::<()>("entry-activated", &[]);
                    }
                    _ => save_status.set_label(
                        "Choose an executable inside this game's installation directory",
                    ),
                }
            });
        });
    }

    if let Some(installed) = installed {
        let installed = Rc::new(RefCell::new(installed));
        let save_state = Rc::new(LaunchSaveState::default());
        for entry in [&executable, &launch_options] {
            let executable = executable.downgrade();
            let launch_options = launch_options.downgrade();
            let installed = installed.clone();
            let save_status = save_status.clone();
            let refresh_after_change = refresh_after_change.clone();
            let save_state = save_state.clone();
            connect_launch_save(
                entry,
                Rc::new(move || {
                    let (Some(executable), Some(launch_options)) =
                        (executable.upgrade(), launch_options.upgrade())
                    else {
                        return;
                    };
                    persist_launch_settings(
                        &executable,
                        &launch_options,
                        &installed,
                        &save_status,
                        &refresh_after_change,
                        session,
                        &save_state,
                    );
                }),
            );
        }
    }

    window.present();
}

#[allow(clippy::too_many_arguments)]
fn wire_branch_actions(
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
    marker: crate::installation::InstallationMarker,
    branches: Vec<Option<String>>,
    selector: gtk::DropDown,
    password: adw::PasswordEntryRow,
    status: gtk::Label,
    switch: gtk::Button,
    forget: gtk::Button,
) {
    let user_id = model
        .borrow()
        .account_profile
        .as_ref()
        .map(|profile| profile.user_id.clone())
        .unwrap_or_default();
    let product_id = game.product_id;
    {
        let branches = branches.clone();
        let selector = selector.clone();
        let status = status.clone();
        let user_id = user_id.clone();
        forget.connect_clicked(move |_| {
            let Some(Some(branch)) = branches.get(selector.selected() as usize) else {
                status.set_label("Master does not use a saved branch password.");
                return;
            };
            match StateStore::open().and_then(|store| {
                crate::gog::depot_service::forget_one(&store, &user_id, product_id, branch)
            }) {
                Ok(()) => status.set_label("Saved branch password forgotten."),
                Err(error) => status.set_label(&format!("Could not forget password: {error}")),
            }
        });
    }
    let library_id = installed.library_id.clone();
    let library_root = installed
        .installation_directory
        .parent()
        .map(std::path::Path::to_path_buf);
    let slug = marker.slug.clone();
    let selected_dlc = marker
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    let language = marker
        .galaxy_depot
        .as_ref()
        .and_then(|depot| depot.language.clone())
        .unwrap_or_else(|| "en".into());
    let bitness = marker
        .galaxy_depot
        .as_ref()
        .and_then(|depot| depot.architecture.clone());
    switch.connect_clicked(move |button| {
        let Some(library_root) = library_root.clone() else {
            status.set_label("Installation has no library root.");
            return;
        };
        let selected_branch = branches
            .get(selector.selected() as usize)
            .cloned()
            .flatten();
        if marker
            .galaxy_depot
            .as_ref()
            .is_some_and(|depot| depot.branch == selected_branch)
        {
            status.set_label("This branch is already installed.");
            return;
        }
        button.set_sensitive(false);
        status.remove_css_class("error");
        status.set_label("Authenticating and preparing branch switch…");
        let supplied = (!password.text().is_empty())
            .then(|| crate::gog::depot_service::BranchPassword::new(password.text().to_string()));
        let request = crate::gog::depot_service::BuildRequest {
            user_id: user_id.clone(),
            product_id,
            platform: "windows".into(),
            generation: 2,
            branch: selected_branch.clone(),
            supplied_password: supplied,
        };
        let marker = marker.clone();
        let library_id = library_id.clone();
        let slug = slug.clone();
        let language = language.clone();
        let bitness = bitness.clone();
        let selected_dlc = selected_dlc.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                let store = StateStore::open()?;
                let client = reqwest::blocking::Client::new();
                let builds = crate::gog::depot_service::list_builds(&store, &client, &request)?;
                let build = crate::gog::depot_service::resolve_operation_build(
                    &builds,
                    &marker,
                    crate::domain::DepotOperationKind::BranchSwitch,
                    selected_branch.as_deref(),
                )?
                .clone();
                crate::gog::depot_service::start_operation(
                    &store,
                    &client,
                    crate::gog::depot_service::PrepareOperationRequest {
                        build,
                        selection: crate::gog::depot_acquisition::Selection {
                            language,
                            bitness,
                            owned_dlc: selected_dlc.clone(),
                            selected_dlc,
                        },
                        operation_id: format!(
                            "{}-{}",
                            product_id,
                            chrono::Utc::now().timestamp_millis()
                        ),
                        kind: crate::domain::DepotOperationKind::BranchSwitch,
                        library_id,
                        library_root,
                        slug,
                    },
                )
            })();
            let _ = sender.send(result);
        });
        let status = status.clone();
        let button = button.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    status.set_label("Branch switch started.");
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    status.add_css_class("error");
                    status.set_label(&format!("Could not switch branch: {error}"));
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.add_css_class("error");
                    status
                        .set_label("Branch preparation stopped unexpectedly. Try switching again.");
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            }
        });
    });
}

fn present_source_migration(
    parent: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
) {
    if crate::installation::installation_operation_snapshot(game.product_id).is_some()
        || crate::installation::depot_operation_snapshot_for_product(game.product_id).is_some_and(
            |snapshot| {
                !matches!(
                    snapshot.state.as_str(),
                    "complete" | "failed" | "cancelled" | "abandoned"
                )
            },
        )
    {
        let alert = adw::AlertDialog::builder()
            .heading("Installation operation already active")
            .body("Wait for the current installation operation to finish before changing sources.")
            .build();
        alert.add_response("close", "Close");
        alert.present(Some(parent));
        return;
    }
    let view = MigrationView::new();
    view.dialog.present(Some(parent));
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let game = game.clone();
    let installed = installed.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn({
        let game = game.clone();
        let installed = installed.clone();
        move || {
            let _ = sender.send(prepare_migration_choices(&game, &installed));
        }
    });
    let parent = parent.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if view.closed.get() {
            return glib::ControlFlow::Break;
        }
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            view.stop("Account changed. Close this dialog and reopen game properties.");
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!("Source inspection stopped unexpectedly.")),
        };
        view.progress.set_visible(false);
        match result {
            Ok(prepared) => wire_migration_choices(&parent, &model, &game, &installed, &view, prepared, epoch, session),
            Err(error) => view.stop(&format!("Could not inspect installation sources: {error:#}. Close and choose a source again.")),
        }
        glib::ControlFlow::Break
    });
}

#[derive(Clone)]
struct MigrationView {
    dialog: adw::Dialog,
    selector: gtk::DropDown,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    proceed: gtk::Button,
    close: gtk::Button,
    closed: Rc<std::cell::Cell<bool>>,
    started: Rc<std::cell::Cell<bool>>,
}

impl MigrationView {
    fn new() -> Self {
        let dialog = adw::Dialog::builder().content_width(560).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&adw::HeaderBar::new());
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_start(20);
        body.set_margin_end(20);
        body.set_margin_bottom(20);
        let heading = gtk::Label::new(Some("Reinstall using another source"));
        heading.add_css_class("title-2");
        heading.set_xalign(0.0);
        body.append(&heading);
        let selector = gtk::DropDown::new(None::<gtk::StringList>, gtk::Expression::NONE);
        selector.set_sensitive(false);
        body.append(&selector);
        let status = gtk::Label::new(Some(
            "Inspecting installation sources and saved-game locations…",
        ));
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_selectable(true);
        body.append(&status);
        let progress = gtk::ProgressBar::new();
        body.append(&progress);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        let close = gtk::Button::with_label("Cancel");
        let proceed = gtk::Button::with_label("Continue");
        proceed.add_css_class("destructive-action");
        proceed.set_sensitive(false);
        actions.append(&close);
        actions.append(&proceed);
        body.append(&actions);
        root.append(&body);
        dialog.set_child(Some(&root));
        close.connect_clicked({
            let dialog = dialog.clone();
            move |_| {
                dialog.close();
            }
        });
        let closed = Rc::new(std::cell::Cell::new(false));
        dialog.connect_closed({
            let closed = closed.clone();
            move |_| closed.set(true)
        });
        let weak = progress.downgrade();
        glib::timeout_add_local(Duration::from_millis(100), {
            let closed = closed.clone();
            move || {
                let Some(progress) = weak.upgrade().filter(|_| !closed.get()) else {
                    return glib::ControlFlow::Break;
                };
                if progress.is_visible() {
                    progress.pulse();
                }
                glib::ControlFlow::Continue
            }
        });
        Self {
            dialog,
            selector,
            status,
            progress,
            proceed,
            close,
            closed,
            started: Rc::new(std::cell::Cell::new(false)),
        }
    }

    fn stop(&self, message: &str) {
        self.progress.set_visible(false);
        self.selector.set_sensitive(false);
        self.proceed.set_sensitive(false);
        self.close.set_label("Close");
        self.status
            .set_label(super::notifications::failure_message("", message).trim_start());
    }
}

struct MigrationChoices {
    config: Config,
    marker: crate::installation::InstallationMarker,
    candidates: crate::installation::InstallerCandidates,
    choices: Vec<crate::installation::FreshInstallSource>,
    current: Option<crate::config::PreferredInstallationSource>,
    locations: Vec<crate::domain::CloudSaveLocation>,
    galaxy_preflight: Result<(), String>,
}

fn prepare_migration_choices(
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
) -> anyhow::Result<MigrationChoices> {
    let config = crate::storage::read_config()?;
    let store = StateStore::open()?;
    let marker = crate::installation::load_installation_marker(&installed.installation_directory)?
        .ok_or_else(|| anyhow::anyhow!("The installation record is missing. Use Repair or Review File Reset from the game's Manage menu."))?;
    let galaxy_preflight = super::download_chooser::cached_galaxy_available(game, &config);
    let candidates = crate::installation::detect_installer_candidates(
        game.product_id,
        &store.load_all_download_revisions(game.product_id)?,
        &store.managed_files()?,
        &config,
    );
    let galaxy_available = super::download_chooser::newest_master_windows_build(game).is_some()
        && galaxy_preflight.is_ok();
    let current = match marker.source {
        crate::domain::InstallationSource::GalaxyDepot => {
            Some(crate::config::PreferredInstallationSource::WindowsGalaxy)
        }
        crate::domain::InstallationSource::OfflineInstaller
            if marker
                .base
                .operating_system
                .as_deref()
                .is_some_and(|os| os.eq_ignore_ascii_case("linux")) =>
        {
            Some(crate::config::PreferredInstallationSource::LinuxOffline)
        }
        crate::domain::InstallationSource::OfflineInstaller => {
            Some(crate::config::PreferredInstallationSource::WindowsOffline)
        }
    };
    let choices = crate::installation::rank_fresh_install_sources(
        &config,
        &candidates.usable,
        galaxy_available,
    )
    .into_iter()
    .filter(|choice| match choice {
        crate::installation::FreshInstallSource::GalaxyWindows => {
            current != Some(crate::config::PreferredInstallationSource::WindowsGalaxy)
        }
        crate::installation::FreshInstallSource::OfflineInstaller(index) => {
            let source = if candidates.usable[*index].method
                == crate::installation::InstallationMethod::NativeLinux
            {
                crate::config::PreferredInstallationSource::LinuxOffline
            } else {
                crate::config::PreferredInstallationSource::WindowsOffline
            };
            current != Some(source)
        }
    })
    .collect::<Vec<_>>();
    let locations = store.cloud_save_record(game.product_id)?.locations;
    Ok(MigrationChoices {
        config,
        marker,
        candidates,
        choices,
        current,
        locations,
        galaxy_preflight,
    })
}

#[allow(clippy::too_many_arguments)]
fn wire_migration_choices(
    parent: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
    view: &MigrationView,
    prepared: MigrationChoices,
    epoch: u64,
    session: u64,
) {
    let MigrationChoices {
        config,
        marker,
        candidates,
        choices,
        current,
        locations,
        galaxy_preflight,
    } = prepared;
    if choices.is_empty() {
        view.stop("No alternate installation source is available. Download an alternate installer or refresh Galaxy metadata, then choose a source again.");
        return;
    }
    let labels = choices
        .iter()
        .map(|choice| match choice {
            crate::installation::FreshInstallSource::GalaxyWindows => {
                "Windows · Galaxy build".to_owned()
            }
            crate::installation::FreshInstallSource::OfflineInstaller(index) => {
                let candidate = &candidates.usable[*index];
                format!(
                    "{} · Offline installer · {}",
                    if candidate.method == crate::installation::InstallationMethod::NativeLinux {
                        "Linux"
                    } else {
                        "Windows"
                    },
                    candidate.version.as_deref().unwrap_or("Unknown version")
                )
            }
        })
        .collect::<Vec<_>>();
    let selector = view.selector.clone();
    selector.set_model(Some(&gtk::StringList::new(
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    selector.set_sensitive(true);
    view.proceed.set_sensitive(true);
    let status_text = galaxy_preflight
        .as_ref()
        .err()
        .map(|error| format!("Galaxy build unavailable: {error}"))
        .unwrap_or_else(|| {
            "The current installation will be removed only after known saves are safely backed up."
                .into()
        });
    view.status.set_label(&status_text);
    let game = game.clone();
    let installed = installed.clone();
    let choice_parent = parent.clone();
    let view = view.clone();
    let proceed = view.proceed.clone();
    let model = model.clone();
    let windows_product = {
        let selector = selector.clone();
        let choices = choices.clone();
        let candidates = candidates.usable.clone();
        let product_id = game.product_id;
        move || {
            (current == Some(crate::config::PreferredInstallationSource::WindowsOffline)
                || choices
                    .get(selector.selected() as usize)
                    .is_some_and(|choice| match choice {
                        crate::installation::FreshInstallSource::GalaxyWindows => true,
                        crate::installation::FreshInstallSource::OfflineInstaller(index) => {
                            candidates.get(*index).is_some_and(|candidate| {
                                candidate.operating_system.as_deref() != Some("linux")
                            })
                        }
                    }))
            .then_some(product_id)
        }
    };
    connect_windows_action(&proceed, parent, false, windows_product, move |_| {
        if view.closed.get() || view.started.get() {
            return;
        }
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            view.stop("Account changed. Close this dialog and reopen game properties.");
            return;
        }
        let Some(choice) = choices.get(selector.selected() as usize).copied() else {
            return;
        };
        let mut locations = locations.clone();
        let target_os = match choice {
            crate::installation::FreshInstallSource::GalaxyWindows => Some("windows"),
            crate::installation::FreshInstallSource::OfflineInstaller(index) => candidates
                .usable
                .get(index)
                .and_then(|candidate| candidate.operating_system.as_deref()),
        };
        if target_os.is_none_or(|target| {
            marker
                .base
                .operating_system
                .as_deref()
                .is_none_or(|current| !current.eq_ignore_ascii_case(target))
        }) {
            locations.clear();
        }
        if locations.is_empty() {
            let warning = adw::AlertDialog::builder()
                .heading("Saved-game locations are unknown")
                .body("Ludomere cannot back up this game's saves automatically. Continuing will remove the current payload and UMU prefix and may permanently delete saved games. Back them up manually before continuing.")
                .build();
            warning.add_responses(&[
                ("cancel", "Cancel"),
                ("continue", "Continue Without Save Backup"),
            ]);
            warning.set_response_appearance("continue", adw::ResponseAppearance::Destructive);
            warning.set_default_response(Some("cancel"));
            warning.set_close_response("cancel");
            let view = view.clone();
            let model = model.clone();
            let config = config.clone();
            let candidates = candidates.usable.clone();
            let game = game.clone();
            let installed = installed.clone();
            warning.choose(
                Some(&choice_parent),
                gio::Cancellable::NONE,
                move |response| {
                    if view.closed.get() || view.started.get() {
                        return;
                    }
                    if model.borrow().account_epoch != epoch
                        || model.borrow().logout_pending
                        || online::account_session() != session
                    {
                        view.stop("Account changed. Close this dialog and reopen game properties.");
                        return;
                    }
                    if response == "continue" {
                        launch_source_migration(
                            &view,
                            &model,
                            session,
                            &config,
                            &game,
                            &installed,
                            &candidates,
                            choice,
                            Vec::new(),
                        );
                    }
                },
            );
        } else {
            launch_source_migration(
                &view,
                &model,
                session,
                &config,
                &game,
                &installed,
                &candidates.usable,
                choice,
                locations,
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn launch_source_migration(
    view: &MigrationView,
    model: &Rc<RefCell<AppModel>>,
    session: u64,
    config: &crate::config::Config,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
    candidates: &[crate::installation::InstallerCandidate],
    choice: crate::installation::FreshInstallSource,
    saves: Vec<crate::domain::CloudSaveLocation>,
) {
    if view.closed.get() || view.started.replace(true) {
        return;
    }
    view.selector.set_sensitive(false);
    view.proceed.set_sensitive(false);
    view.close.set_label("Close");
    view.close.set_tooltip_text(Some(
        "Reinstallation continues if you close this progress view.",
    ));
    view.status.set_label("Preparing replacement installation…");
    view.progress.set_visible(true);
    let config = config.clone();
    let game = game.clone();
    let installed = installed.clone();
    let candidates = candidates.to_vec();
    let (sender, receiver) = mpsc::channel();
    let (stages, stage_receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                online::account_session() == session,
                "Account changed; reopen game properties."
            );
            let _activity =
                crate::profile_reset::begin_activity("preparing installation migration")?;
            let Some(library) = config.game_libraries.iter().find(|library| {
                library.id == installed.library_id
                    || installed.installation_directory.parent() == Some(library.path.as_path())
            }) else {
                anyhow::bail!("The installed game's library is unavailable.");
            };
            let operation_id = format!(
                "{}-{}",
                game.product_id,
                chrono::Utc::now().timestamp_millis()
            );
            let target = match choice {
                crate::installation::FreshInstallSource::OfflineInstaller(index) => {
                    let Some(candidate) = candidates.get(index) else {
                        anyhow::bail!(
                            "The selected installer is no longer available. Reopen source selection."
                        );
                    };
                    let now = chrono::Utc::now().timestamp();
                    let plan = crate::domain::InstalledGame {
                        product_id: game.product_id,
                        library_id: library.id.clone(),
                        installed_version: candidate.version.clone(),
                        installation_directory: library.path.join(&game.slug),
                        installer_revision_id: candidate.revision_id,
                        installer_job_id: None,
                        installer_files: candidate.paths.clone(),
                        installer_complete: candidate.complete,
                        installer_operating_system: candidate.operating_system.clone(),
                        installer_language: candidate.language.clone(),
                        compatibility: None,
                        primary_executable: None,
                        launch_arguments: Vec::new(),
                        state: crate::domain::InstallationState::Pending,
                        error: None,
                        installed_at: None,
                        verified_at: None,
                        last_played_at: installed.last_played_at,
                        playtime_seconds: installed.playtime_seconds,
                        created_at: installed.created_at,
                        updated_at: now,
                    };
                    let additional_installers = game
                        .dlcs
                        .iter()
                        .filter(|dlc| dlc.owned)
                        .filter_map(|dlc| {
                            let store = StateStore::open().ok()?;
                            let detected = crate::installation::detect_installer_candidates(
                                dlc.product_id,
                                &store.load_all_download_revisions(dlc.product_id).ok()?,
                                &store.managed_files().ok()?,
                                &config,
                            );
                            let installer = detected.usable.into_iter().find(|installer| {
                                installer.method == candidate.method
                                    && installer.version == candidate.version
                                    && installer.complete
                            })?;
                            Some(crate::installation::AdditionalInstaller {
                                product_id: dlc.product_id,
                                revision_id: installer.revision_id,
                                version: installer.version,
                                title: dlc.title.clone(),
                                files: installer.paths,
                            })
                        })
                        .collect();
                    crate::installation::source_migration::MigrationTarget::Offline {
                        game: plan,
                        additional_installers,
                        interactive_prompts: false,
                    }
                }
                crate::installation::FreshInstallSource::GalaxyWindows => {
                    let Some(build) = game
                        .galaxy_builds
                        .iter()
                        .filter(|build| {
                            build.generation == 2
                                && build.currently_returned
                                && build.branch.is_none()
                        })
                        .max_by_key(|build| build.published_at)
                        .cloned()
                    else {
                        anyhow::bail!(
                            "No current Galaxy build is available. Refresh metadata and choose a source again."
                        );
                    };
                    let owned_dlc = game
                        .dlcs
                        .iter()
                        .filter(|dlc| dlc.owned)
                        .map(|dlc| dlc.product_id)
                        .collect::<BTreeSet<_>>();
                    let language = game
                        .metadata
                        .localizations
                        .iter()
                        .find(|value| value.language_code.starts_with("en"))
                        .map(|value| value.language_code.clone())
                        .unwrap_or_else(|| "en".into());
                    crate::installation::source_migration::MigrationTarget::Galaxy(
                        crate::gog::depot_service::PrepareOperationRequest {
                            build,
                            selection: crate::gog::depot_acquisition::Selection {
                                language,
                                bitness: Some("64".into()),
                                owned_dlc: owned_dlc.clone(),
                                selected_dlc: owned_dlc,
                            },
                            operation_id: format!("migration-{operation_id}"),
                            kind: crate::domain::DepotOperationKind::Install,
                            library_id: library.id.clone(),
                            library_root: library.path.clone(),
                            slug: game.slug.clone(),
                        },
                    )
                }
            };
            let save_locations = saves
                .into_iter()
                .map(
                    |location| crate::installation::source_migration::SaveLocation {
                        name: location.name,
                        path: location.path,
                    },
                )
                .collect::<Vec<_>>();
            let library_path = library.path.clone();
            let slug = game.slug.clone();
            let current = crate::storage::validate_path(
                &crate::storage::read_config()?,
                crate::config::LibraryKind::GameFiles,
                &installed.installation_directory,
            )?;
            anyhow::ensure!(
                current.path == library_path,
                "The selected Game Files library changed; reopen source selection."
            );
            anyhow::ensure!(
                online::account_session() == session,
                "Account changed; reopen game properties."
            );
            let _ = stages
                .try_send(crate::installation::source_migration::MigrationPhase::PreparingBackup);
            let mut journal = crate::installation::source_migration::begin_backup(
                &library_path,
                &operation_id,
                installed.product_id,
                &slug,
                &save_locations,
            )?;
            crate::installation::source_migration::configure(
                &library_path,
                &mut journal,
                installed,
                target,
            )?;
            anyhow::ensure!(
                online::account_session() == session,
                "Account changed; migration stopped before removing the existing installation."
            );
            let _ =
                stages.try_send(crate::installation::source_migration::MigrationPhase::BackedUp);
            let events =
                crate::installation::source_migration::start(library_path, journal, save_locations);
            loop {
                match events.recv()? {
                    crate::installation::source_migration::MigrationEvent::Complete => {
                        return Ok(());
                    }
                    crate::installation::source_migration::MigrationEvent::Failed {
                        message,
                        backup,
                    } => {
                        anyhow::bail!("{message}. Save backup retained at {}", backup.display())
                    }
                    crate::installation::source_migration::MigrationEvent::Phase(phase) => {
                        let _ = stages.try_send(phase);
                    }
                }
            }
        })();
        let _ = sender.send(result);
    });
    monitor_source_migration(view, model, session, receiver, stage_receiver);
}

fn monitor_source_migration(
    view: &MigrationView,
    model: &Rc<RefCell<AppModel>>,
    session: u64,
    receiver: mpsc::Receiver<anyhow::Result<()>>,
    stages: mpsc::Receiver<crate::installation::source_migration::MigrationPhase>,
) {
    let view = view.clone();
    let model = model.clone();
    let epoch = model.borrow().account_epoch;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if view.closed.get() {
            return glib::ControlFlow::Break;
        }
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            view.stop("Account changed. File work already started may still be finishing; reopen game properties to inspect its state.");
            return glib::ControlFlow::Break;
        }
        for phase in stages.try_iter().take(8) {
            use crate::installation::source_migration::MigrationPhase;
            view.status.set_label(match phase {
                MigrationPhase::PreparingBackup => "Backing up saved games…",
                MigrationPhase::BackedUp => {
                    "Save backup complete. Preparing and removing the previous installation…"
                }
                MigrationPhase::Uninstalled => {
                    "Previous installation removed. Downloading or installing the replacement…"
                }
                MigrationPhase::Installed | MigrationPhase::Restoring => {
                    "Replacement installed. Restoring saved games…"
                }
                MigrationPhase::Complete => "Finishing reinstallation…",
            });
        }
        match receiver.try_recv() {
            Ok(Ok(())) => {
                view.stop("Reinstallation complete.");
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                view.status.add_css_class("error");
                view.stop(&format!("Source migration stopped: {error:#}. Close this view and inspect the game's current state before retrying."));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                view.stop("Reinstallation stopped unexpectedly. Close this view and inspect the game's current state before retrying.");
                glib::ControlFlow::Break
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn load_compatibility_fix_preferences(
    group: &adw::PreferencesGroup,
    status: &gtk::Label,
    rows: Vec<(String, adw::SwitchRow)>,
    reset: &gtk::Button,
    resetting: Rc<std::cell::Cell<bool>>,
    product_id: i64,
    session: u64,
    installed: bool,
) {
    let retry = gtk::Button::with_label("Retry");
    retry.set_widget_name("compatibility-fixes-retry");
    retry.set_halign(gtk::Align::Start);
    group.add(&retry);
    let load: Rc<dyn Fn()> = Rc::new({
        let group = group.downgrade();
        let retry = retry.downgrade();
        let status = status.clone();
        let reset = reset.clone();
        let loading = Rc::new(std::cell::Cell::new(false));
        move || {
            let Some(retry) = retry.upgrade() else { return };
            if group.upgrade().is_none() || loading.get() {
                return;
            }
            for (_, row) in &rows {
                row.set_sensitive(false);
            }
            reset.set_sensitive(false);
            if online::account_session() != session {
                retry.set_sensitive(false);
                status
                    .set_label("The account changed. Reopen Properties before changing settings.");
                return;
            }
            loading.set(true);
            retry.set_visible(false);
            status.remove_css_class("error");
            status.set_label("Loading compatibility fixes…");
            let receiver = update_policies::policy_request(move || {
                let _activity =
                    crate::profile_reset::begin_activity("loading compatibility fixes")?;
                anyhow::ensure!(
                    online::account_session() == session,
                    "Account changed; reopen Properties."
                );
                StateStore::open()?.compatibility_fix_overrides(product_id)
            });
            let group = group.clone();
            let retry = retry.downgrade();
            let status = status.clone();
            let reset = reset.clone();
            let rows = rows.clone();
            let loading = loading.clone();
            let resetting = resetting.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let Some(retry) = retry.upgrade().filter(|_| group.upgrade().is_some()) else {
                    return glib::ControlFlow::Break;
                };
                if online::account_session() != session {
                    status.set_label(
                        "The account changed. Reopen Properties before changing settings.",
                    );
                    retry.set_sensitive(false);
                    loading.set(false);
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err(anyhow::anyhow!(
                        "Compatibility preference loading stopped unexpectedly."
                    )),
                };
                loading.set(false);
                match result {
                    Ok(overrides) => {
                        let recommended = crate::compatibility::recommended_fix_ids(product_id);
                        resetting.set(true);
                        for (id, row) in &rows {
                            row.set_active(
                                overrides
                                    .get(id)
                                    .copied()
                                    .unwrap_or(recommended.contains(id)),
                            );
                            row.set_sensitive(installed);
                        }
                        resetting.set(false);
                        reset.set_sensitive(installed);
                        status.set_label(if installed {
                            "Changes apply the next time the game launches."
                        } else {
                            "Install this game before changing compatibility fixes."
                        });
                    }
                    Err(error) => {
                        status.add_css_class("error");
                        status.set_label(&super::notifications::failure_message(
                            "Could not load compatibility fixes",
                            &format!("{error:#}"),
                        ));
                        retry.set_visible(true);
                    }
                }
                glib::ControlFlow::Break
            });
        }
    });
    retry.connect_clicked({
        let load = load.clone();
        move |_| load()
    });
    load();
}

#[derive(Clone)]
struct CloudActionSession {
    auth: u64,
    online: u64,
    epoch: u64,
    model: std::rc::Weak<RefCell<AppModel>>,
}

impl CloudActionSession {
    fn is_current(&self) -> bool {
        auth::session_is_current(self.auth)
            && self.online == online::account_session()
            && self.model.upgrade().is_some_and(|model| {
                model
                    .try_borrow()
                    .is_ok_and(|state| state.account_epoch == self.epoch && !state.logout_pending)
            })
    }

    fn check(&self, button: &gtk::Button, status: &gtk::Label) -> bool {
        let current = self.is_current();
        if !current {
            button.set_sensitive(false);
            status.set_label(
                "Account changed. Close and reopen game properties before synchronizing saves.",
            );
        }
        current
    }
}

fn run_cloud_action(
    button: &gtk::Button,
    status: &gtk::Label,
    origin: &CloudActionSession,
    request: crate::cloud_saves::CloudSyncRequest,
) {
    if !origin.check(button, status) {
        return;
    }
    button.set_sensitive(false);
    status.set_label("Synchronizing…");
    let (sender, receiver) = std::sync::mpsc::channel();
    let auth_session = origin.auth;
    let online_session = origin.online;
    std::thread::spawn(move || {
        let result = if online::account_session() == online_session {
            crate::cloud_saves::sync_for_session(request, auth_session)
                .map_err(|error| format!("{error:#}"))
        } else {
            Err("Account changed before synchronization started.".into())
        };
        let _ = sender.send(result);
    });
    monitor_cloud_action(button, status, origin, receiver);
}

fn monitor_cloud_action(
    button: &gtk::Button,
    status: &gtk::Label,
    origin: &CloudActionSession,
    receiver: mpsc::Receiver<Result<crate::domain::CloudSyncResult, String>>,
) {
    let button = button.downgrade();
    let status = status.downgrade();
    let origin = origin.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let (Some(button), Some(status)) = (button.upgrade(), status.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if !origin.check(&button, &status) {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(result)) => {
                button.set_sensitive(true);
                if result.conflicts.is_empty() {
                    status.set_label(&format!(
                        "Synchronized: {} uploaded, {} downloaded",
                        result.uploaded, result.downloaded
                    ));
                } else {
                    status.set_label(&format!(
                        "{} conflict(s) need a force upload or force download choice",
                        result.conflicts.len()
                    ));
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                button.set_sensitive(true);
                status.set_label(&super::notifications::failure_message(
                    "Cloud synchronization failed",
                    &error,
                ));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                button.set_sensitive(true);
                status.set_label("Cloud-save worker stopped unexpectedly");
                glib::ControlFlow::Break
            }
        }
    });
}

fn load_cloud_inventory(
    game: crate::domain::InstalledGame,
    row: adw::ActionRow,
    button: gtk::Button,
    origin: CloudActionSession,
) {
    if !origin.is_current() {
        button.set_sensitive(false);
        row.set_subtitle(
            "Account changed. Close and reopen game properties before checking cloud storage.",
        );
        return;
    }
    button.set_sensitive(false);
    row.set_subtitle("Checking remote save files…");
    let (sender, receiver) = std::sync::mpsc::channel();
    let auth_session = origin.auth;
    let online_session = origin.online;
    std::thread::spawn(move || {
        let result = if online::account_session() == online_session {
            crate::cloud_saves::inventory(&game, auth_session).map_err(|error| format!("{error:#}"))
        } else {
            Err("Account changed before checking cloud storage.".into())
        };
        let _ = sender.send(result);
    });
    monitor_cloud_inventory(&row, &button, origin, receiver);
}

fn monitor_cloud_inventory(
    row: &adw::ActionRow,
    button: &gtk::Button,
    origin: CloudActionSession,
    receiver: mpsc::Receiver<Result<crate::cloud_saves::CloudSaveInventory, String>>,
) {
    let row = row.downgrade();
    let button = button.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let (Some(row), Some(button)) = (row.upgrade(), button.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if !origin.is_current() {
            button.set_sensitive(false);
            row.set_subtitle(
                "Account changed. Close and reopen game properties before checking cloud storage.",
            );
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(inventory)) => {
                button.set_sensitive(true);
                let files = match inventory.file_count {
                    0 => "No remote save files".into(),
                    1 => "1 remote save file".into(),
                    count => format!("{count} remote save files"),
                };
                let modified = inventory
                    .latest_modified_at
                    .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                    .map(|time| format!("last modified {}", time.format("%Y-%m-%d %H:%M UTC")));
                let mut summary = format!(
                    "{files} · {}",
                    crate::domain::human_size(inventory.total_size)
                );
                if let Some(modified) = modified {
                    summary.push_str(" · ");
                    summary.push_str(&modified);
                }
                row.set_subtitle(&summary);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                button.set_sensitive(true);
                row.set_subtitle(&super::notifications::failure_message(
                    "Could not check cloud storage",
                    &error,
                ));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                button.set_sensitive(true);
                row.set_subtitle("Cloud-storage check stopped unexpectedly");
                glib::ControlFlow::Break
            }
        }
    });
}

fn confirm_force_cloud_action(
    parent: &adw::ApplicationWindow,
    button: &gtk::Button,
    status: &gtk::Label,
    origin: &CloudActionSession,
    request: crate::cloud_saves::CloudSyncRequest,
) {
    if !origin.check(button, status) {
        return;
    }
    let (heading, body, response) = match request.mode {
        crate::domain::CloudSyncMode::ForceDownload => (
            "Replace local saves?",
            "This replaces matching local save files with GOG Cloud copies. Local backups are created, but unsynchronized local progress may be lost.",
            "Force download",
        ),
        crate::domain::CloudSyncMode::ForceUpload => (
            "Replace cloud saves?",
            "This replaces matching GOG Cloud save files with local copies. Previous cloud versions may be permanently lost.",
            "Force upload",
        ),
        crate::domain::CloudSyncMode::Normal => return,
    };
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("force", response)]);
    dialog.set_response_appearance("force", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let button = button.clone();
    let status = status.clone();
    let origin = origin.clone();
    dialog.choose(Some(parent), gio::Cancellable::NONE, move |response| {
        if response == "force" {
            run_cloud_action(&button, &status, &origin, request);
        }
    });
}

fn cloud_location_summary(locations: &[crate::domain::CloudSaveLocation]) -> String {
    if locations.is_empty() {
        return "No resolved save locations".into();
    }
    locations
        .iter()
        .map(|location| format!("{} · {}", location.name, location.path.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn connect_launch_save(entry: &adw::EntryRow, save: Rc<dyn Fn()>) {
    let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
    entry.connect_changed({
        let pending = pending.clone();
        let save = save.clone();
        move |_| {
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let pending_done = pending.clone();
            let save = save.clone();
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(
                Duration::from_millis(400),
                move || {
                    pending_done.borrow_mut().take();
                    save();
                },
            ));
        }
    });
    let flush: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(source) = pending.borrow_mut().take() {
            source.remove();
        }
        save();
    });
    entry.connect_entry_activated({
        let flush = flush.clone();
        move |_| flush()
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let flush = flush.clone();
        move |_| flush()
    });
    entry.add_controller(focus);
    entry.connect_unmap(move |_| flush());
}

fn save_game_setting(
    controls: gtk::Widget,
    status: &gtk::Label,
    session: u64,
    operation: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    finish: impl Fn(bool) + 'static,
) {
    if online::account_session() != session {
        status.set_label("The account changed. Reopen Properties before changing settings.");
        controls.set_sensitive(false);
        return;
    }
    controls.set_sensitive(false);
    status.set_label("Saving settings…");
    let receiver = update_policies::policy_request(move || {
        let _activity = crate::profile_reset::begin_activity("saving game settings")?;
        online::with_account_session(session, operation)
    });
    let status = status.downgrade();
    let controls = controls.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let (Some(status), Some(controls)) = (status.upgrade(), controls.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if online::account_session() != session {
            controls.set_sensitive(false);
            status.set_label("The account changed. Reopen Properties before changing settings.");
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!("Saving stopped unexpectedly. Try again.")),
        };
        controls.set_sensitive(true);
        status.set_label(&match &result {
            Ok(()) => "Settings saved".to_owned(),
            Err(error) => format!("Could not save settings: {error}"),
        });
        finish(result.is_ok());
        glib::ControlFlow::Break
    });
}

#[derive(Default)]
struct LaunchSaveState {
    revision: std::cell::Cell<u64>,
    submitted: RefCell<Option<(glib::GString, glib::GString)>>,
}

fn persist_launch_settings(
    executable: &adw::EntryRow,
    launch_options: &adw::EntryRow,
    installed: &Rc<RefCell<crate::domain::InstalledGame>>,
    status: &gtk::Label,
    refresh_after_change: &Rc<dyn Fn()>,
    session: u64,
    save_state: &Rc<LaunchSaveState>,
) {
    if online::account_session() != session {
        executable.set_sensitive(false);
        launch_options.set_sensitive(false);
        status.set_label("The account changed. Reopen Properties before changing launch settings.");
        return;
    }
    let submitted = (executable.text(), launch_options.text());
    if save_state.submitted.borrow().as_ref() == Some(&submitted) {
        return;
    }
    save_state.revision.set(save_state.revision.get() + 1);
    let revision = save_state.revision.get();
    save_state.submitted.borrow_mut().take();
    let arguments = match shell_words::split(launch_options.text().as_str()) {
        Ok(arguments) => arguments,
        Err(error) => {
            status.set_label(&format!("Invalid launch options: {error}"));
            status.add_css_class("error");
            return;
        }
    };
    let relative = std::path::PathBuf::from(executable.text().as_str());
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        status.set_label("The executable must be a path inside the game directory");
        status.add_css_class("error");
        return;
    }
    let mut updated = installed.borrow().clone();
    let full_path = updated.installation_directory.join(&relative);
    updated.primary_executable = (!relative.as_os_str().is_empty()).then_some(full_path);
    updated.launch_arguments = arguments;
    updated.updated_at = chrono::Utc::now().timestamp();
    *save_state.submitted.borrow_mut() = Some(submitted.clone());
    status.remove_css_class("error");
    status.set_label("Saving launch settings…");
    let receiver = update_policies::policy_request(move || {
        let _activity = crate::profile_reset::begin_activity("saving launch settings")?;
        if let Some(path) = &updated.primary_executable {
            anyhow::ensure!(
                path.is_file()
                    && path
                        .canonicalize()?
                        .starts_with(updated.installation_directory.canonicalize()?),
                "The selected executable must exist inside the game directory"
            );
        }
        online::with_account_session(session, || {
            crate::installation::save_game_preferences(&StateStore::open()?, &updated)
        })?;
        Ok(updated)
    });
    let executable = executable.downgrade();
    let launch_options = launch_options.downgrade();
    let status = status.downgrade();
    let installed = installed.clone();
    let refresh = refresh_after_change.clone();
    let save_state = save_state.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let (Some(executable), Some(launch_options), Some(status)) = (
            executable.upgrade(),
            launch_options.upgrade(),
            status.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        if online::account_session() != session {
            executable.set_sensitive(false);
            launch_options.set_sensitive(false);
            status.set_label(
                "The account changed. Reopen Properties before changing launch settings.",
            );
            return glib::ControlFlow::Break;
        }
        if save_state.revision.get() != revision {
            return glib::ControlFlow::Break;
        }
        if executable.text() != submitted.0 || launch_options.text() != submitted.1 {
            save_state.submitted.borrow_mut().take();
            status.remove_css_class("error");
            status.set_label("Waiting to save the latest changes…");
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!(
                "Saving stopped unexpectedly. Edit the field or press Enter to retry."
            )),
        };
        match result {
            Ok(updated) => {
                *installed.borrow_mut() = updated;
                status.set_label("Saved automatically");
                refresh();
            }
            Err(error) => {
                save_state.submitted.borrow_mut().take();
                status.set_label(&format!("Could not save: {error}"));
                status.add_css_class("error");
            }
        }
        glib::ControlFlow::Break
    });
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    let value = gtk::Label::new(Some(value));
    value.set_selectable(true);
    value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    value.set_max_width_chars(55);
    value.add_css_class("dim-label");
    row.add_suffix(&value);
    row
}

#[cfg(test)]
mod control_tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn compatibility_fix_loading_retries_errors_without_saving_defaults() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p264-")
        );
        adw::init().unwrap();
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let group = adw::PreferencesGroup::new();
        let status = gtk::Label::new(None);
        let row = adw::SwitchRow::new();
        let reset = gtk::Button::with_label("Reapply Recommended");
        group.add(&status);
        group.add(&row);
        group.add(&reset);
        let resetting = Rc::new(std::cell::Cell::new(false));
        let changes = Rc::new(std::cell::Cell::new(0));
        row.connect_active_notify({
            let resetting = resetting.clone();
            let changes = changes.clone();
            move |_| {
                if !resetting.get() {
                    changes.set(changes.get() + 1);
                }
            }
        });
        let database = crate::identity::database();
        assert!(database.starts_with(std::env::var_os("XDG_DATA_HOME").unwrap()));
        std::fs::create_dir_all(&database).unwrap();
        let session = online::account_session();
        load_compatibility_fix_preferences(
            &group,
            &status,
            vec![("synthetic-fix".into(), row.clone())],
            &reset,
            resetting.clone(),
            9264001,
            session,
            true,
        );
        assert_eq!(status.label(), "Loading compatibility fixes…");
        assert!(!row.is_sensitive());
        assert!(!reset.is_sensitive());
        wait_until(|| {
            status
                .label()
                .contains("Could not load compatibility fixes")
        });
        assert!(!row.is_sensitive());
        assert!(!reset.is_sensitive());
        assert_eq!(changes.get(), 0);
        std::fs::remove_dir(&database).unwrap();
        StateStore::open()
            .unwrap()
            .set_compatibility_fix_override(9264001, "synthetic-fix", true)
            .unwrap();
        let retry = find_named_descendant(group.upcast_ref(), "compatibility-fixes-retry")
            .and_downcast::<gtk::Button>()
            .unwrap();
        assert!(retry.is_visible());
        retry.emit_clicked();
        retry.emit_clicked();
        assert_eq!(status.label(), "Loading compatibility fixes…");
        wait_until(|| row.is_sensitive());
        assert!(row.is_active());
        assert!(reset.is_sensitive());
        assert!(!retry.is_visible());
        assert_eq!(
            changes.get(),
            0,
            "initial population must not trigger saves"
        );
        assert!(
            StateStore::open()
                .unwrap()
                .compatibility_fix_overrides(9264001)
                .unwrap()["synthetic-fix"]
        );
        for (installed, expected_session) in [(false, session), (true, session.wrapping_add(1))] {
            let group = adw::PreferencesGroup::new();
            let status = gtk::Label::new(None);
            let row = adw::SwitchRow::new();
            let reset = gtk::Button::new();
            group.add(&status);
            group.add(&row);
            group.add(&reset);
            load_compatibility_fix_preferences(
                &group,
                &status,
                vec![("synthetic-fix".into(), row.clone())],
                &reset,
                Rc::new(std::cell::Cell::new(false)),
                9264001,
                expected_session,
                installed,
            );
            wait_until(|| {
                status.label().contains(if installed {
                    "account changed"
                } else {
                    "Install this game"
                })
            });
            assert!(!row.is_sensitive());
            assert!(!reset.is_sensitive());
            assert_eq!(
                row.is_active(),
                !installed,
                "stale session must not apply data"
            );
        }
        let weak = group.downgrade();
        drop(group);
        assert!(
            weak.upgrade().is_none(),
            "Retry callback must not keep its group alive"
        );
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus; no real cloud calls"]
    fn cloud_actions_reject_stale_confirmation_and_completion() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p267-")
        );
        adw::init().unwrap();
        fn click_response(widget: &gtk::Widget, label: &str) -> bool {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.label().as_deref() == Some(label)
            {
                button.emit_clicked();
                return true;
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                if click_response(&current, label) {
                    return true;
                }
                child = current.next_sibling();
            }
            false
        }
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudSessionTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let button = gtk::Button::with_label("Sync now");
        let status = gtk::Label::new(Some("Ready"));
        content.append(&button);
        content.append(&status);
        window.set_content(Some(&content));
        window.present();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let origin = CloudActionSession {
            auth: auth::session(),
            online: online::account_session(),
            epoch: 0,
            model: Rc::downgrade(&model),
        };
        let mut request = crate::cloud_saves::CloudSyncRequest {
            game: crate::domain::InstalledGame {
                product_id: 9267001,
                library_id: "synthetic".into(),
                installed_version: None,
                installation_directory: "/inert/not-a-real-game".into(),
                installer_revision_id: None,
                installer_job_id: None,
                installer_files: vec![],
                installer_complete: false,
                installer_operating_system: None,
                installer_language: None,
                compatibility: None,
                primary_executable: None,
                launch_arguments: vec![],
                state: crate::domain::InstallationState::Pending,
                error: None,
                installed_at: None,
                verified_at: None,
                last_played_at: None,
                playtime_seconds: 0,
                created_at: 0,
                updated_at: 0,
            },
            locations: vec![],
            mode: crate::domain::CloudSyncMode::Normal,
        };
        let stale_auth = origin.auth.wrapping_add(1);
        assert!(
            crate::cloud_saves::sync_for_session(request.clone(), stale_auth)
                .unwrap_err()
                .to_string()
                .contains("session changed")
        );
        assert!(
            crate::cloud_saves::inventory(&request.game, stale_auth)
                .unwrap_err()
                .to_string()
                .contains("session changed")
        );
        for stale in [
            CloudActionSession {
                auth: origin.auth.wrapping_add(1),
                ..origin.clone()
            },
            CloudActionSession {
                online: origin.online.wrapping_add(1),
                ..origin.clone()
            },
        ] {
            button.set_sensitive(true);
            run_cloud_action(&button, &status, &stale, request.clone());
            assert!(!button.is_sensitive());
            assert!(status.label().contains("Account changed"));
        }
        model.borrow_mut().logout_pending = true;
        run_cloud_action(&button, &status, &origin, request.clone());
        assert!(status.label().contains("Account changed"));
        model.borrow_mut().logout_pending = false;
        button.set_sensitive(true);
        status.set_label("Ready");
        request.mode = crate::domain::CloudSyncMode::ForceDownload;
        confirm_force_cloud_action(&window, &button, &status, &origin, request.clone());
        let dialog = window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>()
            .unwrap();
        assert!(click_response(dialog.upcast_ref(), "Cancel"));
        wait_until(|| window.visible_dialog().is_none());
        assert_eq!(status.label(), "Ready");
        assert!(button.is_sensitive());
        confirm_force_cloud_action(&window, &button, &status, &origin, request.clone());
        let dialog = window
            .visible_dialog()
            .and_downcast::<adw::AlertDialog>()
            .unwrap();
        model.borrow_mut().account_epoch += 1;
        assert!(click_response(dialog.upcast_ref(), "Force download"));
        wait_until(|| status.label().contains("Account changed"));
        assert!(!button.is_sensitive());
        model.borrow_mut().account_epoch = 0;

        let (sender, receiver) = mpsc::channel();
        button.set_sensitive(false);
        status.set_label("Synchronizing…");
        monitor_cloud_action(&button, &status, &origin, receiver);
        model.borrow_mut().account_epoch += 1;
        sender
            .send(Ok(crate::domain::CloudSyncResult::default()))
            .unwrap();
        wait_until(|| status.label().contains("Account changed"));
        assert!(!button.is_sensitive());
        assert!(!status.label().contains("Synchronized:"));
        model.borrow_mut().account_epoch = 0;

        for outcome in [
            Some(Ok(crate::domain::CloudSyncResult {
                uploaded: 2,
                ..Default::default()
            })),
            Some(Err(
                "synthetic failure https://example.invalid/?token=SECRET".into(),
            )),
            None,
        ] {
            button.set_sensitive(false);
            status.set_label("Synchronizing…");
            let (sender, receiver) = mpsc::channel();
            monitor_cloud_action(&button, &status, &origin, receiver);
            let expected = match &outcome {
                Some(Ok(_)) => "2 uploaded",
                Some(Err(_)) => "synthetic failure",
                None => "stopped unexpectedly",
            };
            if let Some(result) = outcome {
                sender.send(result).unwrap();
            }
            drop(sender);
            wait_until(|| status.label().contains(expected));
            assert!(button.is_sensitive());
            assert!(!status.label().contains("SECRET"));
        }
        let inventory = adw::ActionRow::new();
        let check = gtk::Button::with_label("Check now");
        inventory.add_suffix(&check);
        content.append(&inventory);
        load_cloud_inventory(
            request.game,
            inventory.clone(),
            check.clone(),
            CloudActionSession {
                auth: origin.auth.wrapping_add(1),
                ..origin.clone()
            },
        );
        assert!(!check.is_sensitive());
        assert!(inventory.subtitle().unwrap().contains("Account changed"));
        let (sender, receiver) = mpsc::channel();
        inventory.set_subtitle("Checking…");
        monitor_cloud_inventory(&inventory, &check, origin.clone(), receiver);
        model.borrow_mut().account_epoch += 1;
        sender
            .send(Ok(crate::cloud_saves::CloudSaveInventory {
                file_count: 99,
                total_size: 0,
                latest_modified_at: None,
            }))
            .unwrap();
        wait_until(|| inventory.subtitle().unwrap().contains("Account changed"));
        assert!(!check.is_sensitive());
        model.borrow_mut().account_epoch = 0;
        for outcome in [
            Some(Ok(crate::cloud_saves::CloudSaveInventory {
                file_count: 1,
                total_size: 42,
                latest_modified_at: None,
            })),
            Some(Err(
                "synthetic inventory failure https://example.invalid/?token=SECRET".into(),
            )),
            None,
        ] {
            inventory.set_subtitle("Checking…");
            check.set_sensitive(false);
            let (sender, receiver) = mpsc::channel();
            monitor_cloud_inventory(&inventory, &check, origin.clone(), receiver);
            let expected = match &outcome {
                Some(Ok(_)) => "1 remote save file",
                Some(Err(_)) => "synthetic inventory failure",
                None => "stopped unexpectedly",
            };
            if let Some(result) = outcome {
                sender.send(result).unwrap();
            }
            drop(sender);
            wait_until(|| inventory.subtitle().unwrap().contains(expected));
            assert!(check.is_sensitive());
            assert!(!inventory.subtitle().unwrap().contains("SECRET"));
        }
        window.close();
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, private GTK display and D-Bus"]
    fn migration_dialog_reports_stages_errors_close_and_account_changes() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p252-")
        );
        adw::init().unwrap();
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let model = Rc::new(RefCell::new(AppModel::default()));
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.MigrationFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.set_content(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
        window.present();
        let session = online::account_session();
        let view = MigrationView::new();
        view.dialog.present(Some(&window));
        assert!(view.progress.is_visible());
        assert!(!view.proceed.is_sensitive());
        let (done, receiver) = mpsc::channel();
        let (phases, stages) = mpsc::channel();
        monitor_source_migration(&view, &model, session, receiver, stages);
        phases
            .send(crate::installation::source_migration::MigrationPhase::Uninstalled)
            .unwrap();
        wait_until(|| view.status.label().contains("installing the replacement"));
        phases
            .send(crate::installation::source_migration::MigrationPhase::Restoring)
            .unwrap();
        wait_until(|| view.status.label().contains("Restoring saved games"));
        done.send(Ok(())).unwrap();
        wait_until(|| view.status.label() == "Reinstallation complete.");
        assert!(!view.progress.is_visible());
        assert!(
            !view.closed.get(),
            "completion must not navigate or dismiss the view"
        );
        view.close.emit_clicked();
        wait_until(|| view.closed.get());

        for change_account in [false, true] {
            let view = MigrationView::new();
            view.dialog.present(Some(&window));
            let (done, receiver) = mpsc::channel();
            let (_phases, stages) = mpsc::channel();
            monitor_source_migration(&view, &model, session, receiver, stages);
            if change_account {
                model.borrow_mut().account_epoch += 1;
            }
            drop(done);
            wait_until(|| !view.progress.is_visible());
            assert!(view.status.label().contains(if change_account {
                "Account changed"
            } else {
                "stopped unexpectedly"
            }));
            view.close.emit_clicked();
            wait_until(|| view.closed.get());
        }
        let view = MigrationView::new();
        view.dialog.present(Some(&window));
        let (done, receiver) = mpsc::channel();
        let (_phases, stages) = mpsc::channel();
        monitor_source_migration(&view, &model, session, receiver, stages);
        done.send(Err(anyhow::anyhow!(
            "synthetic failure https://example.invalid/?token=secret"
        )))
        .unwrap();
        wait_until(|| !view.progress.is_visible());
        assert!(view.status.label().contains("synthetic failure"));
        assert!(!view.status.label().contains("secret"));
        view.close.emit_clicked();
        wait_until(|| view.closed.get());

        let game = DetailPageModel::game(
            Game {
                product_id: 9252001,
                slug: "synthetic-game".into(),
                ..Game::default()
            },
            false,
        );
        let installed = crate::domain::InstalledGame {
            product_id: game.product_id,
            library_id: "missing".into(),
            installed_version: None,
            installation_directory: std::env::temp_dir().join("ludomere-p252-absent/game"),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: vec![],
            installer_complete: false,
            installer_operating_system: Some("linux".into()),
            installer_language: None,
            compatibility: None,
            primary_executable: None,
            launch_arguments: vec![],
            state: crate::domain::InstallationState::Pending,
            error: None,
            installed_at: None,
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 0,
            updated_at: 0,
        };
        let view = MigrationView::new();
        view.dialog.present(Some(&window));
        let config = Config {
            game_libraries: vec![],
            ..Config::default()
        };
        launch_source_migration(
            &view,
            &model,
            session,
            &config,
            &game,
            &installed,
            &[],
            crate::installation::FreshInstallSource::OfflineInstaller(0),
            vec![],
        );
        assert!(view.started.get());
        assert!(!view.proceed.is_sensitive());
        assert_eq!(view.close.label().as_deref(), Some("Close"));
        launch_source_migration(
            &view,
            &model,
            session,
            &config,
            &game,
            &installed,
            &[],
            crate::installation::FreshInstallSource::OfflineInstaller(0),
            vec![],
        );
        wait_until(|| view.status.label().contains("library is unavailable"));
        assert!(!view.progress.is_visible());
        view.close.emit_clicked();
        wait_until(|| view.closed.get());
        window.close();
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, private GTK display and D-Bus"]
    fn launch_entry_signals_and_setting_save_results_are_visible() {
        adw::init().unwrap();
        let entry = adw::EntryRow::new();
        let calls = Rc::new(std::cell::Cell::new(0));
        connect_launch_save(
            &entry,
            Rc::new({
                let calls = calls.clone();
                move || calls.set(calls.get() + 1)
            }),
        );
        assert!(!entry.shows_apply_button());
        entry.emit_by_name::<()>("entry-activated", &[]);
        assert_eq!(
            calls.get(),
            1,
            "Enter and the executable-picker dispatch must reach saving"
        );
        let focus = entry
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .find_map(|object| object.downcast::<gtk::EventControllerFocus>().ok())
            .unwrap();
        focus.emit_by_name::<()>("leave", &[]);
        assert_eq!(
            calls.get(),
            2,
            "focus leaving the inner editor must reach saving"
        );
        entry.set_text("one");
        entry.set_text("two");
        assert_eq!(calls.get(), 2, "typing is debounced");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while calls.get() == 2 && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(calls.get(), 3, "one save receives the latest text");
        entry.set_text("close immediately");
        entry.emit_by_name::<()>("unmap", &[]);
        assert_eq!(calls.get(), 4, "unmapping flushes a pending edit");

        let status = gtk::Label::new(None);
        let control = gtk::Button::new();
        let outcome = Rc::new(std::cell::Cell::new(None));
        for success in [false, true] {
            outcome.set(None);
            save_game_setting(
                control.clone().upcast(),
                &status,
                online::account_session(),
                move || {
                    if success {
                        Ok(())
                    } else {
                        anyhow::bail!("synthetic save failure")
                    }
                },
                {
                    let outcome = outcome.clone();
                    move |saved| outcome.set(Some(saved))
                },
            );
            assert!(!control.is_sensitive());
            assert_eq!(status.label(), "Saving settings…");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while outcome.get().is_none() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(outcome.get(), Some(success));
            assert!(control.is_sensitive());
            assert_eq!(status.label().contains("Settings saved"), success);
            if !success {
                assert!(status.label().contains("synthetic save failure"));
            }
        }
    }
}
