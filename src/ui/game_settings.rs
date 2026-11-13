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
        let directory = adw::ActionRow::new();
        directory.set_title("Installation directory");
        directory.set_use_markup(false);
        directory.set_subtitle(&installed.installation_directory.to_string_lossy());
        directory.set_subtitle_selectable(true);
        files_group.add(&directory);
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

    let cloud_page = cloud_settings_page(&window, model, installed.as_ref(), &cloud_session);

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
            &window,
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
            "general",
            "General",
            "preferences-system-symbolic",
            general_page.upcast::<gtk::Widget>(),
        ),
        (
            "cloud-saves",
            "Cloud Saves",
            "folder-remote-symbolic",
            cloud_page.upcast::<gtk::Widget>(),
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
        let save_state = Rc::new(LaunchSaveState {
            submitted: RefCell::new(Some((executable.text(), launch_options.text()))),
            ..Default::default()
        });
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

#[cfg(test)]
thread_local! {
    static TEST_CLOUD_SETTINGS_DELAYS: RefCell<VecDeque<mpsc::Receiver<()>>> = const { RefCell::new(VecDeque::new()) };
    static TEST_CLOUD_SETTINGS_DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn cloud_settings_page(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    installed: Option<&crate::domain::InstalledGame>,
    origin: &CloudActionSession,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Cloud Saves");
    let group = adw::PreferencesGroup::new();
    group.set_title("GOG Cloud Saves");
    page.add(&group);
    let Some(installed) = installed else {
        group.set_description(Some(
            "Install the Windows version to configure cloud saves.",
        ));
        return page;
    };
    if installed.compatibility.is_none()
        || !installed
            .installer_operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        group.set_description(Some(
            "Cloud saves currently support Windows games installed into a managed UMU prefix only.",
        ));
        return page;
    }
    let spinner = gtk::Spinner::new();
    spinner.set_widget_name("cloud-settings-spinner");
    let status = gtk::Label::new(None);
    status.set_widget_name("cloud-settings-load-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_selectable(true);
    let retry = gtk::Button::with_label("Retry loading settings");
    retry.set_widget_name("cloud-settings-load-retry");
    group.add(&spinner);
    group.add(&status);
    group.add(&retry);
    let closed = Rc::new(std::cell::Cell::new(false));
    window.connect_close_request({
        let closed = closed.clone();
        move |_| {
            closed.set(true);
            glib::Propagation::Proceed
        }
    });
    let loading = Rc::new(std::cell::Cell::new(false));
    let load = {
        let window = window.downgrade();
        let page = page.downgrade();
        let group = group.downgrade();
        let status = status.downgrade();
        let spinner = spinner.downgrade();
        let retry = retry.downgrade();
        let model = Rc::downgrade(model);
        let installed = installed.clone();
        let origin = origin.clone();
        move || {
            let (
                Some(window),
                Some(page),
                Some(group),
                Some(status),
                Some(spinner),
                Some(retry),
                Some(model),
            ) = (
                window.upgrade(),
                page.upgrade(),
                group.upgrade(),
                status.upgrade(),
                spinner.upgrade(),
                retry.upgrade(),
                model.upgrade(),
            )
            else {
                return;
            };
            if closed.get() || loading.get() || !group.is_ancestor(&page) {
                return;
            }
            if model.borrow().account_epoch != origin.epoch
                || model.borrow().logout_pending
                || (online::account_session(), auth::session()) != (origin.online, origin.auth)
            {
                spinner.set_spinning(false);
                spinner.set_visible(false);
                retry.set_visible(false);
                status.set_label("Your account session changed. Close and reopen Properties to load cloud-save settings.");
                return;
            }
            loading.set(true);
            spinner.set_visible(true);
            spinner.set_spinning(true);
            status.remove_css_class("error");
            status.set_label("Loading cloud-save settings…");
            retry.set_sensitive(false);
            retry.set_visible(false);
            let (sender, receiver) = mpsc::channel();
            match crate::profile_reset::begin_activity("loading cloud-save settings") {
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
                Ok(activity) => {
                    let product_id = installed.product_id;
                    let session = (origin.online, origin.auth);
                    #[cfg(test)]
                    let delay =
                        TEST_CLOUD_SETTINGS_DELAYS.with(|delays| delays.borrow_mut().pop_front());
                    #[cfg(test)]
                    TEST_CLOUD_SETTINGS_DISPATCHES.with(|count| count.set(count.get() + 1));
                    std::thread::spawn(move || {
                        let result = (|| -> anyhow::Result<_> {
                            let _activity = activity;
                            #[cfg(test)]
                            if let Some(delay) = delay {
                                delay.recv_timeout(Duration::from_secs(10))?;
                            }
                            anyhow::ensure!(
                                session == (online::account_session(), auth::session()),
                                "The account changed before loading cloud-save settings."
                            );
                            let store = StateStore::open()?;
                            let record = store.cloud_save_record(product_id)?;
                            anyhow::ensure!(
                                session == (online::account_session(), auth::session()),
                                "The account changed while loading cloud-save settings."
                            );
                            Ok(record)
                        })();
                        let _ = sender.send(result);
                    });
                }
            }
            let window = window.downgrade();
            let page = page.downgrade();
            let group = group.downgrade();
            let status = status.downgrade();
            let spinner = spinner.downgrade();
            let retry = retry.downgrade();
            let model = Rc::downgrade(&model);
            let closed = closed.clone();
            let loading = loading.clone();
            let installed = installed.clone();
            let origin = origin.clone();
            let mut receiver = Some(receiver);
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let (
                    Some(window),
                    Some(page),
                    Some(group),
                    Some(status),
                    Some(spinner),
                    Some(retry),
                    Some(model),
                ) = (
                    window.upgrade(),
                    page.upgrade(),
                    group.upgrade(),
                    status.upgrade(),
                    spinner.upgrade(),
                    retry.upgrade(),
                    model.upgrade(),
                )
                else {
                    return glib::ControlFlow::Break;
                };
                if closed.get()
                    || !window.is_visible()
                    || page.root().as_ref() != Some(window.upcast_ref())
                    || !group.is_ancestor(&page)
                {
                    return glib::ControlFlow::Break;
                }
                if model.borrow().account_epoch != origin.epoch
                    || model.borrow().logout_pending
                    || (online::account_session(), auth::session()) != (origin.online, origin.auth)
                {
                    drop(receiver.take());
                    spinner.set_spinning(false);
                    spinner.set_visible(false);
                    retry.set_visible(false);
                    status.set_label("Your account session changed. Close and reopen Properties to load cloud-save settings.");
                    loading.set(false);
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.as_ref().unwrap().try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => Err(anyhow::anyhow!("Loading stopped unexpectedly. Try again.")),
                };
                drop(receiver.take());
                spinner.set_spinning(false);
                spinner.set_visible(false);
                match result {
                    Ok(record) => {
                        page.remove(&group);
                        populate_cloud_settings(
                            &window, &model, &page, &installed, record, &origin,
                        );
                        loading.set(false);
                    }
                    Err(error) => {
                        status.set_label(&format!("Could not load cloud-save settings: {error:#}"));
                        status.add_css_class("error");
                        retry.set_visible(true);
                        loading.set(false);
                        retry.set_sensitive(true);
                    }
                }
                glib::ControlFlow::Break
            });
        }
    };
    retry.connect_clicked({
        let load = load.clone();
        move |_| load()
    });
    load();
    page
}

#[cfg(test)]
struct TestCloudDiscovery {
    before_entry: mpsc::Receiver<()>,
    entered: mpsc::Sender<((u64, u64), Vec<crate::domain::CloudSaveLocation>)>,
    result: mpsc::Receiver<Result<crate::domain::CloudSaveDiscovery, String>>,
}

#[cfg(test)]
struct TestCloudBackup {
    gtk_thread: std::thread::ThreadId,
    before_io: mpsc::Receiver<()>,
    inspected: mpsc::Sender<()>,
    finish: mpsc::Receiver<()>,
    disconnect: bool,
}

#[cfg(test)]
thread_local! {
    static TEST_CLOUD_BACKUPS: RefCell<Option<VecDeque<TestCloudBackup>>> = const { RefCell::new(None) };
    static TEST_CLOUD_DISCOVERIES: RefCell<Option<VecDeque<TestCloudDiscovery>>> = const { RefCell::new(None) };
    static TEST_CLOUD_PICKERS: RefCell<Option<VecDeque<TestCloudPicker>>> = const { RefCell::new(None) };
    static TEST_CLOUD_LOCATION_SAVES: RefCell<Option<VecDeque<TestCloudLocationSave>>> = const { RefCell::new(None) };
    static TEST_CLOUD_LOCATION_STATE: RefCell<std::rc::Weak<RefCell<Vec<crate::domain::CloudSaveLocation>>>> = const { RefCell::new(std::rc::Weak::new()) };
}

#[cfg(test)]
type TestCloudPicker = mpsc::Receiver<Result<gio::File, glib::Error>>;

#[cfg(test)]
struct TestCloudLocationSave {
    before_open: mpsc::Receiver<()>,
    entered: mpsc::Sender<()>,
    finish: mpsc::Receiver<()>,
    disconnect: bool,
}

fn populate_cloud_settings(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    cloud_page: &adw::PreferencesPage,
    installed_game: &crate::domain::InstalledGame,
    record: crate::state::CloudSaveRecord,
    cloud_session: &CloudActionSession,
) {
    let session = cloud_session.online;
    let cloud_group = adw::PreferencesGroup::new();
    cloud_group.set_title("GOG Cloud Saves");
    let cloud_status = gtk::Label::new(None);
    cloud_status.set_widget_name("cloud-save-status");
    cloud_status.set_xalign(0.0);
    cloud_status.set_wrap(true);
    let enabled = adw::SwitchRow::new();
    enabled.set_title("Synchronize saves with GOG");
    enabled.set_subtitle("Runs before launch and after the monitored game process exits");
    enabled.set_active(record.preference == crate::domain::CloudSavePreference::Enabled);
    let supported = record.availability == crate::domain::CloudSaveAvailability::Supported;
    let management = cloud_management::cloud_management_group(window, model, installed_game);
    management.set_visible(supported);
    let locations_state = Rc::new(RefCell::new(record.locations.clone()));
    #[cfg(test)]
    if TEST_CLOUD_PICKERS.with(|requests| requests.borrow().is_some()) {
        TEST_CLOUD_LOCATION_STATE
            .with(|state| *state.borrow_mut() = Rc::downgrade(&locations_state));
    }
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
    let inventory_status = inventory_row.downgrade();
    let origin = cloud_session.clone();
    check_inventory.connect_clicked(move |button| {
        let Some(inventory_status) = inventory_status.upgrade() else {
            return;
        };
        load_cloud_inventory(
            inventory_game.clone(),
            inventory_status,
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
        let parent = window.downgrade();
        let popover = popover.downgrade();
        let origin = cloud_session.clone();
        button.connect_clicked(move |button| {
            let Some(parent) = parent.upgrade() else {
                return;
            };
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
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
    backup_row.add_suffix(&backup);
    cloud_group.add(&backup_row);
    let backup_status = gtk::Label::new(None);
    backup_status.set_widget_name("cloud-backup-status");
    backup_status.set_xalign(0.0);
    backup_status.set_wrap(true);
    backup_status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    backup_status.set_selectable(true);
    backup_status.set_visible(false);
    cloud_group.add(&backup_status);

    let override_row = adw::ActionRow::new();
    override_row.set_title("Override save directory");
    override_row.set_subtitle("Use only when GOG's configured location cannot be resolved");
    let choose = gtk::Button::with_label("Choose…");
    choose.set_widget_name("cloud-location-choose");
    choose.set_valign(gtk::Align::Start);
    choose.set_margin_top(10);
    choose.set_sensitive(
        supported || record.availability == crate::domain::CloudSaveAvailability::Unknown,
    );
    let retry = gtk::Button::with_label("Retry metadata discovery");
    retry.set_widget_name("cloud-discovery-retry");
    let pending = Rc::new(std::cell::Cell::new(false));
    let backup_pending = Rc::new(std::cell::Cell::new(false));
    let availability = Rc::new(std::cell::Cell::new(record.availability));
    let closed = Rc::new(std::cell::Cell::new(false));
    window.connect_close_request({
        let closed = closed.clone();
        move |_| {
            closed.set(true);
            glib::Propagation::Proceed
        }
    });
    let refresh_location_actions = Rc::new({
        let choose = choose.downgrade();
        let retry = retry.downgrade();
        let backup = backup.downgrade();
        let pending = pending.clone();
        let backup_pending = backup_pending.clone();
        let availability = availability.clone();
        let origin = cloud_session.clone();
        let closed = closed.clone();
        let window = window.downgrade();
        let page = cloud_page.downgrade();
        move || 'refresh: loop {
            let (Some(window), Some(page)) = (window.upgrade(), page.upgrade()) else {
                return;
            };
            if !window.is_visible() || page.root().as_ref() != Some(window.upcast_ref()) {
                return;
            }
            let snapshot = || {
                (
                    closed.get(),
                    pending.get(),
                    backup_pending.get(),
                    availability.get(),
                    origin.is_current_local(),
                    origin.is_current(),
                )
            };
            let current = snapshot();
            if let Some(choose) = choose.upgrade() {
                choose.set_sensitive(
                    !closed.get()
                        && !pending.get()
                        && origin.is_current_local()
                        && matches!(
                            availability.get(),
                            crate::domain::CloudSaveAvailability::Supported
                                | crate::domain::CloudSaveAvailability::Unknown
                        ),
                );
                if snapshot() != current {
                    continue 'refresh;
                }
            }
            // The first setter can synchronously admit another request.
            if !window.is_visible() || page.root().as_ref() != Some(window.upcast_ref()) {
                return;
            }
            if let Some(retry) = retry.upgrade() {
                retry.set_sensitive(
                    !closed.get()
                        && !pending.get()
                        && origin.is_current()
                        && availability.get() == crate::domain::CloudSaveAvailability::Unknown,
                );
                if snapshot() != current {
                    continue 'refresh;
                }
            }
            if !window.is_visible() || page.root().as_ref() != Some(window.upcast_ref()) {
                return;
            }
            if let Some(backup) = backup.upgrade() {
                backup.set_sensitive(
                    !closed.get()
                        && !backup_pending.get()
                        && origin.is_current_local()
                        && availability.get() == crate::domain::CloudSaveAvailability::Supported,
                );
                if snapshot() != current {
                    continue 'refresh;
                }
            }
            break;
        }
    });
    backup.connect_clicked({
        let window = window.downgrade();
        let page = cloud_page.downgrade();
        let status = backup_status.downgrade();
        let origin = cloud_session.clone();
        let pending = backup_pending.clone();
        let closed = closed.clone();
        let availability = availability.clone();
        let refresh = refresh_location_actions.clone();
        move |button| {
            let (Some(parent), Some(page_widget), Some(status_widget)) =
                (window.upgrade(), page.upgrade(), status.upgrade()) else { return; };
            if pending.get() || closed.get() || !parent.is_visible()
                || page_widget.root().as_ref() != Some(parent.upcast_ref())
                || !button.is_ancestor(&page_widget)
                || availability.get() != crate::domain::CloudSaveAvailability::Supported
            { return; }
            if !origin.is_current_local() {
                status_widget.set_label("Your account session changed. Close and reopen Properties before opening backups.");
                status_widget.set_visible(true);
                refresh();
                return;
            }
            pending.set(true);
            refresh();
            status_widget.remove_css_class("error");
            status_widget.set_label("Preparing backup folder…");
            status_widget.set_visible(true);
            if closed.get() || !parent.is_visible()
                || page_widget.root().as_ref() != Some(parent.upcast_ref())
                || !button.is_ancestor(&page_widget)
            {
                pending.set(false);
                return;
            }
            if !origin.is_current_local() {
                status_widget.set_label("Your account session changed. Close and reopen Properties before opening backups.");
                pending.set(false);
                refresh();
                return;
            }
            if availability.get() != crate::domain::CloudSaveAvailability::Supported {
                status_widget.set_label("Cloud-save support changed; the backup folder was not opened.");
                pending.set(false);
                refresh();
                return;
            }
            let (sender, receiver) = mpsc::channel();
            match crate::profile_reset::begin_activity("opening cloud-save backup folder") {
                Err(error) => { let _ = sender.send(Err(format!("{error:#}"))); }
                Ok(activity) => {
                    let path = backup_path.clone();
                    let session = (origin.online, origin.auth);
                    #[cfg(test)]
                    let fixture = TEST_CLOUD_BACKUPS.with(|requests| requests.borrow_mut().as_mut().map(|requests| requests.pop_front().expect("missing inert cloud backup request")));
                    std::thread::spawn(move || {
                        let result = (|| -> anyhow::Result<_> {
                            let _activity = activity;
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                assert_ne!(std::thread::current().id(), fixture.gtk_thread);
                                fixture.before_io.recv_timeout(Duration::from_secs(10))?;
                            }
                            anyhow::ensure!(session == (online::account_session(), auth::session()), "The account changed before opening backups.");
                            std::fs::create_dir_all(&path).map_err(|error| anyhow::anyhow!("Could not create the cloud-save backup folder: {error}"))?;
                            let config = crate::storage::read_config()?;
                            if let Some(kind) = crate::config::LibraryKind::ALL.into_iter().find(|kind| {
                                config.libraries(*kind).iter().any(|library| path.starts_with(&library.path))
                            }) {
                                crate::storage::validate_path(&config, kind, &path)?;
                            }
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                let _ = fixture.inspected.send(());
                                fixture.finish.recv_timeout(Duration::from_secs(10))?;
                            }
                            anyhow::ensure!(session == (online::account_session(), auth::session()), "The account changed while opening backups.");
                            Ok(path)
                        })();
                        #[cfg(test)]
                        if fixture.as_ref().is_some_and(|fixture| fixture.disconnect) { return; }
                        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
                    });
                }
            }
            let window = window.clone();
            let page = page.clone();
            let status = status.clone();
            let button = button.downgrade();
            let origin = origin.clone();
            let pending = pending.clone();
            let closed = closed.clone();
            let availability = availability.clone();
            let refresh = refresh.clone();
            let mut receiver = Some(receiver);
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let (Some(parent), Some(page_widget), Some(status_widget), Some(button_widget)) =
                    (window.upgrade(), page.upgrade(), status.upgrade(), button.upgrade()) else {
                        pending.set(false);
                        return glib::ControlFlow::Break;
                    };
                if closed.get() || !parent.is_visible()
                    || page_widget.root().as_ref() != Some(parent.upcast_ref())
                    || !button_widget.is_ancestor(&page_widget)
                {
                    pending.set(false);
                    return glib::ControlFlow::Break;
                }
                if !origin.is_current_local() {
                    drop(receiver.take());
                    status_widget.set_label("Your account session changed. Close and reopen Properties before opening backups.");
                    pending.set(false);
                    refresh();
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.as_ref().unwrap().try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => Err("Backup folder inspection stopped. Try opening it again.".into()),
                };
                drop(receiver.take());
                match result {
                    Ok(path) if availability.get() == crate::domain::CloudSaveAvailability::Supported => {
                        status_widget.set_label("Backup folder is ready.");
                        let origin = origin.clone();
                        let window = window.clone();
                        let page = page.clone();
                        let button = button.clone();
                        let closed = closed.clone();
                        let availability = availability.clone();
                        super::widgets::file_open::launch_validated_directory(&path, &parent, "cloud-save backup directory", move || {
                            !closed.get() && origin.is_current_local()
                                && availability.get() == crate::domain::CloudSaveAvailability::Supported
                                && window.upgrade().zip(page.upgrade()).zip(button.upgrade()).is_some_and(|((window, page), button)| {
                                    window.is_visible() && page.root().as_ref() == Some(window.upcast_ref()) && button.is_ancestor(&page)
                                })
                        });
                    }
                    Ok(_) => status_widget.set_label("Cloud-save support changed; the backup folder was not opened."),
                    Err(error) => {
                        status_widget.add_css_class("error");
                        status_widget.set_label(&notifications::failure_message("Could not open backup folder", &error));
                    }
                }
                pending.set(false);
                refresh();
                glib::ControlFlow::Break
            });
        }
    });
    choose.connect_clicked({
        let window = window.downgrade();
        let page = cloud_page.downgrade();
        let status = cloud_status.downgrade();
        let locations_row = locations_row.downgrade();
        let open_folder = open_save_folder.downgrade();
        let locations = locations_state.clone();
        let origin = cloud_session.clone();
        let pending = pending.clone();
        let closed = closed.clone();
        let availability = availability.clone();
        let refresh = refresh_location_actions.clone();
        move |button| {
            let (Some(parent), Some(page_widget), Some(status_widget)) = (window.upgrade(), page.upgrade(), status.upgrade()) else { return; };
            if pending.get() || closed.get() || !parent.is_visible()
                || page_widget.root().as_ref() != Some(parent.upcast_ref())
                || !button.is_ancestor(&page_widget)
                || !matches!(availability.get(), crate::domain::CloudSaveAvailability::Supported | crate::domain::CloudSaveAvailability::Unknown)
            { return; }
            if !origin.is_current_local() {
                status_widget.set_label("Your account session changed. Close and reopen Properties before changing the save directory.");
                refresh();
                return;
            }
            pending.set(true);
            refresh();
            let window = window.clone();
            let page = page.clone();
            let status = status.clone();
            let locations_row = locations_row.clone();
            let open_folder = open_folder.clone();
            let locations = locations.clone();
            let origin = origin.clone();
            let pending = pending.clone();
            let closed = closed.clone();
            let refresh = refresh.clone();
            let button = button.downgrade();
            let complete = move |result: Result<gio::File, glib::Error>| {
                let (Some(window_widget), Some(page_widget), Some(button_widget), Some(status_widget)) = (window.upgrade(), page.upgrade(), button.upgrade(), status.upgrade()) else { return; };
                if closed.get() || !window_widget.is_visible()
                    || page_widget.root().as_ref() != Some(window_widget.upcast_ref())
                    || !button_widget.is_ancestor(&page_widget)
                { return; }
                if !origin.is_current_local() {
                    status_widget.set_label("Your account session changed. Close and reopen Properties before changing the save directory.");
                    pending.set(false);
                    refresh();
                    return;
                }
                let path = match result {
                    Ok(file) => match file.path() {
                        Some(path) => path,
                        None => {
                            status_widget.set_label("Choose a local directory for your save files.");
                            pending.set(false);
                            refresh();
                            return;
                        }
                    },
                    Err(error) => {
                        if !error.matches(gtk::DialogError::Dismissed) && !error.matches(gtk::DialogError::Cancelled) {
                            status_widget.set_label(&notifications::failure_message("Could not choose a save directory", &error.to_string()));
                        }
                        pending.set(false);
                        refresh();
                        return;
                    }
                };
                status_widget.remove_css_class("error");
                status_widget.set_label("Saving override save directory…");
                let location = crate::domain::CloudSaveLocation { name: "override".into(), path, remote_namespace: "override".into(), user_override: true };
                let (sender, receiver) = mpsc::channel();
                match crate::profile_reset::begin_activity("saving cloud-save directory") {
                    Err(error) => { let _ = sender.send(Err(format!("{error:#}"))); }
                    Ok(activity) => {
                        let session = (origin.online, origin.auth);
                        let location = location.clone();
                        #[cfg(test)]
                        let fixture = TEST_CLOUD_LOCATION_SAVES.with(|requests| requests.borrow_mut().as_mut().map(|requests| requests.pop_front().expect("missing inert cloud location save")));
                        std::thread::spawn(move || {
                            let result = (|| -> anyhow::Result<()> {
                                let _activity = activity;
                                #[cfg(test)]
                                if let Some(fixture) = fixture.as_ref() { fixture.before_open.recv_timeout(Duration::from_secs(10))?; }
                                anyhow::ensure!(session == (online::account_session(), auth::session()), "The account changed before saving the directory.");
                                let store = StateStore::open()?;
                                anyhow::ensure!(session == (online::account_session(), auth::session()), "The account changed before saving the directory.");
                                #[cfg(test)]
                                if let Some(fixture) = fixture.as_ref() { let _ = fixture.entered.send(()); }
                                store.set_cloud_save_locations(product_id, std::slice::from_ref(&location))?;
                                #[cfg(test)]
                                if let Some(fixture) = fixture.as_ref() { fixture.finish.recv_timeout(Duration::from_secs(10))?; }
                                Ok(())
                            })();
                            #[cfg(test)]
                            if fixture.as_ref().is_some_and(|fixture| fixture.disconnect) { return; }
                            let _ = sender.send(result.map_err(|error| format!("{error:#}")));
                        });
                    }
                }
                let mut receiver = Some(receiver);
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    let (Some(window), Some(page), Some(button), Some(status), Some(row), Some(open_folder)) = (window.upgrade(), page.upgrade(), button.upgrade(), status.upgrade(), locations_row.upgrade(), open_folder.upgrade()) else { return glib::ControlFlow::Break; };
                    if closed.get() || !window.is_visible() || page.root().as_ref() != Some(window.upcast_ref()) || !button.is_ancestor(&page) { return glib::ControlFlow::Break; }
                    if !origin.is_current_local() {
                        drop(receiver.take());
                        status.set_label("Your account session changed. Close and reopen Properties before changing the save directory.");
                        pending.set(false);
                        refresh();
                        return glib::ControlFlow::Break;
                    }
                    let result = match receiver.as_ref().unwrap().try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => Err("Saving stopped unexpectedly. Try again.".into()),
                    };
                    drop(receiver.take());
                    match result {
                        Ok(()) => {
                            *locations.borrow_mut() = vec![location.clone()];
                            row.set_subtitle(&cloud_location_summary(&locations.borrow()));
                            row.set_visible(true);
                            open_folder.set_sensitive(true);
                            status.set_label("Override save directory updated");
                        }
                        Err(error) => {
                            status.add_css_class("error");
                            status.set_label(&notifications::failure_message("Could not save override", &error));
                        }
                    }
                    pending.set(false);
                    refresh();
                    glib::ControlFlow::Break
                });
            };
            #[cfg(test)]
            if let Some(receiver) = TEST_CLOUD_PICKERS.with(|requests| requests.borrow_mut().as_mut().map(|requests| requests.pop_front().expect("missing inert cloud picker"))) {
                let mut complete = Some(complete);
                glib::timeout_add_local(Duration::from_millis(20), move || {
                    let result = match receiver.try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => Err(glib::Error::new(gtk::DialogError::Failed, "Inert picker disconnected")),
                    };
                    complete.take().unwrap()(result);
                    glib::ControlFlow::Break
                });
                return;
            }
            gtk::FileDialog::builder().title("Choose save directory").modal(true).build()
                .select_folder(Some(&parent), gio::Cancellable::NONE, complete);
        }
    });
    override_row.add_suffix(&choose);
    cloud_group.add(&override_row);

    if record.availability == crate::domain::CloudSaveAvailability::Unknown {
        retry.set_valign(gtk::Align::Start);
        retry.set_margin_top(10);
        let game = installed_game.clone();
        let locations = locations_state.clone();
        let status = cloud_status.downgrade();
        let locations_row = locations_row.downgrade();
        let open_save_folder = open_save_folder.downgrade();
        let inventory_row = inventory_row.downgrade();
        let check_inventory = check_inventory.downgrade();
        let enabled = enabled.downgrade();
        let sync_now = sync_now.downgrade();
        let advanced = advanced.downgrade();
        let choose = choose.downgrade();
        let management = management.downgrade();
        let origin = cloud_session.clone();
        let refresh = refresh_location_actions.clone();
        let window = window.downgrade();
        let page = cloud_page.downgrade();
        retry.connect_clicked(move |button| {
            let (Some(window), Some(page), Some(status)) =
                (window.upgrade(), page.upgrade(), status.upgrade())
            else {
                return;
            };
            if pending.get()
                || closed.get()
                || !window.is_visible()
                || !button.get_visible()
                || page.root().as_ref() != Some(window.upcast_ref())
                || !button.is_ancestor(&page)
            {
                return;
            }
            if !origin.is_current() {
                status.set_label("Your account session changed or you are signed out. Sign in, then close and reopen Properties to check cloud-save support.");
                refresh();
                return;
            }
            pending.set(true);
            refresh();
            status.remove_css_class("error");
            status.set_label("Checking GOG cloud-save support…");
            let (sender, receiver) = mpsc::channel();
            match crate::profile_reset::begin_activity("discovering cloud-save metadata") {
                Err(error) => {
                    let _ = sender.send(Err(format!("{error:#}")));
                }
                Ok(activity) => {
                    let game = game.clone();
                    let stored_locations = locations.borrow().clone();
                    let session = (origin.online, origin.auth);
                    #[cfg(test)]
                    let fixture = TEST_CLOUD_DISCOVERIES.with(|requests| {
                        requests.borrow_mut().as_mut().map(|requests| {
                            requests.pop_front().expect("missing inert cloud discovery")
                        })
                    });
                    std::thread::spawn(move || {
                        #[cfg(test)]
                        let mut disconnected = false;
                        let result = (|| -> anyhow::Result<_> {
                            let _activity = activity;
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                fixture.before_entry.recv_timeout(Duration::from_secs(10))?;
                            }
                            anyhow::ensure!(
                                online::account_session() == session.0
                                    && auth::session_is_current(session.1),
                                "The account changed before checking cloud-save support."
                            );
                            #[cfg(test)]
                            let discovery = if let Some(fixture) = fixture {
                                let _ = fixture.entered.send((session, stored_locations));
                                match fixture.result.recv_timeout(Duration::from_secs(10)) {
                                    Ok(result) => result.map_err(anyhow::Error::msg)?,
                                    Err(_) => {
                                        disconnected = true;
                                        anyhow::bail!("Synthetic cloud discovery disconnected");
                                    }
                                }
                            } else {
                                crate::cloud_saves::discover_and_store_for_session(
                                    &game, &stored_locations, session.1,
                                )?
                            };
                            #[cfg(not(test))]
                            let discovery = crate::cloud_saves::discover_and_store_for_session(
                                &game, &stored_locations, session.1,
                            )?;
                            anyhow::ensure!(
                                online::account_session() == session.0
                                    && auth::session_is_current(session.1),
                                "The account changed while checking cloud-save support."
                            );
                            Ok(discovery)
                        })();
                        #[cfg(test)]
                        if disconnected {
                            return;
                        }
                        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
                    });
                }
            }
            let button = button.downgrade();
            let status = status.downgrade();
            let locations = locations.clone();
            let locations_row = locations_row.clone();
            let open_save_folder = open_save_folder.clone();
            let inventory_row = inventory_row.clone();
            let check_inventory = check_inventory.clone();
            let enabled = enabled.clone();
            let sync_now = sync_now.clone();
            let advanced = advanced.clone();
            let choose = choose.clone();
            let management = management.clone();
            let window = window.downgrade();
            let page = page.downgrade();
            let origin = origin.clone();
            let pending = pending.clone();
            let closed = closed.clone();
            let refresh = refresh.clone();
            let availability = availability.clone();
            let mut receiver = Some(receiver);
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let (
                    Some(window), Some(page), Some(button), Some(status), Some(locations_row),
                    Some(open_save_folder), Some(inventory_row), Some(check_inventory),
                    Some(enabled), Some(sync_now), Some(advanced), Some(_choose),
                    Some(management),
                ) = (
                    window.upgrade(), page.upgrade(), button.upgrade(), status.upgrade(),
                    locations_row.upgrade(), open_save_folder.upgrade(), inventory_row.upgrade(),
                    check_inventory.upgrade(), enabled.upgrade(), sync_now.upgrade(),
                    advanced.upgrade(), choose.upgrade(), management.upgrade(),
                ) else {
                    pending.set(false);
                    return glib::ControlFlow::Break;
                };
                if closed.get()
                    || !window.is_visible()
                    || page.root().as_ref() != Some(window.upcast_ref())
                    || !button.is_ancestor(&page)
                {
                    pending.set(false);
                    return glib::ControlFlow::Break;
                }
                if !origin.is_current() {
                    drop(receiver.take());
                    status.set_label("Your account session changed or you are signed out. Sign in, then close and reopen Properties to check cloud-save support.");
                    pending.set(false);
                    refresh();
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.as_ref().unwrap().try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Err("Cloud-save discovery worker stopped unexpectedly".into())
                    }
                };
                drop(receiver.take());
                match result {
                    Ok(discovery) => {
                        availability.set(discovery.availability);
                        let supported = discovery.availability
                            == crate::domain::CloudSaveAvailability::Supported;
                        management.set_visible(supported);
                        *locations.borrow_mut() = discovery.locations.clone();
                        locations_row.set_subtitle(&cloud_location_summary(&discovery.locations));
                        locations_row.set_visible(supported);
                        open_save_folder
                            .set_sensitive(supported && !discovery.locations.is_empty());
                        inventory_row.set_visible(supported);
                        check_inventory.set_sensitive(supported);
                        if supported {
                            inventory_row.set_subtitle("Remote save files have not been checked");
                        }
                        enabled.set_sensitive(supported);
                        sync_now.set_sensitive(supported);
                        advanced.set_sensitive(supported);
                        status.set_label(match discovery.availability {
                            crate::domain::CloudSaveAvailability::Supported => {
                                "GOG cloud saves are supported for this game."
                            }
                            crate::domain::CloudSaveAvailability::Unsupported => {
                                "GOG reports cloud saves are disabled for this game."
                            }
                            crate::domain::CloudSaveAvailability::Unavailable => discovery
                                .reason
                                .as_deref()
                                .unwrap_or("GOG cloud-save metadata is unavailable for this game."),
                            crate::domain::CloudSaveAvailability::Unknown => discovery
                                .reason
                                .as_deref()
                                .unwrap_or("Cloud-save discovery failed; retry is available."),
                        });
                        button.set_visible(
                            discovery.availability == crate::domain::CloudSaveAvailability::Unknown,
                        );
                    }
                    Err(error) => {
                        status.add_css_class("error");
                        status.set_label(&notifications::failure_message(
                            "Cloud-save discovery failed", &error,
                        ));
                    }
                }
                pending.set(false);
                refresh();
                glib::ControlFlow::Break
            });
        });
        let retry_row = adw::ActionRow::new();
        retry_row.set_title("Cloud-save support");
        retry_row.set_subtitle("Check GOG metadata again");
        retry_row.add_suffix(&retry);
        cloud_group.add(&retry_row);
    }
    cloud_group.add(&cloud_status);
    cloud_page.add(&cloud_group);
    cloud_page.add(&management);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchActionState {
    Idle,
    Preparing,
    Started,
}

#[cfg(test)]
struct TestBranchForget {
    before_open: mpsc::Receiver<()>,
    entered: mpsc::Sender<()>,
    finish: mpsc::Receiver<()>,
    error: Option<String>,
    disconnect: bool,
}

#[cfg(test)]
struct TestBranchControls {
    state: Rc<std::cell::Cell<BranchActionState>>,
    refresh: Rc<dyn Fn()>,
    started: Rc<dyn Fn()>,
}

#[cfg(test)]
thread_local! {
    static TEST_BRANCH_FORGET: RefCell<Option<VecDeque<TestBranchForget>>> = const { RefCell::new(None) };
    static TEST_BRANCH_CONTROLS: RefCell<Option<TestBranchControls>> = const { RefCell::new(None) };
}

#[allow(clippy::too_many_arguments)]
fn connect_branch_selection(
    branches: &[Option<String>],
    installed_branch: Option<&Option<String>>,
    selector: &gtk::DropDown,
    switch: &gtk::Button,
    forget: &gtk::Button,
    state: &Rc<std::cell::Cell<BranchActionState>>,
    forget_pending: &Rc<std::cell::Cell<bool>>,
    refusal: Rc<dyn Fn() -> Option<&'static str>>,
) -> Rc<dyn Fn()> {
    let refresh: Rc<dyn Fn()> = {
        let branches = branches.to_vec();
        let installed_branch = installed_branch.cloned();
        let selector = selector.downgrade();
        let switch = switch.downgrade();
        let forget = forget.downgrade();
        let state = state.clone();
        let forget_pending = forget_pending.clone();
        Rc::new(move || {
            let (Some(selector), Some(switch), Some(forget)) =
                (selector.upgrade(), switch.upgrade(), forget.upgrade())
            else {
                return;
            };
            let snapshot = || {
                (
                    selector.selected(),
                    state.get(),
                    forget_pending.get(),
                    refusal(),
                )
            };
            'refresh: loop {
                let current = snapshot();
                let branch = branches.get(current.0 as usize);
                let busy = current.3.or(current
                    .2
                    .then_some("Wait for the saved password to be forgotten."));
                let switch_reason = busy.or(match current.1 {
                BranchActionState::Preparing => Some("Wait for branch preparation to finish."),
                BranchActionState::Started => Some(
                    "Branch switch already started. Close and reopen Properties after it finishes.",
                ),
                BranchActionState::Idle => match branch {
                    None => Some("Choose a branch before switching."),
                    Some(branch) if Some(branch) == installed_branch.as_ref() => {
                        Some("This branch is already installed.")
                    }
                    Some(_) => None,
                },
            });
                let forget_reason = busy.or(if current.1 == BranchActionState::Preparing {
                    Some("Wait for branch preparation to finish before forgetting a password.")
                } else {
                    match branch {
                        None => Some("Choose a named branch before forgetting its password."),
                        Some(None) => Some("Master does not use a saved branch password."),
                        Some(Some(_)) => None,
                    }
                });
                for (button, reason) in [(&switch, switch_reason), (&forget, forget_reason)] {
                    button.set_tooltip_text(reason);
                    if snapshot() != current {
                        continue 'refresh;
                    }
                    button.set_sensitive(reason.is_none());
                    if snapshot() != current {
                        continue 'refresh;
                    }
                }
                break;
            }
        })
    };
    selector.connect_selected_notify({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    refresh();
    refresh
}

#[allow(clippy::too_many_arguments)]
fn wire_branch_actions(
    window: &adw::ApplicationWindow,
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
        .map(|profile| profile.user_id.clone());
    let product_id = game.product_id;
    let session = (online::account_session(), auth::session());
    let epoch = model.borrow().account_epoch;
    let closed = Rc::new(std::cell::Cell::new(false));
    window.connect_close_request({
        let closed = closed.clone();
        move |_| {
            closed.set(true);
            glib::Propagation::Proceed
        }
    });
    let refusal: Rc<dyn Fn() -> Option<&'static str>> = {
        let model = Rc::downgrade(model);
        let user_id = user_id.clone();
        let closed = closed.clone();
        Rc::new(move || {
            if user_id.as_ref().is_none_or(|id| id.is_empty()) {
                return Some("The account identity is unavailable. Reopen game settings.");
            }
            let current = !closed.get()
                && session == (online::account_session(), auth::session())
                && model.upgrade().is_some_and(|model| {
                    model.try_borrow().is_ok_and(|state| {
                        state.account_epoch == epoch
                            && !state.logout_pending
                            && state
                                .account_profile
                                .as_ref()
                                .map(|profile| &profile.user_id)
                                == user_id.as_ref()
                    })
                });
            (!current).then_some("The account changed. Reopen game settings.")
        })
    };
    let action_state = Rc::new(std::cell::Cell::new(BranchActionState::Idle));
    let forget_pending = Rc::new(std::cell::Cell::new(false));
    let refresh_actions = connect_branch_selection(
        &branches,
        marker.galaxy_depot.as_ref().map(|depot| &depot.branch),
        &selector,
        &switch,
        &forget,
        &action_state,
        &forget_pending,
        refusal.clone(),
    );
    let switch_started: Rc<dyn Fn()> = {
        let action_state = action_state.clone();
        let status = status.downgrade();
        let refresh_actions = refresh_actions.clone();
        Rc::new(move || {
            action_state.set(BranchActionState::Started);
            if let Some(status) = status.upgrade() {
                status.set_label("Branch switch started.");
            }
            refresh_actions();
        })
    };
    #[cfg(test)]
    if TEST_BRANCH_FORGET.with(|requests| requests.borrow().is_some()) {
        TEST_BRANCH_CONTROLS.with(|controls| {
            *controls.borrow_mut() = Some(TestBranchControls {
                state: action_state.clone(),
                refresh: refresh_actions.clone(),
                started: switch_started.clone(),
            })
        });
    }
    {
        let branches = branches.clone();
        let selector = selector.downgrade();
        let status = status.downgrade();
        let window = window.downgrade();
        let action_state = action_state.clone();
        let forget_pending = forget_pending.clone();
        let refusal = refusal.clone();
        let refresh_actions = refresh_actions.clone();
        forget.connect_clicked(move |button| {
            let (Some(window), Some(selector), Some(status)) =
                (window.upgrade(), selector.upgrade(), status.upgrade())
            else {
                return;
            };
            if forget_pending.get()
                || closed.get()
                || !window.is_visible()
                || button.root().as_ref() != Some(window.upcast_ref())
                || selector.root().as_ref() != Some(window.upcast_ref())
                || status.root().as_ref() != Some(window.upcast_ref())
            {
                return;
            }
            if let Some(reason) = refusal() {
                status.set_label(reason);
                refresh_actions();
                return;
            }
            if action_state.get() == BranchActionState::Preparing {
                status.set_label(
                    "Wait for branch preparation to finish before forgetting a password.",
                );
                return;
            }
            let Some(branch) = branches.get(selector.selected() as usize) else {
                status.set_label("Choose a named branch before forgetting its password.");
                return;
            };
            let Some(branch) = branch else {
                status.set_label("Master does not use a saved branch password.");
                return;
            };
            let branch = branch.clone();
            forget_pending.set(true);
            refresh_actions();
            status.remove_css_class("error");
            status.set_label("Forgetting saved branch password…");
            let (sender, receiver) = mpsc::channel();
            match crate::profile_reset::begin_activity("forgetting a saved branch password") {
                Err(error) => {
                    let _ = sender.send(Err(format!("{error:#}")));
                }
                Ok(activity) => {
                    let user_id = user_id.clone().unwrap();
                    let branch = branch.clone();
                    #[cfg(test)]
                    let fixture = TEST_BRANCH_FORGET.with(|requests| {
                        requests.borrow_mut().as_mut().map(|requests| {
                            requests
                                .pop_front()
                                .expect("missing inert branch forget request")
                        })
                    });
                    std::thread::spawn(move || {
                        let result = (|| -> anyhow::Result<()> {
                            let _activity = activity;
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                fixture.before_open.recv_timeout(Duration::from_secs(10))?;
                            }
                            anyhow::ensure!(
                                session == (online::account_session(), auth::session()),
                                "The account changed before forgetting the password."
                            );
                            let store = StateStore::open()?;
                            anyhow::ensure!(
                                session == (online::account_session(), auth::session()),
                                "The account changed before forgetting the password."
                            );
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                fixture.entered.send(()).ok();
                                if let Some(error) = &fixture.error {
                                    anyhow::bail!("{error}");
                                }
                            }
                            crate::gog::depot_service::forget_one(
                                &store, &user_id, product_id, &branch,
                            )?;
                            #[cfg(test)]
                            if let Some(fixture) = fixture.as_ref() {
                                fixture.finish.recv_timeout(Duration::from_secs(10))?;
                            }
                            Ok(())
                        })();
                        #[cfg(test)]
                        if fixture.as_ref().is_some_and(|fixture| fixture.disconnect) {
                            return;
                        }
                        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
                    });
                }
            }
            let button = button.downgrade();
            let window = window.downgrade();
            let selector = selector.downgrade();
            let status = status.downgrade();
            let closed = closed.clone();
            let refusal = refusal.clone();
            let forget_pending = forget_pending.clone();
            let refresh_actions = refresh_actions.clone();
            let mut receiver = Some(receiver);
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let (Some(window), Some(button), Some(selector), Some(status)) = (
                    window.upgrade(),
                    button.upgrade(),
                    selector.upgrade(),
                    status.upgrade(),
                ) else {
                    return glib::ControlFlow::Break;
                };
                if closed.get()
                    || !window.is_visible()
                    || button.root().as_ref() != Some(window.upcast_ref())
                    || selector.root().as_ref() != Some(window.upcast_ref())
                    || status.root().as_ref() != Some(window.upcast_ref())
                {
                    return glib::ControlFlow::Break;
                }
                if let Some(reason) = refusal() {
                    drop(receiver.take());
                    status.set_label(reason);
                    forget_pending.set(false);
                    refresh_actions();
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.as_ref().unwrap().try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Err("Password removal worker stopped unexpectedly. Try again.".into())
                    }
                };
                drop(receiver.take());
                match result {
                    Ok(()) => status
                        .set_label(&format!("Saved password for branch ‘{branch}’ forgotten.")),
                    Err(error) => {
                        status.add_css_class("error");
                        status.set_label(&notifications::failure_message(
                            "Could not forget password",
                            &error,
                        ));
                    }
                }
                forget_pending.set(false);
                refresh_actions();
                glib::ControlFlow::Break
            });
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
    let epoch = model.borrow().account_epoch;
    let model = model.clone();
    switch.connect_clicked(move |_| {
        if forget_pending.get() || action_state.get() != BranchActionState::Idle {
            return;
        }
        if let Some(reason) = refusal() {
            status.set_label(reason);
            refresh_actions();
            return;
        }
        if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
            status.set_label("The account changed. Reopen game settings.");
            return;
        }
        let authentication = match super::download_chooser::current_depot_session(&model) {
            Ok(authentication) => authentication,
            Err(error) => {
                status.set_label(&error.to_string());
                status.add_css_class("error");
                return;
            }
        };
        let session = (online::account_session(), auth::session());
        let Some(library_root) = library_root.clone() else {
            status.set_label("Installation has no library root.");
            return;
        };
        let Some(selected_branch) = branches.get(selector.selected() as usize).cloned() else {
            status.set_label("Choose a branch before switching.");
            return;
        };
        if marker
            .galaxy_depot
            .as_ref()
            .is_some_and(|depot| depot.branch == selected_branch)
        {
            status.set_label("This branch is already installed.");
            return;
        }
        action_state.set(BranchActionState::Preparing);
        refresh_actions();
        status.remove_css_class("error");
        status.set_label("Authenticating and preparing branch switch…");
        let supplied = (!password.text().is_empty())
            .then(|| crate::gog::depot_service::BranchPassword::new(password.text().to_string()));
        let request = crate::gog::depot_service::BuildRequest {
            user_id: authentication.token.user_id.clone(),
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
                let builds = crate::gog::depot_service::list_builds(
                    &store,
                    &client,
                    &authentication,
                    &request,
                )?;
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
                    &authentication,
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
        let action_state = action_state.clone();
        let refresh_actions = refresh_actions.clone();
        let model = model.clone();
        let switch_started = switch_started.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
                || session.0 != online::account_session()
                || !auth::session_is_current(session.1)
            {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    switch_started();
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    status.add_css_class("error");
                    status.set_label(&format!("Could not switch branch: {error}"));
                    action_state.set(BranchActionState::Idle);
                    refresh_actions();
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.add_css_class("error");
                    status
                        .set_label("Branch preparation stopped unexpectedly. Try switching again.");
                    action_state.set(BranchActionState::Idle);
                    refresh_actions();
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
            let _ = sender.send(prepare_migration_choices(&game, &installed, session));
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
            let dialog = dialog.downgrade();
            move |_| {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
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
    session: u64,
) -> anyhow::Result<MigrationChoices> {
    online::with_account_session(session, || Ok(()))?;
    let _activity = crate::profile_reset::begin_activity("inspecting installation sources")?;
    let config = crate::storage::read_config()?;
    let store = online::with_account_session(session, StateStore::open)?;
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
    let proceed = view.proceed.clone();
    let dialog = view.dialog.downgrade();
    let status = view.status.clone();
    let progress = view.progress.clone();
    let close = view.close.clone();
    let closed = view.closed.clone();
    let started = view.started.clone();
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
    connect_windows_action(&proceed, parent, false, windows_product, move |button| {
        let Some(dialog) = dialog.upgrade() else {
            return;
        };
        let view = MigrationView {
            dialog,
            selector: selector.clone(),
            status: status.clone(),
            progress: progress.clone(),
            proceed: button.clone(),
            close: close.clone(),
            closed: closed.clone(),
            started: started.clone(),
        };
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
    let auth_session = auth::session();
    let (sender, receiver) = mpsc::channel();
    let (stages, stage_receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                online::account_session() == session && auth::session() == auth_session,
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
                    anyhow::ensure!(
                        online::account_session() == session && auth::session() == auth_session,
                        "Account changed; reopen game properties."
                    );
                    let plan = prepare_offline_migration_plan(
                        &StateStore::open()?,
                        &game,
                        &installed,
                        library,
                        candidate,
                    )?;
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
                online::account_session() == session && auth::session() == auth_session,
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
                online::account_session() == session && auth::session() == auth_session,
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

fn prepare_offline_migration_plan(
    store: &StateStore,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
    library: &crate::config::GameLibrary,
    candidate: &crate::installation::InstallerCandidate,
) -> anyhow::Result<crate::domain::InstalledGame> {
    let preferences = store.game_preferences(game.product_id)?;
    let mut plan = offline_installation_plan(game.product_id, library, &game.slug, candidate);
    plan.last_played_at = installed.last_played_at;
    plan.playtime_seconds = installed.playtime_seconds;
    plan.created_at = installed.created_at;
    retain_offline_launch_preferences(&mut plan, preferences.as_ref(), Some(installed));
    Ok(plan)
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
    fn is_current_local(&self) -> bool {
        self.auth == auth::session()
            && self.online == online::account_session()
            && self.model.upgrade().is_some_and(|model| {
                model
                    .try_borrow()
                    .is_ok_and(|state| state.account_epoch == self.epoch && !state.logout_pending)
            })
    }

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
    let activity = match crate::profile_reset::begin_activity("saving launch settings") {
        Ok(activity) => activity,
        Err(error) => {
            status.set_label(&format!("Could not save: {error}"));
            status.add_css_class("error");
            return;
        }
    };
    *save_state.submitted.borrow_mut() = Some(submitted.clone());
    status.remove_css_class("error");
    status.set_label("Saving launch settings…");
    let receiver = update_policies::policy_request(move || {
        let _activity = activity;
        anyhow::ensure!(
            online::account_session() == session,
            "The account changed before saving launch settings"
        );
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
            StateStore::open()?.set_game_launch_preferences(
                updated.product_id,
                (!relative.as_os_str().is_empty()).then_some(relative.as_path()),
                &updated.launch_arguments,
            )
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
    #[ignore = "private p386 HOME/all XDG/TMP, GTK and D-Bus; no cloud actions"]
    fn cloud_components_release_after_detachment() {
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
                    .starts_with("/tmp/ludomere-p386-"),
                "{key}"
            );
        }
        assert!(!crate::identity::database().exists());
        adw::init().unwrap();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(8);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn nodes(root: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut result = vec![root.clone()];
            for child in std::iter::successors(root.first_child(), |child| child.next_sibling()) {
                result.extend(nodes(&child));
            }
            result
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudComponentLifetimeTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let model = Rc::new(RefCell::new(AppModel::default()));
        for opened in [false, true] {
            let window = adw::ApplicationWindow::builder()
                .application(&app)
                .default_width(900)
                .default_height(1000)
                .build();
            let page = adw::PreferencesPage::new();
            let game = serde_json::from_value(serde_json::json!({
                "product_id": 9386001, "library_id": "inert",
                "installation_directory": std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("absent-game"),
                "installer_files": [], "installer_complete": true, "installer_operating_system": "windows",
                "launch_arguments": [], "state": "installed", "playtime_seconds": 0, "created_at": 1, "updated_at": 1
            })).unwrap();
            let origin = CloudActionSession {
                auth: auth::session(),
                online: online::account_session(),
                epoch: model.borrow().account_epoch,
                model: Rc::downgrade(&model),
            };
            populate_cloud_settings(
                &window,
                &model,
                &page,
                &game,
                crate::state::CloudSaveRecord {
                    preference: crate::domain::CloudSavePreference::Undecided,
                    availability: crate::domain::CloudSaveAvailability::Supported,
                    locations: Vec::new(),
                    metadata_build_id: None,
                    metadata_checked_at: None,
                    metadata_error: None,
                    last_successful_sync: None,
                    status: crate::domain::CloudSaveStatus::NeverSynced,
                    error: None,
                    conflicts: Vec::new(),
                },
                &origin,
            );
            window.set_content(Some(&page));
            window.present();
            wait(|| page.is_mapped());
            let inventory = nodes(page.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<adw::ActionRow>().ok())
                .find(|row| row.title() == "GOG Cloud storage")
                .unwrap();
            let check = nodes(inventory.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some("Check now"))
                .unwrap();
            let group = inventory
                .ancestor(adw::PreferencesGroup::static_type())
                .and_downcast::<adw::PreferencesGroup>()
                .unwrap();
            // Ordinary detach/reinsert retains the same live controls.
            group.remove(&inventory);
            group.add(&inventory);
            wait(|| check.is_mapped());
            assert!(check.is_sensitive());
            let weak_inventory = inventory.downgrade();
            let weak_check = check.downgrade();
            group.remove(&inventory);
            drop(check);
            drop(inventory);
            wait(|| weak_inventory.upgrade().is_none() && weak_check.upgrade().is_none());
            assert!(window.is_visible() && page.is_mapped());

            let advanced = nodes(page.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
                .find(|button| button.label().as_deref() == Some("Advanced…"))
                .unwrap();
            let popover = advanced.popover().unwrap();
            let force = nodes(popover.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .filter(|button| {
                    matches!(
                        button.label().as_deref(),
                        Some("Force download" | "Force upload")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(force.len(), 2);
            if opened {
                for _ in 0..2 {
                    advanced.popup();
                    wait(|| popover.is_mapped() && force.iter().all(|button| button.is_mapped()));
                    assert!(force.iter().all(|button| button.is_sensitive()));
                    advanced.popdown();
                    wait(|| !popover.is_mapped());
                }
            }
            let weak_popover = popover.downgrade();
            let weak_force = force
                .iter()
                .map(|button| button.downgrade())
                .collect::<Vec<_>>();
            advanced.set_popover(gtk::Popover::NONE);
            drop(force);
            drop(popover);
            wait(|| {
                weak_popover.upgrade().is_none()
                    && weak_force.iter().all(|button| button.upgrade().is_none())
            });
            assert!(window.is_visible() && page.is_mapped());
            assert!(window.visible_dialog().is_none());
            window.set_content(gtk::Widget::NONE);
            window.close();
            window.destroy();
            // Whole-page retention is a separate unresolved observation, not this proof.
        }
        assert!(!crate::identity::database().exists());
        assert!(crate::profile_reset::reserve().is_ok());
    }

    #[test]
    #[ignore = "private p384 HOME/all XDG/TMP, GTK and D-Bus; captured directory launches only"]
    fn backup_folder_open_is_responsive_local_and_origin_guarded() {
        use super::super::widgets::file_open::DIRECTORY_LAUNCHES;
        use crate::domain::CloudSaveAvailability as Availability;
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
                    .starts_with("/tmp/ludomere-p384-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn heartbeat() {
            let tick = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(150), {
                let tick = tick.clone();
                move || tick.set(true)
            });
            wait(|| tick.get());
        }
        fn nodes(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
            let mut result = vec![root.as_ref().clone()];
            for child in
                std::iter::successors(root.as_ref().first_child(), |child| child.next_sibling())
            {
                result.extend(nodes(&child));
            }
            result
        }
        fn button(page: &adw::PreferencesPage, title: &str) -> gtk::Button {
            nodes(page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some(title))
                .unwrap()
        }
        struct View {
            window: adw::ApplicationWindow,
            page: adw::PreferencesPage,
            backup: gtk::Button,
            status: gtk::Label,
            path: std::path::PathBuf,
        }
        fn host(
            app: &adw::Application,
            model: &Rc<RefCell<AppModel>>,
            id: i64,
            availability: Availability,
        ) -> View {
            let window = adw::ApplicationWindow::builder()
                .application(app)
                .default_width(800)
                .default_height(720)
                .build();
            let page = adw::PreferencesPage::new();
            let game = serde_json::from_value(serde_json::json!({
                "product_id": id, "library_id": "inert", "installation_directory": std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("absent-game"),
                "installer_files": [], "installer_complete": true, "installer_operating_system": "windows",
                "launch_arguments": [], "state": "installed", "playtime_seconds": 0, "created_at": 1, "updated_at": 1
            })).unwrap();
            populate_cloud_settings(
                &window,
                model,
                &page,
                &game,
                crate::state::CloudSaveRecord {
                    preference: crate::domain::CloudSavePreference::Undecided,
                    availability,
                    locations: Vec::new(),
                    metadata_build_id: None,
                    metadata_checked_at: None,
                    metadata_error: None,
                    last_successful_sync: None,
                    status: crate::domain::CloudSaveStatus::NeverSynced,
                    error: None,
                    conflicts: Vec::new(),
                },
                &CloudActionSession {
                    auth: auth::session(),
                    online: online::account_session(),
                    epoch: model.borrow().account_epoch,
                    model: Rc::downgrade(model),
                },
            );
            window.set_content(Some(&page));
            window.present();
            let backup = button(&page, "Open backup folder");
            let status = find_named_descendant(page.upcast_ref(), "cloud-backup-status")
                .and_downcast::<gtk::Label>()
                .unwrap();
            wait(|| backup.is_mapped());
            View {
                window,
                page,
                backup,
                status,
                path: crate::cloud_saves::sync::backup_directory(id),
            }
        }
        struct Attempt {
            start: mpsc::Sender<()>,
            inspected: mpsc::Receiver<()>,
            finish: mpsc::Sender<()>,
        }
        fn queue(disconnect: bool) -> Attempt {
            let (start, before_io) = mpsc::channel();
            let (inspected, capture) = mpsc::channel();
            let (finish, release) = mpsc::channel();
            TEST_CLOUD_BACKUPS.with(|requests| {
                requests
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .push_back(TestCloudBackup {
                        gtk_thread: std::thread::current().id(),
                        before_io,
                        inspected,
                        finish: release,
                        disconnect,
                    })
            });
            Attempt {
                start,
                inspected: capture,
                finish,
            }
        }
        fn registered() {
            let error = crate::profile_reset::reserve().err().unwrap().to_string();
            assert!(
                error.contains("opening cloud-save backup folder"),
                "{error}"
            );
        }
        fn launches() -> usize {
            DIRECTORY_LAUNCHES.with(|paths| paths.borrow().as_ref().unwrap().len())
        }
        fn discover(view: &View, availability: Availability) {
            let (start, before_entry) = mpsc::channel();
            let (entered, capture) = mpsc::channel();
            let (finish, result) = mpsc::channel();
            TEST_CLOUD_DISCOVERIES.with(|requests| {
                requests
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .push_back(TestCloudDiscovery {
                        before_entry,
                        entered,
                        result,
                    })
            });
            let retry = button(&view.page, "Retry metadata discovery");
            // Retain and expose the real receiver for a second availability change;
            // normal UI hides Retry after Supported. No backend discovery is run.
            retry.set_visible(true);
            retry.emit_clicked();
            start.send(()).unwrap();
            capture.recv_timeout(Duration::from_secs(5)).unwrap();
            finish
                .send(Ok(crate::domain::CloudSaveDiscovery {
                    availability,
                    ..Default::default()
                }))
                .unwrap();
            let status = find_named_descendant(view.page.upcast_ref(), "cloud-save-status")
                .and_downcast::<gtk::Label>()
                .unwrap();
            wait(|| !status.text().starts_with("Checking GOG"));
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudBackupOpenTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        Config::default().save().unwrap();
        let model = Rc::new(RefCell::new(AppModel::default()));
        TEST_CLOUD_BACKUPS.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        TEST_CLOUD_DISCOVERIES.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        DIRECTORY_LAUNCHES.with(|paths| *paths.borrow_mut() = Some(Vec::new()));

        let view = host(&app, &model, 9384001, Availability::Supported);
        let attempt = queue(false);
        view.backup.emit_clicked();
        view.backup.emit_clicked();
        registered();
        heartbeat();
        assert!(!view.path.exists());
        assert_eq!(view.status.text(), "Preparing backup folder…");
        assert!(!view.backup.is_sensitive());
        assert!(button(&view.page, "Choose…").is_sensitive());
        assert!(button(&view.page, "Sync now").is_sensitive());
        assert_eq!(launches(), 0);
        attempt.start.send(()).unwrap();
        attempt
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(view.path.is_dir());
        registered();
        heartbeat();
        assert_eq!(launches(), 0);
        attempt.finish.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert_eq!(view.status.text(), "Backup folder is ready.");
        assert_eq!(
            DIRECTORY_LAUNCHES.with(|paths| paths.borrow().as_ref().unwrap().clone()),
            vec![view.path.clone()]
        );
        assert!(crate::profile_reset::reserve().is_ok());

        // Restoration can synchronously begin a fresh actual click.
        let first = queue(false);
        let second = queue(false);
        view.backup.emit_clicked();
        let fired = Rc::new(std::cell::Cell::new(false));
        let handler = view.backup.connect_sensitive_notify({
            let fired = fired.clone();
            move |button| {
                if button.is_sensitive() && !fired.replace(true) {
                    button.emit_clicked();
                }
            }
        });
        first.start.send(()).unwrap();
        first
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        first.finish.send(()).unwrap();
        wait(|| fired.get());
        view.backup.disconnect(handler);
        heartbeat();
        assert!(!view.backup.is_sensitive());
        assert_eq!(view.status.text(), "Preparing backup folder…");
        registered();
        second.start.send(()).unwrap();
        second
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        second.finish.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert_eq!(launches(), 3);
        view.window.close();

        let view = host(&app, &model, 9384002, Availability::Supported);
        std::fs::write(&view.path, b"not a directory").unwrap();
        let attempt = queue(false);
        view.backup.emit_clicked();
        attempt.start.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert!(
            view.status
                .text()
                .contains("Could not create the cloud-save backup folder")
        );
        assert_eq!(launches(), 3);
        std::fs::remove_file(&view.path).unwrap();
        std::fs::write(Config::path(), "invalid = [").unwrap();
        let attempt = queue(false);
        view.backup.emit_clicked();
        attempt.start.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert!(view.status.text().contains("Reading storage configuration"));
        assert_eq!(launches(), 3);
        Config::default().save().unwrap();
        std::fs::remove_dir(&view.path).unwrap();
        let reservation = crate::profile_reset::reserve().unwrap();
        view.backup.emit_clicked();
        wait(|| view.backup.is_sensitive());
        assert!(view.status.text().contains("Profile reset"));
        assert!(!view.path.exists());
        drop(reservation);
        let attempt = queue(true);
        view.backup.emit_clicked();
        attempt.start.send(()).unwrap();
        attempt
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert!(view.status.text().contains("inspection stopped"));
        assert_eq!(launches(), 3);
        view.window.close();

        // A terminal text observer can revoke the page before the guarded launcher runs.
        let view = host(&app, &model, 9384004, Availability::Supported);
        let attempt = queue(false);
        view.backup.emit_clicked();
        let handler = view.status.connect_label_notify({
            let model = model.clone();
            move |label| {
                if label.text() == "Backup folder is ready." {
                    model.borrow_mut().account_epoch += 1;
                }
            }
        });
        attempt.start.send(()).unwrap();
        attempt
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.status.text() == "Backup folder is ready.");
        view.status.disconnect(handler);
        assert!(!view.backup.is_sensitive());
        assert_eq!(launches(), 3);
        view.window.close();

        let view = host(&app, &model, 9384003, Availability::Unknown);
        view.backup.emit_clicked();
        assert!(!view.path.exists());
        discover(&view, Availability::Supported);
        let attempt = queue(false);
        view.backup.emit_clicked();
        discover(&view, Availability::Supported);
        assert!(!view.backup.is_sensitive());
        discover(&view, Availability::Unsupported);
        attempt.start.send(()).unwrap();
        attempt
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.status.text().contains("support changed"));
        assert!(!view.backup.is_sensitive());
        assert_eq!(launches(), 3);
        view.window.close();

        for (index, case) in [
            "epoch", "logout", "close", "detach", "destroy", "online", "auth",
        ]
        .into_iter()
        .enumerate()
        {
            model.borrow_mut().logout_pending = false;
            let view = host(
                &app,
                &model,
                9384010 + index as i64,
                Availability::Supported,
            );
            let attempt = queue(false);
            view.backup.emit_clicked();
            if case == "online" {
                attempt.start.send(()).unwrap();
                attempt
                    .inspected
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            match case {
                "epoch" => model.borrow_mut().account_epoch += 1,
                "logout" => model.borrow_mut().logout_pending = true,
                "close" => {
                    view.window.close();
                }
                "detach" => view.window.set_content(gtk::Widget::NONE),
                "destroy" => {
                    let row = view
                        .backup
                        .ancestor(adw::ActionRow::static_type())
                        .and_downcast::<adw::ActionRow>()
                        .unwrap();
                    row.remove(&view.backup);
                }
                "online" => online::invalidate_library_session(),
                "auth" => auth::invalidate_session(),
                _ => unreachable!(),
            }
            let weak = view.backup.downgrade();
            drop(view.backup);
            heartbeat();
            if case == "destroy" {
                assert!(weak.upgrade().is_none());
            }
            registered();
            if case != "online" {
                attempt.start.send(()).unwrap();
            }
            if case != "auth" {
                if case != "online" {
                    attempt
                        .inspected
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                attempt.finish.send(()).unwrap();
            }
            wait(|| crate::profile_reset::reserve().is_ok());
            heartbeat();
            assert_eq!(launches(), 3, "{case}");
            if case == "auth" {
                assert!(!view.path.exists());
            }
            view.window.destroy();
        }

        // A newly opened current local page remains usable while signed out/offline.
        assert!(!auth::session_is_current(auth::session()));
        model.borrow_mut().logout_pending = false;
        let view = host(&app, &model, 9384020, Availability::Supported);
        let attempt = queue(false);
        view.backup.emit_clicked();
        attempt.start.send(()).unwrap();
        attempt
            .inspected
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.backup.is_sensitive());
        assert_eq!(launches(), 4);
        view.window.destroy();
        assert!(
            TEST_CLOUD_BACKUPS.with(|requests| requests.borrow_mut().take().unwrap().is_empty())
        );
        assert!(
            TEST_CLOUD_DISCOVERIES.with(|requests| requests
                .borrow_mut()
                .take()
                .unwrap()
                .is_empty())
        );
        DIRECTORY_LAUNCHES.with(|paths| *paths.borrow_mut() = None);
    }

    #[test]
    #[ignore = "private p379 HOME/all XDG/TMP, GTK and D-Bus; synthetic SQLite and inert pickers only"]
    fn cloud_override_saves_preserve_order_sessions_and_responsive_controls() {
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
                    .starts_with("/tmp/ludomere-p379-"),
                "{key}"
            );
        }
        assert!(!crate::identity::database().exists());
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn heartbeat() {
            let tick = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(150), {
                let tick = tick.clone();
                move || tick.set(true)
            });
            wait(|| tick.get());
        }
        fn nodes(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
            let mut result = vec![root.as_ref().clone()];
            let mut child = root.as_ref().first_child();
            while let Some(widget) = child {
                result.extend(nodes(&widget));
                child = widget.next_sibling();
            }
            result
        }
        struct View {
            window: adw::ApplicationWindow,
            page: adw::PreferencesPage,
            choose: gtk::Button,
            retry: gtk::Button,
            status: gtk::Label,
            row: adw::ActionRow,
            locations: Rc<RefCell<Vec<crate::domain::CloudSaveLocation>>>,
        }
        fn host(
            app: &adw::Application,
            model: &Rc<RefCell<AppModel>>,
            game: &crate::domain::InstalledGame,
        ) -> View {
            let window = adw::ApplicationWindow::builder()
                .application(app)
                .default_width(800)
                .default_height(720)
                .build();
            let page = adw::PreferencesPage::new();
            populate_cloud_settings(
                &window,
                model,
                &page,
                game,
                StateStore::open()
                    .unwrap()
                    .cloud_save_record(game.product_id)
                    .unwrap(),
                &CloudActionSession {
                    auth: auth::session(),
                    online: online::account_session(),
                    epoch: model.borrow().account_epoch,
                    model: Rc::downgrade(model),
                },
            );
            window.set_content(Some(&page));
            window.present();
            let choose = find_named_descendant(page.upcast_ref(), "cloud-location-choose")
                .and_downcast::<gtk::Button>()
                .unwrap();
            let retry = find_named_descendant(page.upcast_ref(), "cloud-discovery-retry")
                .and_downcast::<gtk::Button>()
                .unwrap();
            let status = find_named_descendant(page.upcast_ref(), "cloud-save-status")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let row = nodes(&page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<adw::ActionRow>().ok())
                .find(|row| row.title() == "Save locations")
                .unwrap();
            let locations =
                TEST_CLOUD_LOCATION_STATE.with(|state| state.borrow().upgrade().unwrap());
            wait(|| choose.is_mapped());
            View {
                window,
                page,
                choose,
                retry,
                status,
                row,
                locations,
            }
        }
        fn picker() -> mpsc::Sender<Result<gio::File, glib::Error>> {
            let (sender, receiver) = mpsc::channel();
            TEST_CLOUD_PICKERS
                .with(|requests| requests.borrow_mut().as_mut().unwrap().push_back(receiver));
            sender
        }
        struct Save {
            start: mpsc::Sender<()>,
            entered: mpsc::Receiver<()>,
            finish: mpsc::Sender<()>,
        }
        fn save(disconnect: bool) -> Save {
            let (start, before_open) = mpsc::channel();
            let (entered, capture) = mpsc::channel();
            let (finish, done) = mpsc::channel();
            TEST_CLOUD_LOCATION_SAVES.with(|requests| {
                requests
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .push_back(TestCloudLocationSave {
                        before_open,
                        entered,
                        finish: done,
                        disconnect,
                    })
            });
            Save {
                start,
                entered: capture,
                finish,
            }
        }
        fn saving() {
            wait(|| crate::profile_reset::reserve().is_err());
            assert!(
                crate::profile_reset::reserve()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("saving cloud-save directory")
            );
        }
        fn cancel(sender: mpsc::Sender<Result<gio::File, glib::Error>>) {
            sender
                .send(Err(glib::Error::new(
                    gtk::DialogError::Dismissed,
                    "cancelled",
                )))
                .unwrap();
        }
        TEST_CLOUD_PICKERS.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        TEST_CLOUD_LOCATION_SAVES.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        TEST_CLOUD_DISCOVERIES.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudOverrideTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let game: crate::domain::InstalledGame = serde_json::from_value(serde_json::json!({
            "product_id": 9379002, "library_id": "inert", "installation_directory": "/inert/not-created",
            "primary_executable": null, "installer_files": [], "installer_complete": true,
            "installer_operating_system": "windows", "launch_arguments": [], "state": "installed", "playtime_seconds": 0, "created_at": 1, "updated_at": 1
        })).unwrap();
        let store = StateStore::open().unwrap();
        let old = vec![crate::domain::CloudSaveLocation {
            name: "old".into(),
            path: std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("old"),
            remote_namespace: "old".into(),
            user_override: true,
        }];
        store
            .set_cloud_save_locations(game.product_id, &old)
            .unwrap();
        // Observe full-container retention separately from the owned callback lifetime proof.
        let baseline = host(&app, &model, &game);
        baseline.window.set_content(gtk::Widget::NONE);
        baseline.window.close();
        heartbeat();
        let baseline_page = baseline.page.downgrade();
        drop(baseline);
        heartbeat();
        eprintln!(
            "[p379] never-clicked-detached-page-retained={}",
            baseline_page.upgrade().is_some()
        );
        let view = host(&app, &model, &game);
        let chosen = std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("new-choice");

        // An open picker owns only UI admission, never profile activity.
        let selected = picker();
        view.choose.emit_clicked();
        assert!(!view.choose.is_sensitive() && !view.retry.is_sensitive());
        view.choose.emit_clicked();
        view.retry.emit_clicked();
        assert!(crate::profile_reset::reserve().is_ok());
        cancel(selected);
        wait(|| view.choose.is_sensitive() && view.retry.is_sensitive());
        assert_eq!(*view.locations.borrow(), old);
        assert_eq!(
            store.cloud_save_record(game.product_id).unwrap().locations,
            old
        );
        let selected = picker();
        view.choose.emit_clicked();
        selected
            .send(Ok(gio::File::for_uri("https://invalid.example/inert")))
            .unwrap();
        wait(|| view.choose.is_sensitive());
        assert!(view.status.text().contains("local directory"));
        assert!(crate::profile_reset::reserve().is_ok());

        // Real SQLite contention cannot freeze GTK; UI changes only after commit/result.
        let blocker = rusqlite::Connection::open(crate::identity::database()).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let attempt = save(false);
        let selected = picker();
        view.choose.emit_clicked();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        saving();
        assert!(attempt.entered.try_recv().is_err());
        attempt.start.send(()).unwrap();
        attempt
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        heartbeat();
        assert!(view.status.text().starts_with("Saving override"));
        assert_eq!(*view.locations.borrow(), old);
        assert!(!view.choose.is_sensitive() && !view.retry.is_sensitive());
        view.retry.emit_clicked();
        view.choose.emit_clicked();
        blocker.execute_batch("COMMIT").unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.choose.is_sensitive());
        let fresh = store.cloud_save_record(game.product_id).unwrap().locations;
        assert_eq!(fresh[0].path, chosen);
        assert!(fresh[0].user_override);
        assert_eq!(
            *view.locations.borrow(),
            fresh,
            "Open Folder reads this exact shared state"
        );
        assert_eq!(
            view.row.subtitle().as_deref(),
            Some(cloud_location_summary(&fresh).as_str())
        );
        assert_eq!(view.status.text(), "Override save directory updated");
        assert!(
            !chosen.exists(),
            "saving a preference must not create/traverse the selected directory"
        );

        // A real UPDATE failure preserves both persisted and displayed locations.
        blocker.execute_batch("CREATE TRIGGER reject_override BEFORE UPDATE OF locations_json ON cloud_save_settings BEGIN SELECT RAISE(ABORT, 'inert refusal'); END;").unwrap();
        let attempt = save(false);
        let selected = picker();
        view.choose.emit_clicked();
        selected
            .send(Ok(gio::File::for_path(chosen.join("rejected"))))
            .unwrap();
        saving();
        attempt.start.send(()).unwrap();
        wait(|| view.choose.is_sensitive());
        assert!(view.status.text().contains("Could not save override"));
        assert_eq!(
            store.cloud_save_record(game.product_id).unwrap().locations,
            fresh
        );
        assert_eq!(*view.locations.borrow(), fresh);
        blocker
            .execute_batch("DROP TRIGGER reject_override")
            .unwrap();

        // Reset refusal happens after the picker, before any persistence worker.
        let selected = picker();
        view.choose.emit_clicked();
        let reset = crate::profile_reset::reserve().unwrap();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        wait(|| view.choose.is_sensitive());
        assert!(view.status.text().contains("Profile reset"));
        drop(reset);

        // Each control's sensitivity observer can admit fresh work; the old result must not unlock it.
        for reenter_choose in [true, false] {
            let first = picker();
            view.choose.emit_clicked();
            let fired = Rc::new(std::cell::Cell::new(false));
            let (start, before_entry) = mpsc::channel();
            let (entered, capture) = mpsc::channel();
            let (finish, result) = mpsc::channel();
            let second = if reenter_choose {
                Some(picker())
            } else {
                TEST_CLOUD_DISCOVERIES.with(|requests| {
                    requests
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .push_back(TestCloudDiscovery {
                            before_entry,
                            entered,
                            result,
                        })
                });
                None
            };
            let control = if reenter_choose {
                &view.choose
            } else {
                &view.retry
            };
            let handler = control.connect_sensitive_notify({
                let fired = fired.clone();
                move |button| {
                    if button.is_sensitive() && !fired.replace(true) {
                        button.emit_clicked();
                    }
                }
            });
            cancel(first);
            wait(|| fired.get());
            control.disconnect(handler);
            assert!(!view.choose.is_sensitive() && !view.retry.is_sensitive());
            view.choose.emit_clicked();
            view.retry.emit_clicked();
            if let Some(second) = second {
                assert!(crate::profile_reset::reserve().is_ok());
                cancel(second);
            } else {
                assert!(view.status.text().contains("Checking GOG"));
                start.send(()).unwrap();
                capture.recv_timeout(Duration::from_secs(5)).unwrap();
                finish
                    .send(Ok(crate::domain::CloudSaveDiscovery {
                        availability: crate::domain::CloudSaveAvailability::Unknown,
                        locations: fresh.clone(),
                        ..Default::default()
                    }))
                    .unwrap();
            }
            wait(|| view.choose.is_sensitive() && view.retry.is_sensitive());
        }

        // A disconnected result cannot claim success or change the displayed target.
        let attempt = save(true);
        let selected = picker();
        view.choose.emit_clicked();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        saving();
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.choose.is_sensitive());
        assert!(view.status.text().contains("stopped unexpectedly"));

        // Original online generation is checked after activity admission and before DB access.
        let attempt = save(false);
        let selected = picker();
        view.choose.emit_clicked();
        selected
            .send(Ok(gio::File::for_path(chosen.join("stale"))))
            .unwrap();
        saving();
        online::invalidate_library_session();
        heartbeat();
        assert!(view.status.text().contains("session changed"));
        assert!(!view.choose.is_sensitive() && !view.retry.is_sensitive());
        attempt.start.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(attempt.entered.try_recv().is_err());
        assert_eq!(
            store.cloud_save_record(game.product_id).unwrap().locations,
            fresh
        );
        view.window.destroy();

        // Closing/detaching retires UI only; admitted persistence remains tracked until done.
        let view = host(&app, &model, &game);
        let attempt = save(false);
        let selected = picker();
        view.choose.emit_clicked();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        saving();
        view.window.set_content(gtk::Widget::NONE);
        view.window.close();
        heartbeat();
        assert!(crate::profile_reset::reserve().is_err());
        assert_eq!(view.status.text(), "Saving override save directory…");
        assert_eq!(*view.locations.borrow(), fresh);
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        heartbeat();
        assert_eq!(view.status.text(), "Saving override save directory…");
        assert_eq!(*view.locations.borrow(), fresh);
        let weak_page = view.page.downgrade();
        drop(view);
        heartbeat();
        eprintln!(
            "[p379] completed-save-detached-page-retained={}",
            weak_page.upgrade().is_some()
        );

        // The owned Choose callback/receiver must release its button before the worker completes.
        let view = host(&app, &model, &game);
        let attempt = save(false);
        let selected = picker();
        view.choose.emit_clicked();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        saving();
        let override_row = nodes(&view.page)
            .into_iter()
            .filter_map(|widget| widget.downcast::<adw::ActionRow>().ok())
            .find(|row| row.title() == "Override save directory")
            .unwrap();
        let group = override_row
            .ancestor(adw::PreferencesGroup::static_type())
            .and_downcast::<adw::PreferencesGroup>()
            .unwrap();
        group.remove(&override_row);
        let weak_choose = view.choose.downgrade();
        let window = view.window.clone();
        drop((override_row, view));
        wait(|| weak_choose.upgrade().is_none());
        assert!(attempt.entered.try_recv().is_err());
        saving();
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        window.destroy();

        // Auth invalidation while the human picker waits refuses dispatch, with no activity.
        let view = host(&app, &model, &game);
        let selected = picker();
        view.choose.emit_clicked();
        auth::invalidate_session();
        selected
            .send(Ok(gio::File::for_path(chosen.join("old-auth"))))
            .unwrap();
        wait(|| view.status.text().contains("session changed"));
        assert!(crate::profile_reset::reserve().is_ok());
        assert_eq!(
            store.cloud_save_record(game.product_id).unwrap().locations,
            fresh
        );
        view.window.destroy();

        // A queued save also rejects auth invalidation at the post-admission barrier.
        let view = host(&app, &model, &game);
        let selected = picker();
        let attempt = save(false);
        view.choose.emit_clicked();
        selected
            .send(Ok(gio::File::for_path(chosen.join("queued-old-auth"))))
            .unwrap();
        saving();
        auth::invalidate_session();
        heartbeat();
        assert!(view.status.text().contains("session changed"));
        attempt.start.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(attempt.entered.try_recv().is_err());
        assert_eq!(
            store.cloud_save_record(game.product_id).unwrap().locations,
            fresh
        );
        view.window.destroy();

        // A newly opened, unchanged signed-out origin can still edit this local preference.
        assert!(!auth::session_is_current(auth::session()));
        let view = host(&app, &model, &game);
        let selected = picker();
        let attempt = save(false);
        view.choose.emit_clicked();
        selected.send(Ok(gio::File::for_path(&chosen))).unwrap();
        saving();
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| view.choose.is_sensitive());
        assert!(!view.retry.is_sensitive());
        assert_eq!(view.status.text(), "Override save directory updated");
        assert_eq!(*view.locations.borrow(), fresh);
        view.window.destroy();
        assert!(TEST_CLOUD_PICKERS.with(|requests| requests.borrow().as_ref().unwrap().is_empty()));
        assert!(
            TEST_CLOUD_LOCATION_SAVES.with(|requests| requests
                .borrow()
                .as_ref()
                .unwrap()
                .is_empty())
        );
        assert!(
            TEST_CLOUD_DISCOVERIES.with(|requests| requests.borrow().as_ref().unwrap().is_empty())
        );
        assert!(crate::profile_reset::reserve().is_ok());
    }

    #[test]
    #[ignore = "private p376 HOME/all XDG/TMP, GTK and D-Bus; synthetic SQLite only"]
    fn branch_forget_tracks_identity_activity_and_reentrant_controls() {
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
                    .starts_with("/tmp/ludomere-p376-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn heartbeat() {
            let tick = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(120), {
                let tick = tick.clone();
                move || tick.set(true)
            });
            wait(|| tick.get());
        }
        fn registered() {
            assert!(
                crate::profile_reset::reserve()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("forgetting a saved branch password")
            );
        }
        struct Attempt {
            start: mpsc::Sender<()>,
            entered: mpsc::Receiver<()>,
            finish: mpsc::Sender<()>,
        }
        fn queue(error: Option<&str>, disconnect: bool) -> Attempt {
            let (start, before_open) = mpsc::channel();
            let (entered, capture) = mpsc::channel();
            let (finish, release) = mpsc::channel();
            TEST_BRANCH_FORGET.with(|requests| {
                requests
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .push_back(TestBranchForget {
                        before_open,
                        entered,
                        finish: release,
                        error: error.map(str::to_owned),
                        disconnect,
                    })
            });
            Attempt {
                start,
                entered: capture,
                finish,
            }
        }
        struct Host {
            window: adw::ApplicationWindow,
            content: gtk::Box,
            selector: gtk::DropDown,
            password: adw::PasswordEntryRow,
            status: gtk::Label,
            switch: gtk::Button,
            forget: gtk::Button,
            controls: TestBranchControls,
        }
        fn host(app: &adw::Application, model: &Rc<RefCell<AppModel>>) -> Host {
            let window = adw::ApplicationWindow::builder()
                .application(app)
                .default_width(650)
                .default_height(400)
                .build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let selector = gtk::DropDown::from_strings(&["Master", "beta", "stable"]);
            selector.set_selected(1);
            let password = adw::PasswordEntryRow::new();
            password.set_text("untouched synthetic entry");
            let status = gtk::Label::new(Some("Initial branch status"));
            let switch = gtk::Button::with_label("Switch");
            let forget = gtk::Button::with_label("Forget Password");
            for widget in [
                selector.clone().upcast::<gtk::Widget>(),
                password.clone().upcast(),
                status.clone().upcast(),
                switch.clone().upcast(),
                forget.clone().upcast(),
            ] {
                content.append(&widget);
            }
            window.set_content(Some(&content));
            let installed: crate::domain::InstalledGame = serde_json::from_value(serde_json::json!({
                "product_id": 9376001, "library_id": "synthetic", "installation_directory": std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("inert-game"),
                "primary_executable": null, "installer_files": [], "installer_complete": true,
                "installer_operating_system": "windows", "launch_arguments": [], "state": "installed",
                "playtime_seconds": 0, "created_at": 1, "updated_at": 1
            })).unwrap();
            let mut marker = crate::installation::installation_marker_from_game(&installed, vec![]);
            marker.galaxy_depot = Some(crate::domain::GalaxyDepotProvenance {
                build_id: "inert".into(),
                repository_id: "inert".into(),
                manifest_fingerprint: "inert".into(),
                branch: Some("beta".into()),
                language: None,
                architecture: None,
                depots: vec![],
                dlc: vec![],
            });
            let game = DetailPageModel::game(
                Game {
                    product_id: installed.product_id,
                    ..Default::default()
                },
                false,
            );
            wire_branch_actions(
                &window,
                model,
                &game,
                &installed,
                marker,
                vec![None, Some("beta".into()), Some("stable".into())],
                selector.clone(),
                password.clone(),
                status.clone(),
                switch.clone(),
                forget.clone(),
            );
            let controls =
                TEST_BRANCH_CONTROLS.with(|controls| controls.borrow_mut().take().unwrap());
            window.present();
            wait(|| forget.is_mapped());
            Host {
                window,
                content,
                selector,
                password,
                status,
                switch,
                forget,
                controls,
            }
        }
        fn completed(host: &Host) {
            wait(|| host.forget.is_sensitive());
            wait(|| crate::profile_reset::reserve().is_ok());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.BranchForgetTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let model = Rc::new(RefCell::new(AppModel {
            account_profile: Some(auth::Profile {
                user_id: "fixture-user".into(),
                ..Default::default()
            }),
            ..Default::default()
        }));
        TEST_BRANCH_FORGET.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));
        let store = StateStore::open().unwrap();
        let seed = || {
            for (user, product, branch) in [
                ("fixture-user", 9376001, "beta"),
                ("fixture-user", 9376001, "stable"),
                ("other-user", 9376001, "beta"),
                ("fixture-user", 9376002, "beta"),
            ] {
                store
                    .save_galaxy_branch_credential(user, product, branch, 1, &[1, 2], &[3, 4])
                    .unwrap();
            }
        };
        seed();
        let view = host(&app, &model);
        let lock = rusqlite::Connection::open(crate::identity::database()).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let attempt = queue(None, false);
        attempt.start.send(()).unwrap();
        view.forget.emit_clicked();
        view.forget.emit_clicked();
        attempt
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        registered();
        heartbeat();
        assert_eq!(view.status.text(), "Forgetting saved branch password…");
        assert!(!view.switch.is_sensitive() && !view.forget.is_sensitive());
        view.selector.set_selected(2);
        view.switch.emit_clicked();
        assert_eq!(view.status.text(), "Forgetting saved branch password…");
        assert!(!view.switch.is_sensitive() && !view.forget.is_sensitive());
        lock.execute_batch("ROLLBACK").unwrap();
        attempt.finish.send(()).unwrap();
        completed(&view);
        assert!(view.status.text().contains("‘beta’ forgotten"));
        assert_eq!(view.selector.selected(), 2);
        assert_eq!(view.password.text(), "untouched synthetic entry");
        assert!(
            store
                .galaxy_branch_credential("fixture-user", 9376001, "beta")
                .unwrap()
                .is_none()
        );
        for (user, product, branch) in [
            ("fixture-user", 9376001, "stable"),
            ("other-user", 9376001, "beta"),
            ("fixture-user", 9376002, "beta"),
        ] {
            assert_eq!(
                store
                    .galaxy_branch_credential(user, product, branch)
                    .unwrap(),
                Some((1, vec![1, 2], vec![3, 4]))
            );
        }
        // Missing rows remain idempotent and Started is not reset to Idle.
        view.selector.set_selected(1);
        view.controls.state.set(BranchActionState::Started);
        (view.controls.refresh)();
        let attempt = queue(None, false);
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        view.forget.emit_clicked();
        completed(&view);
        assert!(!view.switch.is_sensitive());
        assert!(view.controls.state.get() == BranchActionState::Started);
        view.controls.state.set(BranchActionState::Preparing);
        (view.controls.refresh)();
        view.forget.emit_clicked();
        assert!(view.status.text().contains("before forgetting"));
        view.controls.state.set(BranchActionState::Idle);
        (view.controls.refresh)();
        view.selector.set_selected(0);
        view.forget.emit_clicked();
        assert!(view.status.text().contains("Master"));
        let names = view
            .selector
            .model()
            .and_downcast::<gtk::StringList>()
            .unwrap();
        names.splice(0, names.n_items(), &[]);
        view.forget.emit_clicked();
        assert_eq!(view.selector.selected(), gtk::INVALID_LIST_POSITION);
        assert!(view.status.text().contains("Choose a named branch"));
        view.window.destroy();

        // Real controls reenter on both sensitivity setters; old refresh must converge.
        for first_switch in [true, false] {
            let view = host(&app, &model);
            view.selector.set_selected(2);
            view.controls.state.set(BranchActionState::Preparing);
            (view.controls.refresh)();
            let triggered = Rc::new(std::cell::Cell::new(false));
            let target = if first_switch {
                &view.switch
            } else {
                &view.forget
            };
            let observer = target.connect_sensitive_notify({
                let triggered = triggered.clone();
                let forget = view.forget.downgrade();
                move |button| {
                    if button.is_sensitive() && !triggered.replace(true) {
                        forget.upgrade().unwrap().emit_clicked();
                    }
                }
            });
            let attempt = queue(None, false);
            view.controls.state.set(BranchActionState::Idle);
            (view.controls.refresh)();
            assert!(triggered.get());
            assert!(!view.switch.is_sensitive() && !view.forget.is_sensitive());
            assert!(
                view.forget
                    .tooltip_text()
                    .unwrap()
                    .contains("Wait for the saved password")
            );
            heartbeat();
            registered();
            target.disconnect(observer);
            attempt.start.send(()).unwrap();
            attempt.finish.send(()).unwrap();
            completed(&view);
            view.window.destroy();
        }
        // A selection change inside a sensitivity observer must also update Forget eligibility.
        let view = host(&app, &model);
        view.selector.set_selected(2);
        view.controls.state.set(BranchActionState::Preparing);
        (view.controls.refresh)();
        let observer = view.switch.connect_sensitive_notify({
            let selector = view.selector.downgrade();
            move |button| {
                if button.is_sensitive() {
                    selector.upgrade().unwrap().set_selected(0);
                }
            }
        });
        view.controls.state.set(BranchActionState::Idle);
        (view.controls.refresh)();
        assert_eq!(view.selector.selected(), 0);
        assert!(view.switch.is_sensitive());
        assert!(!view.forget.is_sensitive());
        assert!(view.forget.tooltip_text().unwrap().contains("Master"));
        view.switch.disconnect(observer);
        view.window.destroy();

        // The exact success handoff used by the Switch receiver may synchronously start Forget.
        let view = host(&app, &model);
        view.controls.state.set(BranchActionState::Preparing);
        (view.controls.refresh)();
        let trigger = Rc::new(std::cell::Cell::new(false));
        let observer = view.forget.connect_sensitive_notify({
            let trigger = trigger.clone();
            move |button| {
                if button.is_sensitive() && !trigger.replace(true) {
                    button.emit_clicked();
                }
            }
        });
        let attempt = queue(None, false);
        (view.controls.started)();
        heartbeat();
        assert_eq!(view.status.text(), "Forgetting saved branch password…");
        assert!(!view.switch.is_sensitive() && !view.forget.is_sensitive());
        view.forget.disconnect(observer);
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        completed(&view);
        assert!(view.controls.state.get() == BranchActionState::Started);
        view.window.destroy();

        for disconnect in [false, true] {
            let view = host(&app, &model);
            let attempt = queue(
                (!disconnect).then_some("Synthetic failure access_token=synthetic-secret"),
                disconnect,
            );
            attempt.start.send(()).unwrap();
            attempt.finish.send(()).unwrap();
            view.forget.emit_clicked();
            completed(&view);
            assert!(view.status.has_css_class("error"));
            assert!(!view.status.text().contains("synthetic-secret"));
            let retry = queue(None, false);
            retry.start.send(()).unwrap();
            retry.finish.send(()).unwrap();
            view.forget.emit_clicked();
            completed(&view);
            assert!(view.status.text().contains("forgotten"));
            view.window.destroy();
        }
        // Frozen recovery admission starts no worker and restores the current selection.
        let view = host(&app, &model);
        let reservation = crate::profile_reset::reserve().unwrap();
        view.forget.emit_clicked();
        wait(|| view.forget.is_sensitive());
        assert!(view.status.has_css_class("error"));
        drop(reservation);
        view.window.destroy();

        // Error restoration may reenter: retire the old result before unlocking controls.
        let view = host(&app, &model);
        let reservation = Rc::new(RefCell::new(Some(crate::profile_reset::reserve().unwrap())));
        let trigger = Rc::new(std::cell::Cell::new(false));
        let observer = view.forget.connect_sensitive_notify({
            let reservation = reservation.clone();
            let trigger = trigger.clone();
            move |button| {
                if button.is_sensitive() && !trigger.replace(true) {
                    drop(reservation.borrow_mut().take());
                    button.emit_clicked();
                    button.emit_clicked();
                }
            }
        });
        let attempt = queue(None, false);
        view.forget.emit_clicked();
        wait(|| trigger.get());
        heartbeat();
        registered();
        assert_eq!(view.status.text(), "Forgetting saved branch password…");
        assert!(!view.forget.is_sensitive());
        view.forget.disconnect(observer);
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        completed(&view);
        view.window.destroy();

        for auth_change in [false, true] {
            seed();
            let view = host(&app, &model);
            let attempt = queue(None, false);
            view.forget.emit_clicked();
            registered();
            if auth_change {
                auth::invalidate_session();
            } else {
                online::invalidate_library_session();
            }
            attempt.start.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            wait(|| view.status.text().contains("account changed"));
            assert!(attempt.entered.try_recv().is_err());
            assert!(
                store
                    .galaxy_branch_credential("fixture-user", 9376001, "beta")
                    .unwrap()
                    .is_some()
            );
            for selected in [2, 1, 0] {
                view.selector.set_selected(selected);
                assert!(!view.switch.is_sensitive() && !view.forget.is_sensitive());
            }
            view.forget.emit_clicked();
            view.switch.emit_clicked();
            assert!(crate::profile_reset::reserve().is_ok());
            view.window.destroy();
        }
        // A fresh known identity can forget locally even with revoked auth and no token.
        assert!(!auth::session_is_current(auth::session()));
        let view = host(&app, &model);
        let attempt = queue(None, false);
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        view.forget.emit_clicked();
        completed(&view);
        assert!(view.status.text().contains("forgotten"));
        view.window.destroy();

        for mode in ["session", "identity", "logout", "close", "detach"] {
            let view = host(&app, &model);
            let attempt = queue(None, false);
            attempt.start.send(()).unwrap();
            view.forget.emit_clicked();
            attempt
                .entered
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if mode == "session" {
                model.borrow_mut().account_epoch += 1;
            } else if mode == "identity" {
                model.borrow_mut().account_profile.as_mut().unwrap().user_id = "other-user".into();
            } else if mode == "logout" {
                model.borrow_mut().logout_pending = true;
            } else if mode == "close" {
                view.window.set_hide_on_close(true);
                view.window.close();
            } else {
                view.content.remove(&view.forget);
            }
            heartbeat();
            registered();
            let before = view.status.text();
            attempt.finish.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            heartbeat();
            assert_eq!(view.status.text(), before);
            assert!(!view.forget.is_sensitive());
            view.window.destroy();
            model.borrow_mut().logout_pending = false;
            model.borrow_mut().account_profile.as_mut().unwrap().user_id = "fixture-user".into();
        }
        // The new Forget callback/receiver do not retain their own button.
        let view = host(&app, &model);
        let attempt = queue(None, false);
        view.forget.emit_clicked();
        view.content.remove(&view.forget);
        let weak = view.forget.downgrade();
        drop(view.forget);
        wait(|| weak.upgrade().is_none());
        heartbeat();
        registered();
        attempt.start.send(()).unwrap();
        attempt.finish.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        view.window.destroy();
        for identity in [None, Some("")] {
            model.borrow_mut().account_profile = identity.map(|id| auth::Profile {
                user_id: id.into(),
                ..Default::default()
            });
            let view = host(&app, &model);
            view.forget.emit_clicked();
            assert!(view.status.text().contains("identity is unavailable"));
            view.selector.set_selected(2);
            assert!(!view.forget.is_sensitive() && !view.switch.is_sensitive());
            assert!(crate::profile_reset::reserve().is_ok());
            view.window.destroy();
        }
        assert!(
            TEST_BRANCH_FORGET.with(|requests| requests.borrow_mut().take().unwrap().is_empty())
        );
        assert!(TEST_BRANCH_CONTROLS.with(|controls| controls.borrow().is_none()));
    }

    #[test]
    #[ignore = "private p372 HOME/all XDG/TMP, GTK and D-Bus; inert metadata backend only"]
    fn metadata_retry_tracks_original_session_activity_and_page_lifetime() {
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
                    .starts_with("/tmp/ludomere-p372-"),
                "{key}"
            );
        }
        assert!(!crate::identity::database().exists());
        assert!(
            auth::session_is_current(auth::session()),
            "run this fixture alone before auth revocation"
        );
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn heartbeat() {
            let tick = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(150), {
                let tick = tick.clone();
                move || tick.set(true)
            });
            wait(|| tick.get());
        }
        #[track_caller]
        fn registered() {
            let error = crate::profile_reset::reserve().err().unwrap().to_string();
            assert!(error.contains("discovering cloud-save metadata"), "{error}");
        }
        struct Attempt {
            start: mpsc::Sender<()>,
            entered: mpsc::Receiver<((u64, u64), Vec<crate::domain::CloudSaveLocation>)>,
            finish: mpsc::Sender<Result<crate::domain::CloudSaveDiscovery, String>>,
        }
        fn queue() -> Attempt {
            let (start, before_entry) = mpsc::channel();
            let (entered, capture) = mpsc::channel();
            let (finish, result) = mpsc::channel();
            TEST_CLOUD_DISCOVERIES.with(|requests| {
                requests
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .push_back(TestCloudDiscovery {
                        before_entry,
                        entered,
                        result,
                    })
            });
            Attempt {
                start,
                entered: capture,
                finish,
            }
        }
        fn nodes(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
            let mut result = vec![root.as_ref().clone()];
            let mut child = root.as_ref().first_child();
            while let Some(widget) = child {
                result.extend(nodes(&widget));
                child = widget.next_sibling();
            }
            result
        }
        fn button(page: &adw::PreferencesPage, title: &str) -> gtk::Button {
            nodes(page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some(title))
                .unwrap()
        }
        fn row(page: &adw::PreferencesPage, title: &str) -> adw::ActionRow {
            nodes(page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<adw::ActionRow>().ok())
                .find(|row| row.title() == title)
                .unwrap()
        }
        fn status(page: &adw::PreferencesPage) -> gtk::Label {
            find_named_descendant(page.upcast_ref(), "cloud-save-status")
                .and_downcast::<gtk::Label>()
                .unwrap()
        }
        fn host(
            app: &adw::Application,
            model: &Rc<RefCell<AppModel>>,
            game: &crate::domain::InstalledGame,
            locations: &[crate::domain::CloudSaveLocation],
        ) -> (
            adw::ApplicationWindow,
            gtk::Stack,
            adw::PreferencesPage,
            gtk::Entry,
        ) {
            let window = adw::ApplicationWindow::builder()
                .application(app)
                .default_width(800)
                .default_height(620)
                .build();
            let stack = gtk::Stack::new();
            let entry = gtk::Entry::new();
            entry.set_text("General stays selected while work finishes");
            stack.add_named(&entry, Some("general"));
            let page = adw::PreferencesPage::new();
            stack.add_named(&page, Some("cloud"));
            populate_cloud_settings(
                &window,
                model,
                &page,
                game,
                crate::state::CloudSaveRecord {
                    preference: crate::domain::CloudSavePreference::Undecided,
                    availability: crate::domain::CloudSaveAvailability::Unknown,
                    locations: locations.to_vec(),
                    metadata_build_id: None,
                    metadata_checked_at: None,
                    metadata_error: None,
                    last_successful_sync: None,
                    status: crate::domain::CloudSaveStatus::NeverSynced,
                    error: None,
                    conflicts: vec![],
                },
                &CloudActionSession {
                    auth: auth::session(),
                    online: online::account_session(),
                    epoch: model.borrow().account_epoch,
                    model: Rc::downgrade(model),
                },
            );
            stack.set_visible_child_name("cloud");
            window.set_content(Some(&stack));
            window.present();
            wait(|| button(&page, "Retry metadata discovery").is_mapped());
            (window, stack, page, entry)
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudDiscoveryLifecycleTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let directory =
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("inert-game");
        let mut game: crate::domain::InstalledGame = serde_json::from_value(serde_json::json!({
            "product_id": 9372001, "library_id": "synthetic", "installation_directory": directory,
            "primary_executable": null, "installer_files": [], "installer_complete": true,
            "installer_operating_system": "windows", "launch_arguments": [], "state": "installed",
            "playtime_seconds": 0, "created_at": 1, "updated_at": 1
        }))
        .unwrap();
        game.compatibility = Some(crate::compatibility::GameCompatibilityPreferences {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            prefix_slug: "inert".into(),
            profile: crate::compatibility::UmuProfile::fallback(),
            pending_profile: None,
        });
        let old_locations = vec![crate::domain::CloudSaveLocation {
            name: "Old local choice".into(),
            path: directory.join("old-saves"),
            remote_namespace: "old".into(),
            user_override: true,
        }];
        let mut discovery = crate::domain::CloudSaveDiscovery {
            availability: crate::domain::CloudSaveAvailability::Supported,
            locations: vec![crate::domain::CloudSaveLocation {
                name: "New synthetic choice".into(),
                path: directory.join("new-saves"),
                remote_namespace: "new".into(),
                user_override: false,
            }],
            checked_at: 123,
            ..Default::default()
        };
        TEST_CLOUD_DISCOVERIES.with(|requests| *requests.borrow_mut() = Some(VecDeque::new()));

        // The real receiver retains the exact availability-specific control mutations.
        for availability in [
            crate::domain::CloudSaveAvailability::Supported,
            crate::domain::CloudSaveAvailability::Unsupported,
            crate::domain::CloudSaveAvailability::Unavailable,
            crate::domain::CloudSaveAvailability::Unknown,
        ] {
            discovery.availability = availability;
            let (window, stack, page, entry) = host(&app, &model, &game, &old_locations);
            let retry = button(&page, "Retry metadata discovery");
            let attempt = queue();
            let origin = (online::account_session(), auth::session());
            retry.emit_clicked();
            retry.emit_clicked();
            assert!(!retry.is_sensitive());
            assert!(
                !button(&page, "Choose…").is_sensitive(),
                "Retry owns the page's location controls until its result is retired"
            );
            registered();
            heartbeat();
            assert!(attempt.entered.try_recv().is_err());
            assert_eq!(status(&page).text(), "Checking GOG cloud-save support…");
            stack.set_visible_child_name("general");
            wait(|| entry.is_mapped());
            assert!(entry.grab_focus());
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            assert!(focus.is_some());
            attempt.start.send(()).unwrap();
            assert_eq!(
                attempt
                    .entered
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap(),
                (origin, old_locations.clone())
            );
            attempt.finish.send(Ok(discovery.clone())).unwrap();
            let terminal_status = match availability {
                crate::domain::CloudSaveAvailability::Supported => {
                    "GOG cloud saves are supported for this game."
                }
                crate::domain::CloudSaveAvailability::Unsupported => {
                    "GOG reports cloud saves are disabled for this game."
                }
                crate::domain::CloudSaveAvailability::Unavailable => {
                    "GOG cloud-save metadata is unavailable for this game."
                }
                crate::domain::CloudSaveAvailability::Unknown => {
                    "Cloud-save discovery failed; retry is available."
                }
            };
            wait(|| status(&page).text() == terminal_status);
            wait(|| crate::profile_reset::reserve().is_ok());
            let supported = availability == crate::domain::CloudSaveAvailability::Supported;
            let enabled = nodes(&page)
                .into_iter()
                .find_map(|widget| widget.downcast::<adw::SwitchRow>().ok())
                .unwrap();
            assert_eq!(enabled.is_sensitive(), supported);
            let advanced = nodes(&page)
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
                .unwrap();
            assert_eq!(advanced.is_sensitive(), supported);
            let management = nodes(&page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<adw::PreferencesGroup>().ok())
                .find(|group| group.title() == "Export and manage remote saves")
                .unwrap();
            assert_eq!(management.get_visible(), supported);
            assert_eq!(button(&page, "Sync now").is_sensitive(), supported);
            assert_eq!(button(&page, "Check now").is_sensitive(), supported);
            assert_eq!(
                button(&page, "Open backup folder").is_sensitive(),
                supported
            );
            assert_eq!(button(&page, "Open save folder").is_sensitive(), supported);
            assert_eq!(row(&page, "Save locations").get_visible(), supported);
            assert_eq!(
                row(&page, "Save locations").subtitle().as_deref(),
                Some(cloud_location_summary(&discovery.locations).as_str())
            );
            assert_eq!(
                retry.get_visible(),
                availability == crate::domain::CloudSaveAvailability::Unknown
            );
            assert_eq!(
                retry.is_sensitive(),
                availability == crate::domain::CloudSaveAvailability::Unknown
            );
            assert_eq!(
                button(&page, "Choose…").is_sensitive(),
                supported || availability == crate::domain::CloudSaveAvailability::Unknown
            );
            assert_eq!(status(&page).text(), terminal_status);
            assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
            assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
            assert!(window.visible_dialog().is_none());
            if !retry.get_visible() {
                retry.emit_clicked();
                assert!(crate::profile_reset::reserve().is_ok());
            }
            window.destroy();
        }

        for disconnected in [false, true] {
            let (window, _, page, _) = host(&app, &model, &game, &old_locations);
            let retry = button(&page, "Retry metadata discovery");
            let attempt = queue();
            retry.emit_clicked();
            attempt.start.send(()).unwrap();
            attempt
                .entered
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let diagnostic = "Synthetic metadata failure\naccess_token=synthetic-secret https://example.invalid/?signed=secret\nComplete diagnostic tail";
            if !disconnected {
                attempt.finish.send(Err(diagnostic.into())).unwrap();
            }
            drop(attempt.finish);
            wait(|| retry.is_sensitive());
            wait(|| crate::profile_reset::reserve().is_ok());
            assert!(retry.get_visible());
            assert_eq!(
                status(&page).text(),
                notifications::failure_message(
                    "Cloud-save discovery failed",
                    if disconnected {
                        "Cloud-save discovery worker stopped unexpectedly"
                    } else {
                        diagnostic
                    }
                )
            );
            assert!(!status(&page).text().contains("synthetic-secret"));
            assert_eq!(
                row(&page, "Save locations").subtitle().as_deref(),
                Some(cloud_location_summary(&old_locations).as_str())
            );
            assert!(!button(&page, "Sync now").is_sensitive());
            window.destroy();
        }

        // Activity refusal does not consume a backend fixture; restoration may synchronously retry.
        let (window, _, page, _) = host(&app, &model, &game, &old_locations);
        let retry = button(&page, "Retry metadata discovery");
        let frozen = Rc::new(RefCell::new(Some(crate::profile_reset::reserve().unwrap())));
        let reentered = Rc::new(std::cell::Cell::new(false));
        retry.connect_sensitive_notify({
            let frozen = frozen.clone();
            let reentered = reentered.clone();
            move |button| {
                if button.is_sensitive() && !reentered.replace(true) {
                    drop(frozen.borrow_mut().take());
                    button.emit_clicked();
                    button.emit_clicked();
                }
            }
        });
        let attempt = queue();
        retry.emit_clicked();
        assert_eq!(
            TEST_CLOUD_DISCOVERIES.with(|requests| requests.borrow().as_ref().unwrap().len()),
            1
        );
        wait(|| reentered.get());
        heartbeat();
        registered();
        assert!(!retry.is_sensitive());
        assert_eq!(status(&page).text(), "Checking GOG cloud-save support…");
        assert_eq!(
            TEST_CLOUD_DISCOVERIES.with(|requests| requests.borrow().as_ref().unwrap().len()),
            0
        );
        attempt.start.send(()).unwrap();
        attempt
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(Ok(discovery.clone())).unwrap();
        wait(|| retry.is_sensitive());
        wait(|| crate::profile_reset::reserve().is_ok());
        window.destroy();

        // Online changes reject both queued work and a backend result already in flight.
        discovery.availability = crate::domain::CloudSaveAvailability::Supported;
        for entered_first in [false, true] {
            let (window, _, page, _) = host(&app, &model, &game, &old_locations);
            let retry = button(&page, "Retry metadata discovery");
            let attempt = queue();
            let epoch = model.borrow().account_epoch;
            retry.emit_clicked();
            if entered_first {
                attempt.start.send(()).unwrap();
                attempt
                    .entered
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            online::invalidate_library_session();
            assert_eq!(model.borrow().account_epoch, epoch);
            wait(|| status(&page).text().contains("close and reopen Properties"));
            registered();
            if entered_first {
                attempt.finish.send(Ok(discovery.clone())).unwrap();
            } else {
                attempt.start.send(()).unwrap();
            }
            wait(|| crate::profile_reset::reserve().is_ok());
            assert!(attempt.entered.try_recv().is_err());
            assert!(!retry.is_sensitive());
            retry.emit_clicked();
            assert!(crate::profile_reset::reserve().is_ok());
            assert!(!button(&page, "Sync now").is_sensitive());
            assert_eq!(
                row(&page, "Save locations").subtitle().as_deref(),
                Some(cloud_location_summary(&old_locations).as_str())
            );
            window.destroy();
        }

        // UI epoch/logout gates apply at admission and again before result mutation.
        for logging_out in [false, true] {
            for already_started in [false, true] {
                let (window, _, page, _) = host(&app, &model, &game, &old_locations);
                let retry = button(&page, "Retry metadata discovery");
                let attempt = queue();
                if already_started {
                    retry.emit_clicked();
                    registered();
                }
                let globals = (online::account_session(), auth::session());
                if logging_out {
                    model.borrow_mut().logout_pending = true;
                } else {
                    model.borrow_mut().account_epoch += 1;
                }
                if !already_started {
                    retry.emit_clicked();
                }
                wait(|| status(&page).text().contains("close and reopen Properties"));
                assert_eq!((online::account_session(), auth::session()), globals);
                assert!(!retry.is_sensitive());
                if already_started {
                    attempt.start.send(()).unwrap();
                    attempt
                        .entered
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    attempt.finish.send(Ok(discovery.clone())).unwrap();
                    wait(|| crate::profile_reset::reserve().is_ok());
                } else {
                    assert_eq!(
                        TEST_CLOUD_DISCOVERIES.with(|requests| requests
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .len()),
                        1
                    );
                    TEST_CLOUD_DISCOVERIES
                        .with(|requests| requests.borrow_mut().as_mut().unwrap().clear());
                }
                heartbeat();
                assert!(!button(&page, "Sync now").is_sensitive());
                assert_eq!(
                    row(&page, "Save locations").subtitle().as_deref(),
                    Some(cloud_location_summary(&old_locations).as_str())
                );
                window.destroy();
                model.borrow_mut().logout_pending = false;
            }
        }

        for closed in [false, true] {
            let (window, stack, page, _) = host(&app, &model, &game, &old_locations);
            let retry = button(&page, "Retry metadata discovery");
            let attempt = queue();
            retry.emit_clicked();
            if closed {
                window.set_hide_on_close(true);
                window.close();
                wait(|| !window.is_visible());
            } else {
                stack.remove(&page);
            }
            heartbeat();
            registered();
            attempt.start.send(()).unwrap();
            attempt
                .entered
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            attempt.finish.send(Ok(discovery.clone())).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            heartbeat();
            assert!(!retry.is_sensitive());
            assert_eq!(status(&page).text(), "Checking GOG cloud-save support…");
            assert!(!button(&page, "Sync now").is_sensitive());
            window.destroy();
        }

        // Removing Retry proves its new receiver/callback do not retain it while work is held.
        let (window, _, page, _) = host(&app, &model, &game, &old_locations);
        let retry = button(&page, "Retry metadata discovery");
        let attempt = queue();
        retry.emit_clicked();
        let retry_row = row(&page, "Cloud-save support");
        let group = retry_row
            .ancestor(adw::PreferencesGroup::static_type())
            .and_downcast::<adw::PreferencesGroup>()
            .unwrap();
        group.remove(&retry_row);
        let weak = retry.downgrade();
        drop((retry_row, retry));
        wait(|| weak.upgrade().is_none());
        heartbeat();
        registered();
        attempt.start.send(()).unwrap();
        attempt
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        attempt.finish.send(Ok(discovery.clone())).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(!button(&page, "Sync now").is_sensitive());
        window.destroy();

        // Auth invalidation is deliberately last: no production auth reset/bypass is needed.
        let (before_window, _, before_page, _) = host(&app, &model, &game, &old_locations);
        let before = queue();
        button(&before_page, "Retry metadata discovery").emit_clicked();
        let (after_window, _, after_page, _) = host(&app, &model, &game, &old_locations);
        let after = queue();
        button(&after_page, "Retry metadata discovery").emit_clicked();
        after.start.send(()).unwrap();
        after.entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let epoch = model.borrow().account_epoch;
        auth::invalidate_session();
        assert_eq!(model.borrow().account_epoch, epoch);
        wait(|| {
            status(&before_page)
                .text()
                .contains("close and reopen Properties")
                && status(&after_page)
                    .text()
                    .contains("close and reopen Properties")
        });
        before.start.send(()).unwrap();
        after.finish.send(Ok(discovery)).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(before.entered.try_recv().is_err());
        for page in [&before_page, &after_page] {
            assert!(!button(page, "Retry metadata discovery").is_sensitive());
            assert!(!button(page, "Sync now").is_sensitive());
            assert_eq!(
                row(page, "Save locations").subtitle().as_deref(),
                Some(cloud_location_summary(&old_locations).as_str())
            );
        }
        before_window.destroy();
        after_window.destroy();
        let (window, _, page, _) = host(&app, &model, &game, &old_locations);
        button(&page, "Retry metadata discovery").emit_clicked();
        assert!(!button(&page, "Retry metadata discovery").is_sensitive());
        assert!(status(&page).text().contains("Sign in"));
        assert!(crate::profile_reset::reserve().is_ok());
        assert!(
            TEST_CLOUD_DISCOVERIES.with(|requests| requests
                .borrow_mut()
                .take()
                .unwrap()
                .is_empty())
        );
        assert!(!crate::identity::database().exists());
        window.destroy();
    }

    #[test]
    #[ignore = "private p369 HOME/all XDG/TMP, GTK and D-Bus; synthetic SQLite only, no cloud calls"]
    fn cloud_settings_load_off_thread_and_reject_stale_pages() {
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
                    .starts_with("/tmp/ludomere-p369-"),
                "{key}"
            );
        }
        let database = crate::identity::database();
        assert!(database.starts_with(std::env::var_os("XDG_DATA_HOME").unwrap()));
        assert!(
            !database.exists(),
            "fixture requires a fresh synthetic profile"
        );
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn heartbeat() {
            let tick = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(100), {
                let tick = tick.clone();
                move || tick.set(true)
            });
            wait(|| tick.get());
        }
        fn delay() -> mpsc::Sender<()> {
            let (send, receive) = mpsc::channel();
            TEST_CLOUD_SETTINGS_DELAYS.with(|delays| delays.borrow_mut().push_back(receive));
            send
        }
        #[track_caller]
        fn registered() {
            let error = crate::profile_reset::reserve().err().unwrap().to_string();
            assert!(error.contains("loading cloud-save settings"), "{error}");
        }
        fn nodes(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
            let mut result = vec![root.as_ref().clone()];
            let mut child = root.as_ref().first_child();
            while let Some(widget) = child {
                result.extend(nodes(&widget));
                child = widget.next_sibling();
            }
            result
        }
        fn text(page: &adw::PreferencesPage) -> String {
            nodes(page)
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        fn loaded(page: &adw::PreferencesPage) -> bool {
            nodes(page)
                .iter()
                .any(|widget| widget.is::<adw::SwitchRow>())
        }
        fn host(
            app: &adw::Application,
            model: &Rc<RefCell<AppModel>>,
            installed: Option<&crate::domain::InstalledGame>,
        ) -> (
            adw::ApplicationWindow,
            gtk::Stack,
            adw::PreferencesPage,
            gtk::Entry,
        ) {
            let window = adw::ApplicationWindow::builder()
                .application(app)
                .default_width(750)
                .default_height(600)
                .build();
            let origin = CloudActionSession {
                auth: auth::session(),
                online: online::account_session(),
                epoch: model.borrow().account_epoch,
                model: Rc::downgrade(model),
            };
            let page = cloud_settings_page(&window, model, installed, &origin);
            let stack = gtk::Stack::new();
            let entry = gtk::Entry::new();
            entry.set_text("Keep General selected");
            stack.add_named(&entry, Some("general"));
            stack.add_named(&page, Some("cloud"));
            stack.set_visible_child_name("general");
            window.set_content(Some(&stack));
            window.present();
            (window, stack, page, entry)
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.CloudSettingsLoadTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let model = Rc::new(RefCell::new(AppModel {
            network_available: false,
            account_token: None,
            ..AppModel::default()
        }));
        let mut installed: crate::domain::InstalledGame = serde_json::from_value(serde_json::json!({
            "product_id": 9369001, "library_id": "synthetic", "installation_directory": database.parent().unwrap().join("inert-game"),
            "primary_executable": null, "installer_files": [], "installer_complete": true,
            "installer_operating_system": "windows", "launch_arguments": [], "state": "installed",
            "playtime_seconds": 0, "created_at": 1, "updated_at": 1
        })).unwrap();
        installed.compatibility = Some(crate::compatibility::GameCompatibilityPreferences {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            prefix_slug: "inert-prefix".into(),
            profile: crate::compatibility::UmuProfile::fallback(),
            pending_profile: None,
        });
        auth::invalidate_session();
        assert!(!auth::session_is_current(auth::session()));

        // Noneligible pages must not even dispatch the local worker.
        let mut native = installed.clone();
        native.installer_operating_system = Some("linux".into());
        let mut unmanaged = installed.clone();
        unmanaged.compatibility = None;
        let unused = delay();
        for game in [None, Some(&native), Some(&unmanaged)] {
            let before = TEST_CLOUD_SETTINGS_DISPATCHES.get();
            let (window, _, page, _) = host(&app, &model, game);
            heartbeat();
            assert_eq!(TEST_CLOUD_SETTINGS_DISPATCHES.get(), before);
            assert!(!loaded(&page));
            assert!(text(&page).contains(if game.is_none() {
                "Install the Windows version"
            } else {
                "managed UMU prefix only"
            }));
            assert!(!database.exists());
            window.destroy();
        }
        assert_eq!(
            TEST_CLOUD_SETTINGS_DELAYS.with(|delays| delays.borrow().len()),
            1
        );
        TEST_CLOUD_SETTINGS_DELAYS.with(|delays| delays.borrow_mut().clear());
        drop(unused);

        // An old queued request cannot borrow a new generation or create its DB.
        for invalidate_auth in [true, false] {
            let release = delay();
            let epoch = model.borrow().account_epoch;
            let (window, _, page, _) = host(&app, &model, Some(&installed));
            registered();
            heartbeat();
            assert!(!loaded(&page));
            assert!(text(&page).contains("Loading cloud-save settings…"));
            if invalidate_auth {
                auth::invalidate_session();
            } else {
                online::invalidate_library_session();
            }
            assert_eq!(model.borrow().account_epoch, epoch);
            wait(|| text(&page).contains("Close and reopen Properties"));
            let spinner = find_named_descendant(page.upcast_ref(), "cloud-settings-spinner")
                .and_downcast::<gtk::Spinner>()
                .unwrap();
            let retry =
                find_named_descendant(page.upcast_ref(), "cloud-settings-load-retry").unwrap();
            assert!(!spinner.get_visible() && !spinner.is_spinning());
            assert!(!retry.get_visible());
            registered();
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            assert!(!database.exists());
            assert!(!loaded(&page));
            window.destroy();
        }

        // Frozen admission uses the real local error UI; a current explicit retry works signed out.
        let frozen = crate::profile_reset::reserve().unwrap();
        let release = delay();
        let before = TEST_CLOUD_SETTINGS_DISPATCHES.get();
        let (window, _, page, _) = host(&app, &model, Some(&installed));
        wait(|| text(&page).contains("Profile reset"));
        assert!(!loaded(&page) && !database.exists());
        assert_eq!(TEST_CLOUD_SETTINGS_DISPATCHES.get(), before);
        let retry = find_named_descendant(page.upcast_ref(), "cloud-settings-load-retry")
            .and_downcast::<gtk::Button>()
            .unwrap();
        assert!(retry.get_visible() && retry.is_sensitive());
        drop(frozen);
        retry.emit_clicked();
        retry.emit_clicked();
        registered();
        assert_eq!(TEST_CLOUD_SETTINGS_DISPATCHES.get(), before + 1);
        assert!(!retry.get_visible());
        release.send(()).unwrap();
        wait(|| loaded(&page));
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(text(&page).contains("Never"));
        assert!(text(&page).contains("Cloud-save support has not been checked yet."));
        assert!(!auth::session_is_current(auth::session()));
        assert!(!model.borrow().network_available && model.borrow().account_token.is_none());
        assert_eq!(
            rusqlite::Connection::open(&database)
                .unwrap()
                .query_row("SELECT count(*) FROM cloud_save_settings", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        window.destroy();

        let store = StateStore::open().unwrap();
        store
            .set_cloud_save_preference(
                installed.product_id,
                crate::domain::CloudSavePreference::Enabled,
            )
            .unwrap();
        let locations = vec![crate::domain::CloudSaveLocation {
            name: "Synthetic saves".into(),
            path: database.parent().unwrap().join("inert-saves"),
            remote_namespace: "saves".into(),
            user_override: true,
        }];
        store
            .set_cloud_save_discovery(
                installed.product_id,
                &crate::domain::CloudSaveDiscovery {
                    availability: crate::domain::CloudSaveAvailability::Supported,
                    locations: locations.clone(),
                    checked_at: 123,
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .record_cloud_save_conflicts(
                installed.product_id,
                &[crate::domain::CloudSaveConflict {
                    location: "saves".into(),
                    relative_path: "inert.sav".into(),
                    local: None,
                    remote: None,
                }],
            )
            .unwrap();
        let full_error = "A synthetic cached cloud failure with its complete diagnostic tail.";
        store
            .set_cloud_save_status(
                installed.product_id,
                crate::domain::CloudSaveStatus::Error,
                Some(full_error),
            )
            .unwrap();
        drop(store);
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection.execute("UPDATE cloud_save_settings SET last_successful_sync=1700000000 WHERE product_id=?1", [installed.product_id]).unwrap();
        let before_record = StateStore::open()
            .unwrap()
            .cloud_save_record(installed.product_id)
            .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE")
            .unwrap();
        let probe = rusqlite::Connection::open(&database).unwrap();
        probe.busy_timeout(Duration::ZERO).unwrap();
        let blocked = probe
            .query_row("SELECT count(*) FROM cloud_save_settings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_err();
        assert_eq!(
            blocked.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        drop(probe);
        let (window, stack, page, entry) = host(&app, &model, Some(&installed));
        stack.set_visible_child_name("cloud");
        let spinner = find_named_descendant(page.upcast_ref(), "cloud-settings-spinner")
            .and_downcast::<gtk::Spinner>()
            .unwrap();
        wait(|| spinner.is_mapped());
        registered();
        heartbeat();
        assert!(!loaded(&page));
        assert!(text(&page).contains("Loading cloud-save settings…"));
        assert!(spinner.is_spinning());
        stack.set_visible_child_name("general");
        wait(|| entry.is_mapped());
        assert!(entry.grab_focus());
        let focus = gtk::prelude::GtkWindowExt::focus(&window);
        assert!(focus.is_some());
        assert!(
            !page.is_mapped(),
            "the Cloud Saves tab stays hidden during loading"
        );
        connection.execute_batch("ROLLBACK").unwrap();
        wait(|| loaded(&page));
        wait(|| crate::profile_reset::reserve().is_ok());
        assert_eq!(
            stack.child_by_name("cloud").as_ref(),
            Some(page.upcast_ref())
        );
        assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
        let switch = nodes(&page)
            .into_iter()
            .find_map(|widget| widget.downcast::<adw::SwitchRow>().ok())
            .unwrap();
        assert!(switch.is_active() && switch.is_sensitive());
        assert!(text(&page).contains(full_error));
        assert!(text(&page).contains("2023-11-14 22:13 UTC"));
        assert!(text(&page).contains(&locations[0].path.display().to_string()));
        let groups = nodes(&page)
            .iter()
            .filter_map(|widget| widget.downcast_ref::<adw::PreferencesGroup>())
            .map(|group| group.title().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            groups,
            ["GOG Cloud Saves", "Export and manage remote saves"]
        );
        assert_eq!(
            StateStore::open()
                .unwrap()
                .cloud_save_record(installed.product_id)
                .unwrap(),
            before_record
        );
        assert_eq!(
            connection
                .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            version
        );
        window.destroy();
        StateStore::open()
            .unwrap()
            .set_cloud_save_status(
                installed.product_id,
                crate::domain::CloudSaveStatus::Conflict,
                None,
            )
            .unwrap();
        let (window, _, page, _) = host(&app, &model, Some(&installed));
        wait(|| loaded(&page));
        assert!(text(&page).contains("1 pending conflict(s) require a manual choice"));
        window.destroy();
        drop(connection);

        // Returned read errors must not invent missing settings; Retry uses the repaired DB.
        std::fs::write(&database, b"invalid private cloud settings database").unwrap();
        let (window, _, page, _) = host(&app, &model, Some(&installed));
        wait(|| text(&page).contains("Could not load cloud-save settings:"));
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(!loaded(&page));
        let status = find_named_descendant(page.upcast_ref(), "cloud-settings-load-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        assert!(status.is_selectable() && status.wraps());
        let error = StateStore::open().err().unwrap();
        assert_eq!(
            status.text(),
            format!("Could not load cloud-save settings: {error:#}")
        );
        std::fs::remove_file(&database).unwrap();
        drop(StateStore::open().unwrap());
        let retry = find_named_descendant(page.upcast_ref(), "cloud-settings-load-retry")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let release = delay();
        let before = TEST_CLOUD_SETTINGS_DISPATCHES.get();
        retry.emit_clicked();
        retry.emit_clicked();
        assert_eq!(TEST_CLOUD_SETTINGS_DISPATCHES.get(), before + 1);
        release.send(()).unwrap();
        wait(|| loaded(&page));
        window.destroy();

        // A sensitivity observer may retry synchronously during error restoration.
        let frozen = Rc::new(RefCell::new(Some(crate::profile_reset::reserve().unwrap())));
        let (window, _, page, _) = host(&app, &model, Some(&installed));
        let retry = find_named_descendant(page.upcast_ref(), "cloud-settings-load-retry")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let spinner = find_named_descendant(page.upcast_ref(), "cloud-settings-spinner")
            .and_downcast::<gtk::Spinner>()
            .unwrap();
        let reentered = Rc::new(std::cell::Cell::new(false));
        let release = delay();
        let before = TEST_CLOUD_SETTINGS_DISPATCHES.get();
        retry.connect_sensitive_notify({
            let frozen = frozen.clone();
            let reentered = reentered.clone();
            move |button| {
                if button.is_sensitive() && !reentered.replace(true) {
                    drop(frozen.borrow_mut().take());
                    button.emit_clicked();
                    button.emit_clicked();
                }
            }
        });
        wait(|| reentered.get());
        heartbeat();
        assert_eq!(TEST_CLOUD_SETTINGS_DISPATCHES.get(), before + 1);
        registered();
        assert!(spinner.get_visible() && spinner.is_spinning());
        assert!(!retry.get_visible() && !retry.is_sensitive());
        assert!(text(&page).contains("Loading cloud-save settings…"));
        assert!(!loaded(&page));
        release.send(()).unwrap();
        wait(|| loaded(&page));
        window.destroy();

        // A completed record waiting for GTK still belongs to its original generations.
        for invalidate_auth in [true, false] {
            let (window, _, page, _) = host(&app, &model, Some(&installed));
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while crate::profile_reset::reserve().is_err() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(crate::profile_reset::reserve().is_ok());
            assert!(!loaded(&page), "the GTK receiver has not been pumped");
            if invalidate_auth {
                auth::invalidate_session();
            } else {
                online::invalidate_library_session();
            }
            wait(|| text(&page).contains("Close and reopen Properties"));
            assert!(!loaded(&page));
            window.destroy();
        }

        for logging_out in [false, true] {
            let release = delay();
            let generations = (online::account_session(), auth::session());
            let (window, _, page, _) = host(&app, &model, Some(&installed));
            if logging_out {
                model.borrow_mut().logout_pending = true;
            } else {
                model.borrow_mut().account_epoch += 1;
            }
            wait(|| text(&page).contains("Close and reopen Properties"));
            assert_eq!((online::account_session(), auth::session()), generations);
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            heartbeat();
            assert!(!loaded(&page));
            window.destroy();
            model.borrow_mut().logout_pending = false;
        }

        for disposition in ["close", "hide", "detach", "destroy"] {
            let release = delay();
            let (window, stack, page, entry) = host(&app, &model, Some(&installed));
            wait(|| window.is_mapped());
            registered();
            match disposition {
                "close" => {
                    window.set_hide_on_close(true);
                    window.close();
                    wait(|| !window.is_visible());
                }
                "hide" => window.set_visible(false),
                "detach" => stack.remove(&page),
                _ => window.destroy(),
            }
            heartbeat();
            registered();
            let weak_window = window.downgrade();
            let weak_page = page.downgrade();
            if disposition == "destroy" {
                drop((window, stack, page, entry));
                wait(|| weak_window.upgrade().is_none() && weak_page.upgrade().is_none());
            } else {
                release.send(()).unwrap();
                wait(|| crate::profile_reset::reserve().is_ok());
                heartbeat();
                assert!(!loaded(&page));
                window.destroy();
                continue;
            }
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
        }
        assert!(TEST_CLOUD_SETTINGS_DELAYS.with(|delays| delays.borrow().is_empty()));
    }

    #[test]
    #[ignore = "requires private p365 HOME/all XDG/TMP, GTK and D-Bus; inert files only"]
    fn launch_field_saves_skip_unchanged_signals_and_preserve_retry_admission() {
        let home = std::env::var("HOME").unwrap();
        assert!(home.starts_with("/tmp/ludomere-p365-"));
        let root = std::path::Path::new(&home).parent().unwrap();
        for key in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(std::path::Path::new(&std::env::var_os(key).unwrap()).starts_with(root));
        }
        assert!(
            crate::identity::database().starts_with(std::env::var_os("XDG_DATA_HOME").unwrap())
        );
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
        let directory = root.join("inert-game");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("game.exe"), b"inert test file").unwrap();
        let installed = Rc::new(RefCell::new(serde_json::from_value::<crate::domain::InstalledGame>(serde_json::json!({
            "product_id": 9265001, "library_id": "synthetic", "installation_directory": directory,
            "primary_executable": directory.join("game.exe"), "installer_files": [],
            "installer_complete": true, "installer_operating_system": "windows",
            "launch_arguments": ["--initial"], "state": "installed", "playtime_seconds": 0,
            "created_at": 10, "updated_at": 20
        })).unwrap()));
        let store = StateStore::open().unwrap();
        let retained_compatibility = crate::compatibility::GameCompatibilityPreferences {
            backend: crate::compatibility::CompatibilityBackendKind::Umu,
            prefix_slug: "current-profile".into(),
            profile: crate::compatibility::UmuProfile::fallback(),
            pending_profile: Some(crate::compatibility::UmuProfile::fallback()),
        };
        store
            .upsert_game_preferences(&crate::domain::GamePreferences {
                product_id: 9265001,
                executable_path: Some("game.exe".into()),
                launch_arguments: vec!["--initial".into()],
                compatibility: Some(retained_compatibility.clone()),
                created_at: 10,
                updated_at: 20,
                ..Default::default()
            })
            .unwrap();
        let executable = adw::EntryRow::new();
        executable.set_text("game.exe");
        let arguments = adw::EntryRow::new();
        arguments.set_text("--initial");
        let status = gtk::Label::new(None);
        let state = Rc::new(LaunchSaveState {
            submitted: RefCell::new(Some((executable.text(), arguments.text()))),
            ..Default::default()
        });
        let refreshed = Rc::new(std::cell::Cell::new(0));
        let refresh: Rc<dyn Fn()> = Rc::new({
            let refreshed = refreshed.clone();
            move || refreshed.set(refreshed.get() + 1)
        });
        let session = online::account_session();
        let save: Rc<dyn Fn()> = Rc::new({
            let executable = executable.downgrade();
            let arguments = arguments.downgrade();
            let status = status.clone();
            let installed = installed.clone();
            let state = state.clone();
            move || {
                let (Some(executable), Some(arguments)) =
                    (executable.upgrade(), arguments.upgrade())
                else {
                    return;
                };
                persist_launch_settings(
                    &executable,
                    &arguments,
                    &installed,
                    &status,
                    &refresh,
                    session,
                    &state,
                );
            }
        });
        connect_launch_save(&executable, save.clone());
        connect_launch_save(&arguments, save);
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.LaunchFieldsTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let group = adw::PreferencesGroup::new();
        group.add(&executable);
        group.add(&arguments);
        group.add(&status);
        window.set_content(Some(&group));
        window.present();
        wait(|| arguments.is_mapped());
        let focus = arguments
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .find_map(|object| object.downcast::<gtk::EventControllerFocus>().ok())
            .unwrap();
        focus.emit_by_name::<()>("leave", &[]);
        arguments.emit_by_name::<()>("entry-activated", &[]);
        window.set_visible(false);
        assert_eq!(
            state.revision.get(),
            0,
            "unchanged signals must submit no work"
        );
        assert_eq!(
            store.game_preferences(9265001).unwrap().unwrap().updated_at,
            20
        );
        assert_eq!(refreshed.get(), 0);
        window.present();
        wait(|| arguments.is_mapped());

        let unrelated_operation = crate::operation_gate::try_acquire().unwrap();
        arguments.set_text("--superseded");
        arguments.set_text("--latest 'two words'");
        wait(|| refreshed.get() == 1);
        assert_eq!(
            store
                .game_preferences(9265001)
                .unwrap()
                .unwrap()
                .compatibility,
            Some(retained_compatibility)
        );
        assert_eq!(
            store
                .game_preferences(9265001)
                .unwrap()
                .unwrap()
                .launch_arguments,
            ["--latest", "two words"]
        );
        drop(unrelated_operation);
        arguments.set_text("--enter");
        arguments.emit_by_name::<()>("entry-activated", &[]);
        wait(|| refreshed.get() == 2);
        arguments.set_text("--focus");
        focus.emit_by_name::<()>("leave", &[]);
        wait(|| refreshed.get() == 3);
        arguments.set_text("--unmap");
        window.set_visible(false);
        wait(|| refreshed.get() == 4);
        assert_eq!(
            store
                .game_preferences(9265001)
                .unwrap()
                .unwrap()
                .launch_arguments,
            ["--unmap"]
        );
        window.present();
        wait(|| arguments.is_mapped());

        arguments.set_text("'");
        arguments.emit_by_name::<()>("entry-activated", &[]);
        assert!(status.text().starts_with("Invalid launch options:"));
        arguments.set_text("--retry");
        executable.set_text("../outside.exe");
        executable.emit_by_name::<()>("entry-activated", &[]);
        assert!(status.text().contains("path inside the game directory"));
        executable.set_text("missing.exe");
        executable.emit_by_name::<()>("entry-activated", &[]);
        wait(|| status.text().starts_with("Could not save:"));
        assert!(state.submitted.borrow().is_none());
        assert_eq!(refreshed.get(), 4);
        std::fs::write(directory.join("missing.exe"), b"inert retry target").unwrap();
        executable.emit_by_name::<()>("entry-activated", &[]);
        wait(|| refreshed.get() == 5);
        assert_eq!(
            store
                .game_preferences(9265001)
                .unwrap()
                .unwrap()
                .executable_path,
            Some("missing.exe".into())
        );

        // Queue behind real policy work: admission must already prevent profile reset.
        let (entered, ready) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let blocker = update_policies::policy_request(move || {
            entered.send(()).unwrap();
            held.recv().unwrap();
            Ok(())
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        arguments.set_text("--obsolete-session");
        arguments.emit_by_name::<()>("entry-activated", &[]);
        assert!(
            crate::profile_reset::reserve()
                .err()
                .unwrap()
                .to_string()
                .contains("saving launch settings")
        );
        drop(store);
        std::fs::remove_file(crate::identity::database()).unwrap();
        online::invalidate_library_session();
        release.send(()).unwrap();
        blocker
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        update_policies::policy_request(|| Ok(()))
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(
            !crate::identity::database().exists(),
            "stale queued save must not recreate the profile"
        );
        wait(|| status.text().contains("account changed"));
        assert_eq!(refreshed.get(), 5);
        drop(crate::profile_reset::reserve().unwrap());
        window.destroy();
    }

    #[test]
    #[ignore = "requires private p363 HOME/all XDG, GTK and D-Bus; no branch workers or keyring"]
    fn branch_controls_track_selection_preparation_and_started_handoff() {
        let home = std::env::var("HOME").unwrap();
        assert!(home.starts_with("/tmp/ludomere-p363-"));
        let root = std::path::Path::new(&home).parent().unwrap();
        for key in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(std::path::Path::new(&std::env::var_os(key).unwrap()).starts_with(root));
        }
        adw::init().unwrap();
        let branches = [None, Some("beta".into()), Some("stable".into())];
        let names = gtk::StringList::new(&["Master", "beta", "stable"]);
        let selector = gtk::DropDown::new(Some(names.clone()), gtk::Expression::NONE);
        selector.set_selected(1);
        let switch = gtk::Button::with_label("Switch");
        let forget = gtk::Button::with_label("Forget Password");
        let state = Rc::new(std::cell::Cell::new(BranchActionState::Idle));
        let refresh = connect_branch_selection(
            &branches,
            Some(&branches[1]),
            &selector,
            &switch,
            &forget,
            &state,
            &Rc::new(std::cell::Cell::new(false)),
            Rc::new(|| None),
        );
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.BranchEligibilityTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&selector);
        content.append(&switch);
        content.append(&forget);
        window.set_content(Some(&content));
        window.present();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !forget.is_mapped() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(forget.is_mapped());
        assert!(!switch.is_sensitive());
        assert_eq!(
            switch.tooltip_text().as_deref(),
            Some("This branch is already installed.")
        );
        assert!(
            forget.is_sensitive(),
            "named-branch forgetting remains idempotent without probing credentials"
        );
        selector.set_selected(0);
        assert!(switch.is_sensitive(), "Master is a valid different branch");
        assert!(!forget.is_sensitive());
        assert_eq!(
            forget.tooltip_text().as_deref(),
            Some("Master does not use a saved branch password.")
        );
        selector.set_selected(2);
        assert!(switch.is_sensitive());
        assert!(forget.is_sensitive());
        assert!(switch.tooltip_text().is_none());
        assert!(forget.tooltip_text().is_none());
        names.splice(0, names.n_items(), &[]);
        assert_eq!(selector.selected(), gtk::INVALID_LIST_POSITION);
        assert!(!switch.is_sensitive());
        assert!(!forget.is_sensitive());
        assert_eq!(
            switch.tooltip_text().as_deref(),
            Some("Choose a branch before switching.")
        );
        assert_eq!(
            forget.tooltip_text().as_deref(),
            Some("Choose a named branch before forgetting its password.")
        );
        names.splice(0, 0, &["Master", "beta", "stable"]);
        selector.set_selected(2);
        state.set(BranchActionState::Preparing);
        refresh();
        for selection in [1, 0, 2] {
            selector.set_selected(selection);
            assert!(!switch.is_sensitive());
            assert!(!forget.is_sensitive());
            assert_eq!(
                switch.tooltip_text().as_deref(),
                Some("Wait for branch preparation to finish.")
            );
            assert!(forget.tooltip_text().unwrap().contains("before forgetting"));
        }
        // Errors recompute the current selection, not the branch originally submitted.
        selector.set_selected(1);
        state.set(BranchActionState::Idle);
        refresh();
        assert!(!switch.is_sensitive());
        assert!(forget.is_sensitive());
        selector.set_selected(2);
        assert!(switch.is_sensitive());
        // Disconnection also returns to Idle, including when no branch is selectable.
        state.set(BranchActionState::Preparing);
        refresh();
        names.splice(0, names.n_items(), &[]);
        state.set(BranchActionState::Idle);
        refresh();
        assert!(!switch.is_sensitive());
        assert!(!forget.is_sensitive());
        names.splice(0, 0, &["Master", "beta", "stable"]);
        selector.set_selected(2);
        state.set(BranchActionState::Preparing);
        refresh();
        state.set(BranchActionState::Started);
        refresh();
        assert!(!switch.is_sensitive());
        assert!(forget.is_sensitive());
        assert!(switch.tooltip_text().unwrap().contains("already started"));
        selector.set_selected(0);
        assert!(!switch.is_sensitive());
        assert!(!forget.is_sensitive());
        selector.set_selected(1);
        assert!(!switch.is_sensitive());
        assert!(forget.is_sensitive());

        // A known installed Master is distinct from absent provenance.
        let master_selector = gtk::DropDown::from_strings(&["Master", "beta", "stable"]);
        let master_switch = gtk::Button::new();
        let master_forget = gtk::Button::new();
        let idle = Rc::new(std::cell::Cell::new(BranchActionState::Idle));
        let _master_refresh = connect_branch_selection(
            &branches,
            Some(&None),
            &master_selector,
            &master_switch,
            &master_forget,
            &idle,
            &Rc::new(std::cell::Cell::new(false)),
            Rc::new(|| None),
        );
        content.append(&master_selector);
        content.append(&master_switch);
        content.append(&master_forget);
        assert!(!master_switch.is_sensitive());
        assert!(!master_forget.is_sensitive());
        master_selector.set_selected(1);
        assert!(master_switch.is_sensitive());
        assert!(master_forget.is_sensitive());
        let unknown_selector = gtk::DropDown::from_strings(&["Master", "beta", "stable"]);
        let unknown_switch = gtk::Button::new();
        let unknown_forget = gtk::Button::new();
        let _unknown_refresh = connect_branch_selection(
            &branches,
            None,
            &unknown_selector,
            &unknown_switch,
            &unknown_forget,
            &idle,
            &Rc::new(std::cell::Cell::new(false)),
            Rc::new(|| None),
        );
        assert!(
            unknown_switch.is_sensitive(),
            "absent provenance must not imply installed Master"
        );
        assert!(!unknown_forget.is_sensitive());
        window.destroy();
    }

    #[test]
    fn offline_migration_plan_preserves_saved_options_and_missing_row_fallback() {
        use crate::compatibility::{
            CompatibilityBackendKind, GameCompatibilityPreferences, UmuProfile, UmuProfileSource,
        };
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.db");
        let store = StateStore::open_at(&database).unwrap();
        let library = crate::config::GameLibrary {
            id: "chosen".into(),
            name: "Chosen".into(),
            path: root.path().join("chosen-library"),
            default: true,
        };
        let retained = GameCompatibilityPreferences {
            backend: CompatibilityBackendKind::Umu,
            prefix_slug: "previous-folder".into(),
            profile: UmuProfile {
                game_id: "umu-saved".into(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            },
            pending_profile: Some(UmuProfile {
                game_id: "umu-pending".into(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            }),
        };
        for (index, (os, saved)) in [
            (Some("windows"), 2),
            (Some("windows"), 1),
            (Some("windows"), 0),
            (Some("linux"), 2),
            (Some("LiNuX"), 1),
            (Some("linux"), 0),
            (Some("unknown"), 2),
            (None, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let id = index as i64 + 1;
            let detail = DetailPageModel::game(
                Game {
                    product_id: id,
                    slug: "chosen-folder".into(),
                    ..Default::default()
                },
                false,
            );
            let candidate = crate::installation::InstallerCandidate {
                product_id: id,
                revision_id: Some(91),
                version: Some("chosen-version".into()),
                operating_system: os.map(str::to_owned),
                language: Some("Polish".into()),
                paths: vec![root.path().join("chosen-installer")],
                launcher: None,
                method: if os == Some("windows") {
                    crate::installation::InstallationMethod::WindowsCompatibility
                } else {
                    crate::installation::InstallationMethod::NativeLinux
                },
                total_size: 4,
                currently_offered: true,
                complete: true,
            };
            let mut installed = offline_installation_plan(id, &library, "old-folder", &candidate);
            installed.installer_operating_system = Some("windows".into());
            installed.compatibility = Some(GameCompatibilityPreferences {
                profile: UmuProfile {
                    game_id: "umu-existing-fallback".into(),
                    ..retained.profile.clone()
                },
                ..retained.clone()
            });
            installed.launch_arguments = vec!["--existing-fallback".into()];
            installed.primary_executable = Some(installed.installation_directory.join("old.exe"));
            installed.last_played_at = Some(100);
            installed.playtime_seconds = 345;
            installed.created_at = 50;
            if saved != 0 {
                store
                    .upsert_game_preferences(&crate::domain::GamePreferences {
                        product_id: id,
                        launch_arguments: if saved == 2 {
                            vec!["--saved".into()]
                        } else {
                            vec![]
                        },
                        compatibility: (saved == 2).then_some(retained.clone()),
                        ..Default::default()
                    })
                    .unwrap();
            }
            let before = store.game_preferences(id).unwrap();
            let plan =
                prepare_offline_migration_plan(&store, &detail, &installed, &library, &candidate)
                    .unwrap();
            assert_eq!(plan.product_id, id);
            assert_eq!(plan.library_id, library.id);
            assert_eq!(
                plan.installation_directory,
                library.path.join("chosen-folder")
            );
            assert_eq!(plan.installer_operating_system.as_deref(), os);
            assert_eq!(plan.installed_version, candidate.version);
            assert_eq!(plan.installer_language, candidate.language);
            assert_eq!(plan.installer_revision_id, candidate.revision_id);
            assert_eq!(plan.installer_files, candidate.paths);
            assert_eq!(plan.primary_executable, None);
            assert_eq!(
                plan.launch_arguments,
                match saved {
                    2 => vec!["--saved"],
                    1 => vec![],
                    _ => vec!["--existing-fallback"],
                }
            );
            assert_eq!(
                plan.compatibility,
                if os == Some("windows") {
                    match saved {
                        2 => Some(retained.clone()),
                        1 => None,
                        _ => installed.compatibility.clone(),
                    }
                } else {
                    None
                }
            );
            assert_eq!(plan.last_played_at, installed.last_played_at);
            assert_eq!(plan.playtime_seconds, installed.playtime_seconds);
            assert_eq!(plan.created_at, installed.created_at);
            assert_eq!(store.game_preferences(id).unwrap(), before);
            assert!(
                !library.path.exists(),
                "construction must precede destructive migration"
            );
        }
        let detail = DetailPageModel::game(
            Game {
                product_id: 99,
                slug: "chosen-folder".into(),
                ..Default::default()
            },
            false,
        );
        let candidate = crate::installation::InstallerCandidate {
            product_id: 99,
            revision_id: None,
            version: None,
            operating_system: Some("linux".into()),
            language: None,
            paths: vec![],
            launcher: None,
            method: crate::installation::InstallationMethod::NativeLinux,
            total_size: 0,
            currently_offered: false,
            complete: true,
        };
        let installed = offline_installation_plan(99, &library, "old-folder", &candidate);
        let no_preferences =
            prepare_offline_migration_plan(&store, &detail, &installed, &library, &candidate)
                .unwrap();
        assert!(no_preferences.compatibility.is_none());
        assert!(no_preferences.launch_arguments.is_empty());
        rusqlite::Connection::open(database)
            .unwrap()
            .execute("DROP TABLE game_preferences", [])
            .unwrap();
        assert!(
            prepare_offline_migration_plan(&store, &detail, &installed, &library, &candidate)
                .is_err()
        );
        assert!(!library.path.exists());
    }

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
        let closed_dialog = view.dialog.downgrade();
        drop(view);
        wait_until(|| closed_dialog.upgrade().is_none());

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
        assert!(prepare_migration_choices(&game, &installed, session.wrapping_add(1)).is_err());
        let reservation = crate::profile_reset::reserve().unwrap();
        assert!(prepare_migration_choices(&game, &installed, session).is_err());
        drop(reservation);
        assert!(!crate::identity::database().exists());

        let ready = MigrationView::new();
        ready.dialog.present(Some(&window));
        wire_migration_choices(
            &window,
            &model,
            &game,
            &installed,
            &ready,
            MigrationChoices {
                config: Config::default(),
                marker: crate::installation::installation_marker_from_game(&installed, vec![]),
                candidates: crate::installation::InstallerCandidates::default(),
                choices: vec![crate::installation::FreshInstallSource::GalaxyWindows],
                current: Some(crate::config::PreferredInstallationSource::LinuxOffline),
                locations: vec![],
                galaxy_preflight: Ok(()),
            },
            model.borrow().account_epoch,
            session,
        );
        assert!(ready.proceed.is_sensitive());
        let ready_dialog = ready.dialog.downgrade();
        let ready_button = ready.proceed.downgrade();
        ready.close.emit_clicked();
        wait_until(|| ready.closed.get());
        drop(ready);
        wait_until(|| ready_dialog.upgrade().is_none() && ready_button.upgrade().is_none());

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
