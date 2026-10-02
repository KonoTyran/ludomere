use super::*;

pub(super) fn show_game(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    force_refresh: Option<bool>,
) {
    if force_refresh.is_none()
        && model.borrow().selected == Some(id)
        && model.borrow().detail_target == Some((id, None))
        && w.content.visible_child_name().as_deref() == Some("details")
    {
        let adjustment = w.details_scroll.vadjustment();
        adjustment.set_value(adjustment.lower());
        return;
    }
    let mut m = model.borrow_mut();
    m.selected = Some(id);
    let Some(game) = m.games.iter().find(|g| g.product_id == id).cloned() else {
        return;
    };
    let favorite = m.favorites.contains(&id);
    drop(m);
    render_detail_page(w, model, DetailPageModel::game(game, favorite));
}

pub(super) fn render_detail_page(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    game: DetailPageModel,
) {
    let favorite = game.favorite.unwrap_or(false);
    {
        let mut state = model.borrow_mut();
        state.detail_generation = state.detail_generation.wrapping_add(1);
        state.detail_target = Some((game.product_id, game.parent_id));
    }
    refresh_local_products(
        w,
        model,
        &HashSet::from([game.parent_id.unwrap_or(game.product_id)]),
    );
    if game.parent_id.is_none()
        && model
            .borrow()
            .local_actions
            .get(&game.product_id)
            .is_some_and(|local| local.depot)
    {
        request_product_section(
            w,
            model,
            game.product_id,
            online::DetailSection::Builds,
            false,
        );
    }
    while let Some(child) = w.details.first_child() {
        w.details.remove(&child);
    }
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.set_margin_start(36);
    page.set_margin_end(36);
    page.set_margin_top(12);
    page.set_margin_bottom(48);
    if let Some(parent_id) = game.parent_id {
        let back = gtk::Button::with_label("← Back to game");
        back.set_halign(gtk::Align::Start);
        back.add_css_class("flat");
        let widgets = w.clone_refs();
        let model_for_back = model.clone();
        back.connect_clicked(move |_| {
            show_game(&widgets, &model_for_back, parent_id, Some(false));
            let root: gtk::Widget = widgets.details.clone().upcast();
            if let Some(stack) =
                find_named_descendant(&root, "game-tabs").and_downcast::<gtk::Stack>()
            {
                stack.set_visible_child_name("dlc");
            }
        });
        page.append(&back);
    }
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    title_row.set_valign(gtk::Align::End);
    title_row.set_halign(gtk::Align::Fill);
    title_row.add_css_class("detail-hero-content");
    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    title_box.set_hexpand(true);
    let title = gtk::Label::new(Some(&game.title));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_width_chars(1);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.add_css_class("game-title");
    title.add_css_class("detail-hero-title");
    title.set_widget_name("detail-hero-text-title");
    title.set_visible(game.hero_logo.is_none());
    let hero_logo = picture(game.hero_logo.as_ref(), 280, 90, "detail-hero-logo");
    hero_logo.set_content_fit(gtk::ContentFit::Contain);
    hero_logo.set_widget_name("detail-hero-logo");
    hero_logo.set_halign(gtk::Align::Start);
    hero_logo.set_visible(game.hero_logo.is_some());
    let disk_usage = game.disk_usage;
    let subtitle = gtk::Label::new(Some(&format!(
        "{}{}{}{} · {}",
        game.release_year
            .map(|x| x.to_string() + " · ")
            .unwrap_or_default(),
        game.kind
            .as_ref()
            .map(|kind| format!("{kind} · "))
            .unwrap_or_default(),
        if game.owned { "" } else { "Not owned · " },
        game.platform_label,
        human_size(disk_usage)
    )));
    subtitle.set_widget_name(&format!("managed-product-subtitle-{}", game.product_id));
    subtitle.set_xalign(0.0);
    subtitle.set_width_chars(1);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.add_css_class("detail-action-metadata");
    title_box.append(&hero_logo);
    title_box.append(&title);
    title_row.append(&title_box);
    let action_bar = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    action_bar.set_halign(gtk::Align::Fill);
    action_bar.add_css_class("detail-action-bar");
    let local = model
        .borrow()
        .local_actions
        .get(&game.product_id)
        .cloned()
        .unwrap_or_default();
    let installed = model
        .borrow()
        .installed_games
        .get(&game.product_id)
        .cloned()
        .map(|game| (game, local.installed_update));
    let backup_update = local.backup_update;
    let dlc_action = local.dlc;
    let current_installer_downloaded = local.downloaded;
    let primary_action = current_primary_action(&model.borrow(), game.product_id, game.parent_id);
    let activity = installed
        .as_ref()
        .map(|(installed, _)| (installed.last_played_at, installed.playtime_seconds))
        .or_else(|| {
            model
                .borrow()
                .product_activity
                .get(&game.product_id)
                .map(|activity| (activity.last_played_at, activity.playtime_seconds))
        })
        .unwrap_or((None, 0));
    let last_played_value = gtk::Label::new(Some(&format_last_played(activity.0)));
    let playtime_value = gtk::Label::new(Some(&format_playtime(activity.1)));
    let download_button = gtk::Button::new();
    download_button.set_widget_name("game-primary-action");
    download_button.set_width_request(180);
    set_primary_button_content(
        &download_button,
        if game.owned {
            primary_action.icon()
        } else {
            "external-link-symbolic"
        },
        if game.owned {
            primary_action.label()
        } else {
            "View in Store"
        },
    );
    download_button.add_css_class("suggested-action");
    download_button.add_css_class("detail-download-action");
    download_button.add_css_class("steam-primary-action");
    download_button.set_tooltip_text(Some(if !game.owned {
        "Purchase this DLC"
    } else if primary_action == GamePrimaryAction::Install {
        "Choose a downloaded installer"
    } else if primary_action == GamePrimaryAction::DownloadUpdate {
        "Download files for the latest GOG revision"
    } else if primary_action == GamePrimaryAction::InstallUpdate {
        "Review and apply the game update"
    } else if primary_action == GamePrimaryAction::Play {
        "Launch this game"
    } else {
        "Choose a download source, installers, DLC, or extras"
    }));
    let primary_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    primary_actions.add_css_class("detail-primary-actions");
    let cloud_launch_status = Rc::new(RefCell::new(None::<CloudLaunchStatus>));
    let launch_pending = Rc::new(std::cell::Cell::new(false));
    {
        let detail = game.clone();
        let model = model.clone();
        let widgets = w.clone_refs();
        let primary_button = download_button.clone();
        let action_group = primary_actions.clone();
        let last_played_value = last_played_value.clone();
        let activity_model = model.clone();
        let activity_widgets = widgets.clone_refs();
        let playtime_value = playtime_value.clone();
        let previous_playtime = activity.1;
        let cloud_launch_status = cloud_launch_status.clone();
        let launch_pending = launch_pending.clone();
        download_button.connect_clicked(move |_| {
            {
                let state = model.borrow();
                if state
                    .installed_games
                    .get(&detail.product_id)
                    .is_some_and(|game| {
                        matches!(
                            crate::storage::path_status(
                                &state.library_statuses,
                                &game.installation_directory
                            ),
                            Some(
                                crate::storage::LibraryCompatibility::Incompatible(_)
                                    | crate::storage::LibraryCompatibility::Unavailable(_)
                            )
                        )
                    })
                {
                    return;
                }
            }
            let Some(detail) = current_detail(&model.borrow(), detail.product_id, detail.parent_id)
            else {
                return;
            };
            let primary_action =
                current_primary_action(&model.borrow(), detail.product_id, detail.parent_id);
            if !detail.owned {
                if let Some(uri) = detail.links.store.as_deref() {
                    let launcher = gtk::UriLauncher::new(uri);
                    let parent = widgets.window.clone();
                    launcher.launch(
                        Some(&widgets.window),
                        gio::Cancellable::NONE,
                        move |result| {
                            widgets::file_open::report_launch_result(
                                &parent,
                                "DLC store page",
                                result,
                            );
                        },
                    );
                }
                return;
            }
            if let Some(snapshot) =
                crate::installation::depot_operation_snapshot_for_product(detail.product_id)
                && matches!(snapshot.state.as_str(), "interrupted" | "failed")
            {
                let Some(token) = model
                    .borrow()
                    .account_token
                    .as_ref()
                    .map(|token| token.access_token.clone())
                else {
                    widgets.reconnect.emit_clicked();
                    return;
                };
                let resume_window = widgets.window.clone();
                let resume_model = model.clone();
                with_windows_components(
                    &widgets.window,
                    detail.product_id,
                    true,
                    None,
                    move || {
                        download_chooser::review_depot_resume(
                            &resume_window,
                            &resume_model,
                            snapshot.operation_id,
                            token,
                        );
                    },
                );
                return;
            }
            if crate::installation::installation_operation_snapshot(detail.product_id).is_some_and(
                |snapshot| {
                    snapshot.queued
                        || matches!(
                            snapshot.state,
                            crate::domain::InstallationState::Installing
                                | crate::domain::InstallationState::Uninstalling
                        )
                },
            ) {
                crate::installation::cancel_operation(detail.product_id);
                return;
            }
            let active_downloads = model
                .borrow()
                .download_jobs
                .iter()
                .filter(|job| {
                    job.product_id == detail.product_id
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
            if crate::installation::is_game_running(detail.product_id) {
                if crate::installation::stop_game(detail.product_id) {
                    set_primary_button_content(
                        &primary_button,
                        "process-stop-symbolic",
                        "Stopping",
                    );
                    primary_button.add_css_class("operational-action");
                    action_group.add_css_class("operational-state");
                    primary_button.set_sensitive(false);
                }
                return;
            }
            if primary_action == GamePrimaryAction::Install {
                show_install_dialog(&widgets.window, &model, &detail);
                return;
            }
            if primary_action == GamePrimaryAction::InstallUpdate {
                show_update_dialog(&widgets.window, &model, &detail);
                return;
            }
            if primary_action == GamePrimaryAction::Play {
                let installed = model
                    .borrow()
                    .installed_games
                    .get(&detail.product_id)
                    .cloned();
                let Some(installed) = installed else { return };
                if installed.primary_executable.is_none() {
                    let retry_button = primary_button.clone();
                    if prompt_for_windows_executable(
                        &widgets.window,
                        &detail.title,
                        &installed,
                        Rc::new(move || retry_button.emit_clicked()),
                    ) {
                        return;
                    }
                }
                if let Some(status) = cloud_launch_status.borrow().as_ref() {
                    status.hide();
                }
                let session = online::account_session();
                let auth_session = auth::session();
                let epoch = model.borrow().account_epoch;
                let launch_generation = model.borrow().detail_generation;
                launch_pending.set(true);
                let receiver = launch_with_components(&widgets.window, installed);
                let button = primary_button.clone();
                let action_group = action_group.clone();
                button.set_sensitive(false);
                let window = widgets.window.clone();
                let last_played_value = last_played_value.clone();
                let playtime_value = playtime_value.clone();
                let activity_model = activity_model.clone();
                let activity_widgets = activity_widgets.clone();
                let cloud_launch_status = cloud_launch_status.clone();
                let launch_pending = launch_pending.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if activity_model.borrow().account_epoch != epoch
                        || activity_model.borrow().logout_pending
                        || auth::session() != auth_session
                        || online::account_session() != session
                    {
                        launch_pending.set(false);
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(
                            event @ (crate::installation::LaunchEvent::EnablementRequired {
                                ..
                            }
                            | crate::installation::LaunchEvent::PreLaunchConflict { .. }
                            | crate::installation::LaunchEvent::LaunchWithoutSyncRequired {
                                ..
                            }
                            | crate::installation::LaunchEvent::SyncWarning(_)
                            | crate::installation::LaunchEvent::PostExitSync(_)
                            | crate::installation::LaunchEvent::PostExitConflict(_)),
                        ) => {
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.hide();
                            }
                            present_cloud_launch_event(&window, event);
                            glib::ControlFlow::Continue
                        }
                        Ok(crate::installation::LaunchEvent::CloudSyncStarted(phase)) => {
                            launch_pending.set(true);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.show(phase);
                            }
                            button.set_sensitive(false);
                            button.add_css_class("operational-action");
                            action_group.add_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                "emblem-synchronizing-symbolic",
                                "Syncing saves",
                            );
                            glib::ControlFlow::Continue
                        }
                        Ok(crate::installation::LaunchEvent::Started) => {
                            launch_pending.set(false);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.hide();
                            }
                            button.set_sensitive(true);
                            button.add_css_class("operational-action");
                            action_group.add_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                "media-playback-stop-symbolic",
                                "Stop",
                            );
                            glib::ControlFlow::Continue
                        }
                        Ok(crate::installation::LaunchEvent::Exited {
                            started_at,
                            seconds,
                            ..
                        }) => {
                            launch_pending.set(false);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.hide();
                            }
                            button.set_sensitive(true);
                            button.remove_css_class("operational-action");
                            action_group.remove_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                primary_action.icon(),
                                primary_action.label(),
                            );
                            button.set_tooltip_text(Some(&format!(
                                "Last session: {}",
                                format_playtime(seconds)
                            )));
                            last_played_value.set_label(&format_last_played(Some(started_at)));
                            playtime_value.set_label(&format_playtime(previous_playtime + seconds));
                            {
                                let mut state = activity_model.borrow_mut();
                                let activity =
                                    state.product_activity.entry(detail.product_id).or_default();
                                activity.last_played_at = Some(started_at);
                                activity.last_activity_at = Some(started_at);
                                activity.playtime_seconds =
                                    activity.playtime_seconds.saturating_add(seconds);
                                if state.sidebar_sort_mode == SidebarSortMode::LastPlayed {
                                    rebuild_sidebar_presentation(&activity_widgets, &mut state);
                                } else {
                                    refresh_filters(&activity_widgets, &state);
                                }
                            }
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::LaunchEvent::PrefixRecoveryRequired {
                            message,
                            game,
                            setup_required,
                        }) => {
                            launch_pending.set(false);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                *status.failure.borrow_mut() = Some(message.clone());
                            }
                            button.set_sensitive(true);
                            button.remove_css_class("operational-action");
                            action_group.remove_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                primary_action.icon(),
                                primary_action.label(),
                            );
                            offer_prefix_recovery(
                                &window,
                                &activity_model,
                                *game,
                                &detail.title,
                                &message,
                                setup_required,
                                launch_generation,
                            );
                            glib::ControlFlow::Break
                        }
                        Ok(crate::installation::LaunchEvent::Failed(error)) => {
                            launch_pending.set(false);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                *status.failure.borrow_mut() = Some(
                                    notifications::failure_message("", &error)
                                        .trim_start()
                                        .to_owned(),
                                );
                            }
                            button.set_sensitive(true);
                            button.remove_css_class("operational-action");
                            action_group.remove_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                primary_action.icon(),
                                primary_action.label(),
                            );
                            show_status(
                                &activity_widgets,
                                &notifications::failure_message(
                                    &format!("{}: Could not run game", detail.title),
                                    &error,
                                ),
                            );
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            launch_pending.set(false);
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.hide();
                            }
                            button.set_sensitive(true);
                            button.remove_css_class("operational-action");
                            action_group.remove_css_class("operational-state");
                            set_primary_button_content(
                                &button,
                                primary_action.icon(),
                                primary_action.label(),
                            );
                            glib::ControlFlow::Break
                        }
                    }
                });
                return;
            }
            show_primary_download(&widgets, &model, &detail);
        });
    }
    if game.owned
        && matches!(
            primary_action,
            GamePrimaryAction::Download
                | GamePrimaryAction::Install
                | GamePrimaryAction::DownloadUpdate
                | GamePrimaryAction::InstallUpdate
        )
    {
        primary_actions.add_css_class("download-state");
    }
    primary_actions.append(&download_button);
    if game.owned
        && (matches!(
            primary_action,
            GamePrimaryAction::DownloadUpdate | GamePrimaryAction::InstallUpdate
        ) || primary_action == GamePrimaryAction::Play
            || (primary_action == GamePrimaryAction::Install && !game.remote_artifacts.is_empty()))
    {
        let alternate_actions = gtk::MenuButton::new();
        alternate_actions.set_widget_name("game-alternate-actions");
        alternate_actions.set_width_request(34);
        alternate_actions.set_icon_name("pan-down-symbolic");
        alternate_actions.add_css_class("detail-action-menu");
        alternate_actions.set_tooltip_text(Some("More game actions"));
        let popover = gtk::Popover::new();
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions.set_margin_start(6);
        actions.set_margin_end(6);
        actions.set_margin_top(6);
        actions.set_margin_bottom(6);
        if installed.is_some() {
            let check = gtk::Button::with_label("Check for Updates");
            check.set_sensitive(false);
            check.set_visible(false);
            check.add_css_class("flat");
            check.set_halign(gtk::Align::Fill);
            actions.append(&check);
            popover.connect_show({
                let model = model.clone();
                let check = check.downgrade();
                let generation = Rc::new(std::cell::Cell::new(0_u64));
                let id = game.product_id;
                move |_| {
                    generation.set(generation.get() + 1);
                    let current = generation.get();
                    let check = check.clone();
                    let model = model.clone();
                    let generation = generation.clone();
                    glib::idle_add_local_once(move || {
                        let Some(check) = check.upgrade() else {
                            return;
                        };
                        let (config, epoch, online, depot) = {
                            let state = model.borrow();
                            (
                                state.config.clone(),
                                state.account_epoch,
                                state.network_available
                                    && state.account_token.is_some()
                                    && !state.logout_pending,
                                state
                                    .local_actions
                                    .get(&id)
                                    .is_some_and(|local| local.depot),
                            )
                        };
                        let session = online::account_session();
                        check.set_visible(true);
                        check.set_sensitive(false);
                        check.set_label("Checking update policy…");
                        let receiver = update_policies::policy_request(move || {
                            let _activity =
                                crate::profile_reset::begin_activity("loading update preferences")?;
                            online::with_account_session(session, || {
                                let preferences = StateStore::open()?.game_preferences(id)?;
                                let policy = crate::updates::UpdatePolicy::resolve(
                                    &config,
                                    preferences.as_ref(),
                                );
                                Ok(if depot {
                                    policy.auto_update_galaxy
                                } else {
                                    policy.auto_download_offline_installer
                                })
                            })
                        });
                        let check = check.downgrade();
                        glib::timeout_add_local(Duration::from_millis(50), move || {
                            if generation.get() != current
                                || model.borrow().account_epoch != epoch
                                || model.borrow().logout_pending
                                || online::account_session() != session
                            {
                                return glib::ControlFlow::Break;
                            }
                            let Some(check) = check.upgrade() else {
                                return glib::ControlFlow::Break;
                            };
                            match receiver.try_recv() {
                                Err(mpsc::TryRecvError::Empty) => {
                                    return glib::ControlFlow::Continue;
                                }
                                Ok(Ok(automatic)) => {
                                    check.set_label("Check for Updates");
                                    check.set_visible(!automatic);
                                    check.set_sensitive(online);
                                    check.set_tooltip_text(
                                        (!online).then_some(
                                            "Sign in and go online to check for updates",
                                        ),
                                    );
                                }
                                Ok(Err(error)) => {
                                    check.set_label("Could not load update policy");
                                    check.set_tooltip_text(Some(&error.to_string()));
                                }
                                Err(_) => {
                                    check.set_label("Update policy loading stopped");
                                    check.set_tooltip_text(Some(
                                        "Close and reopen this menu to retry",
                                    ));
                                }
                            }
                            glib::ControlFlow::Break
                        });
                    });
                }
            });
            check.connect_clicked({
                let model = model.clone();
                let window = w.window.downgrade();
                let popover = popover.downgrade();
                let detail = game.clone();
                move |_| {
                    if let Some(popover) = popover.upgrade() {
                        popover.popdown();
                    }
                    if let Some(window) = window.upgrade() {
                        show_manual_update_check(&window, &model, &detail);
                    }
                }
            });
        }
        if installed.is_some() {
            let update = gtk::Button::with_label("Install update");
            update.add_css_class("flat");
            update.set_halign(gtk::Align::Fill);
            update.set_visible(false);
            popover.connect_show({
                let model = model.clone();
                let update = update.downgrade();
                let id = game.product_id;
                move |_| {
                    let model = model.clone();
                    let update = update.clone();
                    glib::idle_add_local_once(move || {
                        let local = model
                            .borrow()
                            .local_actions
                            .get(&id)
                            .cloned()
                            .unwrap_or_default();
                        if let Some(update) = update.upgrade() {
                            update.set_visible(
                                local.installed_update
                                    || local.backup_update
                                    || local.dlc.missing_download
                                    || local.dlc.missing_install,
                            );
                            update.set_label(
                                if local.installed_update && (local.depot || local.downloaded)
                                    || local.dlc.missing_install && !local.dlc.missing_download
                                {
                                    "Install update"
                                } else {
                                    "Download update"
                                },
                            );
                        }
                    });
                }
            });
            let widgets = w.clone_refs();
            let model = model.clone();
            let detail = game.clone();
            let action_popover = popover.downgrade();
            update.connect_clicked(move |_| {
                if let Some(popover) = action_popover.upgrade() {
                    popover.popdown();
                }
                let local = model
                    .borrow()
                    .local_actions
                    .get(&detail.product_id)
                    .cloned()
                    .unwrap_or_default();
                let install_update = local.installed_update && (local.depot || local.downloaded)
                    || local.dlc.missing_install && !local.dlc.missing_download;
                if install_update {
                    show_update_dialog(&widgets.window, &model, &detail);
                } else {
                    show_download_selector(&widgets, &model, &detail);
                }
            });
            actions.append(&update);
        }
        if matches!(
            primary_action,
            GamePrimaryAction::DownloadUpdate | GamePrimaryAction::InstallUpdate
        ) && let Some((installed_game, _)) = installed.clone()
        {
            let play_current = gtk::Button::with_label("Play");
            play_current.add_css_class("flat");
            play_current.set_halign(gtk::Align::Fill);
            let widgets = w.clone_refs();
            let title = game.title.clone();
            let model = model.clone();
            let action_popover = popover.clone();
            play_current.connect_clicked(move |_| {
                action_popover.popdown();
                launch_installed_game(&widgets, &model, installed_game.clone(), &title);
            });
            actions.append(&play_current);
        }
        let downloaded_install_alternative = primary_action == GamePrimaryAction::DownloadUpdate
            && (current_installer_downloaded || backup_update || dlc_action.missing_install);
        if downloaded_install_alternative || primary_action == GamePrimaryAction::Play {
            let install_downloaded = gtk::Button::with_label("Install");
            if primary_action == GamePrimaryAction::Play {
                install_downloaded.set_label("Manage installed content");
            }
            install_downloaded.add_css_class("flat");
            install_downloaded.set_halign(gtk::Align::Fill);
            let detail = game.clone();
            let window = w.window.clone();
            let model = model.clone();
            let action_popover = popover.clone();
            install_downloaded.connect_clicked(move |_| {
                action_popover.popdown();
                show_install_dialog(&window, &model, &detail);
            });
            actions.append(&install_downloaded);
        }
        let manage_downloads = gtk::Button::with_label("Additional Downloads");
        manage_downloads.add_css_class("flat");
        manage_downloads.set_halign(gtk::Align::Fill);
        let detail = game.clone();
        let model = model.clone();
        let widgets = w.clone_refs();
        let action_popover = popover.clone();
        manage_downloads.connect_clicked(move |_| {
            action_popover.popdown();
            show_download_selector(&widgets, &model, &detail);
        });
        actions.append(&manage_downloads);
        popover.set_child(Some(&actions));
        alternate_actions.set_popover(Some(&popover));
        primary_actions.append(&alternate_actions);
    }
    action_bar.append(&primary_actions);
    let installation_was_running = installed.as_ref().is_some_and(|(installed, _)| {
        matches!(
            installed.state,
            crate::domain::InstallationState::Installing
                | crate::domain::InstallationState::Uninstalling
        )
    });
    let refresh_after_install: Rc<dyn Fn()> = {
        let widgets = w.clone_refs();
        let model = model.clone();
        let product_id = game.parent_id.unwrap_or(game.product_id);
        Rc::new(move || {
            refresh_local_products(&widgets, &model, &HashSet::from([product_id]));
            update_sidebar_download_styles(&widgets, &model.borrow());
            refresh_filters(&widgets, &model.borrow());
        })
    };
    action_bar.append(&installation_status_panel(
        game.product_id,
        w,
        (&download_button, &primary_actions, &cloud_launch_status),
        &launch_pending,
        installation_was_running,
        model,
        refresh_after_install.clone(),
    ));
    if game.parent_id.is_none() {
        action_bar.append(&activity_stat("LAST PLAYED", &last_played_value));
        action_bar.append(&activity_stat("PLAY TIME", &playtime_value));
    }
    action_bar.append(&subtitle);
    let action_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    action_spacer.set_hexpand(true);
    action_bar.append(&action_spacer);
    let file_management = game.owned.then(|| {
        detail_file_management(
            &game,
            &w.window,
            model,
            installed
                .as_ref()
                .filter(|(game, _)| {
                    matches!(
                        game.state,
                        crate::domain::InstallationState::Installed
                            | crate::domain::InstallationState::UninstallFailed
                    )
                })
                .map(|(game, _)| game.clone()),
            refresh_after_install.clone(),
            {
                let download_button = download_button.clone();
                Rc::new(move || download_button.emit_clicked())
            },
        )
    });
    if let Some(management) = &file_management {
        action_bar.append(&management.menu);
    }
    if game.favorite.is_some() {
        let star = gtk::Button::from_icon_name(if favorite {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        star.set_widget_name(&format!("detail-favorite-{}", game.product_id));
        star.set_tooltip_text(Some(if favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        }));
        star.set_action_name(Some("win.favorite"));
        star.set_action_target_value(Some(&game.product_id.to_variant()));
        star.add_css_class("square-action");
        star.add_css_class("steam-utility-action");
        action_bar.append(&star);
    }
    let hero = gtk::Overlay::new();
    hero.set_child(Some(&parallax_detail_hero(
        game.detail_artwork.as_ref(),
        &w.details_scroll.vadjustment(),
    )));
    hero.add_overlay(&title_row);
    hero.add_css_class("detail-hero-container");
    w.details.append(&hero);
    w.details.append(&action_bar);

    let tabs = gtk::Stack::new();
    tabs.set_widget_name("game-tabs");
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_vhomogeneous(false);
    tabs.set_vexpand(true);
    let switcher = gtk::StackSwitcher::builder()
        .stack(&tabs)
        .halign(gtk::Align::Start)
        .build();
    switcher.add_css_class("detail-tabs");
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    navigation.add_css_class("game-navigation");
    navigation.set_margin_start(36);
    navigation.set_margin_end(36);
    navigation.append(&switcher);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    navigation.append(&spacer);
    let external_links = [
        ("Store Page", game.links.store.as_deref()),
        ("Community Forum", game.links.forum.as_deref()),
        ("Support", game.links.support.as_deref()),
    ];
    let overflow = gtk::MenuButton::new();
    overflow.set_icon_name("view-more-symbolic");
    overflow.set_tooltip_text(Some("More links"));
    overflow.add_css_class("navigation-overflow");
    let overflow_popover = gtk::Popover::new();
    let overflow_content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    overflow_content.set_margin_start(6);
    overflow_content.set_margin_end(6);
    overflow_content.set_margin_top(6);
    overflow_content.set_margin_bottom(6);
    let mut has_external_links = false;
    for (label, url) in external_links {
        let Some(url) = url else { continue };
        has_external_links = true;
        overflow_content.append(&uri_button(label, url, &w.window));
    }
    overflow_popover.set_child(Some(&overflow_content));
    overflow.set_popover(Some(&overflow_popover));
    overflow.set_visible(has_external_links);
    navigation.append(&overflow);
    let navigation_shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    navigation_shell.set_margin_top(12);
    navigation_shell.add_css_class("game-navigation-shell");
    navigation_shell.append(&navigation);
    w.details.append(&navigation_shell);

    let overview = gtk::Box::new(gtk::Orientation::Vertical, 20);
    overview.set_margin_top(12);
    overview.append(&detail_section(
        w,
        model,
        &game,
        online::DetailSection::Product,
        "Overview",
        true,
        Rc::new(|game| {
            expandable_section("Overview", text::html_to_text(&game.description), 1_600).upcast()
        }),
    ));
    let screenshot_window = w.window.clone();
    overview.append(&detail_section(
        w,
        model,
        &game,
        online::DetailSection::Product,
        "Screenshots",
        true,
        Rc::new(move |game| {
            screenshot_strip(game.product_id, &game.screenshots, &screenshot_window).upcast()
        }),
    ));
    {
        let facts = format!(
            "{}Slug: {}\nLanguages: {}\nFeatures: {}\nLocation: {}",
            game.parent_title
                .as_ref()
                .map(|title| format!("Parent game: {title}\n"))
                .unwrap_or_default(),
            game.slug,
            empty_dash(&game.languages.join(", ")),
            empty_dash(&game.features.join(", ")),
            game.location.display()
        );
        overview.append(&section("Library information", &facts));
        overview.append(&detail_section(
            w,
            model,
            &game,
            online::DetailSection::Metadata,
            "Game information",
            true,
            Rc::new(|game| {
                let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
                append_official_metadata(&content, &game.metadata);
                content.upcast()
            }),
        ));
        overview.append(&organization::tag_editor(w, model, game.product_id));
    }
    tabs.add_titled(&overview, Some("overview"), "Overview");

    if game.parent_id.is_none() {
        let parent_id = game.parent_id.unwrap_or(game.product_id);
        let widgets = w.clone_refs();
        let detail_model = model.clone();
        let dlc_view = detail_section(
            w,
            model,
            &game,
            online::DetailSection::Product,
            "DLC",
            false,
            Rc::new(move |game| {
                build_dlc_catalog(&game.dlcs, &widgets, &detail_model, parent_id).upcast()
            }),
        );
        tabs.add_titled(&dlc_view, Some("dlc"), "DLC");
    }

    if game.parent_id.is_none() {
        let note_model = model.clone();
        tabs.add_titled(
            &detail_section(
                w,
                model,
                &game,
                online::DetailSection::Product,
                "Patch notes",
                false,
                Rc::new(move |game| {
                    build_patch_notes_page(cached_patch_notes(
                        &note_model,
                        game.product_id,
                        &game.changelog,
                    ))
                    .upcast()
                }),
            ),
            Some("patch-notes"),
            "Patch Notes",
        );
    }

    if game.owned && game.parent_id.is_none() {
        let achievements = gtk::Box::new(gtk::Orientation::Vertical, 8);
        achievements.append(&gtk::Label::new(Some("Open this tab to load achievements")));
        tabs.add_titled(&achievements, Some("achievements"), "Achievements");
        let loaded = Rc::new(std::cell::Cell::new(false));
        let model = model.clone();
        let product_id = game.product_id;
        tabs.connect_visible_child_name_notify(move |tabs| {
            if tabs.visible_child_name().as_deref() == Some("achievements") && !loaded.replace(true)
            {
                while let Some(child) = achievements.first_child() {
                    achievements.remove(&child);
                }
                achievements.append(&super::achievements::achievement_page(&model, product_id));
            }
        });
    }

    let logs = gtk::Box::new(gtk::Orientation::Vertical, 12);
    logs.set_margin_top(12);
    logs.append(&super::logs::runtime_log_view(
        &w.window,
        model,
        game.product_id,
    ));
    let operation_logs = gtk::Box::new(gtk::Orientation::Vertical, 12);
    logs.append(&operation_logs);
    tabs.add_titled(&logs, Some("logs"), "Logs");
    tabs.connect_visible_child_name_notify({
        let logs = operation_logs;
        let window = w.window.clone();
        let product_id = game.product_id;
        move |tabs| {
            if tabs.visible_child_name().as_deref() == Some("logs") {
                refresh_product_logs(&logs, product_id, &window);
            }
        }
    });
    {
        let widgets = w.clone_refs();
        let model = model.clone();
        let id = game.parent_id.unwrap_or(game.product_id);
        tabs.connect_visible_child_name_notify(move |tabs| {
            let scope = match tabs.visible_child_name().as_deref() {
                Some("files") => Some(online::DetailSection::Acquisition),
                Some("dlc" | "patch-notes") => Some(online::DetailSection::Product),
                _ => None,
            };
            if let Some(scope) = scope {
                request_product_section(&widgets, &model, id, scope, false);
            }
        });
    }

    {
        if game.owned && game.parent_id.is_none() {
            let window = w.window.clone();
            let management = file_management.clone();
            let files_model = model.clone();
            let files = detail_section(
                w,
                model,
                &game,
                online::DetailSection::Acquisition,
                "Offline installers",
                false,
                Rc::new(move |game| {
                    let (config, token, installed, library_statuses) = {
                        let state = files_model.borrow();
                        (
                            state.config.clone(),
                            state
                                .account_token
                                .as_ref()
                                .map(|token| token.access_token.clone()),
                            state.installed_games.get(&game.product_id).cloned(),
                            state.library_statuses.clone(),
                        )
                    };
                    let installer_defaults = InstallerFilterDefaults {
                        language: config.installer_language.clone(),
                        windows: config.installer_windows,
                        linux: config.installer_linux,
                        macos: config.installer_macos,
                    };
                    build_files_page(
                        game,
                        &window,
                        FilesPageOptions {
                            access_token: token.as_deref(),
                            config: &config,
                            library_statuses: &library_statuses,
                            installer_defaults: &installer_defaults,
                            show_retired_artifacts: config.show_retired_artifacts,
                            management: management.as_ref(),
                            installed: installed.as_ref(),
                        },
                    )
                    .upcast()
                }),
            );
            tabs.add_titled(&files, Some("files"), "Offline Installers");
        }
    }
    page.append(&tabs);
    w.details.append(&page);
    w.content.set_visible_child_name("details");
    let widgets = w.clone_refs();
    let artwork_model = model.clone();
    let artwork_status = detail_section(
        w,
        model,
        &game,
        online::DetailSection::Artwork,
        "Artwork",
        true,
        Rc::new(move |detail| {
            update_streamed_media(
                &widgets,
                &artwork_model,
                detail.product_id,
                None,
                detail.detail_artwork.clone(),
                detail.hero_logo.clone(),
                detail.icon.clone(),
            );
            gtk::Box::new(gtk::Orientation::Vertical, 0).upcast()
        }),
    );
    title_box.append(&artwork_status);
    {
        let epoch = model.borrow().account_epoch;
        let model = model.clone();
        let widgets = w.clone_refs();
        let title_box = title_box.downgrade();
        let id = game.product_id;
        let parent = game.parent_id;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(title_box) = title_box.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if model.borrow().account_epoch == epoch {
                return glib::ControlFlow::Continue;
            }
            let reopen = gtk::Button::with_label("Account changed — reopen game");
            let widgets = widgets.clone_refs();
            let model = model.clone();
            reopen.connect_clicked(move |_| {
                let detail = current_detail(&model.borrow(), id, parent);
                if let Some(detail) = detail {
                    render_detail_page(&widgets, &model, detail);
                }
            });
            title_box.append(&reopen);
            glib::ControlFlow::Break
        });
    }
    let adjustment = w.details_scroll.vadjustment();
    glib::idle_add_local_once(move || adjustment.set_value(adjustment.lower()));
}

fn show_manual_update_check(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    let (game, installed, token, epoch, session) = {
        let state = model.borrow();
        let (Some(game), Some(installed), Some(token)) = (
            state
                .games
                .iter()
                .find(|game| game.product_id == detail.product_id),
            state.installed_games.get(&detail.product_id),
            state.account_token.as_ref(),
        ) else {
            return;
        };
        if state.logout_pending || !state.network_available {
            return;
        }
        (
            game.clone(),
            installed.clone(),
            token.clone(),
            state.account_epoch,
            online::account_session(),
        )
    };
    let dialog = adw::Dialog::builder()
        .title("Check for Updates")
        .content_width(480)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.append(&adw::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_bottom(20);
    let status = gtk::Label::new(Some("Checking for an update…"));
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.append(&status);
    let spinner = gtk::Spinner::builder().spinning(true).build();
    body.append(&spinner);
    let confirm = gtk::Button::with_label("Update");
    confirm.add_css_class("suggested-action");
    confirm.set_sensitive(false);
    body.append(&confirm);
    root.append(&body);
    dialog.set_child(Some(&root));
    let closed = Rc::new(std::cell::Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    let offer = Rc::new(RefCell::new(None::<crate::updates::ManualUpdateOffer>));
    let install_ready = Rc::new(std::cell::Cell::new(false));
    let submit = Rc::new({
        let model = model.clone();
        let closed = closed.clone();
        let status = status.downgrade();
        let confirm = confirm.downgrade();
        let install_ready = install_ready.clone();
        let token = token.clone();
        let dialog = dialog.downgrade();
        let spinner = spinner.downgrade();
        move |offer, library| {
            if closed.get()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
                || online::account_session() != session
            {
                return;
            }
            let (Some(status), Some(confirm)) = (status.upgrade(), confirm.upgrade()) else {
                return;
            };
            confirm.set_sensitive(false);
            if let Some(spinner) = spinner.upgrade() {
                spinner.start();
            }
            status.set_label("Preparing the confirmed update…");
            let token = token.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(crate::updates::confirm_installed_update(
                    offer, &token, session, library,
                ));
            });
            let closed = closed.clone();
            let model = model.clone();
            let install_ready = install_ready.clone();
            let dialog = dialog.clone();
            let spinner = spinner.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if closed.get()
                    || model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                    || online::account_session() != session
                {
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Ok(Ok(
                        crate::updates::ManualUpdateQueued::Depot
                        | crate::updates::ManualUpdateQueued::OfflineDownload,
                    )) => {
                        if let Some(dialog) = dialog.upgrade() {
                            dialog.close();
                        }
                    }
                    Ok(Ok(crate::updates::ManualUpdateQueued::OfflineReady)) => {
                        status.set_label("The update installer is ready. Continue to review installation; the current game remains installed until you confirm it.");
                        install_ready.set(true);
                        confirm.set_label("Install update");
                        confirm.set_sensitive(true);
                    }
                    Ok(Err(error)) => {
                        status.set_label(&format!("Could not queue the update: {error}"));
                        confirm.set_sensitive(true);
                    }
                    Err(_) => status.set_label("The update worker stopped. Close and check again."),
                }
                if let Some(spinner) = spinner.upgrade() {
                    spinner.stop();
                }
                glib::ControlFlow::Break
            });
        }
    });
    confirm.connect_clicked({
        let needs_windows = installed.compatibility.is_some();
        let model = model.clone();
        let window = window.downgrade();
        let dialog = dialog.downgrade();
        let detail = detail.clone();
        let offer = offer.clone();
        let install_ready = install_ready.clone();
        let closed = closed.clone();
        move |_| {
            if closed.get()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
                || online::account_session() != session
            {
                return;
            }
            let (Some(window), Some(dialog)) = (window.upgrade(), dialog.upgrade()) else {
                return;
            };
            if install_ready.get() {
                dialog.close();
                show_update_dialog(&window, &model, &detail);
                return;
            }
            let Some(offer) = offer.borrow().clone() else {
                return;
            };
            let submit = submit.clone();
            if offer.source == crate::domain::InstallationSource::OfflineInstaller
                && offer.download_required
            {
                download_chooser::choose_download_libraries(
                    &window,
                    vec![crate::config::LibraryKind::OfflineInstallers],
                    move |libraries| {
                        if let Some((_, library)) = libraries.into_iter().next() {
                            submit(offer, Some(library));
                        }
                    },
                );
            } else if offer.source == crate::domain::InstallationSource::GalaxyDepot
                && needs_windows
            {
                with_windows_components(&window, detail.product_id, true, None, move || {
                    submit(offer, None)
                });
            } else {
                submit(offer, None);
            }
        }
    });
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(crate::updates::check_installed_update(
            &game, &installed, &token, session,
        ));
    });
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if closed.get()
            || model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Ok(Ok(crate::updates::ManualUpdateCheck::UpToDate)) => status
                .set_label("No update is available. You can keep playing the installed version."),
            Ok(Ok(crate::updates::ManualUpdateCheck::Available(found))) => {
                let found = *found;
                status.set_label(&format!("An update{} is available. Nothing has been queued. Your automatic update preferences will stay unchanged.", found.version.as_ref().map(|version| format!(" ({version})")).unwrap_or_default()));
                confirm.set_label(
                    if found.source == crate::domain::InstallationSource::GalaxyDepot {
                        "Download and apply update"
                    } else if found.download_required {
                        "Download update"
                    } else {
                        "Review installer update"
                    },
                );
                confirm.set_sensitive(true);
                offer.replace(Some(found));
            }
            Ok(Err(error)) => status.set_label(&format!("Could not check for updates: {error}")),
            Err(_) => status.set_label("The update check stopped. Close and try again."),
        }
        spinner.stop();
        glib::ControlFlow::Break
    });
    dialog.present(Some(window));
}

fn detail_section(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    scope: online::DetailSection,
    label: &str,
    start: bool,
    build: Rc<dyn Fn(&DetailPageModel) -> gtk::Widget>,
) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(16, 16);
    let loading_label = format!("Loading {label}…");
    let message = gtk::Label::new(Some(&format!("{label} — load when opened")));
    message.set_xalign(0.0);
    message.set_wrap(true);
    let retry = gtk::Button::with_label("Retry");
    retry.add_css_class("flat");
    retry.set_visible(false);
    status.append(&spinner);
    status.append(&message);
    status.append(&retry);
    section.append(&status);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    section.append(&content);
    let key = (
        section_product(game.product_id, game.parent_id, scope),
        scope,
    );
    let id = game.product_id;
    let parent = game.parent_id;
    let generation = model.borrow().detail_generation;
    let epoch = model.borrow().account_epoch;
    {
        let w = w.clone_refs();
        let model = model.clone();
        retry.connect_clicked(move |_| request_product_section(&w, &model, key.0, key.1, true));
    }
    if start {
        request_product_section(w, model, key.0, key.1, false);
        if matches!(
            model.borrow().section_states.get(&key),
            Some(SectionState::Loading)
        ) {
            spinner.start();
            message.set_label(&loading_label);
        }
    }
    let model = model.clone();
    let request_widgets = w.clone_refs();
    let weak_section = section.downgrade();
    let mut previous = None;
    let mut rendered = false;
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(section) = weak_section.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if model.borrow().account_epoch != epoch {
            content.set_sensitive(false);
            spinner.stop();
            spinner.set_visible(false);
            retry.set_visible(false);
            status.set_visible(true);
            message.set_label("Account changed. Reopen this game to refresh this section.");
            return glib::ControlFlow::Break;
        }
        if model.borrow().detail_generation != generation {
            return glib::ControlFlow::Break;
        }
        if !section.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        let mut state = model.borrow().section_states.get(&key).cloned();
        if state.is_none() {
            request_product_section(&request_widgets, &model, key.0, key.1, false);
            state = model.borrow().section_states.get(&key).cloned();
        }
        let current = current_detail(&model.borrow(), id, parent);
        if state != previous {
            match &state {
                Some(SectionState::Loading) => {
                    spinner.start();
                    spinner.set_visible(true);
                    message.set_label(&loading_label);
                    retry.set_visible(false);
                    status.set_visible(true);
                }
                Some(SectionState::Ready) => {
                    spinner.stop();
                    status.set_visible(false);
                    if let Some(game) = &current {
                        while let Some(child) = content.first_child() {
                            content.remove(&child);
                        }
                        content.append(&build(game));
                        rendered = true;
                    }
                }
                Some(SectionState::Failed(error)) => {
                    spinner.stop();
                    spinner.set_visible(false);
                    message.set_label(error);
                    retry.set_visible(true);
                    status.set_visible(true);
                    if scope == online::DetailSection::Artwork
                        && let Some(game) = &current
                    {
                        while let Some(child) = content.first_child() {
                            content.remove(&child);
                        }
                        content.append(&build(game));
                        rendered = true;
                    }
                }
                None => {}
            }
            previous = state;
        }
        // Cached fields remain visible if refreshing fails; hidden tabs create no rich widgets.
        if !rendered && let Some(game) = &current {
            content.append(&build(game));
            rendered = true;
        }
        glib::ControlFlow::Continue
    });
    section
}

fn cached_patch_notes(
    model: &Rc<RefCell<AppModel>>,
    product_id: i64,
    changelog: &str,
) -> Rc<Vec<PatchNote>> {
    if let Some(notes) = model.borrow().patch_notes.get(&product_id) {
        return notes.clone();
    }
    let notes = Rc::new(crate::patch_notes::parse(changelog));
    model
        .borrow_mut()
        .patch_notes
        .insert(product_id, notes.clone());
    notes
}

fn build_patch_notes_page(notes: Rc<Vec<PatchNote>>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.set_margin_top(12);
    page.set_margin_bottom(24);
    page.add_css_class("patch-notes-page");
    if notes.is_empty() {
        page.append(
            &adw::StatusPage::builder()
                .title("No Patch Notes")
                .description("GOG has not provided patch notes for this game.")
                .build(),
        );
        return page;
    }

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_valign(gtk::Align::Start);
    list.set_width_request(285);
    list.add_css_class("patch-note-list");
    list.add_css_class("patch-note-sidebar");
    for note in notes.iter() {
        let row = gtk::ListBoxRow::new();
        let header = gtk::Box::new(gtk::Orientation::Vertical, 3);
        header.set_margin_top(10);
        header.set_margin_bottom(10);
        header.set_margin_start(12);
        header.set_margin_end(12);
        let title = gtk::Label::new(Some(&note.title));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.add_css_class("patch-note-title");
        header.append(&title);
        let metadata = patch_note_metadata(note);
        if !metadata.is_empty() {
            let metadata = gtk::Label::new(Some(&metadata));
            metadata.set_xalign(0.0);
            metadata.add_css_class("patch-note-metadata");
            header.append(&metadata);
        }
        row.set_child(Some(&header));
        list.append(&row);
    }

    let detail_title = gtk::Label::new(None);
    detail_title.set_xalign(0.0);
    detail_title.set_wrap(true);
    detail_title.add_css_class("patch-note-detail-title");
    let detail_metadata = gtk::Label::new(None);
    detail_metadata.set_xalign(0.0);
    detail_metadata.add_css_class("patch-note-metadata");
    let detail_header = gtk::Box::new(gtk::Orientation::Vertical, 4);
    detail_header.set_margin_top(16);
    detail_header.set_margin_bottom(14);
    detail_header.set_margin_start(20);
    detail_header.set_margin_end(20);
    detail_header.add_css_class("patch-note-detail-header");
    detail_header.append(&detail_title);
    detail_header.append(&detail_metadata);

    let body = gtk::Label::new(None);
    body.set_xalign(0.0);
    body.set_yalign(0.0);
    body.set_wrap(true);
    body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.set_selectable(true);
    body.set_margin_top(16);
    body.set_margin_bottom(20);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.add_css_class("patch-note-body");
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
    detail.set_valign(gtk::Align::Start);
    detail.add_css_class("patch-note-detail");
    detail.append(&detail_header);
    detail.append(&body);

    list.connect_row_selected({
        let notes = notes.clone();
        let detail_title = detail_title.clone();
        let detail_metadata = detail_metadata.clone();
        let body = body.clone();
        move |_, row| {
            let Some(note) = row.and_then(|row| notes.get(row.index() as usize)) else {
                return;
            };
            detail_title.set_label(&note.title);
            let metadata = patch_note_metadata(note);
            detail_metadata.set_label(&metadata);
            detail_metadata.set_visible(!metadata.is_empty());
            body.set_markup(&note.body_markup);
        }
    });

    let reader = gtk::Paned::new(gtk::Orientation::Horizontal);
    reader.set_position(300);
    reader.set_resize_start_child(false);
    reader.set_shrink_start_child(false);
    reader.set_start_child(Some(&list));
    reader.set_end_child(Some(&detail));
    reader.add_css_class("patch-note-reader");
    page.append(&reader);
    list.select_row(list.row_at_index(0).as_ref());
    page
}

fn patch_note_metadata(note: &PatchNote) -> String {
    [
        note.version
            .as_ref()
            .map(|version| format!("Version {version}")),
        note.date.as_ref().map(|date| format!("Date {date}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  •  ")
}

fn set_primary_button_content(button: &gtk::Button, icon: &str, label: &str) {
    button.set_tooltip_text(Some(label));
    if let Some(content) = button.child()
        && content
            .first_child()
            .and_downcast::<gtk::Image>()
            .is_some_and(|image| image.icon_name().as_deref() == Some(icon))
        && content
            .last_child()
            .and_downcast::<gtk::Label>()
            .is_some_and(|text| text.label() == label)
    {
        return;
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    content.set_halign(gtk::Align::Center);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    button.set_child(Some(&content));
}

fn launch_installed_game(
    widgets: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    installed: crate::domain::InstalledGame,
    title: &str,
) {
    let title = title.to_owned();
    let session = online::account_session();
    let auth_session = auth::session();
    let epoch = model.borrow().account_epoch;
    let receiver = launch_with_components(&widgets.window, installed);
    let launch_generation = model.borrow().detail_generation;
    let widgets = widgets.clone_refs();
    let model = model.clone();
    let window = widgets.window.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if auth::session() != auth_session
            || online::account_session() != session
            || model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
        {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(
                event @ (crate::installation::LaunchEvent::EnablementRequired { .. }
                | crate::installation::LaunchEvent::PreLaunchConflict { .. }
                | crate::installation::LaunchEvent::LaunchWithoutSyncRequired { .. }
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
            Ok(crate::installation::LaunchEvent::PrefixRecoveryRequired {
                message,
                game,
                setup_required,
            }) => {
                offer_prefix_recovery(
                    &window,
                    &model,
                    *game,
                    &title,
                    &message,
                    setup_required,
                    launch_generation,
                );
                glib::ControlFlow::Break
            }
            Ok(crate::installation::LaunchEvent::Failed(error)) => {
                show_status(
                    &widgets,
                    &notifications::failure_message(
                        &format!("{title}: Could not run game"),
                        &error,
                    ),
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

#[derive(Clone)]
struct CloudLaunchStatus {
    panel: gtk::Box,
    heading: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
    failure: Rc<RefCell<Option<String>>>,
}

impl CloudLaunchStatus {
    fn show(&self, phase: crate::installation::CloudSyncPhase) {
        self.failure.borrow_mut().take();
        self.heading.set_label(match phase {
            crate::installation::CloudSyncPhase::BeforeLaunch => "SYNCING CLOUD SAVES",
            crate::installation::CloudSyncPhase::AfterExit => "UPLOADING CLOUD SAVES",
        });
        self.detail.set_label(match phase {
            crate::installation::CloudSyncPhase::BeforeLaunch => {
                "Comparing local and GOG Cloud saves before launch…"
            }
            crate::installation::CloudSyncPhase::AfterExit => {
                "Uploading changed saves after the game exited…"
            }
        });
        self.progress.set_fraction(0.0);
        self.progress.set_visible(true);
        self.panel.set_visible(true);
    }

    fn hide(&self) {
        self.failure.borrow_mut().take();
        self.panel.set_visible(false);
        self.progress.set_visible(false);
    }
}

fn installation_status_panel(
    product_id: i64,
    widgets: &Widgets,
    action_widgets: (
        &gtk::Button,
        &gtk::Box,
        &Rc<RefCell<Option<CloudLaunchStatus>>>,
    ),
    launch_pending: &Rc<std::cell::Cell<bool>>,
    initially_installing: bool,
    model: &Rc<RefCell<AppModel>>,
    refresh_after_install: Rc<dyn Fn()>,
) -> gtk::Box {
    let window = &widgets.window;
    let (primary_action, action_group, cloud_launch_status) = action_widgets;
    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    panel.add_css_class("hero-install-status");
    panel.set_visible(false);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let heading = gtk::Label::new(None);
    heading.set_xalign(0.0);
    heading.set_single_line_mode(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    heading.add_css_class("hero-transfer-heading");
    text.append(&heading);
    let detail = gtk::Label::new(None);
    detail.set_xalign(0.0);
    detail.set_single_line_mode(true);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_max_width_chars(42);
    detail.add_css_class("hero-transfer-detail");
    detail.connect_label_notify(|label| label.set_tooltip_text(Some(&label.label())));
    text.append(&detail);
    let progress = gtk::ProgressBar::new();
    progress.set_pulse_step(0.08);
    progress.add_css_class("hero-transfer-progress");
    text.append(&progress);
    let component_progress = gtk::ProgressBar::builder().visible(false).build();
    component_progress.add_css_class("hero-transfer-progress");
    text.append(&component_progress);
    panel.append(&text);
    let view_error = gtk::Button::with_label("View error");
    view_error.set_widget_name("installation-error-details");
    view_error.set_visible(false);
    view_error.set_valign(gtk::Align::Center);
    panel.append(&view_error);
    let failure = Rc::new(RefCell::new(None::<String>));
    let account_epoch = model.borrow().account_epoch;
    view_error.connect_clicked({
        let window = window.clone();
        let notifications = widgets.notifications.clone();
        let heading = heading.clone();
        let detail = detail.clone();
        let model = model.clone();
        move |_| {
            let title = {
                let state = model.borrow();
                if state.account_epoch != account_epoch || state.logout_pending {
                    return;
                }
                state
                    .games
                    .iter()
                    .find(|game| game.product_id == product_id)
                    .map(|game| game.title.clone())
                    .unwrap_or_else(|| "Game".into())
            };
            notifications.show_message(
                &window,
                &notifications::failure_message(
                    &format!("{title}: {}", heading.label()),
                    &detail.label(),
                ),
            );
        }
    });
    *cloud_launch_status.borrow_mut() = Some(CloudLaunchStatus {
        panel: panel.clone(),
        heading: heading.clone(),
        detail: detail.clone(),
        progress: progress.clone(),
        failure: failure.clone(),
    });
    let cancel = gtk::Button::from_icon_name("process-stop-symbolic");
    cancel.add_css_class("flat");
    cancel.add_css_class("destructive-action");
    cancel.set_tooltip_text(Some("Pause installation"));
    cancel.set_valign(gtk::Align::Center);
    cancel.set_visible(false);
    {
        let window = window.clone();
        let cancel_for_dialog = cancel.clone();
        cancel.connect_clicked(move |_| {
            let depot = crate::installation::depot_operation_snapshot_for_product(product_id);
            let abandoned = depot.as_ref().is_some_and(|snapshot| {
                matches!(snapshot.state.as_str(), "interrupted" | "failed")
            });
            let pausing = depot.is_some() && !abandoned;
            let snapshot = crate::installation::installation_operation_snapshot(product_id);
            let queued = snapshot.as_ref().is_some_and(|snapshot| snapshot.queued);
            let dialog = adw::AlertDialog::builder()
                .heading(if abandoned {
                    "Cancel this installation?"
                } else if queued {
                    "Remove queued operation?"
                } else {
                    "Pause download?"
                })
                .body(if abandoned {
                    "This abandons the current attempt and deletes its resumable temporary files. Files already published into the game directory are not rolled back."
                } else if queued {
                    "This removes the operation from the installation queue."
                } else {
                    "You can resume this download later."
                })
                .build();
            dialog.add_responses(&[
                (
                    "keep",
                    if pausing {
                        "Continue Downloading"
                    } else if queued {
                        "Keep Queued"
                    } else {
                        "Keep Installation"
                    },
                ),
                (
                    "cancel",
                    if abandoned {
                        "Cancel Installation"
                    } else if pausing {
                        "Pause Download"
                    } else {
                        "Cancel Operation"
                    },
                ),
            ]);
            dialog.set_default_response(Some("keep"));
            dialog.set_close_response("keep");
            if !pausing {
                dialog.set_response_appearance("cancel", adw::ResponseAppearance::Destructive);
            }
            let cancel = cancel_for_dialog.clone();
            dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response == "cancel" {
                    let cancelled = depot.as_ref().is_some_and(|snapshot| {
                        if abandoned {
                            crate::installation::abandon_depot_operation(&snapshot.operation_id)
                        } else {
                            crate::installation::cancel_depot_operation(&snapshot.operation_id)
                        }
                    }) || crate::installation::cancel_operation(product_id);
                    if cancelled {
                        cancel.set_sensitive(false);
                    }
                }
            });
        });
    }
    panel.append(&cancel);
    let cloud_decline = gtk::Button::with_label("Not now");
    cloud_decline.set_visible(false);
    panel.append(&cloud_decline);
    let cloud_enable = gtk::Button::with_label("Enable cloud saves");
    cloud_enable.add_css_class("suggested-action");
    cloud_enable.set_visible(false);
    panel.append(&cloud_enable);
    let determinate = Rc::new(std::cell::Cell::new(false));
    for (button, preference) in [
        (&cloud_decline, crate::domain::CloudSavePreference::Disabled),
        (&cloud_enable, crate::domain::CloudSavePreference::Enabled),
    ] {
        let panel = panel.clone();
        let heading = heading.clone();
        let detail = detail.clone();
        let progress = progress.clone();
        let determinate = determinate.clone();
        let decline = cloud_decline.clone();
        let enable = cloud_enable.clone();
        let model = model.clone();
        let refresh = refresh_after_install.clone();
        button.connect_clicked(move |_| {
            decline.set_sensitive(false);
            enable.set_sensitive(false);
            heading.set_label("SAVING CLOUD-SAVE PREFERENCE");
            detail.set_label("Saving your choice…");
            determinate.set(false);
            progress.set_visible(true);
            let session = online::account_session();
            let generation = model.borrow().detail_generation;
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = online::with_account_session(session, || {
                    StateStore::open()?.set_cloud_save_preference(product_id, preference)
                });
                let _ = sender.send(result);
            });
            let panel = panel.clone();
            let heading = heading.clone();
            let detail = detail.clone();
            let progress = progress.clone();
            let decline = decline.clone();
            let enable = enable.clone();
            let model = model.clone();
            let refresh = refresh.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if panel.root().is_none()
                    || online::account_session() != session
                    || model.borrow().account_epoch != account_epoch
                    || model.borrow().logout_pending
                    || model.borrow().detail_generation != generation
                {
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!(
                        "Saving stopped unexpectedly. Choose an option to try again."
                    )),
                };
                progress.set_visible(false);
                decline.set_sensitive(true);
                enable.set_sensitive(true);
                match result {
                    Ok(()) => {
                        decline.set_visible(false);
                        enable.set_visible(false);
                        panel.set_visible(false);
                        refresh();
                    }
                    Err(error) => {
                        heading.set_label("COULD NOT SAVE CLOUD-SAVE PREFERENCE");
                        detail.set_label(
                            notifications::failure_message("", &error.to_string()).trim_start(),
                        );
                    }
                }
                glib::ControlFlow::Break
            });
        });
    }
    let receiver = crate::installation::subscribe_installation_events();
    let depot_receiver = crate::installation::subscribe_depot_events();
    let initial_snapshot = crate::installation::installation_operation_snapshot(product_id);
    let panel_for_poll = panel.clone();
    let primary_action = primary_action.clone();
    let action_group = action_group.clone();
    let was_installing = Rc::new(std::cell::Cell::new(initially_installing));
    let was_uninstalling = Rc::new(std::cell::Cell::new(false));
    let pending_snapshot = Rc::new(RefCell::new(initial_snapshot));
    let model = model.clone();
    let was_downloading = Rc::new(std::cell::Cell::new(false));
    let was_game_running = Rc::new(std::cell::Cell::new(false));
    let action_visual = Rc::new(std::cell::Cell::new(0_u8));
    let checking_cloud = Rc::new(std::cell::Cell::new(false));
    let launch_pending = launch_pending.clone();
    let detail_generation = model.borrow().detail_generation;
    let depot_rate = Rc::new(RefCell::new(SmoothedTransferRate::default()));
    let mut depot_operation_id = None::<String>;
    let mut download_completed = false;
    let mut active_download_ids = HashSet::new();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if panel_for_poll.root().is_none() || model.borrow().detail_generation != detail_generation
        {
            return glib::ControlFlow::Break;
        }
        if model.borrow().logout_pending {
            panel_for_poll.set_visible(false);
            return glib::ControlFlow::Continue;
        }
        {
            let state = model.borrow();
            if let Some(game) = state.installed_games.get(&product_id)
                && let Some(
                    crate::storage::LibraryCompatibility::Incompatible(reason)
                    | crate::storage::LibraryCompatibility::Unavailable(reason),
                ) = crate::storage::path_status(
                    &state.library_statuses,
                    &game.installation_directory,
                )
            {
                panel_for_poll.set_visible(true);
                heading.set_label("LIBRARY UNAVAILABLE");
                detail.set_label(reason);
                detail.set_tooltip_text(Some(reason));
                primary_action.set_sensitive(false);
                set_alternate_game_actions_sensitive(&action_group, false);
                return glib::ControlFlow::Continue;
            }
        }
        let normal_action = {
            let state = model.borrow();
            let parent = state
                .detail_target
                .filter(|(id, _)| *id == product_id)
                .and_then(|(_, parent)| parent);
            current_primary_action(&state, product_id, parent)
        };
        if model.borrow().account_epoch != account_epoch {
            panel_for_poll.set_visible(false);
            if !launch_pending.get() {
                if crate::installation::is_game_running(product_id) {
                    let stopping = crate::installation::is_game_stopping(product_id);
                    set_primary_button_content(
                        &primary_action,
                        "process-stop-symbolic",
                        if stopping { "Stopping" } else { "Stop" },
                    );
                    primary_action.set_sensitive(!stopping);
                    primary_action.add_css_class("operational-action");
                    action_group.add_css_class("operational-state");
                } else {
                    set_idle_primary_action(&primary_action, &action_group, normal_action);
                }
            }
            return glib::ControlFlow::Continue;
        }
        if launch_pending.get() {
            return glib::ControlFlow::Continue;
        }
        let depot_snapshot = crate::installation::depot_operation_snapshot_for_product(product_id);
        let archive_active = model.borrow().download_jobs.iter().any(|job| {
            job.product_id == product_id
                && matches!(
                    job.state,
                    DownloadState::Queued | DownloadState::Downloading
                )
        });
        if archive_active
            || depot_snapshot.as_ref().is_some_and(|snapshot| {
                !matches!(
                    snapshot.state.as_str(),
                    "complete" | "failed" | "cancelled" | "abandoned" | "interrupted" | "paused"
                )
            })
        {
            failure.borrow_mut().take();
        }
        component_progress.set_visible(false);
        if progress.is_visible() && !determinate.get() {
            progress.pulse();
        }
        while let Ok(event) = receiver.try_recv() {
            let event_product_id = match event {
                crate::installation::InstallationManagerEvent::OperationQueued(snapshot)
                | crate::installation::InstallationManagerEvent::OperationRecovered(snapshot) => {
                    snapshot.product_id
                }
                crate::installation::InstallationManagerEvent::OperationCancelled(snapshot) => {
                    snapshot.product_id
                }
                crate::installation::InstallationManagerEvent::Installation {
                    product_id, ..
                }
                | crate::installation::InstallationManagerEvent::Uninstallation {
                    product_id,
                    ..
                } => product_id,
            };
            if event_product_id == product_id {
                failure.borrow_mut().take();
                download_completed = false;
                depot_operation_id = None;
                *pending_snapshot.borrow_mut() =
                    crate::installation::installation_operation_snapshot(product_id);
            }
        }
        while let Ok(crate::installation::DepotManagerEvent::Snapshot(snapshot)) =
            depot_receiver.try_recv()
        {
            if snapshot.product_id == product_id {
                failure.borrow_mut().take();
                depot_operation_id = Some(snapshot.operation_id.clone());
            }
            if snapshot.product_id == product_id
                && matches!(
                    snapshot.state.as_str(),
                    "complete" | "cancelled" | "abandoned"
                )
            {
                view_error.set_visible(false);
                refresh_after_install();
            }
        }
        if let Some(error) = failure.borrow().as_ref() {
            set_idle_primary_action(&primary_action, &action_group, normal_action);
            heading.set_label("LAUNCH FAILED");
            detail.set_label(error);
            panel_for_poll.set_visible(true);
            progress.set_visible(false);
            view_error.set_visible(true);
            return glib::ControlFlow::Continue;
        }
        if let Some(snapshot) = depot_snapshot.as_ref()
            && matches!(snapshot.state.as_str(), "interrupted" | "failed")
        {
            panel_for_poll.set_visible(true);
            heading.set_label(if snapshot.state == "failed" {
                "INSTALLATION FAILED"
            } else {
                "INSTALLATION PAUSED"
            });
            detail.set_label(
                notifications::failure_message(
                    "",
                    snapshot
                        .error
                        .as_deref()
                        .unwrap_or("Resume this installation or cancel it to start over"),
                )
                .trim_start(),
            );
            view_error.set_visible(snapshot.error.is_some());
            progress.set_visible(snapshot.download_total_bytes.is_some());
            if let Some(total) = snapshot.download_total_bytes {
                progress.set_fraction(if total == 0 {
                    1.0
                } else {
                    (snapshot.bytes_downloaded as f64 / total as f64).min(1.0)
                });
            }
            set_primary_button_content(&primary_action, "media-playback-start-symbolic", "Resume");
            primary_action.set_sensitive(true);
            set_alternate_game_actions_sensitive(&action_group, false);
            cancel.set_tooltip_text(Some("Cancel installation"));
            cancel.set_sensitive(true);
            cancel.set_visible(true);
            return glib::ControlFlow::Continue;
        }
        if let Some(snapshot) = depot_snapshot.as_ref()
            && !matches!(
                snapshot.state.as_str(),
                "complete" | "failed" | "cancelled" | "abandoned"
            )
        {
            depot_operation_id = Some(snapshot.operation_id.clone());
            view_error.set_visible(false);
            panel_for_poll.set_visible(true);
            primary_action.set_sensitive(false);
            set_alternate_game_actions_sensitive(&action_group, false);
            let display_state = match snapshot.state.as_str() {
                "queued" => "INSTALL QUEUED".to_owned(),
                "preparing" => "PREPARING DOWNLOAD".to_owned(),
                "verifying" => "VERIFYING FILES".to_owned(),
                "verifying_existing" => "CHECKING EXISTING FILES".to_owned(),
                "calculating" => "CALCULATING DOWNLOAD SIZE".to_owned(),
                "downloading" => "STARTING DOWNLOAD".to_owned(),
                "materializing" => "DOWNLOADING".to_owned(),
                "extracting" => "EXTRACTING GAME FILES".to_owned(),
                "committing" => "INSTALLING".to_owned(),
                "finalizing" => "FINALIZING".to_owned(),
                "dependencies" => "DOWNLOADING REQUIRED COMPONENTS".to_owned(),
                "setup" => "SETTING UP REQUIRED COMPONENTS".to_owned(),
                _ => snapshot.state.replace('_', " ").to_uppercase(),
            };
            heading.set_label(&display_state);
            if snapshot.state == "setup" {
                determinate.set(false);
                depot_rate.borrow_mut().reset();
                if let Some(setup) = &snapshot.setup {
                    if setup.total > 0 {
                        detail.set_label(&format!(
                            "{} · {} of {} components completed",
                            setup.component, setup.completed, setup.total
                        ));
                        component_progress
                            .set_fraction((setup.completed as f64 / setup.total as f64).min(1.0));
                        component_progress.set_visible(true);
                    } else {
                        heading.set_label("SETTING UP GAME");
                        detail.set_label(&setup.component);
                    }
                } else {
                    detail.set_label("Applying required game setup");
                }
            } else if matches!(
                snapshot.state.as_str(),
                "dependencies" | "extracting" | "verifying" | "verifying_existing"
            ) {
                depot_rate.borrow_mut().reset();
                if snapshot.total_bytes > 0 {
                    progress.set_fraction(
                        (snapshot.bytes_completed as f64 / snapshot.total_bytes as f64).min(1.0),
                    );
                    determinate.set(true);
                    detail.set_label(&format!(
                        "{} / {}",
                        human_size(snapshot.bytes_completed),
                        human_size(snapshot.total_bytes)
                    ));
                } else {
                    determinate.set(false);
                    detail.set_label(match snapshot.state.as_str() {
                        "extracting" => "Extracting downloaded game files",
                        "verifying" | "verifying_existing" => "Checking game files",
                        _ => "Preparing required components",
                    });
                }
            } else if matches!(snapshot.state.as_str(), "committing" | "finalizing") {
                determinate.set(false);
                depot_rate.borrow_mut().reset();
                detail.set_label(if snapshot.state == "committing" {
                    "Installing game files"
                } else {
                    "Finishing game installation"
                });
            } else if let Some(total) = snapshot.download_total_bytes {
                let fraction = if total == 0 {
                    1.0
                } else {
                    (snapshot.bytes_downloaded as f64 / total as f64).min(1.0)
                };
                progress.set_fraction(fraction);
                determinate.set(true);
                let mut text = format!(
                    "{} / {}",
                    human_size(snapshot.bytes_downloaded),
                    human_size(total)
                );
                if snapshot.state == "materializing" {
                    if let Some(speed) = depot_rate
                        .borrow_mut()
                        .sample(std::time::Instant::now(), snapshot.bytes_downloaded)
                        .filter(|speed| *speed > 0.0)
                    {
                        text.push_str(&format!(" · {}", format_transfer_rate(speed)));
                    }
                } else {
                    depot_rate.borrow_mut().reset();
                }
                detail.set_label(&text);
            } else {
                determinate.set(false);
                detail.set_label(match snapshot.state.as_str() {
                    "queued" => "Waiting to start",
                    "preparing" => "Reading game download information",
                    "calculating" => "Checking which chunks require downloading",
                    "downloading" => "Preparing secure download",
                    _ => "Preparing Galaxy installation",
                });
            }
            progress.set_visible(true);
            cancel.set_sensitive(true);
            cancel.set_tooltip_text(Some("Pause installation"));
            cancel.set_visible(true);
            return glib::ControlFlow::Continue;
        }
        let installation_snapshot = pending_snapshot
            .borrow_mut()
            .take()
            .or_else(|| crate::installation::installation_operation_snapshot(product_id));
        if installation_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.queued
                || matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Installing
                        | crate::domain::InstallationState::Uninstalling
                )
        }) {
            download_completed = false;
            depot_operation_id = None;
        }
        if installation_snapshot.is_none() || archive_active || was_downloading.get() {
            set_alternate_game_actions_sensitive(&action_group, true);
            let jobs = model
                .borrow()
                .download_jobs
                .iter()
                .filter(|job| job.product_id == product_id)
                .cloned()
                .collect::<Vec<_>>();
            let active = jobs
                .iter()
                .filter(|job| {
                    matches!(
                        job.state,
                        DownloadState::Queued | DownloadState::Downloading
                    )
                })
                .collect::<Vec<_>>();
            if !active.is_empty() {
                download_completed = false;
                depot_operation_id = None;
                active_download_ids.extend(active.iter().map(|job| job.job_id.clone()));
                view_error.set_visible(false);
                if action_visual.replace(1) != 1 {
                    set_primary_button_content(
                        &primary_action,
                        "media-playback-pause-symbolic",
                        "Pause",
                    );
                    action_group.add_css_class("operational-state");
                }
                cancel.set_visible(false);
                was_downloading.set(true);
                panel_for_poll.set_visible(true);
                let finalizing = active
                    .iter()
                    .any(|job| job.status_message.as_deref() == Some("Finalizing…"));
                let downloading = active
                    .iter()
                    .filter(|job| job.state == DownloadState::Downloading)
                    .count();
                let queued = active
                    .iter()
                    .filter(|job| job.state == DownloadState::Queued)
                    .count();
                if finalizing {
                    heading.set_label("FINALIZING");
                    detail.set_label("Preparing downloaded files");
                    determinate.set(false);
                } else if downloading > 0 {
                    heading.set_label("DOWNLOADING");
                    let downloaded = active.iter().map(|job| job.bytes_downloaded).sum::<u64>();
                    let total = active
                        .iter()
                        .map(|job| job.total_bytes)
                        .collect::<Option<Vec<_>>>()
                        .map(|sizes| sizes.into_iter().sum::<u64>());
                    if let Some(total) = total.filter(|total| *total > 0) {
                        progress.set_fraction(downloaded as f64 / total as f64);
                        detail.set_label(&format!(
                            "{} / {}{}",
                            human_size(downloaded),
                            human_size(total),
                            if queued > 0 {
                                format!(" · {queued} queued")
                            } else {
                                String::new()
                            }
                        ));
                        determinate.set(true);
                    } else {
                        detail.set_label("Downloading game content");
                        determinate.set(false);
                    }
                } else {
                    heading.set_label("QUEUED");
                    detail.set_label(&format!(
                        "{} download{} waiting",
                        queued,
                        if queued == 1 { "" } else { "s" }
                    ));
                    determinate.set(false);
                }
                progress.set_visible(true);
                primary_action.set_sensitive(true);
                return glib::ControlFlow::Continue;
            }
            if was_downloading.replace(false) {
                download_completed = !active_download_ids.is_empty()
                    && active_download_ids.iter().all(|id| {
                        jobs.iter()
                            .any(|job| &job.job_id == id && job.state == DownloadState::Complete)
                    });
                active_download_ids.clear();
                view_error.set_visible(false);
                refresh_after_install();
            }
            if let Some(failed) = jobs.iter().find(|job| job.state == DownloadState::Failed) {
                action_visual.set(0);
                action_group.remove_css_class("operational-state");
                set_primary_button_content(
                    &primary_action,
                    normal_action.icon(),
                    normal_action.label(),
                );
                panel_for_poll.set_visible(true);
                heading.set_label("DOWNLOAD FAILED");
                let message = failed
                    .error
                    .as_deref()
                    .or(failed.status_message.as_deref())
                    .unwrap_or("The download could not be completed");
                let message = notifications::failure_message("", message);
                detail.set_label(message.trim_start());
                view_error.set_visible(true);
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                return glib::ControlFlow::Continue;
            }
            if jobs.iter().any(|job| job.state == DownloadState::Paused) {
                view_error.set_visible(false);
                action_visual.set(0);
                action_group.remove_css_class("operational-state");
                set_primary_button_content(
                    &primary_action,
                    normal_action.icon(),
                    normal_action.label(),
                );
                panel_for_poll.set_visible(true);
                heading.set_label("DOWNLOAD PAUSED");
                detail.set_label("Resume this download from the Downloads page");
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                return glib::ControlFlow::Continue;
            }
        }
        if crate::installation::is_game_running(product_id) {
            view_error.set_visible(false);
            was_game_running.set(true);
            let stopping = crate::installation::is_game_stopping(product_id);
            let visual = if stopping { 4 } else { 3 };
            if action_visual.replace(visual) != visual {
                set_primary_button_content(
                    &primary_action,
                    if stopping {
                        "process-stop-symbolic"
                    } else {
                        "media-playback-stop-symbolic"
                    },
                    if stopping { "Stopping" } else { "Stop" },
                );
                primary_action.add_css_class("operational-action");
                action_group.add_css_class("operational-state");
            }
            primary_action.set_sensitive(!stopping);
            return glib::ControlFlow::Continue;
        }
        if was_game_running.replace(false) {
            action_visual.set(0);
            primary_action.remove_css_class("operational-action");
            action_group.remove_css_class("operational-state");
            set_primary_button_content(
                &primary_action,
                normal_action.icon(),
                normal_action.label(),
            );
        }
        if cloud_enable.is_visible() || checking_cloud.get() {
            view_error.set_visible(false);
            set_idle_primary_action(&primary_action, &action_group, normal_action);
            return glib::ControlFlow::Continue;
        }
        if let Some(snapshot) = depot_snapshot.as_ref().filter(|snapshot| {
            depot_operation_id.as_ref() == Some(&snapshot.operation_id)
                && matches!(
                    snapshot.state.as_str(),
                    "complete" | "cancelled" | "abandoned"
                )
        }) {
            set_idle_primary_action(&primary_action, &action_group, normal_action);
            panel_for_poll.set_visible(true);
            cancel.set_visible(false);
            view_error.set_visible(false);
            heading.set_label(if snapshot.state == "complete" {
                "INSTALLATION COMPLETE"
            } else {
                "INSTALLATION STOPPED"
            });
            detail.set_label(if snapshot.state == "complete" {
                "Game files and required setup are ready"
            } else {
                "The operation was stopped; review the game before retrying"
            });
            progress.set_visible(snapshot.state == "complete");
            progress.set_fraction(1.0);
            determinate.set(true);
            return glib::ControlFlow::Continue;
        }
        if download_completed
            && installation_snapshot.as_ref().is_none_or(|snapshot| {
                !snapshot.queued && snapshot.state != crate::domain::InstallationState::Installing
            })
        {
            set_idle_primary_action(&primary_action, &action_group, normal_action);
            panel_for_poll.set_visible(true);
            cancel.set_visible(false);
            view_error.set_visible(false);
            heading.set_label("DOWNLOAD COMPLETE");
            detail.set_label("Downloaded files are ready. Installer updates still need review from the arrow beside Play.");
            progress.set_visible(true);
            progress.set_fraction(1.0);
            determinate.set(true);
            return glib::ControlFlow::Continue;
        }
        view_error.set_visible(installation_snapshot.as_ref().is_some_and(|snapshot| {
            !snapshot.queued
                && matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Failed
                        | crate::domain::InstallationState::UninstallFailed
                )
        }));
        if installation_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.state == crate::domain::InstallationState::Pending && !snapshot.queued
        }) && was_uninstalling.replace(false)
        {
            refresh_after_install();
        }
        match installation_snapshot {
            Some(crate::installation::InstallationOperationSnapshot {
                queued: true,
                message,
                ..
            }) => {
                action_visual.set(2);
                set_primary_button_content(&primary_action, "content-loading-symbolic", "Queued");
                action_group.add_css_class("operational-state");
                panel_for_poll.set_visible(true);
                heading.set_label("QUEUED");
                detail.set_label(
                    message
                        .as_deref()
                        .unwrap_or("Waiting for another operation"),
                );
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(false);
                set_alternate_game_actions_sensitive(&action_group, false);
                cancel.set_sensitive(true);
                cancel.set_visible(true);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Installing,
                message,
                percentage,
                queued: false,
                ..
            }) => {
                action_visual.set(2);
                set_primary_button_content(
                    &primary_action,
                    "content-loading-symbolic",
                    "Installing",
                );
                action_group.add_css_class("operational-state");
                was_installing.set(true);
                panel_for_poll.set_visible(true);
                heading.set_label("INSTALLING");
                if let Some(percentage) = percentage {
                    detail.set_label(&format!("{percentage}% Complete"));
                    progress.set_fraction(f64::from(percentage) / 100.0);
                    determinate.set(true);
                } else {
                    detail.set_label(message.as_deref().unwrap_or("Running native installer"));
                    determinate.set(false);
                }
                progress.set_visible(true);
                primary_action.set_sensitive(false);
                set_alternate_game_actions_sensitive(&action_group, false);
                cancel.set_sensitive(true);
                cancel.set_visible(true);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Uninstalling,
                message,
                percentage,
                queued: false,
                ..
            }) => {
                action_visual.set(2);
                set_primary_button_content(
                    &primary_action,
                    "content-loading-symbolic",
                    "Uninstalling",
                );
                action_group.add_css_class("operational-state");
                was_uninstalling.set(true);
                panel_for_poll.set_visible(true);
                heading.set_label("UNINSTALLING");
                if let Some(percentage) = percentage {
                    detail.set_label(&format!("{percentage}% Complete"));
                    progress.set_fraction(f64::from(percentage) / 100.0);
                    determinate.set(true);
                } else {
                    detail.set_label(message.as_deref().unwrap_or("Running native uninstaller"));
                    determinate.set(false);
                }
                progress.set_visible(true);
                primary_action.set_sensitive(false);
                set_alternate_game_actions_sensitive(&action_group, false);
                cancel.set_sensitive(true);
                cancel.set_visible(true);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Failed,
                message: error,
                ..
            }) => {
                was_installing.set(false);
                action_visual.set(0);
                action_group.remove_css_class("operational-state");
                set_primary_button_content(
                    &primary_action,
                    normal_action.icon(),
                    normal_action.label(),
                );
                panel_for_poll.set_visible(true);
                heading.set_label("INSTALLATION FAILED");
                detail.set_label(
                    notifications::failure_message(
                        "",
                        error
                            .as_deref()
                            .unwrap_or("The installer exited with an error"),
                    )
                    .trim_start(),
                );
                view_error.set_visible(true);
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                cancel.set_visible(false);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::UninstallFailed,
                message: error,
                ..
            }) => {
                was_uninstalling.set(false);
                action_visual.set(0);
                action_group.remove_css_class("operational-state");
                set_primary_button_content(
                    &primary_action,
                    normal_action.icon(),
                    normal_action.label(),
                );
                panel_for_poll.set_visible(true);
                heading.set_label("UNINSTALL FAILED");
                detail.set_label(
                    notifications::failure_message(
                        "",
                        error
                            .as_deref()
                            .unwrap_or("The uninstaller exited with an error"),
                    )
                    .trim_start(),
                );
                view_error.set_visible(true);
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                cancel.set_visible(false);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Installed,
                ..
            }) if was_installing.replace(false) => {
                checking_cloud.set(true);
                let libraries = model.borrow().config.game_libraries.clone();
                panel_for_poll.set_visible(true);
                heading.set_label("CHECKING CLOUD SAVES");
                detail.set_label("Checking GOG cloud-save support…");
                progress.set_visible(true);
                cancel.set_visible(false);
                cloud_decline.set_visible(false);
                cloud_enable.set_visible(false);
                primary_action.set_sensitive(true);
                refresh_after_install();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = (|| {
                        let store = StateStore::open()?;
                        let record = store.cloud_save_record(product_id)?;
                        let game =
                            crate::installation::reconcile_installed_games(&store, &libraries)?
                                .into_iter()
                                .find(|game| game.product_id == product_id)
                                .ok_or_else(|| anyhow::anyhow!("installed game is unavailable"))?;
                        let discovery =
                            crate::cloud_saves::discover_and_store(&game, &record.locations)?;
                        anyhow::Ok((discovery, record.preference))
                    })()
                    .map_err(|error| format!("{error:#}"));
                    sender.send(result).ok();
                });
                let panel = panel_for_poll.clone();
                let heading = heading.clone();
                let detail = detail.clone();
                let progress = progress.clone();
                let decline = cloud_decline.clone();
                let enable = cloud_enable.clone();
                let primary = primary_action.clone();
                let refresh = refresh_after_install.clone();
                let checking_cloud = checking_cloud.clone();
                let model = model.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if model.borrow().account_epoch != account_epoch
                        || model.borrow().detail_generation != detail_generation
                        || panel.root().is_none()
                    {
                        checking_cloud.set(false);
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(Ok((discovery, preference)))
                            if discovery.availability
                                == crate::domain::CloudSaveAvailability::Supported
                                && preference == crate::domain::CloudSavePreference::Undecided =>
                        {
                            checking_cloud.set(false);
                            heading.set_label("CLOUD SAVES AVAILABLE");
                            detail.set_label(
                                "Synchronize this game's saves with GOG before launch and after exit?",
                            );
                            progress.set_visible(false);
                            decline.set_visible(true);
                            enable.set_visible(true);
                            primary.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                        Ok(Ok(_)) | Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                            checking_cloud.set(false);
                            panel.set_visible(false);
                            refresh();
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    }
                });
                return glib::ControlFlow::Continue;
            }
            Some(_) | None => {
                action_visual.set(0);
                set_idle_primary_action(&primary_action, &action_group, normal_action);
                panel_for_poll.set_visible(false);
                cancel.set_visible(false);
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
            }
        }
        glib::ControlFlow::Continue
    });
    panel
}

fn set_idle_primary_action(button: &gtk::Button, actions: &gtk::Box, action: GamePrimaryAction) {
    set_primary_button_content(button, action.icon(), action.label());
    button.set_sensitive(true);
    button.remove_css_class("operational-action");
    actions.remove_css_class("operational-state");
    set_alternate_game_actions_sensitive(actions, true);
}

fn set_alternate_game_actions_sensitive(action_group: &gtk::Box, sensitive: bool) {
    let root: gtk::Widget = action_group.clone().upcast();
    if let Some(menu) =
        find_named_descendant(&root, "game-alternate-actions").and_downcast::<gtk::MenuButton>()
    {
        menu.set_sensitive(sensitive);
    }
}

#[cfg(test)]
fn parse_installation_progress(output: &str) -> Option<u8> {
    let marker = "(total progress: ";
    let start = output.rfind(marker)? + marker.len();
    let percentage = output[start..].split('%').next()?.trim().parse().ok()?;
    (percentage <= 100).then_some(percentage)
}

fn format_transfer_rate(bytes_per_second: f64) -> String {
    format!("{}/s", human_size(bytes_per_second.max(0.0) as u64))
}

#[derive(Default)]
struct SmoothedTransferRate {
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
    displayed: Option<(std::time::Instant, f64)>,
}

impl SmoothedTransferRate {
    const WINDOW: Duration = Duration::from_secs(8);
    const MINIMUM: Duration = Duration::from_secs(1);

    fn sample(&mut self, now: std::time::Instant, bytes: u64) -> Option<f64> {
        if self
            .samples
            .back()
            .is_some_and(|(_, previous)| bytes < *previous)
        {
            self.reset();
        }
        self.samples.push_back((now, bytes));
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|(time, _)| now.duration_since(*time) > Self::WINDOW)
        {
            self.samples.pop_front();
        }
        let (first_at, first_bytes) = *self.samples.front()?;
        let elapsed = now.duration_since(first_at);
        if elapsed < Self::MINIMUM {
            return None;
        }
        if self
            .displayed
            .is_none_or(|(updated, _)| now.duration_since(updated) >= Self::MINIMUM)
        {
            self.displayed = Some((
                now,
                bytes.saturating_sub(first_bytes) as f64 / elapsed.as_secs_f64(),
            ));
        }
        self.displayed.map(|(_, rate)| rate)
    }

    fn reset(&mut self) {
        self.samples.clear();
        self.displayed = None;
    }
}

pub(super) fn append_official_metadata(
    container: &gtk::Box,
    metadata: &crate::domain::ProductMetadata,
) {
    let developers = metadata
        .developers
        .iter()
        .map(|company| company.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let publishers = metadata
        .publishers
        .iter()
        .map(|company| company.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if !developers.is_empty()
        || !publishers.is_empty()
        || metadata.series.is_some()
        || !metadata.editions.is_empty()
    {
        let editions = if metadata.editions.is_empty() {
            String::new()
        } else {
            format!(
                "\nEditions: {}",
                metadata
                    .editions
                    .iter()
                    .map(|edition| edition.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let text = format!(
            "Developer: {}\nPublisher: {}{}{}",
            empty_dash(&developers),
            empty_dash(&publishers),
            metadata
                .series
                .as_ref()
                .map(|series| format!("\nSeries: {}", series.name))
                .unwrap_or_default(),
            editions
        );
        container.append(&section("Store information", &text));
    }
    append_term_chips(
        container,
        "Genres and themes",
        metadata.genres.iter().chain(&metadata.themes),
    );
    append_term_chips(
        container,
        "Features and play modes",
        metadata.features.iter().chain(&metadata.game_modes),
    );
    append_term_chips(container, "Store properties", metadata.properties.iter());
    if !metadata.localizations.is_empty() {
        let languages = metadata
            .localizations
            .iter()
            .map(|language| {
                let scope = match (language.text, language.audio) {
                    (true, true) => "text and audio",
                    (false, true) => "audio",
                    _ => "text",
                };
                format!("{} ({scope})", language.name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        container.append(&section("Languages", &languages));
    }
    if !metadata.system_requirements.is_empty() {
        let requirements = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let title = gtk::Label::new(Some("System requirements"));
        title.set_xalign(0.0);
        title.add_css_class("section-title");
        requirements.append(&title);
        for system in &metadata.system_requirements {
            let expander = gtk::Expander::new(Some(&system.operating_system));
            let body = [
                system
                    .minimum
                    .as_ref()
                    .map(|value| format!("Minimum\n{}", text::html_to_text(value))),
                system
                    .recommended
                    .as_ref()
                    .map(|value| format!("Recommended\n{}", text::html_to_text(value))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
            let label = gtk::Label::new(Some(&body));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_selectable(true);
            expander.set_child(Some(&label));
            requirements.append(&expander);
        }
        container.append(&requirements);
    }
}

fn refresh_product_logs(container: &gtk::Box, product_id: i64, window: &adw::ApplicationWindow) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let loading = gtk::Spinner::new();
    loading.set_size_request(16, 16);
    loading.start();
    container.append(&loading);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let logs = [
            (
                "Installer log",
                crate::installation::installation_log_path(product_id).ok(),
            ),
            (
                "Uninstaller log",
                crate::installation::uninstallation_log_path(product_id).ok(),
            ),
        ]
        .into_iter()
        .filter_map(|(title, path)| path.filter(|path| path.is_file()).map(|path| (title, path)))
        .collect::<Vec<_>>();
        let installation_error = crate::installation::installation_operation_snapshot(product_id)
            .and_then(|snapshot| {
                matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Failed
                        | crate::domain::InstallationState::UninstallFailed
                )
                .then_some(snapshot.message)
                .flatten()
            });
        let download_failures = StateStore::open()
            .and_then(|store| store.download_jobs())
            .unwrap_or_default()
            .into_iter()
            .filter(|job| job.product_id == product_id && job.state == DownloadState::Failed)
            .collect::<Vec<_>>();
        let _ = sender.send((logs, installation_error, download_failures));
    });
    let weak = container.downgrade();
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        let Some(container) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        match receiver.try_recv() {
            Ok((logs, error, failures)) => {
                render_product_logs(&container, &window, logs, error, failures);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                loading.stop();
                glib::ControlFlow::Break
            }
        }
    });
}

fn render_product_logs(
    container: &gtk::Box,
    window: &adw::ApplicationWindow,
    logs: Vec<(&str, std::path::PathBuf)>,
    installation_error: Option<String>,
    download_failures: Vec<DownloadJobRecord>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if logs.is_empty() && download_failures.is_empty() {
        let empty = adw::StatusPage::builder()
            .title("No operation logs for this game")
            .description("Installation and download logs will appear here when available.")
            .icon_name("document-open-recent-symbolic")
            .build();
        container.append(&empty);
        return;
    }
    if !download_failures.is_empty() {
        let download_group = adw::PreferencesGroup::new();
        download_group.set_title("Downloads");
        for job in download_failures {
            let row = adw::ExpanderRow::new();
            row.set_title(&job.title);
            let timestamp = chrono::DateTime::from_timestamp(job.updated_at, 0)
                .map(|date| {
                    date.with_timezone(&chrono::Local)
                        .format("Failed %b %-d, %Y at %-I:%M %p")
                        .to_string()
                })
                .unwrap_or_else(|| "Download failed".to_owned());
            row.set_subtitle(&timestamp);
            let discard = gtk::Button::with_label("Discard Partial Download");
            discard.set_valign(gtk::Align::Center);
            discard.add_css_class("destructive-action");
            discard.connect_clicked({
                let job_id = job.job_id.clone();
                let window = window.clone();
                move |_| {
                    let confirmation = adw::AlertDialog::builder()
                        .heading("Discard this download?")
                        .body(
                            "This removes the failed download from the queue and deletes only its partial staging files.",
                        )
                        .build();
                    confirmation
                        .add_responses(&[("cancel", "Cancel"), ("discard", "Discard Download")]);
                    confirmation.set_default_response(Some("cancel"));
                    confirmation.set_close_response("cancel");
                    confirmation.set_response_appearance(
                        "discard",
                        adw::ResponseAppearance::Destructive,
                    );
                    let job_id = job_id.clone();
                    confirmation.choose(
                        Some(&window),
                        gio::Cancellable::NONE,
                        move |response| {
                            if response == "discard" {
                                download::remove(&job_id);
                            }
                        },
                    );
                }
            });
            row.add_suffix(&discard);
            let message = job
                .error
                .as_deref()
                .or(job.status_message.as_deref())
                .unwrap_or("The download could not be completed");
            let error = gtk::Label::new(Some(message));
            error.set_xalign(0.0);
            error.set_wrap(true);
            error.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            error.set_selectable(true);
            error.set_margin_start(12);
            error.set_margin_end(12);
            error.set_margin_top(8);
            error.set_margin_bottom(12);
            error.add_css_class("error");
            row.add_row(&error);
            download_group.add(&row);
        }
        container.append(&download_group);
    }
    let group = adw::PreferencesGroup::new();
    group.set_title("Installation");
    if let Some(error) = installation_error {
        group.set_description(Some(&error));
    }
    for (title, path) in logs {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_subtitle(&path.display().to_string());
        let open = gtk::Button::with_label("Open Log");
        open.set_valign(gtk::Align::Center);
        open.connect_clicked({
            let path = path.clone();
            let window = window.clone();
            move |_| {
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&path)));
                let parent = window.clone();
                launcher.launch(Some(&window), gio::Cancellable::NONE, move |result| {
                    widgets::file_open::report_launch_result(&parent, "installation log", result);
                });
            }
        });
        row.add_suffix(&open);
        group.add(&row);
    }
    if group.first_child().is_some() {
        container.append(&group);
    }
}

fn format_playtime(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} sec")
    } else if seconds < 3_600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{:.1} hours", seconds as f64 / 3_600.0)
    }
}

fn format_last_played(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map_or_else(
            || "Never".to_owned(),
            |date| {
                date.with_timezone(&chrono::Local)
                    .format("%b %-d, %Y")
                    .to_string()
            },
        )
}

fn activity_stat(heading: &str, value: &gtk::Label) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 1);
    content.add_css_class("detail-activity-stat");
    content.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(heading));
    heading.set_xalign(0.0);
    heading.add_css_class("detail-activity-heading");
    value.set_xalign(0.0);
    value.add_css_class("dim-label");
    content.append(&heading);
    content.append(value);
    content
}

pub(super) fn append_term_chips<'a>(
    container: &gtk::Box,
    heading: &str,
    terms: impl Iterator<Item = &'a crate::domain::MetadataTerm>,
) {
    let values = terms.collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::new(Some(heading));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    box_.append(&title);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(false);
    flow.set_column_spacing(8);
    flow.set_row_spacing(8);
    flow.set_max_children_per_line(12);
    for term in values {
        let chip = gtk::Label::new(Some(&term.name));
        chip.add_css_class("tag-chip");
        flow.insert(&chip, -1);
    }
    box_.append(&flow);
    container.append(&box_);
}

pub(super) fn folder_button(
    label: &str,
    path: &std::path::Path,
    window: &adw::ApplicationWindow,
) -> gtk::Button {
    let button = gtk::Button::from_icon_name("folder-open-symbolic");
    button.set_tooltip_text(Some(label));
    button.set_halign(gtk::Align::Start);
    button.add_css_class("square-action");
    button.add_css_class("folder-action");
    button.set_sensitive(!path.as_os_str().is_empty() && path.is_dir());
    let path = path.to_owned();
    let window = window.clone();
    button.connect_clicked(move |_| {
        super::widgets::file_open::open_directory(&path, &window, "folder");
    });
    button
}

pub(super) fn uri_button(label: &str, uri: &str, window: &adw::ApplicationWindow) -> gtk::Button {
    let button = gtk::Button::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Label::new(Some(label)));
    content.append(&gtk::Image::from_icon_name("external-link-symbolic"));
    button.set_child(Some(&content));
    button.add_css_class("flat");
    button.add_css_class("link-action");
    let launcher = gtk::UriLauncher::new(uri);
    let window = window.clone();
    button.connect_clicked(move |_| {
        let parent = window.clone();
        launcher.launch(Some(&window), gio::Cancellable::NONE, move |result| {
            widgets::file_open::report_launch_result(&parent, "link", result);
        });
    });
    button
}

pub(super) fn build_dlc_catalog(
    dlcs: &[Dlc],
    widgets: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    parent_id: i64,
) -> gtk::Box {
    let catalog = gtk::Box::new(gtk::Orientation::Vertical, 0);
    catalog.set_margin_top(12);
    catalog.add_css_class("dlc-catalog");
    for dlc in dlcs.iter().filter(|dlc| dlc.is_catalog_visible()) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.add_css_class("dlc-catalog-row");
        row.set_margin_bottom(1);
        row.set_height_request(130);
        row.set_overflow(gtk::Overflow::Hidden);
        let artwork = card_picture(dlc.artwork.as_ref(), 230, 106);
        artwork.set_halign(gtk::Align::Start);
        artwork.set_valign(gtk::Align::Center);
        artwork.set_size_request(230, 106);
        let artwork_overlay = gtk::Overlay::new();
        artwork_overlay.set_halign(gtk::Align::Start);
        artwork_overlay.set_valign(gtk::Align::Center);
        artwork_overlay.set_size_request(230, 106);
        artwork_overlay.set_child(Some(&artwork));
        let ownership = gtk::Label::new(Some(if dlc.owned { "IN LIBRARY" } else { "NOT OWNED" }));
        ownership.set_halign(gtk::Align::Start);
        ownership.set_valign(gtk::Align::Start);
        ownership.set_margin_top(12);
        ownership.add_css_class("dlc-ownership-badge");
        ownership.add_css_class(if dlc.owned { "in-library" } else { "not-owned" });
        artwork_overlay.add_overlay(&ownership);
        row.append(&artwork_overlay);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 5);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some(&dlc.title.to_uppercase()));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        title.add_css_class("dlc-catalog-title");
        heading.append(&title);
        if let Some(date) = &dlc.release_date {
            let date = gtk::Label::new(Some(&date.format("%b %-d, %Y").to_string()));
            date.add_css_class("dim-label");
            heading.append(&date);
        }
        copy.append(&heading);
        let plain_description = text::html_to_text(&dlc.description)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let summary = gtk::Label::new(Some(&text_excerpt(&plain_description, 220)));
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_lines(2);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.add_css_class("dlc-catalog-summary");
        copy.append(&summary);
        let kind = gtk::Label::new(Some(&format!(
            "{}  ·  {}",
            dlc.kind(),
            human_size(dlc.disk_usage)
        )));
        kind.set_xalign(0.0);
        kind.add_css_class("dim-label");
        copy.append(&kind);
        row.append(&copy);
        let dlc = dlc.clone();
        let widgets = widgets.clone_refs();
        let model = model.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| show_dlc_page(&widgets, &model, parent_id, &dlc));
        row.add_controller(click);
        catalog.append(&row);
    }
    catalog
}

pub(super) fn show_dlc_page(w: &Widgets, model: &Rc<RefCell<AppModel>>, parent_id: i64, dlc: &Dlc) {
    let parent = model
        .borrow()
        .games
        .iter()
        .find(|game| game.product_id == parent_id)
        .cloned();
    if let Some(parent) = parent {
        model.borrow_mut().selected = None;
        render_detail_page(w, model, DetailPageModel::dlc(&parent, dlc.clone()));
    }
}

#[cfg(test)]
mod installation_progress_tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private D-Bus and Xvfb"]
    fn idle_action_restores_controls_after_repeated_operations_without_replacing_the_page() {
        adw::init().expect("GUI regression requires a display");
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let button = gtk::Button::new();
        let alternate = gtk::MenuButton::new();
        alternate.set_widget_name("game-alternate-actions");
        actions.append(&button);
        actions.append(&alternate);
        page.append(&actions);
        for action in [
            GamePrimaryAction::Download,
            GamePrimaryAction::Install,
            GamePrimaryAction::Play,
            GamePrimaryAction::Play,
            GamePrimaryAction::Install,
            GamePrimaryAction::Download,
        ] {
            set_primary_button_content(&button, "content-loading-symbolic", "Installing");
            button.set_sensitive(false);
            alternate.set_sensitive(false);
            actions.add_css_class("operational-state");
            set_idle_primary_action(&button, &actions, action);
            assert!(button.is_sensitive());
            assert_eq!(button.tooltip_text().as_deref(), Some(action.label()));
            assert!(alternate.is_sensitive());
            assert!(!actions.has_css_class("operational-state"));
            assert_eq!(actions.parent().as_ref(), Some(page.upcast_ref()));
            let content = button.child().unwrap();
            assert_eq!(
                content
                    .last_child()
                    .and_downcast::<gtk::Label>()
                    .unwrap()
                    .label(),
                action.label()
            );
            set_idle_primary_action(&button, &actions, action);
            assert_eq!(
                button.child(),
                Some(content),
                "idle poll must not replace the content"
            );
        }
    }

    #[test]
    fn uses_latest_total_progress_and_ignores_file_progress() {
        let output = "file.bin: 90% (total progress: 41%)\r\nnext.bin: 5% (total progress: 42%)";
        assert_eq!(parse_installation_progress(output), Some(42));
        assert_eq!(parse_installation_progress("Uncompressing 77%"), None);
    }

    #[test]
    fn formats_download_rates_for_the_install_status() {
        assert_eq!(format_transfer_rate(999.0), "999 B/s");
        assert_eq!(format_transfer_rate(1000.0), "1.0 kB/s");
        assert_eq!(format_transfer_rate(512.0 * 1024.0), "524.3 kB/s");
        assert_eq!(format_transfer_rate(12_500_000.0), "12.5 MB/s");
        assert_eq!(format_transfer_rate(1_000_000_000.0), "1.0 GB/s");
        assert_eq!(format_transfer_rate(-1.0), "0 B/s");
    }

    #[test]
    fn transfer_rate_uses_an_eight_second_rolling_window() {
        let start = std::time::Instant::now();
        let mut rate = SmoothedTransferRate::default();
        for second in 0..=8 {
            rate.sample(
                start + std::time::Duration::from_secs(second),
                second * 10 * 1024 * 1024,
            );
        }
        let displayed = rate
            .sample(start + std::time::Duration::from_secs(9), 80 * 1024 * 1024)
            .unwrap();
        assert_eq!(displayed as u64, 9_175_040);
        assert_eq!(
            rate.sample(
                start + std::time::Duration::from_millis(9_100),
                90 * 1024 * 1024,
            ),
            Some(displayed)
        );
    }
}
