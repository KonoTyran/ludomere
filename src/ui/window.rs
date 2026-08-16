use super::*;

pub fn build_window(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let config = Config::load_or_create().unwrap_or_else(|error| {
        tracing::error!(%error, "could not load configuration");
        Config::default()
    });
    apply_theme(config.theme);
    let store = Rc::new(StateStore::open().expect("opening application database"));
    let _ = store.prune_completed_download_history(chrono::Utc::now().timestamp() - 30 * 86_400);
    let favorites = store.favorites().unwrap_or_default();
    let tags = store.tags().unwrap_or_default();
    let cached_games = store
        .normalized_games()
        .ok()
        .filter(|games| !games.is_empty())
        .unwrap_or_else(|| store.cached_online_games().unwrap_or_default());
    let cached_profile = store.cached_profile().unwrap_or_default();
    let download_jobs = store.download_jobs().unwrap_or_default();
    let downloaded_products = downloaded_product_ids(&download_jobs);
    let downloaded_installer_products = downloaded_installer_product_ids(&download_jobs);
    let reconciled = crate::installation::reconcile_installed_games(&store, &config.game_libraries)
        .unwrap_or_default();
    let installed_products = reconciled
        .iter()
        .filter(|game| {
            game.state == crate::domain::InstallationState::Installed
                && crate::installation::resolve_installation_directory(game, &config.game_libraries)
                    .is_some()
        })
        .map(|game| game.product_id)
        .collect();
    let playable_products = reconciled
        .iter()
        .filter(|game| sidebar_game_is_playable(game, &config.game_libraries))
        .map(|game| game.product_id)
        .collect();
    let (activity_sender, activity_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let activity = StateStore::open()
            .and_then(|store| store.all_product_activity())
            .unwrap_or_default();
        let _ = activity_sender.send(activity);
    });
    let mut product_activity = activity_receiver.recv().unwrap_or_default();
    // Seed durable activity from existing installation markers once. The marker
    // timestamp is the installation event, not a reconciliation/uninstall time.
    for installed in &reconciled {
        if let Some(installed_at) = installed.installed_at {
            let activity = product_activity.entry(installed.product_id).or_default();
            if activity
                .last_activity_at
                .is_none_or(|previous| installed_at > previous)
            {
                activity.last_activity_at = Some(installed_at);
                if let Err(error) =
                    store.record_product_activity(installed.product_id, installed_at)
                {
                    tracing::warn!(%error, product_id = installed.product_id, "seeding installation activity");
                }
            }
        }
    }
    let (owned_product_count, online_synced_at) = store.owned_library_status().unwrap_or_default();
    let card_width = [140, 180, 220, 260][config.library_card_size.min(3) as usize];
    let sidebar_sort_mode = config.sidebar_sort_mode;
    let model = Rc::new(RefCell::new(AppModel {
        config,
        games: cached_games,
        favorites,
        tags,
        favorites_only: false,
        downloaded_only: false,
        installed_only: false,
        played_only: false,
        unplayed_only: false,
        installed_products,
        playable_products,
        downloaded_products,
        downloaded_installer_products,
        download_jobs,
        depot_operations: crate::installation::depot_operation_snapshots(),
        transfer_history: Rc::new(RefCell::new(VecDeque::new())),
        transfer_totals: None,
        active_transfer_id: None,
        windows_only: false,
        linux_only: false,
        macos_only: false,
        cloud_saves_only: false,
        achievements_only: false,
        language_filter: None,
        genre_theme_filters: BTreeSet::new(),
        game_mode_filters: BTreeSet::new(),
        property_filters: BTreeSet::new(),
        card_width,
        query: String::new(),
        selected: None,
        account_profile: cached_profile,
        account_token: None,
        token_refresh_in_progress: false,
        network_available: true,
        owned_product_count,
        online_synced_at,
        product_activity,
        sidebar_sort_mode,
        sidebar_playable_only: false,
        collapsed_activity_sections: HashSet::new(),
        activity_sections: Vec::new(),
    }));
    download::set_concurrency(model.borrow().config.max_concurrent_downloads);
    let widgets = Rc::new(create_widgets(app, &model.borrow().config));

    connect_actions(&widgets, &model, &store);
    update_account_widgets(&widgets, model.borrow().account_profile.as_ref());
    update_account_library_status(&widgets, &model.borrow());
    start_network_monitor(&widgets, &model);
    widgets.window.present();
    if !model.borrow().games.is_empty() {
        rebuild_library(&widgets, &model);
        widgets.content.set_visible_child_name("home");
    }
    start_managed_reconciliation(&widgets, &model);
    start_account_restore(&widgets, &model, &store);
    start_token_renewal_monitor(&widgets, &model);
    start_download_monitor(&widgets, &model);
    start_installation_monitor(&widgets, &model);
    tray::start_tray(&widgets, &model);
    schedule_activity_rollover(&widgets, &model);
}

fn start_installation_monitor(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let events = crate::installation::subscribe_installation_events();
    let depot_events = crate::installation::subscribe_depot_events();
    let (state_sender, state_receiver) = mpsc::channel::<(HashSet<i64>, HashSet<i64>)>();
    let mut depot_states = HashMap::<String, String>::new();
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut changed_products = HashSet::new();
        let mut terminal_products = HashSet::new();
        let mut activity_changed = false;
        while let Ok(event) = events.try_recv() {
            match event {
                crate::installation::InstallationManagerEvent::OperationQueued(snapshot) => {
                    changed_products.insert(snapshot.product_id);
                    if snapshot.state == crate::domain::InstallationState::Pending
                        && snapshot.message.as_deref().is_some_and(|message| {
                            message.contains("installation") && !message.contains("uninstallation")
                        })
                    {
                        let now = chrono::Utc::now().timestamp();
                        let mut state = model.borrow_mut();
                        let activity = state
                            .product_activity
                            .entry(snapshot.product_id)
                            .or_default();
                        activity.last_activity_at =
                            Some(activity.last_activity_at.map_or(now, |old| old.max(now)));
                        if let Ok(store) = StateStore::open()
                            && let Err(error) =
                                store.record_product_activity(snapshot.product_id, now)
                        {
                            tracing::warn!(%error, product_id = snapshot.product_id, "recording attempted installation activity");
                        }
                        activity_changed = true;
                    }
                }
                crate::installation::InstallationManagerEvent::OperationRecovered(snapshot) => {
                    changed_products.insert(snapshot.product_id);
                }
                crate::installation::InstallationManagerEvent::OperationCancelled(snapshot) => {
                    changed_products.insert(snapshot.product_id);
                    terminal_products.insert(snapshot.product_id);
                }
                crate::installation::InstallationManagerEvent::Installation {
                    product_id,
                    event,
                } => {
                    changed_products.insert(product_id);
                    if matches!(
                        &event,
                        crate::installation::InstallationEvent::Complete { .. }
                    ) {
                        let now = chrono::Utc::now().timestamp();
                        let mut state = model.borrow_mut();
                        let activity = state.product_activity.entry(product_id).or_default();
                        activity.last_activity_at =
                            Some(activity.last_activity_at.map_or(now, |old| old.max(now)));
                        activity_changed = true;
                    }
                    if let crate::installation::InstallationEvent::Prompt {
                        text,
                        choices,
                        context,
                    } = &event
                    {
                        super::download_chooser::present_installer_prompt(
                            &w.window, product_id, text, choices, context,
                        );
                    }
                    if matches!(
                        event,
                        crate::installation::InstallationEvent::Complete { .. }
                            | crate::installation::InstallationEvent::Cancelled
                            | crate::installation::InstallationEvent::Failed(_)
                    ) {
                        terminal_products.insert(product_id);
                    }
                }
                crate::installation::InstallationManagerEvent::Uninstallation {
                    product_id,
                    event,
                } => {
                    changed_products.insert(product_id);
                    if matches!(
                        event,
                        crate::installation::UninstallationEvent::Complete
                            | crate::installation::UninstallationEvent::Cancelled
                            | crate::installation::UninstallationEvent::Failed(_)
                    ) {
                        terminal_products.insert(product_id);
                    }
                }
            }
        }
        while let Ok(crate::installation::DepotManagerEvent::Snapshot(snapshot)) =
            depot_events.try_recv()
        {
            let state_changed = depot_states
                .insert(snapshot.operation_id, snapshot.state.clone())
                .is_none_or(|previous| previous != snapshot.state);
            let terminal = matches!(
                snapshot.state.as_str(),
                "complete" | "failed" | "cancelled" | "abandoned"
            );
            if state_changed || terminal {
                changed_products.insert(snapshot.product_id);
            }
            if terminal {
                terminal_products.insert(snapshot.product_id);
            }
        }
        while let Ok((installed_products, playable_products)) = state_receiver.try_recv() {
            let mut state = model.borrow_mut();
            state.installed_products = installed_products;
            state.playable_products = playable_products;
            drop(state);
            update_sidebar_download_styles(&w, &model.borrow());
            refresh_filters(&w, &model.borrow());
        }
        if activity_changed && model.borrow().sidebar_sort_mode == SidebarSortMode::LastPlayed {
            rebuild_sidebar_presentation(&w, &mut model.borrow_mut());
        }
        if changed_products.is_empty() {
            return glib::ControlFlow::Continue;
        }

        update_sidebar_download_styles(&w, &model.borrow());

        if let Some(product_id) = changed_products.iter().next().copied()
            && let Some(snapshot) = crate::installation::installation_operation_snapshot(product_id)
        {
            let game_title = model
                .borrow()
                .games
                .iter()
                .find(|game| game.product_id == product_id)
                .map(|game| game.title.clone())
                .unwrap_or_else(|| "game".into());
            let action = if snapshot.queued {
                "Queued"
            } else {
                match snapshot.state {
                    crate::domain::InstallationState::Installing => "Installing",
                    crate::domain::InstallationState::Uninstalling => "Uninstalling",
                    crate::domain::InstallationState::Installed => "Installed",
                    crate::domain::InstallationState::UninstallFailed => "Uninstall failed",
                    crate::domain::InstallationState::Failed => "Installation failed",
                    crate::domain::InstallationState::Pending => "Uninstalled",
                }
            };
            w.status.set_label(&format!("{action} {game_title}"));
        }

        if !terminal_products.is_empty() {
            let game_libraries = model.borrow().config.game_libraries.clone();
            let state_sender = state_sender.clone();
            std::thread::spawn(move || {
                let games = StateStore::open()
                    .and_then(|store| {
                        crate::installation::reconcile_installed_games(&store, &game_libraries)
                    })
                    .unwrap_or_default();
                let installed_products = games
                    .iter()
                    .filter(|game| game.state == crate::domain::InstallationState::Installed)
                    .map(|game| game.product_id)
                    .collect();
                let playable_products = games
                    .iter()
                    .filter(|game| sidebar_game_is_playable(game, &game_libraries))
                    .map(|game| game.product_id)
                    .collect();
                let _ = state_sender.send((installed_products, playable_products));
            });
            if model
                .borrow()
                .selected
                .is_some_and(|selected| terminal_products.contains(&selected))
            {
                let selected = model.borrow().selected.unwrap();
                render_product_details(&w, &model, selected);
            }
        }
        glib::ControlFlow::Continue
    });
    std::thread::spawn(|| match crate::installation::recover_backend_operations() {
        Ok(_) => {
            crate::installation::start_recovered_operations();
            if let Ok(config) = Config::load_or_create()
                && let Ok(migrations) =
                    crate::installation::source_migration::discover(&config.game_libraries)
            {
                for (library, journal) in migrations {
                    if journal.target.is_some() && journal.old_game.is_some() {
                        let destinations = journal.locations.clone();
                        let events = crate::installation::source_migration::start(
                            library,
                            journal,
                            destinations,
                        );
                        std::thread::spawn(move || while events.recv().is_ok() {});
                    }
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "could not recover interrupted installation operations");
        }
    });
}

pub(super) fn start_managed_reconciliation(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    struct Reconciliation {
        summary: anyhow::Result<managed::RebuildSummary>,
        files: Vec<crate::state::ManagedFileRecord>,
        jobs: Vec<DownloadJobRecord>,
    }

    let root = model.borrow().config.download_directory.clone();
    let games = model.borrow().games.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let reconciliation = (|| -> anyhow::Result<Reconciliation> {
            let mut store = StateStore::open()?;
            let summary = managed::rebuild(&mut store, &root, &games);
            let files = store.managed_files()?;
            let jobs = store.download_jobs()?;
            Ok(Reconciliation {
                summary,
                files,
                jobs,
            })
        })();
        let _ = sender.send(reconciliation);
    });

    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok(reconciliation)) => {
                let visible_detail_tab =
                    find_named_descendant(w.details.upcast_ref::<gtk::Widget>(), "game-tabs")
                        .and_downcast::<gtk::Stack>()
                        .and_then(|stack| stack.visible_child_name().map(|name| name.to_string()));
                let mut state = model.borrow_mut();
                let download_directory = state.config.download_directory.clone();
                managed::apply_to_games(&mut state.games, &reconciliation.files);
                managed::set_locations(&mut state.games, &download_directory);
                state.download_jobs = reconciliation.jobs;
                state.downloaded_products = downloaded_product_ids(&state.download_jobs);
                state.downloaded_installer_products =
                    downloaded_installer_product_ids(&state.download_jobs);
                let selected = state.selected;
                drop(state);
                update_sidebar_download_styles(&w, &model.borrow());
                if let Some(product_id) = selected {
                    render_product_details(&w, &model, product_id);
                    if let Some(tab) = visible_detail_tab {
                        let details = w.details.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(stack) = find_named_descendant(
                                details.upcast_ref::<gtk::Widget>(),
                                "game-tabs",
                            )
                            .and_downcast::<gtk::Stack>()
                            {
                                stack.set_visible_child_name(&tab);
                            }
                        });
                    }
                }
                if let Err(error) = reconciliation.summary {
                    tracing::warn!(%error, "could not reconcile managed downloads during startup");
                    show_status(&w, &format!("Could not check downloaded files: {error}"));
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "could not start managed-download reconciliation");
                show_status(&w, &format!("Could not check downloaded files: {error}"));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub(super) fn create_widgets(app: &adw::Application, config: &Config) -> Widgets {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(crate::identity::APP_NAME)
        .default_width(config.window_width.max(1100))
        .default_height(config.window_height.max(600))
        .build();
    window.set_size_request(1100, -1);
    if config.window_maximized {
        window.maximize();
    }

    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    let app_icon = gtk::Image::from_icon_name(crate::identity::APP_ID);
    app_icon.set_pixel_size(30);
    app_icon.set_tooltip_text(Some(crate::identity::APP_NAME));
    app_icon.add_css_class("header-app-icon");
    header.pack_start(&app_icon);
    let title = adw::WindowTitle::new(crate::identity::APP_NAME, "Local collection");
    header.set_title_widget(Some(&title));
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh library (Ctrl+R)"));
    refresh.set_action_name(Some("win.refresh"));
    let settings = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings.set_tooltip_text(Some("Settings"));
    settings.set_action_name(Some("win.settings"));
    let header_network_button = gtk::Button::new();
    header_network_button.add_css_class("flat");
    header_network_button.add_css_class("header-network-button");
    header_network_button.set_tooltip_text(Some("Checking network status"));
    let header_network = gtk::Overlay::new();
    let header_network_icon = gtk::Image::from_icon_name("globe-symbolic");
    header_network_icon.set_pixel_size(18);
    header_network.add_css_class("header-network-status");
    header_network.set_child(Some(&header_network_icon));
    let header_network_slash = gtk::Label::new(Some("/"));
    header_network_slash.add_css_class("header-network-slash");
    header_network_slash.set_visible(false);
    header_network.add_overlay(&header_network_slash);
    header_network_button.set_child(Some(&header_network));
    let account_button = gtk::MenuButton::new();
    account_button.set_tooltip_text(Some("GOG account"));
    let account_button_content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    account_button_content.set_height_request(26);
    account_button_content.set_overflow(gtk::Overflow::Hidden);
    let account_button_avatar = adw::Avatar::new(22, None, true);
    account_button_avatar.add_css_class("account-avatar-small");
    let account_button_name = gtk::Label::new(Some("Sign in"));
    account_button_name.set_max_width_chars(12);
    account_button_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    account_button_content.append(&account_button_avatar);
    account_button_content.append(&account_button_name);
    let account_offline_indicator = gtk::Image::from_icon_name("network-offline-symbolic");
    account_offline_indicator.set_tooltip_text(Some("Offline"));
    account_offline_indicator.set_visible(false);
    account_button_content.append(&account_offline_indicator);
    account_button.set_child(Some(&account_button_content));
    let account_popover = gtk::Popover::new();
    let account_panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
    account_panel.set_margin_start(16);
    account_panel.set_margin_end(16);
    account_panel.set_margin_top(16);
    account_panel.set_margin_bottom(16);
    account_panel.set_width_request(290);
    let account_identity = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let account_avatar = gtk::Picture::new();
    account_avatar.set_size_request(64, 64);
    account_avatar.set_content_fit(gtk::ContentFit::Cover);
    account_avatar.set_can_shrink(true);
    account_avatar.set_halign(gtk::Align::Center);
    account_avatar.set_valign(gtk::Align::Center);
    account_avatar.set_overflow(gtk::Overflow::Hidden);
    account_avatar.add_css_class("account-avatar");
    account_identity.append(&account_avatar);
    let account_copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    account_copy.set_hexpand(true);
    account_copy.set_valign(gtk::Align::Center);
    let account_name = gtk::Label::new(Some("Not signed in"));
    account_name.set_xalign(0.0);
    account_name.add_css_class("section-title");
    let account_details = gtk::Label::new(Some(
        "Connect your GOG account to synchronize your library.",
    ));
    account_details.set_xalign(0.0);
    account_details.set_wrap(true);
    account_details.add_css_class("dim-label");
    account_copy.append(&account_name);
    account_copy.append(&account_details);
    account_identity.append(&account_copy);
    account_panel.append(&account_identity);
    let account_library_status = gtk::Label::new(Some("Online library not synchronized"));
    account_library_status.set_xalign(0.0);
    account_library_status.add_css_class("account-library-status");
    account_panel.append(&account_library_status);
    let account_connection_status = gtk::Label::new(Some("Online"));
    account_connection_status.set_xalign(0.0);
    account_connection_status.add_css_class("dim-label");
    account_panel.append(&account_connection_status);
    let sign_in = gtk::Button::with_label("Sign in to GOG");
    sign_in.add_css_class("suggested-action");
    let reconnect = gtk::Button::with_label("Sign in again");
    reconnect.set_visible(false);
    let sign_out = gtk::Button::with_label("Sign out");
    sign_out.add_css_class("destructive-action");
    sign_out.set_visible(false);
    account_panel.append(&sign_in);
    account_panel.append(&reconnect);
    account_panel.append(&sign_out);
    account_popover.set_child(Some(&account_panel));
    account_button.set_popover(Some(&account_popover));
    account_button.add_css_class("compact-account-button");
    header.pack_end(&account_button);
    header.pack_end(&header_network_button);
    header.pack_end(&settings);
    header.pack_end(&refresh);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.set_width_request(328);
    sidebar.add_css_class("library-sidebar");
    let home = gtk::Button::builder().label("Home").build();
    home.add_css_class("home-button");
    home.set_hexpand(true);
    home.set_action_name(Some("win.home"));
    let collections_button = gtk::Button::from_icon_name("view-grid-symbolic");
    collections_button.set_tooltip_text(Some("Collections"));
    collections_button.add_css_class("collections-button");
    collections_button.set_action_name(Some("win.collections"));
    let sort_toggle = gtk::ToggleButton::new();
    sort_toggle.set_child(Some(&gtk::Image::from_icon_name(
        "document-open-recent-symbolic",
    )));
    sort_toggle.set_tooltip_text(Some("Sort by last activity"));
    sort_toggle.update_property(&[gtk::accessible::Property::Label("Sort by last activity")]);
    sort_toggle.add_css_class("sidebar-icon-toggle");
    let playable_toggle = gtk::ToggleButton::new();
    playable_toggle.set_child(Some(&gtk::Image::from_icon_name(
        "media-playback-start-symbolic",
    )));
    playable_toggle.set_tooltip_text(Some("Show games playable now"));
    playable_toggle
        .update_property(&[gtk::accessible::Property::Label("Show playable games only")]);
    playable_toggle.add_css_class("sidebar-icon-toggle");
    let library_views = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    library_views.add_css_class("library-view-buttons");
    library_views.add_css_class("sidebar-toolbar");
    library_views.append(&home);
    library_views.append(&collections_button);
    library_views.append(&sort_toggle);
    library_views.append(&playable_toggle);
    sidebar.append(&library_views);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search library")
        .build();
    search.set_hexpand(true);
    let filter_chips = gtk::FlowBox::new();
    filter_chips.set_selection_mode(gtk::SelectionMode::None);
    filter_chips.set_homogeneous(false);
    filter_chips.set_halign(gtk::Align::Fill);
    filter_chips.set_min_children_per_line(1);
    filter_chips.set_max_children_per_line(20);
    filter_chips.set_column_spacing(4);
    filter_chips.set_row_spacing(4);
    filter_chips.set_visible(false);
    filter_chips.add_css_class("active-filter-chips");
    let search_surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search_surface.set_hexpand(true);
    search_surface.append(&search);
    search_surface.append(&filter_chips);

    let sort_row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    sort_row.set_margin_start(5);
    sort_row.set_margin_end(5);
    sort_row.set_margin_top(5);
    sort_row.set_margin_bottom(5);
    sort_row.add_css_class("sidebar-toolbar");
    let filter_button = gtk::MenuButton::new();
    filter_button.set_valign(gtk::Align::Start);
    filter_button.set_halign(gtk::Align::End);
    filter_button.set_size_request(34, 30);
    filter_button.set_direction(gtk::ArrowType::Right);
    filter_button.add_css_class("sidebar-filter-button");
    filter_button.set_tooltip_text(Some("Library filters"));
    filter_button.update_property(&[gtk::accessible::Property::Label("Library filters")]);
    let filter_button_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    filter_button_content.append(&gtk::Image::from_icon_name("view-filter-symbolic"));
    let filter_count = gtk::Label::new(None);
    filter_count.add_css_class("filter-count");
    filter_count.set_visible(false);
    filter_button.set_child(Some(&filter_button_content));

    let filter_popover = gtk::Popover::new();
    filter_popover.set_position(gtk::PositionType::Right);
    filter_popover.set_has_arrow(false);
    filter_popover.add_css_class("sidebar-filter-popover");
    let filter_panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
    filter_panel.add_css_class("library-filter-panel");
    filter_panel.set_margin_start(6);
    filter_panel.set_margin_end(6);
    filter_panel.set_margin_top(5);
    filter_panel.set_margin_bottom(5);
    filter_panel.set_width_request(540);
    let heading = gtk::Label::new(Some("Library Filters"));
    heading.set_xalign(0.0);
    heading.add_css_class("section-title");
    filter_panel.append(&heading);
    let filter_columns = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    filter_columns.set_homogeneous(true);
    filter_columns.set_vexpand(true);
    filter_columns.set_valign(gtk::Align::Start);
    let library_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let platform_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let metadata_column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    library_column.append(&filter_heading("Library"));
    let favorite_filter = gtk::CheckButton::with_label("Favorites only");
    library_column.append(&favorite_filter);
    let downloaded_filter = gtk::CheckButton::with_label("Downloaded content");
    downloaded_filter.set_tooltip_text(Some("Show games with downloaded base-game or DLC content"));
    library_column.append(&downloaded_filter);
    library_column.append(&filter_heading("Operating system"));
    let windows_filter = gtk::CheckButton::with_label("Windows");
    let linux_filter = gtk::CheckButton::with_label("Linux");
    let macos_filter = gtk::CheckButton::with_label("macOS");
    library_column.append(&windows_filter);
    library_column.append(&linux_filter);
    library_column.append(&macos_filter);
    let property_section = gtk::Box::new(gtk::Orientation::Vertical, 4);
    property_section.append(&filter_heading("Store tags"));
    property_section.set_width_request(245);
    let property_filter_search = gtk::SearchEntry::builder()
        .placeholder_text("enter a tag")
        .build();
    property_filter_search.add_css_class("property-filter-search");
    let property_filter_chips = gtk::FlowBox::new();
    property_filter_chips.set_selection_mode(gtk::SelectionMode::None);
    property_filter_chips.set_homogeneous(false);
    property_filter_chips.set_max_children_per_line(8);
    property_filter_chips.set_column_spacing(3);
    property_filter_chips.set_row_spacing(3);
    property_filter_chips.add_css_class("property-filter-chips");
    let property_filter_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let property_filter_label = gtk::Label::new(None);
    property_filter_label.set_visible(false);
    let property_suggestions = gtk::Popover::new();
    property_suggestions.set_has_arrow(false);
    property_suggestions.set_autohide(false);
    property_suggestions.set_focusable(false);
    property_suggestions.set_position(gtk::PositionType::Bottom);
    property_suggestions.add_css_class("property-filter-suggestions");
    let property_suggestions_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_width(180)
        .max_content_height(220)
        .propagate_natural_height(true)
        .child(&property_filter_box)
        .build();
    property_suggestions.set_child(Some(&property_suggestions_scroll));
    property_suggestions.set_parent(&property_filter_search);
    {
        let suggestions = property_suggestions.clone();
        let options = property_filter_box.clone();
        property_filter_search.connect_search_changed(move |search| {
            let query = search.text().to_lowercase();
            let mut child = options.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                let visible = widget
                    .downcast_ref::<gtk::CheckButton>()
                    .and_then(|check| check.label())
                    .is_some_and(|label| query.is_empty() || label.to_lowercase().contains(&query));
                widget.set_visible(visible);
                child = next;
            }
            if query.is_empty() {
                suggestions.popdown();
            } else if !suggestions.is_visible() {
                suggestions.popup();
                search.grab_focus();
                let search = search.clone();
                glib::idle_add_local_once(move || {
                    search.grab_focus();
                    search.set_position(-1);
                });
            }
        });
    }
    property_section.append(&property_filter_search);
    property_section.append(&property_filter_chips);
    platform_column.append(&filter_heading("Play state"));
    let installed_filter = gtk::CheckButton::with_label("Installed");
    installed_filter.set_tooltip_text(Some("Show games installed in a configured library"));
    platform_column.append(&installed_filter);
    let played_filter = gtk::CheckButton::with_label("Played");
    played_filter.set_tooltip_text(Some("Show games with recorded play activity"));
    platform_column.append(&played_filter);
    let unplayed_filter = gtk::CheckButton::with_label("Unplayed");
    unplayed_filter.set_tooltip_text(Some("Show games that have never been played"));
    platform_column.append(&unplayed_filter);
    platform_column.append(&filter_heading("Features"));
    let cloud_saves_filter = gtk::CheckButton::with_label("Cloud saves");
    let achievements_filter = gtk::CheckButton::with_label("Achievements");
    platform_column.append(&cloud_saves_filter);
    platform_column.append(&achievements_filter);
    let (game_mode_filter_box, game_mode_filter_label) = inline_metadata_filter("Play modes");
    platform_column.append(&game_mode_filter_box);
    metadata_column.append(&filter_heading("Language"));
    let language_options = gtk::StringList::new(&["Any language"]);
    let language_filter =
        gtk::DropDown::new(Some(language_options.clone()), None::<gtk::Expression>);
    language_filter.set_hexpand(true);
    metadata_column.append(&language_filter);
    let (genre_theme_filter_box, genre_theme_filter_label) = inline_metadata_filter("Genre");
    let genre_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .max_content_height(260)
        .propagate_natural_height(true)
        .child(&genre_theme_filter_box)
        .build();
    genre_scroll.add_css_class("inline-filter-scroll");
    metadata_column.append(&genre_scroll);
    filter_columns.append(&library_column);
    filter_columns.append(&platform_column);
    filter_columns.append(&metadata_column);
    filter_panel.append(&filter_columns);
    let bottom_filters = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    bottom_filters.add_css_class("filter-bottom-row");
    bottom_filters.append(&property_section);
    let bottom_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bottom_spacer.set_hexpand(true);
    bottom_filters.append(&bottom_spacer);
    filter_panel.append(&bottom_filters);
    let filter_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(540)
        .max_content_height(620)
        .propagate_natural_height(true)
        .child(&filter_panel)
        .build();
    filter_scroll.add_css_class("library-filter-scroll");
    filter_popover.set_child(Some(&filter_scroll));
    filter_button.set_popover(Some(&filter_popover));
    {
        let anchor = filter_button.clone();
        filter_popover.connect_map(move |popover| {
            let popover = popover.clone();
            let anchor = anchor.clone();
            // `map` fires before Gtk has committed the final popover allocation.
            // Measure on the following main-loop turn, then compensate for the
            // default vertical centering so both top edges line up.
            glib::idle_add_local_once(move || {
                if !popover.is_visible() {
                    return;
                }
                let offset = ((popover.height() - anchor.height()).max(0)) / 2;
                popover.set_offset(0, offset);
            });
        });
    }
    sort_row.append(&search_surface);
    let clear_filters = gtk::Button::from_icon_name("window-close-symbolic");
    clear_filters.set_valign(gtk::Align::Start);
    clear_filters.set_halign(gtk::Align::End);
    clear_filters.set_size_request(28, 30);
    clear_filters.set_tooltip_text(Some("Clear all filters"));
    clear_filters.add_css_class("flat");
    clear_filters.add_css_class("clear-filters");
    clear_filters.set_visible(false);
    sort_row.append(&clear_filters);
    sort_row.append(&filter_button);
    sidebar.append(&sort_row);

    let game_list = gtk::ListBox::new();
    game_list.set_selection_mode(gtk::SelectionMode::Single);
    game_list.set_valign(gtk::Align::Start);
    game_list.set_vexpand(false);
    game_list.add_css_class("navigation-sidebar");
    let list_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&game_list)
        .build();
    sidebar.append(&list_scroll);
    let count = gtk::Label::new(Some("Loading library…"));
    count.add_css_class("dim-label");
    count.set_margin_top(8);
    count.set_margin_bottom(8);
    sidebar.append(&count);

    let home_grid = gtk::FlowBox::builder()
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Fill)
        .homogeneous(false)
        .max_children_per_line(20)
        .min_children_per_line(1)
        .column_spacing(18)
        .row_spacing(18)
        .selection_mode(gtk::SelectionMode::None)
        .build();
    home_grid.add_css_class("game-grid");
    home_grid.set_margin_start(24);
    home_grid.set_margin_end(24);
    home_grid.set_margin_top(24);
    home_grid.set_margin_bottom(24);
    let home_scroll = gtk::ScrolledWindow::builder().child(&home_grid).build();
    let collections = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let collections_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&collections)
        .build();
    let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let details_scroll = gtk::ScrolledWindow::builder().child(&details).build();
    details_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    install_smooth_wheel_scroll(&details_scroll);
    let downloads = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let downloads_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&downloads)
        .build();
    let empty = adw::StatusPage::builder()
        .title("No games found")
        .description("Sign in to GOG to load your library.")
        .icon_name("folder-games-symbolic")
        .build();

    let content = gtk::Stack::new();
    content.set_transition_type(gtk::StackTransitionType::Crossfade);
    content.add_named(&home_scroll, Some("home"));
    content.add_named(&collections_scroll, Some("collections"));
    content.add_named(&details_scroll, Some("details"));
    content.add_named(&downloads_scroll, Some("downloads"));
    content.add_named(&empty, Some("empty"));

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&sidebar));
    paned.set_end_child(Some(&content));
    paned.set_resize_start_child(false);
    paned.set_shrink_start_child(false);
    paned.set_position(328);
    let status_bar = gtk::Button::new();
    status_bar.add_css_class("application-status-bar");
    status_bar.set_action_name(Some("win.downloads"));
    status_bar.set_tooltip_text(Some("Open downloads"));
    let status_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    status_content.set_height_request(34);
    status_content.set_vexpand(false);
    status_content.set_valign(gtk::Align::Center);
    let sync_content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sync_content.set_width_request(190);
    sync_content.set_valign(gtk::Align::Center);
    let sync_heading = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    sync_heading.set_valign(gtk::Align::Center);
    let sync_spinner = gtk::Spinner::new();
    sync_spinner.set_spinning(false);
    sync_spinner.set_visible(false);
    sync_spinner.set_valign(gtk::Align::Center);
    sync_spinner.set_tooltip_text(Some("Updating GOG library"));
    sync_heading.append(&sync_spinner);
    let sync_status = gtk::Label::new(Some("Updating library"));
    sync_status.set_xalign(0.0);
    sync_status.set_valign(gtk::Align::Center);
    sync_status.set_visible(false);
    sync_heading.append(&sync_status);
    sync_content.append(&sync_heading);
    let sync_progress = gtk::ProgressBar::new();
    sync_progress.set_visible(false);
    sync_progress.set_valign(gtk::Align::Center);
    sync_content.append(&sync_progress);
    status_content.append(&sync_content);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer.set_valign(gtk::Align::Center);
    status_content.append(&spacer);
    let download_artwork = gtk::Image::new();
    download_artwork.set_pixel_size(24);
    download_artwork.set_size_request(24, 24);
    download_artwork.set_hexpand(false);
    download_artwork.set_vexpand(false);
    download_artwork.set_halign(gtk::Align::Center);
    download_artwork.set_valign(gtk::Align::Center);
    download_artwork.add_css_class("download-status-icon");
    download_artwork.set_visible(false);
    status_content.append(&download_artwork);
    let download_content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    download_content.set_size_request(230, -1);
    download_content.set_hexpand(false);
    download_content.set_vexpand(false);
    download_content.set_valign(gtk::Align::Center);
    let download_heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    download_heading.set_valign(gtk::Align::Center);
    let status = gtk::Label::new(Some("Ready"));
    status.set_xalign(0.0);
    status.set_valign(gtk::Align::Center);
    status.set_hexpand(false);
    status.set_width_chars(21);
    status.set_max_width_chars(21);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    download_heading.append(&status);
    let download_percent = gtk::Label::new(None);
    download_percent.set_xalign(1.0);
    download_percent.set_valign(gtk::Align::Center);
    download_percent.set_width_chars(4);
    download_percent.set_visible(false);
    download_heading.append(&download_percent);
    download_content.append(&download_heading);
    let download_status_progress = gtk::ProgressBar::new();
    download_status_progress.set_visible(false);
    download_status_progress.set_valign(gtk::Align::Center);
    download_content.append(&download_status_progress);
    status_content.append(&download_content);
    let status_arrow = gtk::Image::from_icon_name("go-next-symbolic");
    status_arrow.set_valign(gtk::Align::Center);
    status_content.append(&status_arrow);
    status_bar.set_child(Some(&status_content));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&paned);
    paned.set_vexpand(true);
    root.append(&status_bar);
    window.set_content(Some(&root));

    Widgets {
        window,
        status,
        status_bar,
        sync_spinner,
        sync_status,
        sync_progress,
        download_artwork,
        download_percent,
        download_status_progress,
        game_list,
        home_grid,
        collections,
        content,
        empty,
        details,
        details_scroll,
        downloads,
        search,
        filter_chips,
        count,
        sort_toggle,
        playable_toggle,
        filter_count,
        filter_button,
        clear_filters,
        favorite_filter,
        downloaded_filter,
        installed_filter,
        played_filter,
        unplayed_filter,
        windows_filter,
        linux_filter,
        macos_filter,
        cloud_saves_filter,
        achievements_filter,
        language_filter,
        language_options,
        genre_theme_filter_box,
        genre_theme_filter_label,
        game_mode_filter_box,
        game_mode_filter_label,
        property_filter_box,
        property_filter_label,
        property_filter_search,
        property_filter_chips,
        account_button_name,
        header_network_icon,
        header_network_slash,
        header_network_button,
        account_offline_indicator,
        account_button_avatar,
        account_avatar,
        account_name,
        account_details,
        account_library_status,
        account_connection_status,
        sign_in,
        reconnect,
        sign_out,
        account_popover,
    }
}

pub(super) fn connect_actions(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    store: &Rc<StateStore>,
) {
    {
        let model = model.clone();
        w.window.connect_close_request(move |window| {
            let maximized = window.is_maximized();
            let mut state = model.borrow_mut();
            state.config.window_maximized = maximized;
            if !maximized {
                state.config.window_width = window.width().max(1100);
                state.config.window_height = window.height().max(600);
            }
            if let Err(error) = state.config.save() {
                tracing::error!(%error, "saving window geometry");
            }
            if tray::should_hide_on_close() {
                window.set_visible(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    w.sort_toggle
        .set_active(model.borrow().sidebar_sort_mode == SidebarSortMode::LastPlayed);
    update_sort_toggle(w, model.borrow().sidebar_sort_mode);
    {
        let widgets = w.clone_refs();
        let model = model.clone();
        let toggle = w.sort_toggle.clone();
        toggle.connect_toggled(move |button| {
            let mode = if button.is_active() {
                SidebarSortMode::LastPlayed
            } else {
                SidebarSortMode::Alphabetical
            };
            {
                let mut state = model.borrow_mut();
                state.sidebar_sort_mode = mode;
                state.config.sidebar_sort_mode = mode;
                if let Err(error) = state.config.save() {
                    tracing::error!(%error, "saving sidebar sort mode");
                }
                rebuild_sidebar_presentation(&widgets, &mut state);
            }
            update_sort_toggle(&widgets, mode);
        });
    }
    {
        let widgets = w.clone_refs();
        let model = model.clone();
        w.playable_toggle.connect_toggled(move |button| {
            model.borrow_mut().sidebar_playable_only = button.is_active();
            if button.is_active() {
                button.add_css_class("sidebar-icon-toggle-active");
            } else {
                button.remove_css_class("sidebar-icon-toggle-active");
            }
            refresh_filters(&widgets, &model.borrow());
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let button = w.sign_in.clone();
        button.connect_clicked(move |_| {
            w.account_popover.popdown();
            show_gog_login(&w, &model);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let button = w.reconnect.clone();
        button.connect_clicked(move |_| {
            w.account_popover.popdown();
            show_gog_login(&w, &model);
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let button = w.header_network_button.clone();
        button.connect_clicked(move |_| {
            let authenticated = model
                .borrow()
                .account_token
                .as_ref()
                .is_some_and(|token| token.expires_at > chrono::Utc::now().timestamp());
            if !authenticated && model.borrow().network_available {
                show_gog_login(&w, &model);
            }
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let button = w.sign_out.clone();
        button.connect_clicked(move |_| {
            if let Err(error) = auth::logout() {
                show_status(&w, &format!("Could not sign out: {error}"));
                return;
            }
            let mut state = model.borrow_mut();
            state.account_profile = None;
            state.account_token = None;
            drop(state);
            download::set_authenticated(false);
            update_header_network_indicator(&w, &model.borrow());
            if let Ok(store) = StateStore::open() {
                let _ = store.clear_cached_profile();
            }
            update_account_widgets(&w, None);
            update_account_library_status(&w, &model.borrow());
            w.account_popover.popdown();
            show_status(&w, "Signed out of GOG");
        });
    }
    {
        let model = model.clone();
        w.game_list.set_filter_func(move |row| {
            // Gtk invokes this synchronously while rows are inserted. A sidebar
            // presentation rebuild can still hold the model's mutable borrow at
            // that point; the deferred invalidation below applies the real state.
            model
                .try_borrow()
                .map_or(true, |state| sidebar_row_visible(&state, row))
        });
    }
    {
        let model = model.clone();
        w.home_grid.set_filter_func(move |child| {
            let Some(id) = child
                .child()
                .and_then(|card| card.widget_name().parse::<i64>().ok())
            else {
                return false;
            };
            // FlowBox invalidation is synchronous and may be triggered while a
            // sidebar callback is updating session-only presentation state.
            model
                .try_borrow()
                .map_or(true, |state| game_matches_library_filters(&state, id))
        });
    }
    let home_action = gio::SimpleAction::new("home", None);
    {
        let w = w.clone();
        home_action.connect_activate(move |_, _| w.content.set_visible_child_name("home"));
    }

    connect_check_filter(w, model, &w.favorite_filter, |m, active| {
        m.favorites_only = active
    });
    connect_check_filter(w, model, &w.downloaded_filter, |m, active| {
        m.downloaded_only = active
    });
    connect_check_filter(w, model, &w.installed_filter, |m, active| {
        m.installed_only = active
    });
    connect_check_filter(w, model, &w.played_filter, |m, active| {
        m.played_only = active
    });
    connect_check_filter(w, model, &w.unplayed_filter, |m, active| {
        m.unplayed_only = active
    });
    connect_check_filter(w, model, &w.windows_filter, |m, active| {
        m.windows_only = active
    });
    connect_check_filter(w, model, &w.linux_filter, |m, active| m.linux_only = active);
    connect_check_filter(w, model, &w.macos_filter, |m, active| m.macos_only = active);
    connect_check_filter(w, model, &w.cloud_saves_filter, |m, active| {
        m.cloud_saves_only = active
    });
    connect_check_filter(w, model, &w.achievements_filter, |m, active| {
        m.achievements_only = active
    });
    {
        let w = w.clone();
        let model = model.clone();
        let selector = w.language_filter.clone();
        let options = w.language_options.clone();
        selector.connect_selected_notify(move |selector| {
            let selected = selector.selected();
            {
                let mut state = model.borrow_mut();
                state.language_filter = if selected == 0 {
                    None
                } else {
                    options.string(selected).map(|value| value.to_string())
                };
                if state.language_filter.is_some() {
                    state.query.clear();
                }
            }
            if selected != 0 && !w.search.text().is_empty() {
                w.search.set_text("");
            }
            refresh_filters(&w, &model.borrow());
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        let button = w.clear_filters.clone();
        button.connect_clicked(move |_| {
            {
                let mut model = model.borrow_mut();
                model.favorites_only = false;
                model.downloaded_only = false;
                model.installed_only = false;
                model.played_only = false;
                model.unplayed_only = false;
                model.windows_only = false;
                model.linux_only = false;
                model.macos_only = false;
                model.cloud_saves_only = false;
                model.achievements_only = false;
                model.language_filter = None;
                model.genre_theme_filters.clear();
                model.game_mode_filters.clear();
                model.property_filters.clear();
            }
            w.favorite_filter.set_active(false);
            w.downloaded_filter.set_active(false);
            w.installed_filter.set_active(false);
            w.played_filter.set_active(false);
            w.unplayed_filter.set_active(false);
            w.windows_filter.set_active(false);
            w.linux_filter.set_active(false);
            w.macos_filter.set_active(false);
            w.cloud_saves_filter.set_active(false);
            w.achievements_filter.set_active(false);
            w.language_filter.set_selected(0);
            update_metadata_filter_options(&w, &model);
            refresh_filters(&w, &model.borrow());
        });
    }
    w.window.add_action(&home_action);

    let remove_filter_action = gio::SimpleAction::new(
        "remove-library-filter",
        Some(&String::static_variant_type()),
    );
    {
        let w = w.clone();
        let model = model.clone();
        remove_filter_action.connect_activate(move |_, value| {
            let Some(key) = value.and_then(|value| value.get::<String>()) else {
                return;
            };
            {
                let mut state = model.borrow_mut();
                match key.as_str() {
                    "favorite" => state.favorites_only = false,
                    "downloaded" => state.downloaded_only = false,
                    "installed" => state.installed_only = false,
                    "played" => state.played_only = false,
                    "unplayed" => state.unplayed_only = false,
                    "windows" => state.windows_only = false,
                    "linux" => state.linux_only = false,
                    "macos" => state.macos_only = false,
                    "cloud" => state.cloud_saves_only = false,
                    "achievements" => state.achievements_only = false,
                    "language" => state.language_filter = None,
                    _ if key.starts_with("genre:") => {
                        state.genre_theme_filters.remove(&key[6..]);
                    }
                    _ if key.starts_with("mode:") => {
                        state.game_mode_filters.remove(&key[5..]);
                    }
                    _ if key.starts_with("property:") => {
                        state.property_filters.remove(&key[9..]);
                    }
                    _ => return,
                }
            }
            let flags = {
                let state = model.borrow();
                (
                    state.favorites_only,
                    state.downloaded_only,
                    state.installed_only,
                    state.played_only,
                    state.unplayed_only,
                    state.windows_only,
                    state.linux_only,
                    state.macos_only,
                    state.cloud_saves_only,
                    state.achievements_only,
                    state.language_filter.is_none(),
                )
            };
            w.favorite_filter.set_active(flags.0);
            w.downloaded_filter.set_active(flags.1);
            w.installed_filter.set_active(flags.2);
            w.played_filter.set_active(flags.3);
            w.unplayed_filter.set_active(flags.4);
            w.windows_filter.set_active(flags.5);
            w.linux_filter.set_active(flags.6);
            w.macos_filter.set_active(flags.7);
            w.cloud_saves_filter.set_active(flags.8);
            w.achievements_filter.set_active(flags.9);
            if flags.10 {
                w.language_filter.set_selected(0);
            }
            update_metadata_filter_options(&w, &model);
            refresh_filters(&w, &model.borrow());
        });
    }
    w.window.add_action(&remove_filter_action);

    let collections_action = gio::SimpleAction::new("collections", None);
    {
        let w = w.clone();
        let model = model.clone();
        collections_action.connect_activate(move |_, _| {
            rebuild_collections_index(&w, &model);
            w.content.set_visible_child_name("collections");
        });
    }
    w.window.add_action(&collections_action);

    let downloads_action = gio::SimpleAction::new("downloads", None);
    {
        let w = w.clone();
        let model = model.clone();
        downloads_action.connect_activate(move |_, _| {
            rebuild_downloads_page(&w, &model.borrow());
            w.content.set_visible_child_name("downloads");
        });
    }
    w.window.add_action(&downloads_action);

    let refresh_action = gio::SimpleAction::new("refresh", None);
    {
        let w = w.clone();
        let model = model.clone();
        refresh_action.connect_activate(move |_, _| {
            let summary = reconcile_managed_directory(&mut model.borrow_mut());
            tracing::info!(%summary, "managed download directory reconciled");
            if let Some(token) = model.borrow().account_token.clone() {
                start_owned_library_sync(&w, &model, token, true, true);
            } else {
                rebuild_library(&w, &model);
                show_status(
                    &w,
                    "Managed downloads refreshed; sign in to synchronize GOG",
                );
            }
        });
    }
    w.window.add_action(&refresh_action);
    w.window
        .application()
        .unwrap()
        .set_accels_for_action("win.refresh", &["<Control>r"]);

    let refresh_files_action =
        gio::SimpleAction::new("refresh-files", Some(&i64::static_variant_type()));
    {
        let w = w.clone();
        let model = model.clone();
        refresh_files_action.connect_activate(move |_, value| {
            let Some(target_id) = value.and_then(|value| value.get::<i64>()) else {
                return;
            };
            start_product_file_refresh(&w, &model, target_id);
        });
    }
    w.window.add_action(&refresh_files_action);

    let settings_action = gio::SimpleAction::new("settings", None);
    {
        let w = w.clone();
        let model = model.clone();
        settings_action.connect_activate(move |_, _| show_settings(&w, &model));
    }
    w.window.add_action(&settings_action);
    let settings_page_action =
        gio::SimpleAction::new("settings-page", Some(&String::static_variant_type()));
    {
        let w = w.clone();
        let model = model.clone();
        settings_page_action.connect_activate(move |_, value| {
            let page = value
                .and_then(|value| value.get::<String>())
                .unwrap_or_else(|| "account".into());
            show_settings_page(&w, &model, &page);
        });
    }
    w.window.add_action(&settings_page_action);
    w.window
        .application()
        .unwrap()
        .set_accels_for_action("win.settings", &["<Control>comma"]);

    {
        let w = w.clone();
        let model = model.clone();
        let search = w.search.clone();
        search.connect_search_changed(move |entry| {
            model.borrow_mut().query = entry.text().to_string();
            refresh_filters(&w, &model.borrow());
        });
    }
    w.window
        .application()
        .unwrap()
        .set_accels_for_action("win.home", &["<Alt>Home"]);

    {
        let w = w.clone();
        let model = model.clone();
        let game_list = w.game_list.clone();
        game_list.connect_row_activated(move |_, row| {
            if let Some(key) = section_from_name(&row.widget_name()) {
                {
                    let mut state = model.borrow_mut();
                    if !state.collapsed_activity_sections.insert(key) {
                        state.collapsed_activity_sections.remove(&key);
                    }
                }
                refresh_filters(&w, &model.borrow());
                return;
            }
            let index = row.widget_name().parse::<i64>().ok();
            if let Some(id) = index {
                show_game(&w, &model, id, None);
            }
        });
    }

    let favorite_action = gio::SimpleAction::new("favorite", Some(&i64::static_variant_type()));
    {
        let w = w.clone();
        let model = model.clone();
        let store = store.clone();
        favorite_action.connect_activate(move |_, value| {
            let Some(id) = value.and_then(|v| v.get::<i64>()) else {
                return;
            };
            let favorite = {
                let mut m = model.borrow_mut();
                if m.favorites.contains(&id) {
                    m.favorites.remove(&id);
                    false
                } else {
                    m.favorites.insert(id);
                    true
                }
            };
            if let Err(error) = store.set_favorite(id, favorite) {
                tracing::error!(%error, "saving favorite");
            }
            update_favorite_widgets(&w, &model.borrow(), id, favorite);
            refresh_filters(&w, &model.borrow());
            if model.borrow().selected == Some(id)
                && w.content.visible_child_name().as_deref() == Some("details")
            {
                show_game(&w, &model, id, Some(favorite));
            }
        });
    }
    w.window.add_action(&favorite_action);
}

fn update_sort_toggle(w: &Widgets, mode: SidebarSortMode) {
    let (tooltip, label) = match mode {
        SidebarSortMode::Alphabetical => ("Sort by last activity", "Sort by last activity"),
        SidebarSortMode::LastPlayed => ("Sort alphabetically", "Sort alphabetically"),
    };
    w.sort_toggle.set_tooltip_text(Some(tooltip));
    w.sort_toggle
        .update_property(&[gtk::accessible::Property::Label(label)]);
    if mode == SidebarSortMode::LastPlayed {
        w.sort_toggle.add_css_class("sidebar-icon-toggle-active");
    } else {
        w.sort_toggle.remove_css_class("sidebar-icon-toggle-active");
    }
}

fn sidebar_game_is_playable(
    game: &crate::domain::InstalledGame,
    libraries: &[crate::config::GameLibrary],
) -> bool {
    game.state == crate::domain::InstallationState::Installed
        && crate::installation::resolve_installation_directory(game, libraries)
            .is_some_and(|(_, directory)| directory.is_dir())
        && game
            .primary_executable
            .as_ref()
            .is_some_and(|path| path.is_file())
        && crate::installation::installation_operation_snapshot(game.product_id).is_none()
}

fn schedule_activity_rollover(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    use chrono::{Datelike, Local, TimeZone};
    let now = Local::now();
    let tomorrow = now.date_naive().succ_opt().unwrap();
    let next_midnight = Local
        .with_ymd_and_hms(tomorrow.year(), tomorrow.month(), tomorrow.day(), 0, 0, 0)
        .earliest()
        .unwrap_or_else(|| now + chrono::Duration::hours(24));
    let delay = (next_midnight - now)
        .to_std()
        .unwrap_or(Duration::from_secs(3600));
    let widgets = w.clone();
    let model = model.clone();
    glib::timeout_add_local_once(delay, move || {
        if model.borrow().sidebar_sort_mode == SidebarSortMode::LastPlayed {
            rebuild_sidebar_presentation(&widgets, &mut model.borrow_mut());
        }
        schedule_activity_rollover(&widgets, &model);
    });
}
