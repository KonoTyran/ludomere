use super::*;

pub(super) fn detail_file_management(
    game: &DetailPageModel,
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    installed: Option<crate::domain::InstalledGame>,
    refresh_after_change: Rc<dyn Fn()>,
    activate_primary_action: Rc<dyn Fn()>,
) -> DetailFileManagement {
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("emblem-system-symbolic");
    menu.set_tooltip_text(Some("Manage game files"));
    menu.set_widget_name("game-management-menu");
    menu.add_css_class("square-action");
    menu.add_css_class("steam-utility-action");

    let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
    actions.set_margin_start(6);
    actions.set_margin_end(6);
    actions.set_margin_top(6);
    actions.set_margin_bottom(6);
    let mut main_actions = Vec::<gtk::Button>::new();
    let mut manage_submenu_actions = Vec::<gtk::Button>::new();

    let primary_action = context_primary_action(game, installed.as_ref(), &model.borrow().config);
    let active_operation = crate::installation::installation_operation_snapshot(game.product_id)
        .filter(|snapshot| {
            snapshot.queued
                || matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Installing
                        | crate::domain::InstallationState::Uninstalling
                )
        });
    let active_download = model.borrow().download_jobs.iter().any(|job| {
        job.product_id == game.product_id
            && matches!(
                job.state,
                DownloadState::Queued | DownloadState::Downloading
            )
    });
    let game_running = crate::installation::is_game_running(game.product_id);
    let game_stopping = crate::installation::is_game_stopping(game.product_id);
    let proxy_action = gtk::Button::new();
    let proxy_content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    proxy_content.set_halign(gtk::Align::Center);
    let (proxy_icon, proxy_label) = active_operation.as_ref().map_or_else(
        || {
            if active_download {
                ("media-playback-pause-symbolic", "Pause")
            } else if game_stopping {
                ("process-stop-symbolic", "Stopping")
            } else if game_running {
                ("media-playback-stop-symbolic", "Stop")
            } else {
                (primary_action.icon(), primary_action.label())
            }
        },
        |snapshot| {
            let _ = snapshot;
            ("process-stop-symbolic", "Cancel")
        },
    );
    proxy_content.append(&gtk::Image::from_icon_name(proxy_icon));
    proxy_content.append(&gtk::Label::new(Some(proxy_label)));
    proxy_action.set_child(Some(&proxy_content));
    proxy_action.add_css_class("steam-primary-action");
    proxy_action.add_css_class("context-primary-action");
    if active_operation.is_some() || active_download || game_running {
        proxy_action.add_css_class("operational-action");
    }
    proxy_action.set_sensitive(!game_stopping);
    if matches!(
        primary_action,
        GamePrimaryAction::Download
            | GamePrimaryAction::Install
            | GamePrimaryAction::DownloadUpdate
            | GamePrimaryAction::InstallUpdate
    ) {
        proxy_action.add_css_class("download-state");
    }
    proxy_action.connect_clicked(move |_| activate_primary_action());
    actions.append(&proxy_action);
    main_actions.push(proxy_action);

    if let Some(favorite) = game.favorite {
        let favorite_action = management_menu_button(if favorite {
            "Remove from Favorites"
        } else {
            "Add to Favorites"
        });
        favorite_action.set_action_name(Some("win.favorite"));
        favorite_action.set_action_target_value(Some(&game.product_id.to_variant()));
        actions.append(&favorite_action);
        main_actions.push(favorite_action);
    }

    let verify = management_menu_button("Verify Downloads");
    let check_updates = management_menu_button("Check for Updates");
    check_updates.set_action_name(Some("win.refresh-files"));
    check_updates.set_action_target_value(Some(&game.product_id.to_variant()));

    let manage = gtk::MenuButton::new();
    manage.set_label("Manage");
    manage.set_direction(gtk::ArrowType::Right);
    manage.add_css_class("flat");
    manage.add_css_class("context-menu-item");
    manage.set_halign(gtk::Align::Fill);
    manage.set_hexpand(true);

    let manage_actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
    manage_actions.set_margin_start(6);
    manage_actions.set_margin_end(6);
    manage_actions.set_margin_top(6);
    manage_actions.set_margin_bottom(6);
    manage_actions.append(&check_updates);
    manage_actions.append(&verify);
    manage_submenu_actions.push(check_updates.clone());
    manage_submenu_actions.push(verify.clone());

    if let Some(installed) = installed.clone() {
        manage_actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let browse = management_menu_button("Browse Local Files");
        {
            let path = installed.installation_directory.clone();
            let window = window.clone();
            browse.connect_clicked(move |_| {
                super::widgets::file_open::open_directory(
                    &path,
                    &window,
                    "installed-game directory",
                );
            });
        }
        manage_actions.append(&browse);
        manage_submenu_actions.push(browse);

        let repair = management_menu_button("Repair Installation");
        repair.set_tooltip_text(Some(
            "Rerun the current full game installer and all available DLC installers",
        ));
        {
            let window = window.clone();
            let game = game.clone();
            let model = model.clone();
            repair.connect_clicked(move |_| show_repair_dialog(&window, &model, &game));
        }
        manage_actions.append(&repair);
        manage_submenu_actions.push(repair);
        manage_actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let uninstall = management_menu_button("Uninstall");
        uninstall.add_css_class("destructive-action");
        let window = window.clone();
        let title = game.title.clone();
        let refresh_after_uninstall = refresh_after_change.clone();
        uninstall.connect_clicked(move |button| {
            let confirmation = adw::AlertDialog::builder()
                .heading(format!("Uninstall {title}?"))
                .body(format!(
                    "Remove the installed game from {}? Downloaded installers, patches, extras, and DLC backups will be kept.",
                    installed.installation_directory.display()
                ))
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("uninstall", "Uninstall")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation
                .set_response_appearance("uninstall", adw::ResponseAppearance::Destructive);
            let installed = installed.clone();
            let button = button.clone();
            let response_window = window.clone();
            let refresh_after_change = refresh_after_uninstall.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "uninstall" {
                    return;
                }
                button.set_sensitive(false);
                let receiver = crate::installation::subscribe_installation_events();
                let product_id = installed.product_id;
                if !crate::installation::enqueue_uninstallation(installed) {
                    button.set_sensitive(true);
                    return;
                }
                let button = button.clone();
                let window = response_window.clone();
                let refresh_after_change = refresh_after_change.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match receiver.try_recv() {
                        Ok(crate::installation::InstallationManagerEvent::Uninstallation {
                            product_id: event_product_id,
                            event: crate::installation::UninstallationEvent::Started,
                        }) if event_product_id == product_id => {
                            refresh_after_change();
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::InstallationManagerEvent::Uninstallation {
                            product_id: event_product_id,
                            event: crate::installation::UninstallationEvent::Complete,
                        }) if event_product_id == product_id => {
                            refresh_after_change();
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::InstallationManagerEvent::Uninstallation {
                            product_id: event_product_id,
                            event: crate::installation::UninstallationEvent::Cancelled,
                        }) if event_product_id == product_id => {
                            button.set_sensitive(true);
                            refresh_after_change();
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::InstallationManagerEvent::Uninstallation {
                            product_id: event_product_id,
                            event: crate::installation::UninstallationEvent::Failed(error),
                        }) if event_product_id == product_id => {
                            button.set_sensitive(true);
                            let dialog = adw::AlertDialog::builder()
                                .heading("Could not uninstall game")
                                .body(error)
                                .build();
                            dialog.add_response("close", "Close");
                            dialog.present(Some(&window));
                            glib::ControlFlow::Break
                        }
                        Ok(_) | Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            button.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                    }
                });
            });
        });
        manage_actions.append(&uninstall);
        manage_submenu_actions.push(uninstall);
    }
    let manage_popover = gtk::Popover::new();
    manage_popover.add_css_class("game-management-popover");
    manage_popover.set_child(Some(&manage_actions));
    manage.set_popover(Some(&manage_popover));
    let hover = gtk::EventControllerMotion::new();
    {
        let manage_popover = manage_popover.clone();
        hover.connect_enter(move |_, _, _| manage_popover.popup());
    }
    manage.add_controller(hover);
    actions.append(&manage);

    actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let game_settings = management_menu_button("Properties");
    {
        let window = window.clone();
        let game = game.clone();
        let installed = installed.clone();
        let model = model.clone();
        let refresh_after_change = refresh_after_change.clone();
        game_settings.connect_clicked(move |_| {
            show_game_settings(
                &window,
                &model,
                &game,
                installed.clone(),
                refresh_after_change.clone(),
            );
        });
    }
    actions.append(&game_settings);
    main_actions.push(game_settings);

    let popover = gtk::Popover::new();
    popover.add_css_class("game-management-popover");
    popover.set_child(Some(&actions));
    menu.set_popover(Some(&popover));
    for action in main_actions {
        let popover = popover.clone();
        action.connect_clicked(move |_| popover.popdown());
    }
    for action in manage_submenu_actions {
        let popover = popover.clone();
        let manage_popover = manage_popover.clone();
        action.connect_clicked(move |_| {
            manage_popover.popdown();
            popover.popdown();
        });
    }

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    status.set_visible(false);
    let progress = gtk::ProgressBar::new();
    progress.set_hexpand(true);
    progress.set_visible(false);

    {
        let window = window.clone();
        let product_id = game.product_id;
        let title = game.title.clone();
        let artifacts = game.remote_artifacts.clone();
        let access_token = model
            .borrow()
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        let status = status.clone();
        let progress = progress.clone();
        verify.connect_clicked(move |button| {
            let confirmation = adw::AlertDialog::builder()
                .heading("Verify and repair downloads?")
                .body("Valid files are unchanged. Files that fail GOG checksum verification will be permanently deleted and downloaded again.")
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("verify", "Verify and Repair")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation.set_response_appearance("verify", adw::ResponseAppearance::Destructive);
            let request = VerificationRequest {
                product_id,
                title: title.clone(),
                artifacts: artifacts.clone(),
                access_token: access_token.clone(),
            };
            let button = button.clone();
            let window = window.clone();
            let response_window = window.clone();
            let status = status.clone();
            let progress = progress.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response == "verify" {
                    start_product_verification(
                        request,
                        &button,
                        &response_window,
                        &status,
                        &progress,
                    );
                }
            });
        });
    }
    restore_verification_display(game.product_id, &verify, &status, &progress);

    DetailFileManagement {
        menu,
        status,
        progress,
    }
}

fn management_menu_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("flat");
    button.add_css_class("context-menu-item");
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

pub(super) fn activate_context_primary_action(
    widgets: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    game: DetailPageModel,
) {
    if crate::installation::installation_operation_snapshot(game.product_id).is_some_and(
        |snapshot| {
            snapshot.queued
                || matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Installing
                        | crate::domain::InstallationState::Uninstalling
                )
        },
    ) {
        crate::installation::cancel_operation(game.product_id);
        return;
    }
    let active_downloads = model
        .borrow()
        .download_jobs
        .iter()
        .filter(|job| {
            job.product_id == game.product_id
                && matches!(
                    job.state,
                    DownloadState::Queued | DownloadState::Downloading
                )
        })
        .map(|job| job.job_id.clone())
        .collect::<Vec<_>>();
    if !active_downloads.is_empty() {
        for job_id in active_downloads {
            crate::download::cancel(&job_id);
        }
        return;
    }
    if crate::installation::is_game_running(game.product_id) {
        crate::installation::stop_game(game.product_id);
        return;
    }
    let libraries = model.borrow().config.game_libraries.clone();
    let installed = StateStore::open().ok().and_then(|store| {
        crate::installation::reconcile_installed_games(&store, &libraries)
            .ok()?
            .into_iter()
            .find(|installed| installed.product_id == game.product_id)
    });
    match context_primary_action(&game, installed.as_ref(), &model.borrow().config) {
        GamePrimaryAction::Install => show_install_dialog(&widgets.window, model, &game),
        GamePrimaryAction::InstallUpdate => {
            if model.borrow().config.prefer_patch_updates
                && installed.as_ref().is_some_and(|installed| {
                    try_run_preferred_patch(&widgets.window, &game, installed)
                })
            {
                return;
            }
            show_update_dialog(&widgets.window, model, &game)
        }
        GamePrimaryAction::Play => {
            let Some(installed) = installed else { return };
            if installed.primary_executable.is_none() {
                let retry_widgets = widgets.clone();
                let retry_model = model.clone();
                let retry_game = game.clone();
                if prompt_for_windows_executable(
                    &widgets.window,
                    &game.title,
                    &installed,
                    Rc::new(move || {
                        activate_context_primary_action(
                            &retry_widgets,
                            &retry_model,
                            retry_game.clone(),
                        )
                    }),
                ) {
                    return;
                }
            }
            let receiver = crate::installation::launch_game(installed);
            let window = widgets.window.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(
                        event @ (crate::installation::LaunchEvent::EnablementRequired { .. }
                        | crate::installation::LaunchEvent::PreLaunchConflict { .. }
                        | crate::installation::LaunchEvent::LaunchWithoutSyncRequired {
                            ..
                        }
                        | crate::installation::LaunchEvent::SyncWarning(_)
                        | crate::installation::LaunchEvent::PostExitSync(_)
                        | crate::installation::LaunchEvent::PostExitConflict(_)),
                    ) => {
                        present_cloud_launch_event(&window, event);
                        glib::ControlFlow::Continue
                    }
                    Ok(crate::installation::LaunchEvent::Started) => glib::ControlFlow::Continue,
                    Ok(crate::installation::LaunchEvent::CloudSyncStarted(_)) => {
                        glib::ControlFlow::Continue
                    }
                    Ok(crate::installation::LaunchEvent::Exited { .. }) => glib::ControlFlow::Break,
                    Ok(crate::installation::LaunchEvent::Failed(error)) => {
                        let dialog = adw::AlertDialog::builder()
                            .heading("Could not run game")
                            .body(error)
                            .build();
                        dialog.add_response("close", "Close");
                        dialog.present(Some(&window));
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                }
            });
        }
        GamePrimaryAction::Download | GamePrimaryAction::DownloadUpdate => {
            if model.borrow().account_token.is_none() {
                widgets.reconnect.emit_clicked();
            } else {
                show_download_selector(widgets, model, &game);
            }
        }
    }
}

fn try_run_preferred_patch(
    window: &adw::ApplicationWindow,
    game: &DetailPageModel,
    installed: &crate::domain::InstalledGame,
) -> bool {
    let Ok(store) = StateStore::open() else {
        return false;
    };
    let Ok(mut patches) = store.managed_files() else {
        return false;
    };
    patches.retain(|file| {
        file.product_id == game.product_id
            && file.kind == ArtifactKind::Patch
            && file.present
            && file.path.is_file()
            && file
                .operating_system
                .as_deref()
                .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
            && file
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            && installed
                .installed_version
                .as_deref()
                .is_none_or(|version| {
                    file.filename.contains(version)
                        || file
                            .version
                            .as_deref()
                            .is_some_and(|label| label.contains(version))
                })
    });
    patches.sort_by_key(|file| file.revision_id.unwrap_or_default());
    let Some(patch) = patches.pop() else {
        return false;
    };
    let target_version = crate::installation::patch_target_version(patch.version.as_deref());
    let confirmation = adw::AlertDialog::builder()
        .heading("Apply preferred patch update?")
        .body("Ludomere will apply the downloaded patch and record the base game and installed DLC as current if it exits successfully. Launch the game afterward to verify it; use Repair Installation if necessary.")
        .build();
    confirmation.add_responses(&[("cancel", "Cancel"), ("run", "Apply Patch")]);
    confirmation.set_default_response(Some("run"));
    confirmation.set_close_response("cancel");
    let window_for_response = window.clone();
    let installed = installed.clone();
    confirmation.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if response != "run" { return; }
        let receiver = crate::installation::run_patch(
            installed.clone(),
            patch.path.clone(),
            target_version.clone(),
        );
        let window = window_for_response.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match receiver.try_recv() {
            Ok(crate::installation::PatchEvent::Started { .. }) => glib::ControlFlow::Continue,
            Ok(crate::installation::PatchEvent::Complete { .. }) => {
                let dialog = adw::AlertDialog::builder()
                    .heading("Patch update completed")
                    .body("Launch the game to verify the update. If it did not apply correctly, choose Repair Installation from the cog menu.")
                    .build();
                dialog.add_response("close", "Close");
                dialog.present(Some(&window));
                glib::ControlFlow::Break
            }
            Ok(crate::installation::PatchEvent::Failed(error)) => {
                let dialog = adw::AlertDialog::builder()
                    .heading("Patch update failed")
                    .body(error)
                    .build();
                dialog.add_response("close", "Close");
                dialog.present(Some(&window));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    });
    true
}

fn context_primary_action(
    game: &DetailPageModel,
    installed: Option<&crate::domain::InstalledGame>,
    config: &Config,
) -> GamePrimaryAction {
    let store = StateStore::open().ok();
    let installed_update = installed.is_some_and(|installed| {
        store.as_ref().is_some_and(|store| {
            store
                .installation_update_available(installed)
                .unwrap_or(false)
        })
    });
    let backup_update = store.is_some_and(|store| {
        store
            .installer_backup_update_available(game.product_id)
            .unwrap_or(false)
    });
    let dlc_action = owned_dlc_action_state(game, config, installed.is_some());
    let current_installer_downloaded = default_installers_are_downloaded(game, config);
    primary_action_for_state(
        installed.is_some(),
        installed_update,
        backup_update,
        current_installer_downloaded,
        dlc_action,
    )
}

pub(super) struct FilesPageOptions<'a> {
    pub access_token: Option<&'a str>,
    pub download_directory: &'a std::path::Path,
    pub installer_defaults: &'a InstallerFilterDefaults,
    pub show_retired_artifacts: bool,
    pub management: Option<&'a DetailFileManagement>,
    pub installed: Option<&'a crate::domain::InstalledGame>,
}

pub(super) fn build_files_page(
    game: &DetailPageModel,
    window: &adw::ApplicationWindow,
    options: FilesPageOptions<'_>,
) -> gtk::Box {
    let FilesPageOptions {
        access_token,
        download_directory,
        installer_defaults,
        show_retired_artifacts,
        management,
        installed,
    } = options;
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_margin_top(12);

    let managed = StateStore::open()
        .and_then(|store| store.managed_files())
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.product_id == game.product_id && file.present)
        .collect::<Vec<_>>();
    let local_files = |kind| {
        managed
            .iter()
            .filter(|file| file.kind == kind)
            .map(|file| LibraryFile {
                name: file.filename.clone(),
                path: file.path.clone(),
                size: file.size,
            })
            .collect::<Vec<_>>()
    };
    let installers = match local_files(ArtifactKind::Installer) {
        files if files.is_empty() => game.installers.clone(),
        files => files,
    };
    let patches = match local_files(ArtifactKind::Patch) {
        files if files.is_empty() => game.patches.clone(),
        files => files,
    };
    let extras = match local_files(ArtifactKind::Extra) {
        files if files.is_empty() => game.extras.clone(),
        files => files,
    };
    let managed_disk_usage = managed.iter().map(|file| file.size).sum::<u64>();
    let disk_usage = if managed_disk_usage == 0 {
        game.disk_usage
    } else {
        managed_disk_usage
    };
    let summary = gtk::Label::new(Some(&format!(
        "{} files available  ·  {} local installers  ·  {} on disk",
        game.remote_artifacts.len(),
        installers.len(),
        human_size(disk_usage)
    )));
    summary.set_widget_name(&format!("managed-files-summary-{}", game.product_id));
    summary.set_xalign(0.0);
    summary.add_css_class("dim-label");
    page.append(&summary);
    if let Some(management) = management {
        page.append(&management.status);
        page.append(&management.progress);
    }

    let remote_installers = game
        .remote_artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Installer)
        .cloned()
        .collect::<Vec<_>>();
    let remote_patches = game
        .remote_artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Patch)
        .cloned()
        .collect::<Vec<_>>();
    let remote_extras = game
        .remote_artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Extra)
        .cloned()
        .collect::<Vec<_>>();
    let installer_context = RemoteFileContext {
        product_id: game.product_id,
        product_slug: &game.slug,
        parent_slug: None,
        product_title: &game.title,
        folder: &game.location,
        window,
        access_token,
        download_directory,
        installer_filters: Some(installer_defaults),
        show_retired_artifacts,
        installed: None,
    };
    page.append(&remote_file_collection(
        "Offline Installers",
        "folder-download-symbolic",
        &remote_installers,
        &installers,
        &installer_context,
    ));
    if !remote_patches.is_empty() {
        let patch_folder = game.location.join("patches");
        let patch_context = RemoteFileContext {
            product_id: game.product_id,
            product_slug: &game.slug,
            parent_slug: None,
            product_title: &game.title,
            folder: &patch_folder,
            window,
            access_token,
            download_directory,
            installer_filters: None,
            show_retired_artifacts,
            installed,
        };
        page.append(&remote_file_collection(
            "Patches",
            "view-refresh-symbolic",
            &remote_patches,
            &patches,
            &patch_context,
        ));
    }
    if !remote_extras.is_empty() {
        let extras_folder = game.location.join("extras");
        let extras_context = RemoteFileContext {
            product_id: game.product_id,
            product_slug: &game.slug,
            parent_slug: None,
            product_title: &game.title,
            folder: &extras_folder,
            window,
            access_token,
            download_directory,
            installer_filters: None,
            show_retired_artifacts,
            installed: None,
        };
        page.append(&remote_file_collection(
            "Extras",
            "folder-documents-symbolic",
            &remote_extras,
            &extras,
            &extras_context,
        ));
    }
    if remote_patches.is_empty() && !patches.is_empty() {
        page.append(&file_collection(
            "Patches",
            "view-refresh-symbolic",
            &patches,
            &game.location.join("patches"),
            window,
        ));
    }
    if remote_extras.is_empty() && !extras.is_empty() {
        page.append(&file_collection(
            "Extras",
            "folder-documents-symbolic",
            &extras,
            &game.location.join("extras"),
            window,
        ));
    }
    let owned_dlcs = game.dlcs.iter().filter(|dlc| dlc.owned).collect::<Vec<_>>();
    if !owned_dlcs.is_empty() {
        let dlc_heading = gtk::Label::new(Some("DLC"));
        dlc_heading.set_xalign(0.0);
        dlc_heading.add_css_class("title-2");
        dlc_heading.set_margin_top(8);
        page.append(&dlc_heading);
        for dlc in owned_dlcs {
            page.append(&dlc_file_section(
                dlc,
                &game.slug,
                window,
                access_token,
                download_directory,
                installer_defaults,
                show_retired_artifacts,
            ));
        }
    }
    page
}

fn dlc_file_section(
    dlc: &Dlc,
    parent_slug: &str,
    window: &adw::ApplicationWindow,
    access_token: Option<&str>,
    download_directory: &std::path::Path,
    installer_defaults: &InstallerFilterDefaults,
    show_retired_artifacts: bool,
) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    section.add_css_class("dlc-files-section");
    let title = gtk::Label::new(Some(&dlc.title));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    section.append(&title);

    let managed = StateStore::open()
        .and_then(|store| store.managed_files())
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.product_id == dlc.product_id && file.path.is_file())
        .collect::<Vec<_>>();
    let local_files = |kind| {
        managed
            .iter()
            .filter(|file| file.kind == kind)
            .map(|file| LibraryFile {
                name: file.filename.clone(),
                path: file.path.clone(),
                size: file.size,
            })
            .collect::<Vec<_>>()
    };
    let installers = local_files(ArtifactKind::Installer);
    let patches = local_files(ArtifactKind::Patch);
    let extras = local_files(ArtifactKind::Extra);
    let dlc_root = download_directory
        .join(parent_slug)
        .join("dlc")
        .join(&dlc.slug);
    let append_collection =
        |container: &gtk::Box, title, icon, kind, local: &[LibraryFile], filters| {
            let remote = dlc
                .remote_artifacts
                .iter()
                .filter(|artifact| artifact.kind == kind)
                .cloned()
                .collect::<Vec<_>>();
            if remote.is_empty() && local.is_empty() {
                return;
            }
            let folder = dlc_root.join(kind.as_str());
            let context = RemoteFileContext {
                product_id: dlc.product_id,
                product_slug: &dlc.slug,
                parent_slug: Some(parent_slug),
                product_title: &dlc.title,
                folder: &folder,
                window,
                access_token,
                download_directory,
                installer_filters: filters,
                show_retired_artifacts,
                installed: None,
            };
            if remote.is_empty() {
                container.append(&file_collection(title, icon, local, &folder, window));
            } else {
                container.append(&remote_file_collection(
                    title, icon, &remote, local, &context,
                ));
            }
        };
    append_collection(
        &section,
        "Offline Installers",
        "folder-download-symbolic",
        ArtifactKind::Installer,
        &installers,
        Some(installer_defaults),
    );
    append_collection(
        &section,
        "Patches",
        "view-refresh-symbolic",
        ArtifactKind::Patch,
        &patches,
        None,
    );
    append_collection(
        &section,
        "Extras",
        "folder-documents-symbolic",
        ArtifactKind::Extra,
        &extras,
        None,
    );
    section
}

pub(super) struct InstallerFilterDefaults {
    pub(super) language: Option<String>,
    pub(super) windows: bool,
    pub(super) linux: bool,
    pub(super) macos: bool,
}

type InstallerFilterRows = Rc<RefCell<Vec<(gtk::Box, Option<String>, Option<String>)>>>;

struct RemoteFileContext<'a> {
    product_id: i64,
    product_slug: &'a str,
    parent_slug: Option<&'a str>,
    product_title: &'a str,
    folder: &'a std::path::Path,
    window: &'a adw::ApplicationWindow,
    access_token: Option<&'a str>,
    download_directory: &'a std::path::Path,
    installer_filters: Option<&'a InstallerFilterDefaults>,
    show_retired_artifacts: bool,
    installed: Option<&'a crate::domain::InstalledGame>,
}

#[derive(Clone, Copy)]
enum UnifiedFileState {
    Retired,
    Local,
}

struct UnifiedFileRow {
    artifact: Option<RemoteArtifact>,
    title: String,
    metadata: String,
    size: Option<String>,
    state: Option<UnifiedFileState>,
    dimmed: bool,
}

struct UnifiedFileRowWidgets {
    row: gtk::Box,
    labels: gtk::Box,
}

fn build_unified_file_row(spec: UnifiedFileRow) -> UnifiedFileRowWidgets {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("file-row");
    if spec.dimmed {
        row.add_css_class("dim-label");
    }
    if let Some(artifact) = spec.artifact.as_ref() {
        row.append(&artifact_identity_badge(artifact));
    } else {
        let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
        icon.set_width_request(34);
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&spec.title));
    name.set_xalign(0.0);
    name.set_width_chars(1);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.add_css_class("file-name");
    labels.append(&name);
    if !spec.metadata.is_empty() || spec.state.is_some() {
        let metadata_line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let metadata = gtk::Label::new(Some(&spec.metadata));
        metadata.set_xalign(0.0);
        metadata.set_width_chars(1);
        metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);
        metadata.add_css_class("dim-label");
        if !spec.metadata.is_empty() {
            metadata_line.append(&metadata);
        }
        if let Some(state) = spec.state {
            let separator = if spec.metadata.is_empty() { "" } else { "· " };
            let (text, tooltip, class) = match state {
                UnifiedFileState::Retired => (
                    "Retired",
                    "This version is no longer offered by GOG and may not be downloadable again.",
                    "warning",
                ),
                UnifiedFileState::Local => (
                    "Local",
                    "These files are stored locally but are not matched to the current GOG catalog.",
                    "dim-label",
                ),
            };
            let state = gtk::Label::new(Some(&format!("{separator}{text}")));
            state.set_xalign(0.0);
            state.add_css_class(class);
            state.set_tooltip_text(Some(tooltip));
            metadata_line.append(&state);
        }
        labels.append(&metadata_line);
    }
    row.append(&labels);
    if let Some(size) = spec.size {
        let size = gtk::Label::new(Some(&size));
        size.add_css_class("dim-label");
        row.append(&size);
    }
    UnifiedFileRowWidgets { row, labels }
}

fn build_file_action_menu(actions: &gtk::Box) -> gtk::Box {
    let sources = {
        let mut sources = Vec::new();
        let mut child = actions.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(button) = widget.downcast::<gtk::Button>() {
                sources.push(button);
            }
        }
        sources
    };
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.set_tooltip_text(Some("File actions"));
    let popover = gtk::Popover::new();
    let menu_actions = file_action_box();
    let mut direct_buttons = Vec::new();
    for source in &sources {
        let direct = gtk::Button::new();
        direct.add_css_class("flat");
        let menu_entry = gtk::Button::new();
        menu_entry.add_css_class("flat");
        menu_entry.set_halign(gtk::Align::Fill);
        sync_action_proxy(source, &direct, false);
        sync_action_proxy(source, &menu_entry, true);
        for proxy in [&direct, &menu_entry] {
            source
                .bind_property("sensitive", proxy, "sensitive")
                .sync_create()
                .build();
            let source = source.clone();
            proxy.connect_clicked(move |_| source.emit_clicked());
        }
        source
            .bind_property("visible", &menu_entry, "visible")
            .sync_create()
            .build();
        {
            let direct = direct.clone();
            let menu_entry = menu_entry.clone();
            source.connect_icon_name_notify(move |source| {
                sync_action_proxy(source, &direct, false);
                sync_action_proxy(source, &menu_entry, true);
            });
        }
        {
            let direct = direct.clone();
            let menu_entry = menu_entry.clone();
            source.connect_tooltip_text_notify(move |source| {
                sync_action_proxy(source, &direct, false);
                sync_action_proxy(source, &menu_entry, true);
            });
        }
        root.append(&direct);
        menu_actions.append(&menu_entry);
        direct_buttons.push(direct);
    }
    popover.set_child(Some(&menu_actions));
    menu.set_popover(Some(&popover));
    root.append(&menu);
    let refresh: Rc<dyn Fn()> = {
        let sources = sources.clone();
        let directs = direct_buttons.clone();
        let menu = menu.clone();
        Rc::new(move || {
            let visible_count = sources.iter().filter(|button| button.is_visible()).count();
            for (source, direct) in sources.iter().zip(&directs) {
                direct.set_visible(visible_count == 1 && source.is_visible());
            }
            menu.set_visible(visible_count > 1);
        })
    };
    for source in &sources {
        let refresh = refresh.clone();
        source.connect_visible_notify(move |_| refresh());
    }
    refresh();
    root
}

fn sync_action_proxy(source: &gtk::Button, proxy: &gtk::Button, include_text: bool) {
    let source_label = source.label().map(|value| value.to_string());
    let icon = source
        .icon_name()
        .map(|value| value.to_string())
        .unwrap_or_else(|| match source_label.as_deref() {
            Some("Run Patch") => "view-refresh-symbolic".into(),
            _ => "emblem-system-symbolic".into(),
        });
    let label = source_label
        .or_else(|| source.tooltip_text().map(|value| value.to_string()))
        .unwrap_or_else(|| "Action".into());
    let compact_label = compact_file_action_label(&label);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_halign(if include_text {
        gtk::Align::Start
    } else {
        gtk::Align::Center
    });
    content.append(&gtk::Image::from_icon_name(&icon));
    if include_text {
        content.append(&gtk::Label::new(Some(compact_label)));
    }
    proxy.set_child(Some(&content));
    proxy.set_tooltip_text(Some(&label));
    proxy.set_visible(source.is_visible());
    if source.has_css_class("destructive-action") {
        proxy.add_css_class("destructive-action");
    }
}

fn compact_file_action_label(label: &str) -> &str {
    let lower = label.to_ascii_lowercase();
    if lower.contains("show downloaded") || lower.contains("open folder") {
        "Open Folder"
    } else if lower.contains("delete") {
        "Delete"
    } else if lower.contains("discard") {
        "Discard"
    } else if lower.contains("cancel download") {
        "Cancel"
    } else if lower.contains("download") || lower.contains("resume") || lower.contains("retry") {
        "Download"
    } else {
        label
    }
}

fn file_action_box() -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
    actions.set_margin_start(6);
    actions.set_margin_end(6);
    actions.set_margin_top(6);
    actions.set_margin_bottom(6);
    actions
}

fn remote_file_collection(
    title: &str,
    icon_name: &str,
    files: &[RemoteArtifact],
    local_files: &[LibraryFile],
    context: &RemoteFileContext<'_>,
) -> gtk::Box {
    let grouped = download_selection::group_artifacts(files);
    let state_store = StateStore::open().ok();
    let managed_group_paths = grouped
        .iter()
        .map(|group| {
            let artifacts = group.artifacts.iter().collect::<Vec<_>>();
            state_store
                .as_ref()
                .and_then(|store| store.current_managed_paths(&artifacts).ok())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let filter_rows: InstallerFilterRows = Rc::new(RefCell::new(Vec::new()));
    let collection = gtk::Box::new(gtk::Orientation::Vertical, 0);
    collection.add_css_class("file-collection");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("file-collection-header");
    header.append(&gtk::Image::from_icon_name(icon_name));
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.add_css_class("section-title");
    header.append(&heading);
    let filter_controls = context.installer_filters.map(|defaults| {
        let mut languages = files
            .iter()
            .filter_map(|artifact| artifact.language.clone())
            .filter(|language| !language.is_empty())
            .collect::<Vec<_>>();
        languages.sort_by_key(|language| language.to_lowercase());
        languages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        languages.insert(0, "Any language".into());
        let language_list =
            gtk::StringList::new(&languages.iter().map(String::as_str).collect::<Vec<_>>());
        let language = gtk::DropDown::new(Some(language_list.clone()), gtk::Expression::NONE);
        language.set_tooltip_text(Some("Installer language"));
        language.set_selected(
            defaults
                .language
                .as_ref()
                .and_then(|configured| {
                    languages
                        .iter()
                        .position(|value| value.eq_ignore_ascii_case(configured))
                })
                .unwrap_or(0) as u32,
        );
        header.append(&language);
        let platforms = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        platforms.add_css_class("installer-platform-filters");
        let windows = gtk::CheckButton::with_label("Windows");
        let linux = gtk::CheckButton::with_label("Linux");
        let macos = gtk::CheckButton::with_label("macOS");
        windows.set_active(defaults.windows);
        linux.set_active(defaults.linux);
        macos.set_active(defaults.macos);
        platforms.append(&windows);
        platforms.append(&linux);
        platforms.append(&macos);
        header.append(&platforms);
        (language, language_list, windows, linux, macos)
    });
    let downloaded = grouped
        .iter()
        .zip(&managed_group_paths)
        .filter(|(group, managed_paths)| {
            let refs = group.artifacts.iter().collect::<Vec<_>>();
            if !managed_paths.is_empty() {
                return artifact_download_is_plausible(&refs, managed_paths);
            }
            matching_download_job(&refs).is_some_and(|job| {
                download_job_is_complete(&job)
                    && artifact_download_is_plausible(&refs, &job.completed_files)
            })
        })
        .count();
    let count_text = format!("{downloaded}/{} Downloaded", grouped.len());
    let count = gtk::Label::new(Some(&count_text));
    count.add_css_class("dim-label");
    header.append(&count);
    header.append(&folder_button(
        &format!("Open {title} folder"),
        context.folder,
        context.window,
    ));
    collection.append(&header);

    if files.is_empty() && local_files.is_empty() {
        let empty = gtk::Label::new(Some("No files listed by GOG"));
        empty.set_xalign(0.0);
        empty.add_css_class("file-empty");
        collection.append(&empty);
        return collection;
    }
    let mut represented_local_files = HashSet::new();
    for (group, managed_paths) in grouped.iter().zip(&managed_group_paths) {
        let refs = group.artifacts.iter().collect::<Vec<_>>();
        let file = &group.artifacts[0];
        if !managed_paths.is_empty() {
            represented_local_files.extend(managed_paths.iter().cloned());
        } else if let Some(job) = matching_download_job(&refs).filter(download_job_is_complete) {
            represented_local_files.extend(job.completed_files);
        }
        let display_name = artifact_display_title(file, context.product_title);
        let mut metadata_parts = [
            file.operating_system.as_deref(),
            file.language.as_deref(),
            file.version.as_deref(),
            file.release_date.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
        if let Some(part_count) = file.part_count {
            metadata_parts.push(format!("{part_count} parts"));
        }
        let total_size = group.total_size;
        let size_text = total_size.map_or_else(
            || {
                file.size_label
                    .clone()
                    .unwrap_or_else(|| "Unknown size".into())
            },
            approximate_download_size,
        );
        let widgets = build_unified_file_row(UnifiedFileRow {
            artifact: Some(file.clone()),
            title: display_name,
            metadata: metadata_parts.join(" · "),
            size: Some(size_text),
            state: None,
            dimmed: false,
        });
        widgets.row.append(&artifact_download_action(
            &refs,
            managed_paths,
            &widgets.labels,
            &count,
            context,
        ));
        filter_rows.borrow_mut().push((
            widgets.row.clone(),
            file.language.as_deref().map(str::to_ascii_lowercase),
            file.operating_system
                .as_deref()
                .map(str::to_ascii_lowercase),
        ));
        collection.append(&widgets.row);
    }
    let retired = if context.show_retired_artifacts {
        let retired_local_identities = StateStore::open()
            .ok()
            .map(|store| {
                local_files
                    .iter()
                    .filter_map(|file| store.retired_artifact_for_file(&file.path).ok().flatten())
                    .map(|artifact| (artifact.download_path, artifact.version))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let collection_kind = files.first().map(|file| file.kind).unwrap_or_else(|| {
            if title.to_ascii_lowercase().contains("patch") {
                ArtifactKind::Patch
            } else if title.to_ascii_lowercase().contains("extra") {
                ArtifactKind::Extra
            } else {
                ArtifactKind::Installer
            }
        });
        StateStore::open()
            .and_then(|store| store.artifact_catalog(context.product_id))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| !entry.currently_offered)
            .filter(|entry| entry.artifact.kind == collection_kind)
            .filter(|entry| {
                !retired_local_identities.contains(&(
                    entry.artifact.download_path.clone(),
                    entry.artifact.version.clone(),
                ))
            })
            .map(|entry| entry.artifact)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for group in download_selection::group_artifacts(&retired) {
        let file = &group.artifacts[0];
        let mut details = [
            file.operating_system.as_deref(),
            file.language.as_deref(),
            file.version.as_deref(),
            file.release_date.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
        if group.artifacts.len() > 1 {
            details.push(format!("{} parts", group.artifacts.len()));
        }
        let widgets = build_unified_file_row(UnifiedFileRow {
            artifact: Some(file.clone()),
            title: artifact_display_title(file, context.product_title),
            metadata: details.join(" · "),
            size: group.total_size.map(human_size),
            state: Some(UnifiedFileState::Retired),
            dimmed: true,
        });
        collection.append(&widgets.row);
    }
    let remaining = local_files
        .iter()
        .filter(|file| !represented_local_files.contains(&file.path))
        .collect::<Vec<_>>();
    let mut historical_groups = std::collections::BTreeMap::<String, Vec<&LibraryFile>>::new();
    for file in remaining {
        historical_groups
            .entry(historical_file_group_key(file))
            .or_default()
            .push(file);
    }
    for files in historical_groups.values() {
        let file = files[0];
        let historical = file.name.contains("not matched to current GOG manifest");
        let retired_artifact = StateStore::open()
            .and_then(|store| store.retired_artifact_for_file(&file.path))
            .ok()
            .flatten();
        let display_name = if let Some(artifact) = &retired_artifact {
            artifact_display_title(artifact, context.product_title)
        } else if historical {
            historical_artifact_title(file, context.product_title)
        } else {
            file.name.clone()
        };
        let metadata_text = if let Some(artifact) = &retired_artifact {
            let mut metadata = [
                artifact.operating_system.as_deref(),
                artifact.language.as_deref(),
                artifact.version.as_deref(),
                artifact.release_date.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
            if let Some(parts) = artifact.part_count.filter(|parts| *parts > 1) {
                metadata.push(format!("{parts} parts"));
            }
            metadata.join(" · ")
        } else if historical {
            let path_metadata = historical_path_metadata(file, context.download_directory);
            if files.len() > 1 {
                format!(
                    "{}{} parts",
                    path_metadata.map_or_else(String::new, |value| format!("{value} · ")),
                    files.len(),
                )
            } else {
                path_metadata.map_or_else(String::new, |value| format!("{value} · "))
            }
        } else {
            String::new()
        };
        let widgets = build_unified_file_row(UnifiedFileRow {
            artifact: retired_artifact
                .clone()
                .or_else(|| inferred_local_artifact(file)),
            title: display_name,
            metadata: metadata_text,
            size: Some(human_size(files.iter().map(|file| file.size).sum())),
            state: Some(if retired_artifact.is_some() {
                UnifiedFileState::Retired
            } else {
                UnifiedFileState::Local
            }),
            dimmed: false,
        });
        let row = widgets.row;
        let local_folder = file.path.parent().unwrap_or(context.folder);
        let open = folder_button("Show downloaded file", local_folder, context.window);
        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.set_tooltip_text(Some("Delete this managed file"));
        let paths = files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let path = file.path.clone();
        let root = context.download_directory.to_path_buf();
        let row_for_delete = row.clone();
        let window = context.window.clone();
        let product_id = context.product_id;
        let deletion_warning = if retired_artifact.is_some() {
            "This previous version is no longer offered by GOG and most likely cannot be downloaded again."
        } else {
            "This local content is not matched to the current GOG manifest and may not be downloadable again."
        };
        delete.connect_clicked(move |button| {
            let total_size = paths
                .iter()
                .filter_map(|path| path.metadata().ok())
                .map(|metadata| metadata.len())
                .sum::<u64>();
            let confirmation = adw::AlertDialog::builder()
                .heading("Permanently delete local files?")
                .body(format!(
                    "This will permanently delete {} file{} ({}). {deletion_warning}",
                    paths.len(),
                    if paths.len() == 1 { "" } else { "s" },
                    human_size(total_size),
                ))
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("delete", "Delete Permanently")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let paths = paths.clone();
            let path = path.clone();
            let root = root.clone();
            let row = row_for_delete.clone();
            let button = button.clone();
            let response_window = window.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "delete" {
                    return;
                }
                button.set_sensitive(false);
                match download::delete_completed_files(path.parent().unwrap_or(&root), &paths) {
                    Ok(()) => {
                        if let Ok(store) = StateStore::open() {
                            for path in &paths {
                                let _ = store.mark_managed_file_absent(path);
                            }
                        }
                        let _ = download::prune_empty_directories(&root);
                        row.set_visible(false);
                        refresh_managed_detail_labels(&response_window, product_id);
                    }
                    Err(error) => {
                        tracing::warn!(%error, path = %path.display(), "could not delete managed file");
                        button.set_sensitive(true);
                    }
                }
            });
        });
        let menu_actions = file_action_box();
        menu_actions.append(&open);
        menu_actions.append(&delete);
        row.append(&build_file_action_menu(&menu_actions));
        collection.append(&row);
    }
    if let Some((language, language_list, windows, linux, macos)) = filter_controls {
        for widget in [
            language.clone().upcast::<gtk::Widget>(),
            windows.clone().upcast(),
            linux.clone().upcast(),
            macos.clone().upcast(),
        ] {
            let rows = filter_rows.clone();
            let language = language.clone();
            let language_list = language_list.clone();
            let windows = windows.clone();
            let linux = linux.clone();
            let macos = macos.clone();
            if let Ok(dropdown) = widget.clone().downcast::<gtk::DropDown>() {
                dropdown.connect_selected_notify(move |_| {
                    apply_installer_file_filters(
                        &rows,
                        &language,
                        &language_list,
                        &windows,
                        &linux,
                        &macos,
                    );
                });
            } else if let Ok(check) = widget.downcast::<gtk::CheckButton>() {
                check.connect_toggled(move |_| {
                    apply_installer_file_filters(
                        &rows,
                        &language,
                        &language_list,
                        &windows,
                        &linux,
                        &macos,
                    );
                });
            }
        }
        apply_installer_file_filters(
            &filter_rows,
            &language,
            &language_list,
            &windows,
            &linux,
            &macos,
        );
    }
    collection
}

pub(super) fn apply_installer_file_filters(
    rows: &InstallerFilterRows,
    language: &gtk::DropDown,
    language_list: &gtk::StringList,
    windows: &gtk::CheckButton,
    linux: &gtk::CheckButton,
    macos: &gtk::CheckButton,
) {
    let selected_language = (language.selected() > 0)
        .then(|| language_list.string(language.selected()))
        .flatten()
        .map(|value| value.to_lowercase());
    for (row, row_language, platform) in rows.borrow().iter() {
        let language_matches = selected_language.as_ref().is_none_or(|selected| {
            row_language
                .as_ref()
                .is_some_and(|language| language == selected)
        });
        let platform_matches = match platform.as_deref() {
            Some("windows") => windows.is_active(),
            Some("linux") => linux.is_active(),
            Some("mac") | Some("osx") | Some("macos") => macos.is_active(),
            _ => true,
        };
        row.set_visible(language_matches && platform_matches);
    }
}

fn artifact_download_action(
    artifacts: &[&RemoteArtifact],
    managed_paths: &[std::path::PathBuf],
    labels: &gtk::Box,
    collection_count: &gtk::Label,
    context: &RemoteFileContext<'_>,
) -> gtk::Box {
    let action = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    action.set_valign(gtk::Align::Center);

    let requested_job_id = download::job_id(artifacts);
    let product_id = artifacts[0].product_id;
    let saved_job = matching_download_job(artifacts);
    let job_id = saved_job
        .as_ref()
        .map(|job| job.job_id.clone())
        .unwrap_or(requested_job_id);
    let refs = artifacts.to_vec();
    let preferred_destination = download::destination(
        context.download_directory,
        context.parent_slug.unwrap_or(context.product_slug),
        context.parent_slug.map(|_| context.product_slug),
        &refs,
    );
    let job_destination = saved_job
        .as_ref()
        .map(|job| job.destination.clone())
        .unwrap_or(preferred_destination);
    let existing_files = if managed_paths.is_empty() {
        saved_job
            .as_ref()
            .filter(|job| job.state == "complete")
            .map(|job| job.completed_files.clone())
            .unwrap_or_default()
    } else {
        managed_paths.to_vec()
    };
    let invalid_download =
        !existing_files.is_empty() && !artifact_download_is_plausible(artifacts, &existing_files);
    let completed_folder = (!invalid_download)
        .then(|| {
            managed_paths
                .first()
                .and_then(|path| path.parent())
                .map(std::path::Path::to_path_buf)
                .or_else(|| {
                    saved_job.as_ref().and_then(|job| {
                        (job.state == "complete"
                            && !job.completed_files.is_empty()
                            && job.completed_files.iter().all(|path| path.is_file()))
                        .then(|| job.destination.clone())
                    })
                })
        })
        .flatten();
    let status_text = if invalid_download {
        "✕"
    } else if completed_folder.is_some() {
        "✓"
    } else {
        match saved_job.as_ref().map(|job| job.state.as_str()) {
            Some("failed") => "Failed — retry",
            Some("paused") => "Paused — resume",
            Some("downloading") | Some("queued") => "Interrupted — resume",
            _ => "",
        }
    };
    let status = gtk::Label::new(Some(status_text));
    status.set_visible(!status_text.is_empty());
    if invalid_download {
        status.add_css_class("error");
        status.set_tooltip_text(Some(
            "Download files are missing or their file sizes do not match.",
        ));
    } else if completed_folder.is_some() {
        status.add_css_class("success");
        status.set_tooltip_text(Some("Downloaded"));
    } else {
        status.add_css_class("dim-label");
    }
    if let Some(error) = saved_job.as_ref().and_then(|job| job.error.as_deref()) {
        status.set_tooltip_text(Some(error));
    }
    action.append(&status);

    let progress = gtk::ProgressBar::new();
    progress.set_hexpand(true);
    progress.set_visible(false);
    progress.add_css_class("download-progress");
    if let Some(job) = &saved_job
        && job.state != "complete"
        && job.bytes_downloaded > 0
        && let Some(total) = job.total_bytes.filter(|total| *total > 0)
    {
        progress.set_fraction((job.bytes_downloaded as f64 / total as f64).min(1.0));
    }
    labels.append(&progress);

    let button = gtk::Button::from_icon_name(if completed_folder.is_some() {
        "folder-open-symbolic"
    } else {
        "folder-download-symbolic"
    });
    button.add_css_class("flat");
    if completed_folder.is_none() {
        button.add_css_class("bulk-download-target");
    }
    button.set_sensitive(completed_folder.is_some() || context.access_token.is_some());
    button.set_tooltip_text(Some(if completed_folder.is_some() {
        "Show downloaded files"
    } else if context.access_token.is_some() {
        "Download all required parts"
    } else {
        "Sign in to GOG to download"
    }));
    let menu_actions = file_action_box();
    menu_actions.append(&button);
    let can_run_patch = artifacts[0].kind == ArtifactKind::Patch
        && artifacts[0]
            .operating_system
            .as_deref()
            .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
        && context
            .installed
            .is_some_and(|game| game.compatibility.is_some());
    let run_patch_button = gtk::Button::with_label("Run Patch");
    run_patch_button.add_css_class("flat");
    run_patch_button.set_tooltip_text(Some(
        "Run this patch in the installed game's UMU environment",
    ));
    run_patch_button.set_visible(can_run_patch && completed_folder.is_some());
    menu_actions.append(&run_patch_button);
    let delete_button = gtk::Button::from_icon_name("user-trash-symbolic");
    delete_button.add_css_class("flat");
    delete_button.add_css_class("destructive-action");
    delete_button.set_tooltip_text(Some("Delete downloaded files"));
    delete_button.set_visible(!existing_files.is_empty());
    menu_actions.append(&delete_button);
    let discard_button = gtk::Button::from_icon_name("edit-delete-symbolic");
    discard_button.add_css_class("flat");
    discard_button.add_css_class("destructive-action");
    discard_button.set_tooltip_text(Some("Cancel download and delete partial files"));
    discard_button.set_visible(
        completed_folder.is_none()
            && saved_job
                .as_ref()
                .is_some_and(|job| job.state != DownloadState::Complete),
    );
    menu_actions.append(&discard_button);
    action.append(&build_file_action_menu(&menu_actions));

    let artifacts = artifacts
        .iter()
        .map(|artifact| (*artifact).clone())
        .collect::<Vec<_>>();
    let title = artifact_display_title(&artifacts[0], context.product_title);
    let token = context.access_token.map(str::to_owned);
    let job_destination = job_destination.clone();
    let downloaded_files = Rc::new(RefCell::new(existing_files));
    let running = Rc::new(RefCell::new(
        None::<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ));
    if can_run_patch {
        let window = context.window.clone();
        let installed = context.installed.cloned().expect("checked above");
        let downloaded_files = downloaded_files.clone();
        let status = status.clone();
        let download_button = button.clone();
        let delete_button = delete_button.clone();
        let run_patch_button_for_click = run_patch_button.clone();
        let target_version =
            crate::installation::patch_target_version(artifacts[0].version.as_deref());
        run_patch_button.connect_clicked(move |_| {
            let Some(patch) = downloaded_files.borrow().iter().find(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
            }).cloned() else {
                let dialog = adw::AlertDialog::builder()
                    .heading("Patch executable not found")
                    .body("The downloaded patch does not contain a Windows .exe file.")
                    .build();
                dialog.add_response("close", "Close");
                dialog.present(Some(&window));
                return;
            };
            let confirmation = adw::AlertDialog::builder()
                .heading("Run this patch?")
                .body("Ludomere will run the downloaded patch in this game's existing UMU compatibility environment. The patch may modify the installed game files.")
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("run", "Run Patch")]);
            confirmation.set_default_response(Some("run"));
            confirmation.set_close_response("cancel");
            let window_for_response = window.clone();
            let installed = installed.clone();
            let status = status.clone();
            let download_button = download_button.clone();
            let delete_button = delete_button.clone();
            let run_patch_button = run_patch_button_for_click.clone();
            let target_version = target_version.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "run" {
                    return;
                }
                status.remove_css_class("success");
                status.add_css_class("dim-label");
                status.set_label("Running patch…");
                status.set_visible(true);
                run_patch_button.set_sensitive(false);
                download_button.set_sensitive(false);
                delete_button.set_sensitive(false);
                let receiver = crate::installation::run_patch(
                    installed.clone(),
                    patch.clone(),
                    target_version.clone(),
                );
                let status_for_event = status.clone();
                let run_button_for_event = run_patch_button.clone();
                let download_for_event = download_button.clone();
                let delete_for_event = delete_button.clone();
                let window_for_event = window_for_response.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match receiver.try_recv() {
                        Ok(crate::installation::PatchEvent::Started { log_path }) => {
                            status_for_event.set_label("Applying patch…");
                            status_for_event.set_tooltip_text(Some(&format!(
                                "Log: {}", log_path.display()
                            )));
                            glib::ControlFlow::Continue
                        }
                        Ok(crate::installation::PatchEvent::Complete { .. }) => {
                            status_for_event.remove_css_class("dim-label");
                            status_for_event.add_css_class("success");
                            status_for_event.set_label("Patch completed");
                            run_button_for_event.set_sensitive(true);
                            download_for_event.set_sensitive(true);
                            delete_for_event.set_sensitive(true);
                            let dialog = adw::AlertDialog::builder()
                                .heading("Patch completed")
                                .body("The patch finished successfully and Ludomere recorded the base game and installed DLC as current. Launch the game to verify the update; use Repair Installation if it did not apply correctly.")
                                .build();
                            dialog.add_response("close", "Close");
                            dialog.present(Some(&window_for_event));
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::PatchEvent::Failed(error)) => {
                            status_for_event.remove_css_class("dim-label");
                            status_for_event.add_css_class("error");
                            status_for_event.set_label("Patch failed");
                            status_for_event.set_tooltip_text(Some(&error));
                            run_button_for_event.set_sensitive(true);
                            download_for_event.set_sensitive(true);
                            delete_for_event.set_sensitive(true);
                            let dialog = adw::AlertDialog::builder()
                                .heading("Could not apply patch")
                                .body(error)
                                .build();
                            dialog.add_response("close", "Close");
                            dialog.present(Some(&window_for_event));
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                    }
                });
            });
        });
    }
    {
        let window = context.window.clone();
        let job_id = job_id.clone();
        let status = status.clone();
        let progress = progress.clone();
        let download_button = button.clone();
        let discard_button_for_response = discard_button.clone();
        discard_button.connect_clicked(move |_| {
            let confirmation = adw::AlertDialog::builder()
                .heading("Discard this download?")
                .body(
                    "This cancels the download, removes it from the queue, and deletes only its partial staging files.",
                )
                .build();
            confirmation
                .add_responses(&[("cancel", "Cancel"), ("discard", "Discard Download")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation
                .set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            let job_id = job_id.clone();
            let status = status.clone();
            let progress = progress.clone();
            let download_button = download_button.clone();
            let discard_button = discard_button_for_response.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "discard" {
                    return;
                }
                if download::remove(&job_id) {
                    status.set_visible(false);
                    progress.set_visible(false);
                    download_button.set_icon_name("folder-download-symbolic");
                    download_button.set_tooltip_text(Some("Download all required parts"));
                    download_button.set_sensitive(true);
                    discard_button.set_visible(false);
                }
            });
        });
    }
    let folder = Rc::new(RefCell::new(completed_folder));
    let running_for_download = running.clone();
    let folder_for_download = folder.clone();
    let downloaded_files_for_download = downloaded_files.clone();
    let delete_button_for_download = delete_button.clone();
    let run_patch_button_for_download = run_patch_button.clone();
    let status_for_download = status.clone();
    let progress_for_download = progress.clone();
    let job_destination_for_download = job_destination.clone();
    let window_for_download = context.window.clone();
    let count_for_download = collection_count.clone();
    let job_id_for_download = job_id.clone();
    button.connect_clicked(move |button| {
        if running_for_download.borrow().is_some() {
            download::cancel(&job_id_for_download);
            status_for_download.set_label("Cancelling…");
            button.set_sensitive(false);
            return;
        }
        if let Some(path) = folder_for_download.borrow().clone() {
            super::widgets::file_open::open_directory(
                &path,
                &window_for_download,
                "download folder",
            );
            return;
        }
        let Some(token) = token.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        *running_for_download.borrow_mut() = Some(download::enqueue(download::DownloadRequest {
            artifacts: artifacts.clone(),
            title: title.clone(),
            access_token: token,
            destination: job_destination_for_download.clone(),
            events: sender,
        }));
        status_for_download.remove_css_class("success");
        status_for_download.add_css_class("dim-label");
        status_for_download.set_label("Starting…");
        status_for_download.set_visible(true);
        progress_for_download.set_visible(true);
        button.set_icon_name("process-stop-symbolic");
        button.set_tooltip_text(Some("Cancel download"));
        let running = running_for_download.clone();
        let folder = folder_for_download.clone();
        let status = status_for_download.clone();
        let progress = progress_for_download.clone();
        let button = button.clone();
        let delete_button = delete_button_for_download.clone();
        let run_patch_button = run_patch_button_for_download.clone();
        let downloaded_files = downloaded_files_for_download.clone();
        let count_for_response = count_for_download.clone();
        let window_for_response = window_for_download.clone();
        let artifacts_for_validation = artifacts.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(download::DownloadEvent::Progress { downloaded, total }) => {
                    status.set_label(&match total {
                        Some(total) if total > 0 => format!(
                            "Downloading {} / {}",
                            human_size(downloaded),
                            human_size(total)
                        ),
                        _ => format!("Downloading {}", human_size(downloaded)),
                    });
                    if let Some(total) = total.filter(|total| *total > 0) {
                        progress.set_fraction((downloaded as f64 / total as f64).min(1.0));
                    } else {
                        progress.pulse();
                    }
                    glib::ControlFlow::Continue
                }
                Ok(download::DownloadEvent::Finalizing) => {
                    status.set_label("Finalizing…");
                    progress.set_fraction(1.0);
                    progress.set_text(Some("Finalizing…"));
                    progress.set_show_text(true);
                    glib::ControlFlow::Continue
                }
                Ok(download::DownloadEvent::Complete { files }) => {
                    *running.borrow_mut() = None;
                    *downloaded_files.borrow_mut() = files.clone();
                    let artifact_refs = artifacts_for_validation.iter().collect::<Vec<_>>();
                    if !artifact_download_is_plausible(&artifact_refs, &files) {
                        *folder.borrow_mut() = None;
                        status.remove_css_class("dim-label");
                        status.remove_css_class("success");
                        status.add_css_class("error");
                        status.set_label("✕");
                        status.set_tooltip_text(Some(
                            "Download files are missing or their file sizes do not match.",
                        ));
                        progress.set_visible(false);
                        button.set_icon_name("folder-download-symbolic");
                        button.set_tooltip_text(Some("Download all required parts again"));
                        button.set_sensitive(true);
                        delete_button.set_visible(true);
                        run_patch_button.set_visible(false);
                        glib::ControlFlow::Break
                    } else {
                        *folder.borrow_mut() = files
                            .first()
                            .and_then(|path| path.parent())
                            .map(std::path::Path::to_owned);
                        status.remove_css_class("dim-label");
                        status.add_css_class("success");
                        status.set_label("✓");
                        status.set_tooltip_text(Some("Downloaded"));
                        progress.set_fraction(1.0);
                        progress.set_visible(false);
                        button.set_icon_name("folder-open-symbolic");
                        button.set_tooltip_text(Some("Show downloaded files"));
                        button.set_sensitive(true);
                        delete_button.set_visible(true);
                        run_patch_button.set_visible(can_run_patch);
                        adjust_downloaded_collection_count(&count_for_response, 1);
                        refresh_managed_detail_labels(&window_for_response, product_id);
                        glib::ControlFlow::Break
                    }
                }
                Ok(download::DownloadEvent::Cancelled) => {
                    *running.borrow_mut() = None;
                    status.set_label("Paused — resume");
                    progress.set_visible(false);
                    button.set_icon_name("folder-download-symbolic");
                    button.set_tooltip_text(Some("Resume download"));
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
                Ok(download::DownloadEvent::Failed(error)) => {
                    *running.borrow_mut() = None;
                    status.set_label("Failed — retry");
                    status.set_tooltip_text(Some(&error.message));
                    progress.set_visible(false);
                    button.set_icon_name("folder-download-symbolic");
                    button.set_tooltip_text(Some("Retry download"));
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    });
    {
        let window = context.window.clone();
        let job_id = job_id.clone();
        let job_destination = job_destination.clone();
        let download_directory = context.download_directory.to_path_buf();
        let downloaded_files = downloaded_files.clone();
        let folder = folder.clone();
        let status = status.clone();
        let progress = progress.clone();
        let download_button = button.clone();
        let delete_button_for_response = delete_button.clone();
        let run_patch_button_for_response = run_patch_button.clone();
        let collection_count = collection_count.clone();
        delete_button.connect_clicked(move |_| {
            let files = downloaded_files.borrow().clone();
            let total_size = files
                .iter()
                .filter_map(|path| path.metadata().ok())
                .map(|metadata| metadata.len())
                .sum::<u64>();
            let confirmation = adw::AlertDialog::builder()
                .heading("Delete downloaded files?")
                .body(format!(
                    "This permanently removes {} file{} ({}). It can be downloaded again later.",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" },
                    human_size(total_size),
                ))
                .build();
            confirmation.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
            confirmation.set_default_response(Some("cancel"));
            confirmation.set_close_response("cancel");
            confirmation.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let job_id = job_id.clone();
            let job_destination = job_destination.clone();
            let download_directory = download_directory.clone();
            let downloaded_files = downloaded_files.clone();
            let folder = folder.clone();
            let status = status.clone();
            let progress = progress.clone();
            let download_button = download_button.clone();
            let delete_button = delete_button_for_response.clone();
            let run_patch_button = run_patch_button_for_response.clone();
            let window_for_response = window.clone();
            let count_for_response = collection_count.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "delete" {
                    return;
                }
                let files = downloaded_files.borrow().clone();
                match download::delete_completed_files(&job_destination, &files) {
                    Ok(()) => {
                        if let Ok(store) = StateStore::open() {
                            let _ = store.delete_download_job(&job_id);
                            for path in &files {
                                let _ = store.mark_managed_file_absent(path);
                            }
                        }
                        if let Err(error) = download::prune_empty_directories(&download_directory) {
                            tracing::warn!(%error, "could not remove empty download directories");
                        }
                        downloaded_files.borrow_mut().clear();
                        *folder.borrow_mut() = None;
                        status.set_visible(false);
                        progress.set_visible(false);
                        download_button.set_icon_name("folder-download-symbolic");
                        download_button.set_tooltip_text(Some("Download all required parts"));
                        download_button.set_sensitive(true);
                        delete_button.set_visible(false);
                        run_patch_button.set_visible(false);
                        adjust_downloaded_collection_count(&count_for_response, -1);
                        refresh_managed_detail_labels(&window_for_response, product_id);
                    }
                    Err(error) => {
                        status.remove_css_class("success");
                        status.add_css_class("error");
                        status.set_label("Could not delete files");
                        status.set_tooltip_text(Some(&format!("{error:#}")));
                        status.set_visible(true);
                    }
                }
            });
        });
    }
    action
}

fn artifact_download_is_plausible(
    artifacts: &[&RemoteArtifact],
    paths: &[std::path::PathBuf],
) -> bool {
    if paths.len() != artifacts.len() || paths.iter().any(|path| !path.is_file()) {
        return false;
    }
    for path in paths {
        if path
            .metadata()
            .is_ok_and(|metadata| metadata.len() <= 64 * 1024)
            && std::fs::read(path).is_ok_and(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                text.trim_start().starts_with('{')
                    && (text.contains("\"downlink\"") || text.contains("\"url\""))
            })
        {
            return false;
        }
    }
    let expected = artifacts
        .iter()
        .map(|artifact| artifact.size_bytes)
        .sum::<Option<u64>>();
    let actual = paths
        .iter()
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    // GOG's per-part sizes can be rounded, so only classify a gross mismatch
    // here. Exact completed sizes remain tracked in the managed-file index.
    expected.is_none_or(|expected| expected == 0 || actual >= expected / 2)
}

pub(super) fn historical_file_group_key(file: &LibraryFile) -> String {
    let parent = file
        .path
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .display();
    let stem = file
        .path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&file.name);
    let extension = file
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let family = if extension.eq_ignore_ascii_case("bin") {
        stem.rsplit_once('-')
            .filter(|(_, suffix)| suffix.chars().all(|character| character.is_ascii_digit()))
            .map_or(stem, |(family, _)| family)
    } else {
        stem
    };
    format!("{parent}|{family}")
}

pub(super) fn historical_artifact_title(file: &LibraryFile, product_title: &str) -> String {
    let stem = file
        .path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&file.name);
    let family = file
        .path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(stem, |extension| {
            if extension.eq_ignore_ascii_case("bin") {
                stem.rsplit_once('-')
                    .filter(|(_, suffix)| {
                        suffix.chars().all(|character| character.is_ascii_digit())
                    })
                    .map_or(stem, |(family, _)| family)
            } else {
                stem
            }
        });
    let without_prefix = family
        .strip_prefix("setup_")
        .or_else(|| family.strip_prefix("patch_"))
        .unwrap_or(family);
    let lower = without_prefix.to_ascii_lowercase();
    let detail_start = ["_release_", "_live_", "_version_", "_v"]
        .into_iter()
        .filter_map(|marker| lower.find(marker))
        .min();
    let detail = detail_start
        .map(|index| &without_prefix[index + 1..])
        .unwrap_or(without_prefix);
    let detail = clean_historical_filename(detail);
    if detail.is_empty() || detail.eq_ignore_ascii_case(product_title) {
        product_title.to_owned()
    } else {
        format!("{product_title} — {detail}")
    }
}

pub(super) fn generic_artifact_title(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "dlc" | "installer" | "game" | "gog download"
    )
}

pub(super) fn artifact_display_title(artifact: &RemoteArtifact, product_title: &str) -> String {
    let title = artifact
        .name
        .split(" (Part ")
        .next()
        .unwrap_or(&artifact.name);
    if artifact.kind == ArtifactKind::Installer && generic_artifact_title(title) {
        product_title.to_owned()
    } else {
        title.to_owned()
    }
}

pub(super) fn clean_historical_filename(value: &str) -> String {
    let mut value = value.to_owned();
    while let Some(open) = value.rfind("_(") {
        let suffix = &value[open + 2..value.len().saturating_sub(1)];
        if !value.ends_with(')')
            || !(suffix.chars().all(|character| character.is_ascii_digit())
                || suffix.eq_ignore_ascii_case("64bit")
                || suffix.eq_ignore_ascii_case("32bit"))
        {
            break;
        }
        value.truncate(open);
    }
    let value = value
        .replace("_-_", " — ")
        .replace('_', " ")
        .replace("Patch Patch", "Patch ")
        .replace("patch patch", "Patch ");
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn historical_path_metadata(
    file: &LibraryFile,
    root: &std::path::Path,
) -> Option<String> {
    let relative = file.path.strip_prefix(root).ok()?;
    let components = relative
        .parent()?
        .components()
        .skip(2)
        .filter_map(|component| component.as_os_str().to_str())
        .map(|value| {
            let mut characters = value.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>();
    (!components.is_empty()).then(|| components.join(" · "))
}

pub(super) fn matching_download_job(artifacts: &[&RemoteArtifact]) -> Option<DownloadJobRecord> {
    let requested_id = download::job_id(artifacts);
    let store = StateStore::open().ok()?;
    store.download_job(&requested_id).ok().flatten()
}

pub(super) fn optional_text_matches(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        (None, None) => true,
        _ => false,
    }
}

pub(super) fn download_job_is_complete(job: &DownloadJobRecord) -> bool {
    job.state == "complete"
        && !job.completed_files.is_empty()
        && job.completed_files.iter().all(|path| path.is_file())
}

pub(super) fn approximate_download_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("~{bytes} B")
    } else {
        format!("~{value:.1} {}", UNITS[unit])
    }
}

pub(super) fn artifact_identity_badge(file: &RemoteArtifact) -> gtk::Box {
    let badge = gtk::Box::new(gtk::Orientation::Vertical, 0);
    badge.set_width_request(34);
    badge.set_halign(gtk::Align::Center);
    badge.set_valign(gtk::Align::Center);
    badge.add_css_class("artifact-identity");
    let platform = file
        .operating_system
        .as_deref()
        .map(str::to_ascii_lowercase);
    let icon_bytes = match platform.as_deref() {
        Some("windows") => {
            Some(include_bytes!("../../resources/icons/platform/windows.svg").as_slice())
        }
        Some("mac") | Some("osx") | Some("macos") => {
            Some(include_bytes!("../../resources/icons/platform/apple.svg").as_slice())
        }
        Some("linux") => {
            Some(include_bytes!("../../resources/icons/platform/linux.svg").as_slice())
        }
        _ => None,
    };
    if let Some(icon_bytes) = icon_bytes {
        let loader = gdk_pixbuf::PixbufLoader::new();
        if loader.write(icon_bytes).is_ok()
            && loader.close().is_ok()
            && let Some(pixbuf) = loader.pixbuf()
        {
            let width = (pixbuf.width() * 22 / pixbuf.height()).max(1);
            if let Some(pixbuf) = pixbuf.scale_simple(width, 22, InterpType::Bilinear) {
                let texture = gdk::Texture::for_pixbuf(&pixbuf);
                let os = gtk::Image::from_paintable(Some(&texture));
                os.add_css_class("artifact-os-icon");
                badge.append(&os);
            }
        }
    } else {
        let os_mark = if file.kind == ArtifactKind::Patch {
            "↻"
        } else if file.kind == ArtifactKind::Extra {
            "◇"
        } else {
            "▣"
        };
        let os = gtk::Label::new(Some(os_mark));
        os.add_css_class("artifact-os-mark");
        badge.append(&os);
    }
    let flag = gtk::Label::new(Some(language_flag(file.language.as_deref())));
    flag.add_css_class("artifact-language-flag");
    badge.append(&flag);
    badge.set_tooltip_text(Some(&format!(
        "{} · {}",
        file.operating_system
            .as_deref()
            .unwrap_or("Any operating system"),
        file.language.as_deref().unwrap_or("Language neutral")
    )));
    badge
}

pub(super) fn language_flag(language: Option<&str>) -> &'static str {
    let Some(language) = language else {
        return "🌐";
    };
    let language = language.to_lowercase();
    if language.contains("português do brasil") || language.contains("brazil") {
        "🇧🇷"
    } else if language.contains("english") {
        "🇬🇧"
    } else if language.contains("deutsch") || language.contains("german") {
        "🇩🇪"
    } else if language.contains("français") || language.contains("french") {
        "🇫🇷"
    } else if language.contains("español") || language.contains("spanish") {
        "🇪🇸"
    } else if language.contains("italiano") || language.contains("italian") {
        "🇮🇹"
    } else if language.contains("português") || language.contains("portuguese") {
        "🇵🇹"
    } else if language.contains("polski") || language.contains("polish") {
        "🇵🇱"
    } else if language.contains("рус") || language.contains("russian") {
        "🇷🇺"
    } else if language.contains("日本") || language.contains("japanese") {
        "🇯🇵"
    } else if language.contains("中文") || language.contains("chinese") {
        "🇨🇳"
    } else if language.contains("한국") || language.contains("korean") {
        "🇰🇷"
    } else if language.contains("česk") || language.contains("czech") {
        "🇨🇿"
    } else if language.contains("magyar") || language.contains("hungarian") {
        "🇭🇺"
    } else if language.contains("nederlands") || language.contains("dutch") {
        "🇳🇱"
    } else if language.contains("dansk") || language.contains("danish") {
        "🇩🇰"
    } else if language.contains("svensk") || language.contains("swedish") {
        "🇸🇪"
    } else if language.contains("norsk") || language.contains("norwegian") {
        "🇳🇴"
    } else if language.contains("suomi") || language.contains("finnish") {
        "🇫🇮"
    } else if language.contains("türk") || language.contains("turkish") {
        "🇹🇷"
    } else if language.contains("укра") || language.contains("ukrainian") {
        "🇺🇦"
    } else {
        "🌐"
    }
}

#[cfg(any())]
pub(super) fn build_dlc_files_page(dlc: &Dlc, window: &adw::ApplicationWindow) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_margin_top(12);
    let management = gtk::Box::new(gtk::Orientation::Vertical, 14);
    management.add_css_class("file-management-card");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some("Manage DLC local copy"));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    let summary = gtk::Label::new(Some(&format!(
        "{} installers  ·  {} extras  ·  {} on disk",
        dlc.installers.len(),
        dlc.extras.len(),
        human_size(dlc.disk_usage)
    )));
    summary.set_xalign(0.0);
    summary.add_css_class("dim-label");
    copy.append(&title);
    copy.append(&summary);
    heading.append(&copy);
    heading.append(&folder_button("Open DLC folder", &dlc.location, window));
    management.append(&heading);
    page.append(&management);
    page.append(&file_collection(
        "Installers",
        "application-x-executable-symbolic",
        &dlc.installers,
        &dlc.location,
        window,
    ));
    if !dlc.extras.is_empty() {
        page.append(&file_collection(
            "Extras",
            "folder-documents-symbolic",
            &dlc.extras,
            &dlc.location.join("extras"),
            window,
        ));
    }
    if !dlc.changelog.is_empty() {
        let changelog_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        changelog_card.add_css_class("file-collection");
        changelog_card.append(&lazy_html_section("Changelog", dlc.changelog.clone()));
        page.append(&changelog_card);
    }
    page
}

pub(super) fn verification_state(product_id: i64) -> Option<VerificationDisplayState> {
    VERIFICATION_STATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?
        .get(&product_id)
        .cloned()
}

pub(super) fn set_verification_state(product_id: i64, state: VerificationDisplayState) {
    if let Ok(mut states) = VERIFICATION_STATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        states.insert(product_id, state);
    }
}

pub(super) fn apply_verification_display(
    state: &VerificationDisplayState,
    button: &gtk::Button,
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
) {
    button.set_sensitive(!state.running);
    status.set_label(&state.message);
    status.set_visible(true);
    progress.set_visible(true);
    if let Some(fraction) = state.fraction {
        progress.set_fraction(fraction);
        progress.set_show_text(state.running);
        progress.set_text(
            state
                .running
                .then(|| format!("{:.0}%", fraction * 100.0))
                .as_deref(),
        );
    } else {
        progress.set_show_text(false);
        if state.running {
            progress.pulse();
        }
    }
}

pub(super) fn restore_verification_display(
    product_id: i64,
    button: &gtk::Button,
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
) {
    let Some(state) = verification_state(product_id) else {
        return;
    };
    apply_verification_display(&state, button, status, progress);
    if !state.running {
        return;
    }
    let button = button.clone();
    let status = status.clone();
    let progress = progress.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let Some(state) = verification_state(product_id) else {
            return glib::ControlFlow::Break;
        };
        apply_verification_display(&state, &button, &status, &progress);
        if state.running {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

fn start_product_verification(
    request: VerificationRequest,
    button: &gtk::Button,
    window: &adw::ApplicationWindow,
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
) {
    if verification_state(request.product_id).is_some_and(|state| state.running) {
        return;
    }
    let product_id = request.product_id;
    let (sender, receiver) = mpsc::channel();
    button.set_sensitive(false);
    button.set_tooltip_text(Some("Verifying files…"));
    status.set_label("Preparing verification…");
    status.set_visible(true);
    progress.set_fraction(0.0);
    progress.set_visible(true);
    set_verification_state(
        product_id,
        VerificationDisplayState {
            message: "Preparing verification…".into(),
            fraction: None,
            running: true,
        },
    );
    std::thread::spawn(move || {
        let result = verify_product_files(
            request.product_id,
            &request.title,
            &request.artifacts,
            request.access_token.as_deref(),
            &sender,
        );
        let _ = sender.send(VerificationEvent::Finished(result));
    });
    let button = button.clone();
    let window = window.clone();
    let status = status.clone();
    let progress = progress.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(VerificationEvent::Progress { message, fraction }) => {
                set_verification_state(
                    product_id,
                    VerificationDisplayState {
                        message: message.clone(),
                        fraction,
                        running: true,
                    },
                );
                status.set_label(&message);
                if let Some(fraction) = fraction {
                    progress.set_fraction(fraction);
                    progress.set_show_text(true);
                    progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                } else {
                    progress.set_show_text(false);
                    progress.pulse();
                }
                glib::ControlFlow::Continue
            }
            Ok(VerificationEvent::Finished(result)) => {
                button.set_sensitive(true);
                button.set_tooltip_text(Some(
                    "Check downloaded files using the native download database",
                ));
                progress.set_fraction(1.0);
                progress.set_show_text(false);
                status.set_label(match &result {
                    Ok(report) if report.repair_groups > 0 => {
                        "Verification complete; repairs queued"
                    }
                    Ok(_) => "Verification complete",
                    Err(_) => "Verification failed",
                });
                set_verification_state(
                    product_id,
                    VerificationDisplayState {
                        message: status.label().to_string(),
                        fraction: Some(1.0),
                        running: false,
                    },
                );
                let dialog = match result {
                Ok(report) if report.checked > 0 || report.repair_groups > 0 => adw::AlertDialog::builder()
                    .heading(if report.repair_groups > 0 { "Repair downloads queued" } else { "Verification complete" })
                    .body(format!(
                        "{} files verified against GOG checksums. {} download groups queued for repair. {} downloaded groups could not be matched to a version currently published by GOG.",
                        report.checked, report.repair_groups, report.unavailable
                    ))
                    .build(),
                Ok(_) => adw::AlertDialog::builder()
                    .heading("Nothing to verify")
                    .body("No completed downloads were found for this product.")
                    .build(),
                Err(error) => adw::AlertDialog::builder()
                    .heading("Verification failed")
                    .body(error.to_string())
                    .build(),
            };
                dialog.add_response("ok", "OK");
                dialog.present(Some(&window));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                button.set_sensitive(true);
                set_verification_state(
                    product_id,
                    VerificationDisplayState {
                        message: "Verification stopped unexpectedly".into(),
                        fraction: None,
                        running: false,
                    },
                );
                glib::ControlFlow::Break
            }
        }
    });
}

struct VerificationRequest {
    product_id: i64,
    title: String,
    artifacts: Vec<RemoteArtifact>,
    access_token: Option<String>,
}

enum VerificationEvent {
    Progress {
        message: String,
        fraction: Option<f64>,
    },
    Finished(anyhow::Result<VerificationReport>),
}

#[derive(Default)]
struct VerificationReport {
    checked: usize,
    repair_groups: usize,
    unavailable: usize,
}

fn verify_product_files(
    product_id: i64,
    title: &str,
    remote_artifacts: &[RemoteArtifact],
    access_token: Option<&str>,
    progress: &mpsc::Sender<VerificationEvent>,
) -> anyhow::Result<VerificationReport> {
    let jobs = StateStore::open()?.download_jobs()?;
    let completed = jobs
        .iter()
        .filter(|job| job.product_id == product_id && job.state == "complete")
        .collect::<Vec<_>>();
    if completed.is_empty() {
        return Ok(VerificationReport::default());
    }
    let total_downloaded_files = completed
        .iter()
        .map(|job| job.completed_files.len())
        .sum::<usize>();
    let Some(access_token) = access_token else {
        anyhow::bail!("Sign in to GOG before verifying authoritative checksums");
    };
    let mut groups = std::collections::BTreeMap::<String, Vec<&RemoteArtifact>>::new();
    for artifact in remote_artifacts {
        let key = if artifact.part_count.is_some() {
            format!(
                "{}|{:?}|{:?}|{:?}|{:?}",
                artifact
                    .name
                    .split(" (Part ")
                    .next()
                    .unwrap_or(&artifact.name),
                artifact.kind,
                artifact.language,
                artifact.operating_system,
                artifact.version
            )
        } else {
            format!("file|{}", artifact.download_path)
        };
        groups.entry(key).or_default().push(artifact);
    }
    let mut report = VerificationReport::default();
    let mut processed = 0_usize;
    let mut used_jobs = HashSet::new();
    for group in groups.values_mut() {
        group.sort_by_key(|artifact| artifact.part_number.unwrap_or(1));
        let requested_id = download::job_id(group);
        let job = completed
            .iter()
            .find(|job| job.job_id == requested_id)
            .or_else(|| {
                completed.iter().find(|job| {
                    !used_jobs.contains(&job.job_id)
                        && job.job_id.starts_with("import-")
                        && job.completed_files.len() == group.len()
                        && job.artifacts.first().is_some_and(|imported| {
                            imported.kind == group[0].kind
                                && optional_text_matches(
                                    imported.operating_system.as_deref(),
                                    group[0].operating_system.as_deref(),
                                )
                                && optional_text_matches(
                                    imported.language.as_deref(),
                                    group[0].language.as_deref(),
                                )
                        })
                })
            });
        let Some(job) = job else {
            continue;
        };
        let is_current_download = job.job_id == requested_id;
        let mut corrupt = Vec::new();
        let mut matched = true;
        for artifact in group.iter() {
            let _ = progress.send(VerificationEvent::Progress {
                message: format!("Fetching GOG checksum for {}…", artifact.name),
                fraction: None,
            });
            let checksum = match download::gog_checksum(artifact, access_token) {
                Ok(checksum) => checksum,
                Err(_) => {
                    matched = false;
                    break;
                }
            };
            if let Ok(store) = StateStore::open()
                && let Err(error) = store.observe_part_checksum(artifact, &checksum.md5)
            {
                tracing::warn!(product_id = artifact.product_id, %error, "could not persist GOG checksum identity");
            }
            let local = job.completed_files.iter().find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(&checksum.filename))
            });
            let Some(local) = local else {
                if is_current_download {
                    corrupt.push(job.destination.join(&checksum.filename));
                    continue;
                }
                matched = false;
                break;
            };
            let size_matches = local
                .metadata()
                .is_ok_and(|metadata| metadata.len() == checksum.size);
            let hash_matches = size_matches
                && download::file_md5_with_progress(local, |read, total| {
                    let fraction = if total > 0 {
                        read as f64 / total as f64
                    } else {
                        0.0
                    };
                    let _ = progress.send(VerificationEvent::Progress {
                        message: format!(
                            "Verifying {} (file {} of {})",
                            checksum.filename,
                            processed + 1,
                            total_downloaded_files
                        ),
                        fraction: Some(fraction.min(1.0)),
                    });
                })
                .is_ok_and(|actual| actual.eq_ignore_ascii_case(&checksum.md5));
            processed += 1;
            if hash_matches {
                report.checked += 1;
                if let Ok(store) = StateStore::open()
                    && let Err(error) =
                        store.mark_managed_file_verified(local, artifact, &checksum.md5)
                {
                    tracing::warn!(path = %local.display(), %error, "could not record verified managed file");
                }
            } else {
                corrupt.push(local.clone());
            }
        }
        if !matched {
            continue;
        }
        used_jobs.insert(job.job_id.clone());
        if !corrupt.is_empty() {
            for path in corrupt {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let artifacts = group.iter().map(|artifact| (*artifact).clone()).collect();
            let (sender, _receiver) = mpsc::channel();
            download::enqueue(download::DownloadRequest {
                artifacts,
                title: title.to_owned(),
                access_token: access_token.to_owned(),
                destination: job.destination.clone(),
                events: sender,
            });
            report.repair_groups += 1;
        }
    }
    report.unavailable += completed.len().saturating_sub(used_jobs.len());
    Ok(report)
}

pub(super) fn file_collection(
    title: &str,
    icon_name: &str,
    files: &[crate::domain::LibraryFile],
    folder: &std::path::Path,
    window: &adw::ApplicationWindow,
) -> gtk::Box {
    let collection = gtk::Box::new(gtk::Orientation::Vertical, 0);
    collection.add_css_class("file-collection");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("file-collection-header");
    header.append(&gtk::Image::from_icon_name(icon_name));
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.add_css_class("section-title");
    header.append(&heading);
    let count = gtk::Label::new(Some(&format!("{} files", files.len())));
    count.add_css_class("dim-label");
    header.append(&count);
    header.append(&folder_button(
        &format!("Open {title} folder"),
        folder,
        window,
    ));
    collection.append(&header);

    if files.is_empty() {
        let empty = gtk::Label::new(Some("No local files found"));
        empty.set_xalign(0.0);
        empty.add_css_class("file-empty");
        collection.append(&empty);
        return collection;
    }
    for file in files.iter().take(25) {
        let widgets = build_unified_file_row(UnifiedFileRow {
            artifact: inferred_local_artifact(file),
            title: file.name.clone(),
            metadata: String::new(),
            size: Some(human_size(file.size)),
            state: Some(UnifiedFileState::Local),
            dimmed: false,
        });
        widgets
            .row
            .set_tooltip_text(Some(&file.path.display().to_string()));
        collection.append(&widgets.row);
    }
    if files.len() > 25 {
        let more = gtk::Label::new(Some(&format!("{} additional files", files.len() - 25)));
        more.set_xalign(0.0);
        more.add_css_class("file-empty");
        collection.append(&more);
    }
    collection
}

fn inferred_local_artifact(file: &LibraryFile) -> Option<RemoteArtifact> {
    let components = file
        .path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let operating_system = components.iter().find_map(|component| {
        let normalized = component.to_ascii_lowercase();
        matches!(
            normalized.as_str(),
            "windows" | "linux" | "mac" | "macos" | "osx"
        )
        .then_some(normalized)
    });
    let language = operating_system.as_ref().and_then(|os| {
        components
            .iter()
            .position(|component| component.eq_ignore_ascii_case(os))
            .and_then(|index| components.get(index + 1).cloned())
    });
    if operating_system.is_none() && language.is_none() {
        return None;
    }
    let kind = if components
        .iter()
        .any(|part| part.eq_ignore_ascii_case("patch"))
    {
        ArtifactKind::Patch
    } else if components
        .iter()
        .any(|part| part.eq_ignore_ascii_case("extra"))
    {
        ArtifactKind::Extra
    } else {
        ArtifactKind::Installer
    };
    Some(RemoteArtifact {
        product_id: 0,
        kind,
        name: file.name.clone(),
        language,
        operating_system,
        version: None,
        release_date: None,
        size_label: None,
        size_bytes: Some(file.size),
        part_number: None,
        part_count: None,
        download_path: String::new(),
        provider_group_id: None,
        provider_file_id: None,
        provider_category: None,
    })
}

#[cfg(test)]
mod unified_row_tests {
    use super::*;

    #[test]
    fn local_identity_uses_managed_path_os_and_language() {
        let file = LibraryFile {
            name: "setup.exe".into(),
            path: std::path::PathBuf::from("/downloads/game/installer/windows/english/setup.exe"),
            size: 42,
        };
        let artifact = inferred_local_artifact(&file).unwrap();
        assert_eq!(artifact.operating_system.as_deref(), Some("windows"));
        assert_eq!(artifact.language.as_deref(), Some("english"));
        assert_eq!(artifact.kind, ArtifactKind::Installer);
    }

    #[test]
    fn file_action_labels_are_compact() {
        assert_eq!(
            compact_file_action_label("Show downloaded files"),
            "Open Folder"
        );
        assert_eq!(
            compact_file_action_label("Delete downloaded files"),
            "Delete"
        );
        assert_eq!(compact_file_action_label("Run Patch"), "Run Patch");
    }
}
