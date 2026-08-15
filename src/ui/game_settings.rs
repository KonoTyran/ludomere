use super::*;

pub(super) fn show_game_settings(
    parent: &adw::ApplicationWindow,
    game: &DetailPageModel,
    installed: Option<crate::domain::InstalledGame>,
    refresh_after_change: Rc<dyn Fn()>,
) {
    let Some(application) = parent.application() else {
        return;
    };
    let window_name = format!("ludomere-game-settings-{}", game.product_id);
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

    let fixes_group = adw::PreferencesGroup::new();
    fixes_group.set_title("Compatibility fixes");
    fixes_group.set_description(Some(
        "Recommended fixes are preselected for known games. Changes apply the next time the game launches.",
    ));
    let overrides = crate::state::StateStore::open()
        .and_then(|store| store.compatibility_fix_overrides(game.product_id))
        .unwrap_or_default();
    let recommended = crate::compatibility::recommended_fix_ids(game.product_id);
    let resetting = Rc::new(std::cell::Cell::new(false));
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
        row.set_active(overrides.get(&fix.id).copied().unwrap_or(is_recommended));
        row.set_sensitive(installed.is_some());
        let product_id = game.product_id;
        let fix_id = fix.id.clone();
        let resetting = resetting.clone();
        let refresh_after_change = refresh_after_change.clone();
        row.connect_active_notify(move |row| {
            if resetting.get() {
                return;
            }
            if let Ok(store) = crate::state::StateStore::open()
                && let Err(error) =
                    store.set_compatibility_fix_override(product_id, &fix_id, row.is_active())
            {
                tracing::warn!(%error, product_id, %fix_id, "could not save compatibility fix");
            }
            refresh_after_change();
        });
        fixes_group.add(&row);
        fix_rows.push((fix.id.clone(), row));
    }

    let reset_row = adw::ActionRow::new();
    reset_row.set_title("Recommended settings");
    reset_row
        .set_subtitle("Discard manual changes and use the shipped recommendations for this game.");
    let reset = gtk::Button::with_label("Reapply Recommended");
    reset.set_valign(gtk::Align::Center);
    reset.set_sensitive(installed.is_some());
    reset_row.add_suffix(&reset);
    fixes_group.add(&reset_row);
    let product_id = game.product_id;
    reset.connect_clicked({
        let resetting = resetting.clone();
        let refresh_after_change = refresh_after_change.clone();
        move |_| {
            if let Ok(store) = crate::state::StateStore::open()
                && let Err(error) = store.clear_compatibility_fix_overrides(product_id)
            {
                tracing::warn!(%error, product_id, "could not reset compatibility fixes");
                return;
            }
            resetting.set(true);
            let recommended = crate::compatibility::recommended_fix_ids(product_id);
            for (fix_id, row) in &fix_rows {
                row.set_active(recommended.contains(fix_id));
            }
            resetting.set(false);
            refresh_after_change();
        }
    });
    compatibility_page.add(&fixes_group);
    let updates_page = placeholder_page(
        "Updates",
        "Automatic updates",
        "Per-game update policy will be available when installed-game update workflows are implemented. Offline installer backups remain managed separately.",
    );

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
            let locations_state = Rc::new(RefCell::new(record.locations.clone()));
            enabled.set_sensitive(supported);
            let product_id = installed_game.product_id;
            enabled.connect_active_notify(move |row| {
                let preference = if row.is_active() {
                    crate::domain::CloudSavePreference::Enabled
                } else {
                    crate::domain::CloudSavePreference::Disabled
                };
                if let Ok(store) = StateStore::open() {
                    store.set_cloud_save_preference(product_id, preference).ok();
                }
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
            check_inventory.connect_clicked(move |button| {
                button.set_sensitive(false);
                inventory_status.set_subtitle("Checking remote save files…");
                load_cloud_inventory(
                    inventory_game.clone(),
                    inventory_status.clone(),
                    button.clone(),
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
            sync_now.connect_clicked(move |button| {
                run_cloud_action(
                    button,
                    &status,
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
                button.connect_clicked(move |button| {
                    popover.popdown();
                    confirm_force_cloud_action(
                        &parent,
                        button,
                        &status,
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
            choose.connect_clicked(move |_| {
                let picker = gtk::FileDialog::builder()
                    .title("Choose save directory")
                    .modal(true)
                    .build();
                let override_status = override_status.clone();
                let override_locations = override_locations.clone();
                let override_locations_row = override_locations_row.clone();
                let override_open_folder = override_open_folder.clone();
                picker.select_folder(
                    Some(&choose_parent),
                    gio::Cancellable::NONE,
                    move |result| {
                        let Ok(file) = result else {
                            return;
                        };
                        let Some(path) = file.path() else {
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
                retry.connect_clicked(move |button| {
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
                    glib::timeout_add_local(Duration::from_millis(100), move || {
                        match receiver.try_recv() {
                            Ok(Ok(discovery)) => {
                                let supported = discovery.availability
                                    == crate::domain::CloudSaveAvailability::Supported;
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

    let versions_page = adw::PreferencesPage::new();
    versions_page.set_title("Versions");
    let versions_group = adw::PreferencesGroup::new();
    versions_group.set_title("Installed version");
    versions_group.set_description(Some(
        "Version selection and rollback controls will use locally retained installer revisions in a future update.",
    ));
    versions_group.add(&info_row(
        "Current installation",
        installed
            .as_ref()
            .and_then(|value| value.installed_version.as_deref())
            .unwrap_or("Not installed"),
    ));
    versions_page.add(&versions_group);

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
            "versions",
            "Versions",
            "document-open-recent-symbolic",
            versions_page.upcast::<gtk::Widget>(),
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
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                match path.strip_prefix(&install_directory) {
                    Ok(relative) if relative.components().next().is_some() => {
                        executable.set_text(&relative.to_string_lossy());
                        executable.emit_activate();
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
        for entry in [&executable, &launch_options] {
            entry.connect_notify_local(Some("has-focus"), {
                let executable = executable.clone();
                let launch_options = launch_options.clone();
                let installed = installed.clone();
                let save_status = save_status.clone();
                let refresh_after_change = refresh_after_change.clone();
                move |entry, _| {
                    if !entry.has_focus() {
                        persist_launch_settings(
                            &executable,
                            &launch_options,
                            &installed,
                            &save_status,
                            &refresh_after_change,
                        );
                    }
                }
            });
        }
        for entry in [&executable, &launch_options] {
            let executable = executable.clone();
            let launch_options = launch_options.clone();
            let installed = installed.clone();
            let save_status = save_status.clone();
            let refresh_after_change = refresh_after_change.clone();
            entry.connect_apply(move |_| {
                persist_launch_settings(
                    &executable,
                    &launch_options,
                    &installed,
                    &save_status,
                    &refresh_after_change,
                );
            });
        }
    }

    window.present();
}

fn run_cloud_action(
    button: &gtk::Button,
    status: &gtk::Label,
    request: crate::cloud_saves::CloudSyncRequest,
) {
    button.set_sensitive(false);
    status.set_label("Synchronizing…");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender
            .send(crate::cloud_saves::sync(request).map_err(|error| format!("{error:#}")))
            .ok();
    });
    let button = button.clone();
    let status = status.clone();
    glib::timeout_add_local(
        std::time::Duration::from_millis(100),
        move || match receiver.try_recv() {
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
                status.set_label(&error);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                button.set_sensitive(true);
                status.set_label("Cloud-save worker stopped unexpectedly");
                glib::ControlFlow::Break
            }
        },
    );
}

fn load_cloud_inventory(
    game: crate::domain::InstalledGame,
    row: adw::ActionRow,
    button: gtk::Button,
) {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender
            .send(crate::cloud_saves::inventory(&game).map_err(|error| format!("{error:#}")))
            .ok();
    });
    glib::timeout_add_local(
        std::time::Duration::from_millis(100),
        move || match receiver.try_recv() {
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
                row.set_subtitle(&format!("Could not check cloud storage: {error}"));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                button.set_sensitive(true);
                row.set_subtitle("Cloud-storage check stopped unexpectedly");
                glib::ControlFlow::Break
            }
        },
    );
}

fn confirm_force_cloud_action(
    parent: &adw::ApplicationWindow,
    button: &gtk::Button,
    status: &gtk::Label,
    request: crate::cloud_saves::CloudSyncRequest,
) {
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
    dialog.choose(Some(parent), gio::Cancellable::NONE, move |response| {
        if response == "force" {
            run_cloud_action(&button, &status, request);
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

fn persist_launch_settings(
    executable: &adw::EntryRow,
    launch_options: &adw::EntryRow,
    installed: &Rc<RefCell<crate::domain::InstalledGame>>,
    status: &gtk::Label,
    refresh_after_change: &Rc<dyn Fn()>,
) {
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
    if !relative.as_os_str().is_empty() && !full_path.is_file() {
        status.set_label("The selected executable does not exist inside the game directory");
        status.add_css_class("error");
        return;
    }
    updated.primary_executable = (!relative.as_os_str().is_empty()).then_some(full_path);
    updated.launch_arguments = arguments;
    updated.updated_at = chrono::Utc::now().timestamp();
    match StateStore::open()
        .and_then(|store| crate::installation::save_game_preferences(&store, &updated))
    {
        Ok(()) => {
            *installed.borrow_mut() = updated;
            status.remove_css_class("error");
            status.set_label("Saved automatically");
            refresh_after_change();
        }
        Err(error) => {
            status.set_label(&format!("Could not save: {error}"));
            status.add_css_class("error");
        }
    }
}

fn placeholder_page(title: &str, group_title: &str, description: &str) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(title);
    let group = adw::PreferencesGroup::new();
    group.set_title(group_title);
    group.set_description(Some(description));
    let status = adw::ActionRow::new();
    status.set_title("Planned feature");
    status.set_subtitle("Not available yet");
    status.set_sensitive(false);
    group.add(&status);
    page.add(&group);
    page
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
