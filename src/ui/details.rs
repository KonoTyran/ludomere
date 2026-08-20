use super::*;

pub(super) fn show_game(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    force_refresh: Option<bool>,
) {
    if force_refresh.is_none()
        && model.borrow().selected == Some(id)
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
    let hidden = m.hidden_games.contains(&id);
    drop(m);
    render_detail_page(w, model, DetailPageModel::game(game, favorite, hidden));
}

pub(super) fn render_detail_page(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    game: DetailPageModel,
) {
    let favorite = game.favorite.unwrap_or(false);
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
    let game_libraries = model.borrow().config.game_libraries.clone();
    let installed = StateStore::open().ok().and_then(|store| {
        crate::installation::reconcile_installed_games(&store, &game_libraries)
            .ok()?
            .into_iter()
            .find(|installed| installed.product_id == game.product_id)
            .map(|installed| {
                let update = store
                    .installation_update_available(&installed)
                    .unwrap_or(false);
                (installed, update)
            })
    });
    let backup_update = StateStore::open().is_ok_and(|store| {
        store
            .installer_backup_update_available(game.product_id)
            .unwrap_or(false)
    });
    let dlc_action = owned_dlc_action_state(&game, &model.borrow().config, installed.is_some());
    let current_installer_downloaded =
        default_installers_are_downloaded(&game, &model.borrow().config);
    let installed_update = installed.as_ref().is_some_and(|(_, update)| *update);
    let galaxy_depot_installation = installed
        .as_ref()
        .is_some_and(|(installed, _)| is_galaxy_depot_installation(installed));
    let primary_action = primary_action_for_state(
        installed.is_some(),
        installed_update,
        galaxy_depot_installation,
        backup_update,
        current_installer_downloaded,
        dlc_action,
    );
    let activity = installed
        .as_ref()
        .map(|(installed, _)| (installed.last_played_at, installed.playtime_seconds))
        .or_else(|| {
            StateStore::open()
                .ok()?
                .product_activity(game.product_id)
                .ok()
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
        "Install the downloaded update"
    } else if primary_action == GamePrimaryAction::Play {
        "Launch this game"
    } else {
        "Choose installers, DLC, patches, and extras"
    }));
    let primary_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    primary_actions.add_css_class("detail-primary-actions");
    let cloud_launch_status = Rc::new(RefCell::new(None::<CloudLaunchStatus>));
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
        download_button.connect_clicked(move |_| {
            if !detail.owned {
                if let Some(uri) = detail.links.store.as_deref() {
                    let launcher = gtk::UriLauncher::new(uri);
                    launcher.launch(Some(&widgets.window), gio::Cancellable::NONE, |result| {
                        if let Err(error) = result {
                            tracing::warn!(%error, "could not open DLC store page");
                        }
                    });
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
                if crate::installation::resume_depot_operation(snapshot.operation_id, token) {
                    primary_button.set_sensitive(false);
                }
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
                let libraries = model.borrow().config.game_libraries.clone();
                let installed = StateStore::open().ok().and_then(|store| {
                    crate::installation::reconcile_installed_games(&store, &libraries)
                        .ok()?
                        .into_iter()
                        .find(|game| game.product_id == detail.product_id)
                });
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
                let receiver = crate::installation::launch_game(installed);
                let button = primary_button.clone();
                let action_group = action_group.clone();
                button.set_sensitive(false);
                let window = widgets.window.clone();
                let last_played_value = last_played_value.clone();
                let playtime_value = playtime_value.clone();
                let activity_model = activity_model.clone();
                let activity_widgets = activity_widgets.clone();
                let cloud_launch_status = cloud_launch_status.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
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
                        Ok(crate::installation::LaunchEvent::Failed(error)) => {
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
                            let dialog = adw::AlertDialog::builder()
                                .heading("Could not run game")
                                .body(error)
                                .build();
                            dialog.add_response("close", "Close");
                            dialog.present(Some(&window));
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            if let Some(status) = cloud_launch_status.borrow().as_ref() {
                                status.hide();
                            }
                            button.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                    }
                });
                return;
            }
            if model.borrow().account_token.is_none() {
                widgets.reconnect.emit_clicked();
                return;
            }
            show_download_selector(&widgets, &model, &detail);
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
            || (primary_action == GamePrimaryAction::Install
                && has_additional_download_options(&game)))
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
        if matches!(
            primary_action,
            GamePrimaryAction::DownloadUpdate | GamePrimaryAction::InstallUpdate
        ) && let Some((installed_game, _)) = installed.clone()
        {
            let play_current = gtk::Button::with_label("Play");
            play_current.add_css_class("flat");
            play_current.set_halign(gtk::Align::Fill);
            let window = w.window.clone();
            let action_popover = popover.clone();
            play_current.connect_clicked(move |_| {
                action_popover.popdown();
                launch_installed_game(&window, installed_game.clone());
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
            if model.borrow().account_token.is_none() {
                widgets.reconnect.emit_clicked();
                return;
            }
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
        let detail = game.clone();
        Rc::new(move || {
            let libraries = model.borrow().config.game_libraries.clone();
            let installed_products = StateStore::open()
                .ok()
                .and_then(|store| {
                    crate::installation::reconcile_installed_games(&store, &libraries).ok()
                })
                .unwrap_or_default()
                .into_iter()
                .map(|game| game.product_id)
                .collect();
            model.borrow_mut().installed_products = installed_products;
            render_detail_page(&widgets, &model, detail.clone());
            update_sidebar_download_styles(&widgets, &model.borrow());
            refresh_filters(&widgets, &model.borrow());
        })
    };
    action_bar.append(&installation_status_panel(
        game.product_id,
        &w.window,
        (&download_button, &primary_actions, &cloud_launch_status),
        primary_action,
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
    if !game.description.is_empty() {
        let description = text::html_to_text(&game.description);
        overview.append(&expandable_section("Overview", description, 1_600));
    }
    if !game.screenshots.is_empty() {
        overview.append(&screenshot_strip(
            game.product_id,
            &game.screenshots,
            &w.window,
        ));
    }
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
        append_official_metadata(&overview, &game.metadata);
        let tags = model
            .borrow()
            .tags
            .get(&game.product_id)
            .cloned()
            .unwrap_or_default();
        let tag_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let tag_heading = gtk::Label::new(Some("Personal tags"));
        tag_heading.set_xalign(0.0);
        tag_heading.add_css_class("section-title");
        tag_box.append(&tag_heading);
        let tag_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for tag in &tags {
            let chip = gtk::Label::new(Some(tag));
            chip.add_css_class("tag-chip");
            tag_row.append(&chip);
        }
        let tag_entry = gtk::Entry::builder()
            .placeholder_text("Add a tag")
            .hexpand(true)
            .build();
        let add_tag = gtk::Button::with_label("Add");
        tag_row.append(&tag_entry);
        tag_row.append(&add_tag);
        tag_box.append(&tag_row);
        let w2 = w.clone_refs();
        let model2 = model.clone();
        let detail_id = game.product_id;
        let detail_parent_id = game.parent_id;
        add_tag.connect_clicked(move |_| {
            let tag = tag_entry.text().trim().to_owned();
            if tag.is_empty() {
                return;
            }
            let mut m = model2.borrow_mut();
            let existing = m.tags.entry(detail_id).or_default();
            if existing
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&tag))
            {
                return;
            }
            existing.push(tag.clone());
            drop(m);
            // The database is reopened briefly so this detail callback stays independent of widget lifetime.
            if let Ok(store) = StateStore::open() {
                let _ = store.add_tag(detail_id, &tag);
            }
            refresh_filters(&w2, &model2.borrow());
            if let Some(parent_id) = detail_parent_id {
                let parent = model2
                    .borrow()
                    .games
                    .iter()
                    .find(|parent| parent.product_id == parent_id)
                    .cloned();
                let dlc = parent.as_ref().and_then(|parent| {
                    parent
                        .dlcs
                        .iter()
                        .find(|dlc| dlc.product_id == detail_id)
                        .cloned()
                });
                if let (Some(parent), Some(dlc)) = (parent, dlc) {
                    render_detail_page(&w2, &model2, DetailPageModel::dlc(&parent, dlc));
                }
            } else {
                show_game(&w2, &model2, detail_id, Some(false));
            }
        });
        overview.append(&tag_box);
    }
    tabs.add_titled(&overview, Some("overview"), "Overview");

    let visible_dlc_count = game
        .dlcs
        .iter()
        .filter(|dlc| dlc.is_catalog_visible())
        .count();
    if visible_dlc_count > 0 {
        let parent_id = game.parent_id.unwrap_or(game.product_id);
        let dlc_view = build_dlc_catalog(&game.dlcs, w, model, parent_id);
        tabs.add_titled(
            &dlc_view,
            Some("dlc"),
            &format!("DLC ({visible_dlc_count})"),
        );
    }

    if game.parent_id.is_none() {
        let notes = cached_patch_notes(model, game.product_id, &game.changelog);
        tabs.add_titled(
            &build_patch_notes_page(notes),
            Some("patch-notes"),
            "Patch Notes",
        );
    }

    let logs = gtk::Box::new(gtk::Orientation::Vertical, 12);
    logs.set_margin_top(12);
    refresh_product_logs(&logs, game.product_id, &w.window);
    tabs.add_titled(&logs, Some("logs"), "Logs");
    tabs.connect_visible_child_name_notify({
        let logs = logs.clone();
        let window = w.window.clone();
        let product_id = game.product_id;
        move |tabs| {
            if tabs.visible_child_name().as_deref() == Some("logs") {
                refresh_product_logs(&logs, product_id, &window);
            }
        }
    });

    {
        let access_token = model
            .borrow()
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        let download_directory = model.borrow().config.download_directory.clone();
        let installer_defaults = {
            let config = &model.borrow().config;
            InstallerFilterDefaults {
                language: config.installer_language.clone(),
                windows: config.installer_windows,
                linux: config.installer_linux,
                macos: config.installer_macos,
            }
        };
        let show_retired_artifacts = model.borrow().config.show_retired_artifacts;
        if game.owned && game.parent_id.is_none() {
            let placeholder = adw::StatusPage::builder()
                .title("Offline Installers")
                .description("Select this tab to load installer details")
                .build();
            tabs.add_titled(&placeholder, Some("files"), "Offline Installers");
            let loaded = Rc::new(std::cell::Cell::new(false));
            let game = game.clone();
            let window = w.window.clone();
            let management = file_management.clone();
            let installed_for_files = installed.as_ref().map(|(game, _)| game.clone());
            let tabs_for_load = tabs.clone();
            tabs.connect_visible_child_name_notify(move |tabs| {
                if tabs.visible_child_name().as_deref() != Some("files") || loaded.replace(true) {
                    return;
                }
                let files = build_files_page(
                    &game,
                    &window,
                    FilesPageOptions {
                        access_token: access_token.as_deref(),
                        download_directory: &download_directory,
                        installer_defaults: &installer_defaults,
                        show_retired_artifacts,
                        management: management.as_ref(),
                        installed: installed_for_files.as_ref(),
                    },
                );
                tabs_for_load.remove(&placeholder);
                tabs_for_load.add_titled(&files, Some("files"), "Offline Installers");
                tabs_for_load.set_visible_child_name("files");
            });
        }
    }
    page.append(&tabs);
    w.details.append(&page);
    w.content.set_visible_child_name("details");
    let adjustment = w.details_scroll.vadjustment();
    glib::idle_add_local_once(move || adjustment.set_value(adjustment.lower()));
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
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    content.set_halign(gtk::Align::Center);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    button.set_child(Some(&content));
}

fn launch_installed_game(window: &adw::ApplicationWindow, installed: crate::domain::InstalledGame) {
    let receiver = crate::installation::launch_game(installed);
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
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

#[derive(Clone)]
struct CloudLaunchStatus {
    panel: gtk::Box,
    heading: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
}

impl CloudLaunchStatus {
    fn show(&self, phase: crate::installation::CloudSyncPhase) {
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
        self.panel.set_visible(false);
        self.progress.set_visible(false);
    }
}

fn installation_status_panel(
    product_id: i64,
    window: &adw::ApplicationWindow,
    action_widgets: (
        &gtk::Button,
        &gtk::Box,
        &Rc<RefCell<Option<CloudLaunchStatus>>>,
    ),
    normal_action: GamePrimaryAction,
    initially_installing: bool,
    model: &Rc<RefCell<AppModel>>,
    refresh_after_install: Rc<dyn Fn()>,
) -> gtk::Box {
    let (primary_action, action_group, cloud_launch_status) = action_widgets;
    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    panel.add_css_class("hero-install-status");
    panel.set_visible(false);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let heading = gtk::Label::new(None);
    heading.set_xalign(0.0);
    heading.add_css_class("hero-transfer-heading");
    text.append(&heading);
    let detail = gtk::Label::new(None);
    detail.set_xalign(0.0);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_max_width_chars(42);
    detail.add_css_class("hero-transfer-detail");
    text.append(&detail);
    let progress = gtk::ProgressBar::new();
    progress.set_pulse_step(0.08);
    progress.add_css_class("hero-transfer-progress");
    text.append(&progress);
    panel.append(&text);
    *cloud_launch_status.borrow_mut() = Some(CloudLaunchStatus {
        panel: panel.clone(),
        heading: heading.clone(),
        detail: detail.clone(),
        progress: progress.clone(),
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
    {
        let panel = panel.clone();
        let refresh = refresh_after_install.clone();
        cloud_decline.connect_clicked(move |_| {
            if let Ok(store) = StateStore::open() {
                store
                    .set_cloud_save_preference(
                        product_id,
                        crate::domain::CloudSavePreference::Disabled,
                    )
                    .ok();
            }
            panel.set_visible(false);
            refresh();
        });
    }
    {
        let panel = panel.clone();
        let refresh = refresh_after_install.clone();
        cloud_enable.connect_clicked(move |_| {
            if let Ok(store) = StateStore::open() {
                store
                    .set_cloud_save_preference(
                        product_id,
                        crate::domain::CloudSavePreference::Enabled,
                    )
                    .ok();
            }
            panel.set_visible(false);
            refresh();
        });
    }
    let receiver = crate::installation::subscribe_installation_events();
    let depot_receiver = crate::installation::subscribe_depot_events();
    let initial_snapshot = crate::installation::installation_operation_snapshot(product_id);
    let panel_for_poll = panel.clone();
    let primary_action = primary_action.clone();
    let action_group = action_group.clone();
    let determinate = Rc::new(std::cell::Cell::new(false));
    let was_installing = Rc::new(std::cell::Cell::new(initially_installing));
    let was_uninstalling = Rc::new(std::cell::Cell::new(false));
    let pending_snapshot = Rc::new(RefCell::new(initial_snapshot));
    let model = model.clone();
    let was_downloading = Rc::new(std::cell::Cell::new(false));
    let was_game_running = Rc::new(std::cell::Cell::new(false));
    let action_visual = Rc::new(std::cell::Cell::new(0_u8));
    let depot_rate = Rc::new(RefCell::new(SmoothedTransferRate::default()));
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if panel_for_poll.root().is_none() {
            return glib::ControlFlow::Break;
        }
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
                *pending_snapshot.borrow_mut() =
                    crate::installation::installation_operation_snapshot(product_id);
            }
        }
        while let Ok(crate::installation::DepotManagerEvent::Snapshot(snapshot)) =
            depot_receiver.try_recv()
        {
            if snapshot.product_id == product_id
                && matches!(
                    snapshot.state.as_str(),
                    "complete" | "cancelled" | "abandoned"
                )
            {
                refresh_after_install();
                return glib::ControlFlow::Break;
            }
        }
        if let Some(snapshot) =
            crate::installation::depot_operation_snapshot_for_product(product_id)
            && matches!(snapshot.state.as_str(), "interrupted" | "failed")
        {
            panel_for_poll.set_visible(true);
            heading.set_label(if snapshot.state == "failed" {
                "INSTALLATION FAILED"
            } else {
                "INSTALLATION PAUSED"
            });
            detail.set_label(
                snapshot
                    .error
                    .as_deref()
                    .unwrap_or("Resume this installation or cancel it to start over"),
            );
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
        if let Some(snapshot) =
            crate::installation::depot_operation_snapshot_for_product(product_id)
            && !matches!(
                snapshot.state.as_str(),
                "complete" | "failed" | "cancelled" | "abandoned"
            )
        {
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
                "committing" => "INSTALLING".to_owned(),
                "finalizing" => "FINALIZING".to_owned(),
                _ => snapshot.state.replace('_', " ").to_uppercase(),
            };
            heading.set_label(&display_state);
            if let Some(total) = snapshot.download_total_bytes {
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
            cancel.set_visible(true);
            return glib::ControlFlow::Continue;
        }
        let installation_snapshot = pending_snapshot
            .borrow_mut()
            .take()
            .or_else(|| crate::installation::installation_operation_snapshot(product_id));
        if installation_snapshot.is_none() {
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
                refresh_after_install();
                return glib::ControlFlow::Break;
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
                detail.set_label(message);
                detail.set_tooltip_text(Some(message));
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                return glib::ControlFlow::Continue;
            }
            if jobs.iter().any(|job| job.state == DownloadState::Paused) {
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
            if crate::installation::is_game_running(product_id) {
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
        }
        if cloud_enable.is_visible() {
            return glib::ControlFlow::Continue;
        }
        match installation_snapshot {
            Some(crate::installation::InstallationOperationSnapshot {
                queued: true,
                message,
                ..
            }) => {
                if action_visual.replace(2) != 2 {
                    set_primary_button_content(
                        &primary_action,
                        "content-loading-symbolic",
                        "Queued",
                    );
                    action_group.add_css_class("operational-state");
                }
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
                if action_visual.replace(2) != 2 {
                    set_primary_button_content(
                        &primary_action,
                        "content-loading-symbolic",
                        "Installing",
                    );
                    action_group.add_css_class("operational-state");
                }
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
                if action_visual.replace(2) != 2 {
                    set_primary_button_content(
                        &primary_action,
                        "content-loading-symbolic",
                        "Uninstalling",
                    );
                    action_group.add_css_class("operational-state");
                }
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
                    error
                        .as_deref()
                        .unwrap_or("The installer exited with an error"),
                );
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
                    error
                        .as_deref()
                        .unwrap_or("The uninstaller exited with an error"),
                );
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
                cancel.set_visible(false);
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Installed,
                ..
            }) if was_installing.replace(false) => {
                let libraries = model.borrow().config.game_libraries.clone();
                panel_for_poll.set_visible(true);
                heading.set_label("CHECKING CLOUD SAVES");
                detail.set_label("Checking GOG cloud-save support…");
                progress.set_visible(true);
                cancel.set_visible(false);
                cloud_decline.set_visible(false);
                cloud_enable.set_visible(false);
                primary_action.set_sensitive(false);
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
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match receiver.try_recv() {
                        Ok(Ok((discovery, preference)))
                            if discovery.availability
                                == crate::domain::CloudSaveAvailability::Supported
                                && preference == crate::domain::CloudSavePreference::Undecided =>
                        {
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
                            panel.set_visible(false);
                            refresh();
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    }
                });
                return glib::ControlFlow::Break;
            }
            Some(crate::installation::InstallationOperationSnapshot {
                state: crate::domain::InstallationState::Pending,
                ..
            }) if was_uninstalling.replace(false) => {
                refresh_after_install();
                return glib::ControlFlow::Break;
            }
            Some(_) => {
                panel_for_poll.set_visible(false);
                cancel.set_visible(false);
                progress.set_visible(false);
                determinate.set(false);
                primary_action.set_sensitive(true);
            }
            None => {}
        }
        glib::ControlFlow::Continue
    });
    panel
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
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    if bytes_per_second >= MIB {
        format!("{:.1} MiB/s", bytes_per_second / MIB)
    } else {
        format!("{:.0} KiB/s", bytes_per_second / KIB)
    }
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
    let logs = [
        (
            "Installer log",
            crate::installation::installation_log_path(product_id).ok(),
        ),
        (
            "Uninstaller log",
            crate::installation::uninstallation_log_path(product_id).ok(),
        ),
        (
            "Runtime log",
            crate::installation::runtime_log_path(product_id).ok(),
        ),
    ];
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
    if logs
        .iter()
        .all(|(_, path)| path.as_ref().is_none_or(|path| !path.exists()))
        && download_failures.is_empty()
    {
        let empty = adw::StatusPage::builder()
            .title("No logs for this game")
            .description(
                "Installation, download, and runtime logs will appear here when available.",
            )
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
    for (title, path) in logs
        .into_iter()
        .filter_map(|(title, path)| path.filter(|path| path.exists()).map(|path| (title, path)))
    {
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
                launcher.launch(Some(&window), gio::Cancellable::NONE, |result| {
                    if let Err(error) = result {
                        tracing::warn!(%error, "could not open installation log");
                    }
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
        launcher.launch(Some(&window), gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "could not open link");
            }
        });
    });
    button
}

#[cfg(any())]
pub(super) fn build_dlc_view(dlcs: &[Dlc], window: &adw::ApplicationWindow) -> gtk::Paned {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("boxed-list");
    for (index, dlc) in dlcs.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&index.to_string());
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(10);
        content.set_margin_end(10);
        content.append(&picture(dlc.icon.as_ref(), 46, 46, "game-icon"));
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        labels.set_hexpand(true);
        let title = gtk::Label::new(Some(&dlc.title));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        let kind = gtk::Label::new(Some(dlc.kind()));
        kind.set_xalign(0.0);
        kind.add_css_class("dim-label");
        labels.append(&title);
        labels.append(&kind);
        content.append(&labels);
        row.set_child(Some(&content));
        list.append(&row);
    }
    let list_scroll = gtk::ScrolledWindow::builder()
        .min_content_width(250)
        .min_content_height(520)
        .child(&list)
        .build();

    let detail = gtk::Box::new(gtk::Orientation::Vertical, 16);
    detail.set_margin_start(22);
    detail.set_margin_end(6);
    let initial = adw::StatusPage::builder()
        .title("Select DLC")
        .description("Choose an expansion or content pack to view its details and files.")
        .icon_name("package-x-generic-symbolic")
        .build();
    detail.append(&initial);

    let dlcs = dlcs.to_vec();
    let detail_for_signal = detail.clone();
    let window = window.clone();
    list.connect_row_selected(move |_, row| {
        let Some(index) = row.and_then(|row| row.widget_name().parse::<usize>().ok()) else {
            return;
        };
        let Some(dlc) = dlcs.get(index) else {
            return;
        };
        populate_dlc_detail(&detail_for_signal, dlc, &window);
    });

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_position(290);
    paned.set_resize_start_child(false);
    paned.set_shrink_start_child(false);
    paned.set_start_child(Some(&list_scroll));
    paned.set_end_child(Some(&detail));
    paned
}

#[cfg(any())]
pub(super) fn populate_dlc_detail(
    container: &gtk::Box,
    dlc: &Dlc,
    window: &adw::ApplicationWindow,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    container.append(&picture(dlc.artwork.as_ref(), -1, 210, "dlc-hero"));
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    heading.append(&picture(dlc.icon.as_ref(), 64, 64, "detail-icon"));
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(&dlc.title));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.add_css_class("section-title");
    let subtitle = gtk::Label::new(Some(&format!(
        "{} · {} · {}",
        dlc.kind(),
        empty_dash(&dlc.platform_label()),
        human_size(dlc.disk_usage)
    )));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    labels.append(&title);
    labels.append(&subtitle);
    heading.append(&labels);
    container.append(&heading);
    container.append(&folder_button("Open DLC folder", &dlc.location, window));
    if !dlc.description.is_empty() {
        container.append(&expandable_section(
            "Overview",
            text::html_to_text(&dlc.description),
            1_000,
        ));
    }
    container.append(&section(
        "DLC information",
        &format!(
            "Languages: {}\nLocation: {}",
            empty_dash(&dlc.languages.join(", ")),
            dlc.location.display()
        ),
    ));
    if !dlc.installers.is_empty() {
        container.append(&file_section("Installers", &dlc.installers));
    }
    if !dlc.extras.is_empty() {
        let extras = dlc.location.join("extras");
        container.append(&folder_button("Open DLC extras folder", &extras, window));
        container.append(&file_section("Extras", &dlc.extras));
    }
    if !dlc.changelog.is_empty() {
        container.append(&lazy_html_section("Changelog", dlc.changelog.clone()));
    }
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
    use super::{SmoothedTransferRate, format_transfer_rate, parse_installation_progress};

    #[test]
    fn uses_latest_total_progress_and_ignores_file_progress() {
        let output = "file.bin: 90% (total progress: 41%)\r\nnext.bin: 5% (total progress: 42%)";
        assert_eq!(parse_installation_progress(output), Some(42));
        assert_eq!(parse_installation_progress("Uncompressing 77%"), None);
    }

    #[test]
    fn formats_download_rates_for_the_install_status() {
        assert_eq!(format_transfer_rate(512.0 * 1024.0), "512 KiB/s");
        assert_eq!(format_transfer_rate(12.5 * 1024.0 * 1024.0), "12.5 MiB/s");
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

#[cfg(any())]
pub(super) fn show_dlc_page_old(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    parent_id: i64,
    dlc: &Dlc,
) {
    let Some(parent) = model
        .borrow()
        .games
        .iter()
        .find(|game| game.product_id == parent_id)
        .cloned()
    else {
        return;
    };
    model.borrow_mut().selected = None;
    while let Some(child) = w.details.first_child() {
        w.details.remove(&child);
    }
    w.details
        .append(&detail_hero_picture(dlc.detail_artwork.as_ref()));
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.set_margin_start(36);
    page.set_margin_end(36);
    page.set_margin_top(22);
    page.set_margin_bottom(48);
    let back = gtk::Button::with_label("← Back to game");
    back.set_halign(gtk::Align::Start);
    back.add_css_class("flat");
    let widgets = w.clone_refs();
    let model_for_back = model.clone();
    back.connect_clicked(move |_| {
        show_game(&widgets, &model_for_back, parent_id, Some(false));
        let root: gtk::Widget = widgets.details.clone().upcast();
        if let Some(stack) = find_named_descendant(&root, "game-tabs").and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name("dlc");
        }
    });
    page.append(&back);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    heading.append(&picture(dlc.icon.as_ref(), 72, 72, "detail-icon"));
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(&dlc.title));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.add_css_class("game-title");
    let subtitle = gtk::Label::new(Some(&format!(
        "{} · {} · {}",
        dlc.kind(),
        empty_dash(&dlc.platform_label()),
        human_size(dlc.disk_usage)
    )));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    labels.append(&title);
    labels.append(&subtitle);
    heading.append(&labels);
    heading.append(&folder_button("Open DLC folder", &dlc.location, &w.window));
    page.append(&heading);
    let tabs = gtk::Stack::new();
    tabs.set_widget_name("game-tabs");
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_vexpand(true);
    let switcher = gtk::StackSwitcher::builder()
        .stack(&tabs)
        .halign(gtk::Align::Start)
        .build();
    switcher.add_css_class("detail-tabs");
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    navigation.add_css_class("game-navigation");
    navigation.append(&switcher);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    navigation.append(&spacer);
    if let Some(url) = &dlc.links.store {
        navigation.append(&uri_button("Store Page", url, &w.window));
    }
    if let Some(url) = &dlc.links.forum {
        navigation.append(&uri_button("Community Forum", url, &w.window));
    }
    if let Some(url) = &dlc.links.support {
        navigation.append(&uri_button("Support", url, &w.window));
    }
    page.append(&navigation);

    let overview = gtk::Box::new(gtk::Orientation::Vertical, 20);
    overview.set_margin_top(12);
    if !dlc.description.is_empty() {
        overview.append(&expandable_section(
            "Overview",
            text::html_to_text(&dlc.description),
            1_600,
        ));
    }
    if !dlc.screenshots.is_empty() {
        overview.append(&screenshot_strip(
            dlc.product_id,
            &dlc.screenshots,
            &w.window,
        ));
    }
    overview.append(&section(
        "DLC information",
        &format!(
            "Parent game: {}\nSlug: {}\nLanguages: {}\nLocation: {}",
            parent.title,
            dlc.slug,
            empty_dash(&dlc.languages.join(", ")),
            dlc.location.display()
        ),
    ));
    tabs.add_titled(&overview, Some("overview"), "Overview");

    let visible_dlc_count = parent
        .dlcs
        .iter()
        .filter(|dlc| dlc.is_catalog_visible())
        .count();
    let dlc_view = build_dlc_catalog(&parent.dlcs, w, model, parent_id);
    tabs.add_titled(
        &dlc_view,
        Some("dlc"),
        &format!("DLC ({visible_dlc_count})"),
    );
    tabs.add_titled(
        &build_dlc_files_page(dlc, &w.window),
        Some("files"),
        "Offline Installers",
    );
    let logs = gtk::Box::new(gtk::Orientation::Vertical, 12);
    logs.set_margin_top(12);
    refresh_product_logs(&logs, dlc.product_id, &w.window);
    tabs.add_titled(&logs, Some("logs"), "Logs");
    page.append(&tabs);
    w.details.append(&page);
    let adjustment = w.details_scroll.vadjustment();
    glib::idle_add_local_once(move || adjustment.set_value(adjustment.lower()));
}
