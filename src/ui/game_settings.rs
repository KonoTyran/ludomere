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
