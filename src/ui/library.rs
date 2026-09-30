use super::*;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone};

fn section_name(key: ActivitySectionKey) -> String {
    match key {
        ActivitySectionKey::Recent => "activity:recent".into(),
        ActivitySectionKey::Month { year, month } => format!("activity:month:{year}:{month}"),
        ActivitySectionKey::Year(year) => format!("activity:year:{year}"),
        ActivitySectionKey::NeverPlayed => "activity:never".into(),
    }
}

pub(super) fn section_from_name(name: &str) -> Option<ActivitySectionKey> {
    let parts = name.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["activity", "recent"] => Some(ActivitySectionKey::Recent),
        ["activity", "never"] => Some(ActivitySectionKey::NeverPlayed),
        ["activity", "year", year] => year.parse().ok().map(ActivitySectionKey::Year),
        ["activity", "month", year, month] => Some(ActivitySectionKey::Month {
            year: year.parse().ok()?,
            month: month.parse().ok()?,
        }),
        _ => None,
    }
}

fn activity_key<T: TimeZone>(timestamp: Option<i64>, now: &DateTime<T>) -> ActivitySectionKey {
    let Some(timestamp) = timestamp else {
        return ActivitySectionKey::NeverPlayed;
    };
    let Some(played) = now.timezone().timestamp_opt(timestamp, 0).single() else {
        return ActivitySectionKey::NeverPlayed;
    };
    if played > *now {
        tracing::warn!(
            timestamp,
            "future last-played timestamp; placing game in Recent"
        );
    }
    if played > *now
        || (played.year() == now.year() && played.month() == now.month())
        || played >= now.clone() - ChronoDuration::days(7)
    {
        ActivitySectionKey::Recent
    } else if played.year() == now.year() {
        ActivitySectionKey::Month {
            year: played.year(),
            month: played.month(),
        }
    } else {
        ActivitySectionKey::Year(played.year())
    }
}

fn build_activity_sections(model: &AppModel, now: DateTime<Local>) -> Vec<SidebarSection> {
    let mut entries = model
        .games
        .iter()
        .map(|game| {
            let activity = model
                .product_activity
                .get(&game.product_id)
                .copied()
                .unwrap_or_default();
            SidebarGameEntry {
                product_id: game.product_id,
                normalized_title: game.title.to_lowercase(),
                activity,
                section: activity_key(activity.last_activity_at, &now),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let section_order = |key| match key {
            ActivitySectionKey::Recent => (0, 0, 0),
            ActivitySectionKey::Month { year, month } => (1, -year, -(month as i32)),
            ActivitySectionKey::Year(year) => (2, -year, 0),
            ActivitySectionKey::NeverPlayed => (3, 0, 0),
        };
        section_order(a.section)
            .cmp(&section_order(b.section))
            .then_with(|| match a.section {
                ActivitySectionKey::NeverPlayed => a.normalized_title.cmp(&b.normalized_title),
                _ => b
                    .activity
                    .last_activity_at
                    .cmp(&a.activity.last_activity_at)
                    .then_with(|| a.normalized_title.cmp(&b.normalized_title)),
            })
            .then(a.product_id.cmp(&b.product_id))
    });
    let mut sections = Vec::<SidebarSection>::new();
    for entry in entries {
        if sections
            .last()
            .is_none_or(|section| section.key != entry.section)
        {
            let label = match entry.section {
                ActivitySectionKey::Recent => "RECENT".into(),
                ActivitySectionKey::Month { year, month } => Local
                    .with_ymd_and_hms(year, month, 1, 12, 0, 0)
                    .single()
                    .map(|date| date.format("%B").to_string().to_uppercase())
                    .unwrap_or_else(|| month.to_string()),
                ActivitySectionKey::Year(year) => year.to_string(),
                ActivitySectionKey::NeverPlayed => "NEVER PLAYED".into(),
            };
            sections.push(SidebarSection {
                key: entry.section,
                label,
                members: Vec::new(),
            });
        }
        sections.last_mut().unwrap().members.push(entry.product_id);
    }
    sections
}

fn activity_section_row(section: &SidebarSection) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(&section_name(section.key));
    row.add_css_class("activity-section-row");
    row.set_selectable(false);
    row.set_activatable(true);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    content.set_margin_start(9);
    content.set_margin_end(8);
    let disclosure = gtk::Label::new(Some("−"));
    disclosure.set_widget_name("activity-disclosure");
    disclosure.add_css_class("activity-section-disclosure");
    let title = gtk::Label::new(Some(&section.label));
    title.add_css_class("activity-section-title");
    let count = gtk::Label::new(None);
    count.set_widget_name("activity-count");
    count.add_css_class("activity-section-count");
    content.append(&disclosure);
    content.append(&title);
    content.append(&count);
    row.set_child(Some(&content));
    row
}

pub(super) fn rebuild_sidebar_presentation(w: &Widgets, model: &mut AppModel) {
    let mut rows = HashMap::new();
    let mut child = w.game_list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(id) = widget.widget_name().parse::<i64>()
            && let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
        {
            rows.insert(id, row);
        }
    }
    while let Some(child) = w.game_list.first_child() {
        w.game_list.remove(&child);
    }
    match model.sidebar_sort_mode {
        SidebarSortMode::Alphabetical => {
            let mut games = model
                .games
                .iter()
                .map(|game| (game.title.to_lowercase(), game.product_id))
                .collect::<Vec<_>>();
            games.sort();
            for (_, id) in games {
                if let Some(row) = rows.remove(&id) {
                    w.game_list.append(&row);
                }
            }
        }
        SidebarSortMode::LastPlayed => {
            model.activity_sections = build_activity_sections(model, Local::now());
            for section in &model.activity_sections {
                w.game_list.append(&activity_section_row(section));
                for id in &section.members {
                    if let Some(row) = rows.remove(id) {
                        w.game_list.append(&row);
                    }
                }
            }
        }
    }
    refresh_sidebar_visibility(w, model);
    if let Some(selected) = model.selected
        && let Some(row) = find_list_row(w, &selected.to_string())
        && sidebar_row_visible(model, &row)
    {
        w.game_list.select_row(Some(&row));
    }
    // Appending a ListBoxRow synchronously runs Gtk's filter callback. Defer the
    // authoritative pass until callers have released their AppModel borrow.
    let game_list = w.game_list.clone();
    glib::idle_add_local_once(move || game_list.invalidate_filter());
}

fn refresh_sidebar_visibility(w: &Widgets, model: &AppModel) {
    for section in &model.activity_sections {
        let matching = section
            .members
            .iter()
            .filter(|id| game_matches_sidebar(model, **id))
            .count();
        if let Some(row) = find_list_row(w, &section_name(section.key)) {
            row.set_visible(model.sidebar_sort_mode == SidebarSortMode::LastPlayed && matching > 0);
            if let Some(label) = find_named_descendant(&row.clone().upcast(), "activity-count")
                .and_downcast::<gtk::Label>()
            {
                label.set_label(&format!("({matching})"));
            }
            let collapsed = model.collapsed_activity_sections.contains(&section.key);
            let state = if collapsed { "collapsed" } else { "expanded" };
            let accessible = format!("{}, {matching} games, {state}", section.label);
            row.update_property(&[gtk::accessible::Property::Label(&accessible)]);
            row.update_state(&[gtk::accessible::State::Expanded(Some(!collapsed))]);
            if let Some(label) = find_named_descendant(&row.upcast(), "activity-disclosure")
                .and_downcast::<gtk::Label>()
            {
                label.set_label(if collapsed { "+" } else { "−" });
            }
        }
    }
    w.game_list.invalidate_filter();
}

fn find_list_row(w: &Widgets, name: &str) -> Option<gtk::ListBoxRow> {
    let mut child = w.game_list.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == name {
            return widget.downcast().ok();
        }
        child = widget.next_sibling();
    }
    None
}

pub(super) fn connect_check_filter(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    button: &gtk::CheckButton,
    update: impl Fn(&mut AppModel, bool) + 'static,
) {
    let w = w.clone();
    let model = model.clone();
    button.connect_toggled(move |button| {
        let active = button.is_active();
        {
            let mut state = model.borrow_mut();
            update(&mut state, active);
            if active {
                state.query.clear();
            }
        }
        if active && !w.search.text().is_empty() {
            w.search.set_text("");
        }
        refresh_filters(&w, &model.borrow());
        if active && (model.borrow().cloud_saves_only || model.borrow().achievements_only) {
            request_filter_metadata(&w, &model, false);
        }
    });
}

pub(super) fn request_filter_metadata(w: &Widgets, model: &Rc<RefCell<AppModel>>, retry: bool) {
    let ids = model
        .borrow()
        .games
        .iter()
        .map(|game| game.product_id)
        .collect::<Vec<_>>();
    for id in ids {
        request_product_section(w, model, id, online::DetailSection::Metadata, retry);
    }
    refresh_filters(w, &model.borrow());
}

pub(super) fn initialize_library_loading(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    w.home_grid.set_sort_func(|left, right| {
        let title = |child: &gtk::FlowBoxChild| {
            find_named_descendant(child.upcast_ref(), "card-title")
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_lowercase())
                .unwrap_or_default()
        };
        title(left).cmp(&title(right)).into()
    });
    for (check, label, name) in [
        (&w.cloud_saves_filter, "Cloud saves", "cloud-filter-loading"),
        (
            &w.achievements_filter,
            "Achievements",
            "achievements-filter-loading",
        ),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        row.append(&gtk::Label::new(Some(label)));
        let spinner = gtk::Spinner::new();
        spinner.set_widget_name(name);
        spinner.set_size_request(14, 14);
        row.append(&spinner);
        check.set_child(Some(&row));
    }
    for (container, name) in [
        (
            w.genre_theme_filter_label
                .parent()
                .and_downcast::<gtk::Box>(),
            "genre-filter-loading",
        ),
        (
            w.game_mode_filter_label.parent().and_downcast::<gtk::Box>(),
            "modes-filter-loading",
        ),
        (
            w.property_filter_search.parent().and_downcast::<gtk::Box>(),
            "properties-filter-loading",
        ),
        (
            w.language_filter.parent().and_downcast::<gtk::Box>(),
            "language-filter-loading",
        ),
        (
            w.windows_filter.parent().and_downcast::<gtk::Box>(),
            "platform-filter-loading",
        ),
    ] {
        if let Some(container) = container {
            let spinner = gtk::Spinner::new();
            spinner.set_widget_name(name);
            spinner.set_size_request(14, 14);
            spinner.set_halign(gtk::Align::Start);
            container.append(&spinner);
        }
    }
    if let Some(popover) = w.filter_button.popover() {
        if let Some(panel) = w
            .property_filter_search
            .parent()
            .and_then(|section| section.parent())
            .and_then(|row| row.parent())
            .and_downcast::<gtk::Box>()
        {
            let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let label = gtk::Label::new(None);
            label.set_widget_name("filter-data-status");
            label.set_wrap(true);
            label.set_hexpand(true);
            label.set_xalign(0.0);
            status.append(&label);
            let retry = gtk::Button::with_label("Retry");
            retry.set_widget_name("filter-data-retry");
            let widgets = w.clone_refs();
            let state = model.clone();
            retry.connect_clicked(move |_| request_filter_metadata(&widgets, &state, true));
            status.append(&retry);
            panel.append(&status);
        }
        let widgets = w.clone_refs();
        let state = model.clone();
        popover.connect_show(move |_| request_filter_metadata(&widgets, &state, false));
    }
    if let Some(scroll) = w
        .home_grid
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    {
        let widgets = w.clone_refs();
        scroll
            .vadjustment()
            .connect_value_changed(move |_| prioritize_grid_covers(&widgets));
        let widgets = w.clone_refs();
        scroll.connect_map(move |_| prioritize_grid_covers(&widgets));
    }
    refresh_filters(w, &model.borrow());
}

fn prioritize_grid_covers(w: &Widgets) {
    let Some(scroll) = w
        .home_grid
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    else {
        return;
    };
    let mut visible = Vec::new();
    let mut nearby = Vec::new();
    let mut child = w.home_grid.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widget.is_visible() {
            continue;
        }
        let Some(card) = widget.first_child() else {
            continue;
        };
        let Ok(id) = card.widget_name().parse::<i64>() else {
            continue;
        };
        let Some(bounds) = card.compute_bounds(&scroll) else {
            continue;
        };
        if bounds.y() + bounds.height() >= 0.0 && bounds.y() <= scroll.height() as f32 {
            visible.push(id);
        } else if bounds.y() + bounds.height() >= -(scroll.height() as f32)
            && bounds.y() <= 2.0 * scroll.height() as f32
        {
            nearby.push(id);
        }
    }
    visible.extend(nearby);
    online::prioritize_covers(visible);
}

pub(super) fn rebuild_library(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    static REBUILD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let rebuild = REBUILD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let ids = model
        .borrow()
        .games
        .iter()
        .map(|game| game.product_id)
        .collect::<HashSet<_>>();
    let mut existing = HashSet::new();
    let mut child = w.game_list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(id) = widget.widget_name().parse::<i64>() {
            if ids.contains(&id) {
                existing.insert(id);
                if let Some(game) = model
                    .borrow()
                    .games
                    .iter()
                    .find(|game| game.product_id == id)
                    && let Some(title) = find_named_descendant(&widget, "sidebar-game-title")
                        .and_downcast::<gtk::Label>()
                {
                    title.set_label(&game.title);
                }
            } else {
                w.game_list.remove(&widget);
            }
        }
    }
    let mut child = w.home_grid.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        let Some(card) = widget.first_child() else {
            continue;
        };
        if let Ok(id) = card.widget_name().parse::<i64>() {
            if !ids.contains(&id) {
                w.home_grid.remove(&widget);
            } else if let Some(game) = model
                .borrow()
                .games
                .iter()
                .find(|game| game.product_id == id)
                && let Some(title) =
                    find_named_descendant(&card, "card-title").and_downcast::<gtk::Label>()
            {
                title.set_label(&game.title);
            }
        }
    }
    update_language_options(w, &model.borrow());
    update_metadata_filter_options(w, model);
    let m = model.borrow();
    let empty_message = if m.account_profile.is_none() {
        "Sign in to GOG to load your library."
    } else if !m.network_available && m.online_synced_at.is_none() {
        "Connect to GOG once to download your library metadata."
    } else if m.online_synced_at.is_none() {
        "Refresh your GOG library to load owned games."
    } else {
        "No installable games were found on this GOG account."
    };
    let empty = m.games.is_empty();
    let games = Rc::new(RefCell::new(
        m.games
            .iter()
            .filter(|game| !existing.contains(&game.product_id))
            .cloned()
            .collect::<std::collections::VecDeque<_>>(),
    ));
    let favorites = m.favorites.clone();
    let card_width = m.card_width;
    drop(m);
    // Changing the stack emits a synchronous notification that updates the model.
    w.empty.set_description(Some(empty_message));
    if empty {
        if matches!(
            w.content.visible_child_name().as_deref(),
            Some("home" | "empty")
        ) {
            w.content.set_visible_child_name("empty");
        }
    } else if w.content.visible_child_name().as_deref() == Some("empty") {
        w.content.set_visible_child_name("home");
    }
    let w = w.clone_refs();
    let model = model.clone();
    glib::idle_add_local(move || {
        if rebuild != REBUILD.load(std::sync::atomic::Ordering::Relaxed) {
            return glib::ControlFlow::Break;
        }
        for _ in 0..4 {
            let Some(game) = games.borrow_mut().pop_front() else {
                update_sidebar_download_styles(&w, &model.borrow());
                rebuild_sidebar_presentation(&w, &mut model.borrow_mut());
                refresh_filters(&w, &model.borrow());
                w.home_grid.invalidate_sort();
                prioritize_grid_covers(&w);
                if w.filter_button
                    .popover()
                    .is_some_and(|popover| popover.is_visible())
                {
                    request_filter_metadata(&w, &model, false);
                }
                return glib::ControlFlow::Break;
            };
            let game = model
                .borrow()
                .games
                .iter()
                .find(|current| current.product_id == game.product_id)
                .cloned();
            let Some(game) = game else {
                continue;
            };
            let favorite = favorites.contains(&game.product_id);
            let row = game_row(
                &game,
                favorite,
                model.borrow().config.show_sidebar_game_icons,
            );
            row.set_widget_name(&game.product_id.to_string());
            let context_click = gtk::GestureClick::new();
            context_click.set_button(gtk::gdk::BUTTON_SECONDARY);
            {
                let widgets = w.clone_refs();
                let model = model.clone();
                let game = game.clone();
                let row = row.clone();
                context_click.connect_pressed(move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    let state = model.borrow();
                    let Some(game) = state
                        .games
                        .iter()
                        .find(|current| current.product_id == game.product_id)
                        .cloned()
                    else {
                        return;
                    };
                    let favorite = state.favorites.contains(&game.product_id);
                    let detail = DetailPageModel::game(game.clone(), favorite);
                    let installed = state.installed_games.get(&game.product_id).cloned();
                    drop(state);
                    let action_id = game.product_id;
                    let management = detail_file_management(
                        &detail,
                        &widgets.window,
                        &model,
                        installed,
                        {
                            let widgets = widgets.clone_refs();
                            let model = model.clone();
                            Rc::new(move || refresh_local_action_state(&widgets, &model))
                        },
                        {
                            let widgets = widgets.clone_refs();
                            let model = model.clone();
                            Rc::new(move || {
                                let state = model.borrow();
                                let Some(game) = state
                                    .games
                                    .iter()
                                    .find(|game| game.product_id == action_id)
                                    .cloned()
                                else {
                                    return;
                                };
                                let favorite = state.favorites.contains(&action_id);
                                drop(state);
                                activate_context_primary_action(
                                    &widgets,
                                    &model,
                                    DetailPageModel::game(game, favorite),
                                );
                            })
                        },
                    );
                    let Some(popover) = management.menu.popover() else {
                        return;
                    };
                    management.menu.set_popover(gtk::Popover::NONE);
                    popover.set_parent(&row);
                    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                        x.round() as i32,
                        y.round() as i32,
                        1,
                        1,
                    )));
                    popover.connect_closed(|popover| popover.unparent());
                    popover.popup();
                });
            }
            row.add_controller(context_click);
            w.game_list.append(&row);
            let card = game_card(&game, favorite, card_width);
            apply_card_cover_state(
                card.upcast_ref(),
                model.borrow().cover_states.get(&game.product_id),
            );
            card.set_widget_name(&game.product_id.to_string());
            let id = game.product_id;
            let w2 = w.clone_refs();
            let model = model.clone();
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, _, _| show_game(&w2, &model, id, None));
            card.add_controller(click);
            w.home_grid.insert(&card, -1);
        }
        glib::ControlFlow::Continue
    });
}

pub(super) fn rebuild_home_grid(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    while let Some(child) = w.home_grid.first_child() {
        w.home_grid.remove(&child);
    }
    let state = model.borrow();
    let games = state.games.clone();
    let favorites = state.favorites.clone();
    let card_width = state.card_width;
    drop(state);
    for game in games {
        let card = game_card(&game, favorites.contains(&game.product_id), card_width);
        apply_card_cover_state(
            card.upcast_ref(),
            model.borrow().cover_states.get(&game.product_id),
        );
        card.set_widget_name(&game.product_id.to_string());
        let id = game.product_id;
        let widgets = w.clone_refs();
        let model = model.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| show_game(&widgets, &model, id, None));
        card.add_controller(click);
        w.home_grid.insert(&card, -1);
    }
    refresh_filters(w, &model.borrow());
}

pub(super) fn update_sidebar_download_styles(w: &Widgets, model: &AppModel) {
    let titles = model
        .games
        .iter()
        .map(|game| (game.product_id, game.title.clone()))
        .collect();
    let coverage = model
        .local_actions
        .iter()
        .map(|(&id, state)| (id, state.coverage))
        .collect();
    let required = model
        .local_actions
        .iter()
        .map(|(&id, state)| (id, state.required_dlcs.clone()))
        .collect();
    let dlcs = model
        .local_actions
        .iter()
        .map(|(&id, state)| (id, state.installed_dlcs.clone()))
        .collect();
    let dlc_updates = model
        .local_actions
        .iter()
        .map(|(&id, state)| (id, state.dlc_updates.clone()))
        .collect();
    let updates = model
        .local_actions
        .iter()
        .filter(|(_, state)| state.installed_update)
        .map(|(&id, _)| id)
        .collect();
    let installed = model.installed_games.values().cloned().collect::<Vec<_>>();
    let running = model
        .games
        .iter()
        .filter(|game| crate::installation::is_game_running(game.product_id))
        .map(|game| game.product_id)
        .collect();
    let downloading_products = model
        .download_jobs
        .iter()
        .filter(|job| job.state == DownloadState::Downloading)
        .map(|job| job.product_id)
        .chain(
            model
                .depot_operations
                .iter()
                .filter(|operation| {
                    matches!(
                        operation.state.as_str(),
                        "downloading" | "materializing" | "dependencies"
                    )
                })
                .map(|operation| operation.product_id),
        )
        .collect::<HashSet<_>>();
    let downloading = owning_game_ids(model, &downloading_products);
    apply_sidebar_download_styles(
        w,
        &coverage,
        &required,
        &titles,
        SidebarInstallationSnapshot {
            installed: &installed,
            updates: &updates,
            dlcs: &dlcs,
            dlc_updates: &dlc_updates,
            running: &running,
            downloading: &downloading,
        },
        model.config.show_backup_status,
    );
}

struct SidebarInstallationSnapshot<'a> {
    installed: &'a [crate::domain::InstalledGame],
    updates: &'a HashSet<i64>,
    dlcs: &'a HashMap<i64, HashSet<i64>>,
    dlc_updates: &'a HashMap<i64, HashSet<i64>>,
    running: &'a HashSet<i64>,
    downloading: &'a HashSet<i64>,
}

fn sidebar_state_class(running: bool, downloading: bool, installed: bool) -> &'static str {
    if running {
        "game-state-running"
    } else if downloading {
        "game-state-downloading"
    } else if installed {
        "game-state-installed"
    } else {
        "game-state-unavailable"
    }
}

#[test]
fn sidebar_color_priority_tracks_activity_before_installedness() {
    for installed in [false, true] {
        for downloading in [false, true] {
            assert_eq!(
                sidebar_state_class(true, downloading, installed),
                "game-state-running"
            );
        }
        assert_eq!(
            sidebar_state_class(false, true, installed),
            "game-state-downloading"
        );
    }
    assert_eq!(
        sidebar_state_class(false, false, true),
        "game-state-installed"
    );
    assert_eq!(
        sidebar_state_class(false, false, false),
        "game-state-unavailable"
    );
}

fn apply_sidebar_download_styles(
    w: &Widgets,
    coverage: &HashMap<i64, InstallerCoverage>,
    required_dlcs: &HashMap<i64, HashSet<i64>>,
    titles: &HashMap<i64, String>,
    installation_state: SidebarInstallationSnapshot<'_>,
    show_backup_status: bool,
) {
    let mut row = w.game_list.first_child();
    while let Some(widget) = row {
        if let Ok(id) = widget.widget_name().parse::<i64>() {
            let coverage = coverage.get(&id).copied().unwrap_or_default();
            for class in [
                "game-state-running",
                "game-state-downloading",
                "game-state-installed",
                "game-state-update",
                "game-state-backup",
                "game-state-partial-backup",
                "game-state-unavailable",
                "game-state-pending",
            ] {
                widget.remove_css_class(class);
            }
            let active_operation =
                crate::installation::installation_operation_snapshot(id).filter(|snapshot| {
                    snapshot.queued
                        || matches!(
                            snapshot.state,
                            crate::domain::InstallationState::Installing
                                | crate::domain::InstallationState::Uninstalling
                        )
                });
            let installation = installation_state.installed.iter().find(|game| {
                game.product_id == id
                    && matches!(
                        game.state,
                        crate::domain::InstallationState::Installed
                            | crate::domain::InstallationState::UninstallFailed
                    )
            });
            let missing_installed_dlc = installation.is_some()
                && required_dlcs.get(&id).is_some_and(|required| {
                    !required.is_subset(installation_state.dlcs.get(&id).unwrap_or(&HashSet::new()))
                });
            let tooltip = if let Some(operation) = active_operation.as_ref() {
                if operation.queued {
                    "Installation queued"
                } else if operation.state == crate::domain::InstallationState::Uninstalling {
                    "Uninstalling"
                } else {
                    "Installing"
                }
            } else {
                match installation.map(|game| game.state) {
                    Some(crate::domain::InstallationState::Installed) => {
                        if installation_state.updates.contains(&id)
                            || missing_installed_dlc
                            || installation_state
                                .dlc_updates
                                .get(&id)
                                .is_some_and(|updates| !updates.is_empty())
                            || (show_backup_status && coverage != InstallerCoverage::Complete)
                        {
                            "Download or installation required"
                        } else {
                            if show_backup_status {
                                "Installed and fully backed up"
                            } else {
                                "Installed"
                            }
                        }
                    }
                    Some(crate::domain::InstallationState::UninstallFailed) => {
                        "Installation needs attention"
                    }
                    _ if show_backup_status && coverage == InstallerCoverage::Complete => {
                        "All preferred installers backed up · not installed"
                    }
                    _ if show_backup_status && coverage == InstallerCoverage::Partial => {
                        "Some preferred installers are missing"
                    }
                    _ => "Not installed · no installer backup",
                }
            };
            let running = installation_state.running.contains(&id);
            let downloading = installation_state.downloading.contains(&id);
            widget.add_css_class(sidebar_state_class(
                running,
                downloading,
                installation.is_some(),
            ));
            widget.set_opacity(1.0);
            widget.set_tooltip_text(Some(if running {
                "Running"
            } else if downloading {
                "Downloading"
            } else {
                tooltip
            }));
            if let (Some(base_title), Some(title)) = (
                titles.get(&id),
                find_named_descendant(&widget, "sidebar-game-title").and_downcast::<gtk::Label>(),
            ) {
                let suffix = active_operation.as_ref().map(|operation| {
                    if operation.queued {
                        "queued"
                    } else if operation.state == crate::domain::InstallationState::Uninstalling {
                        "Uninstalling"
                    } else {
                        "Installing"
                    }
                });
                title.set_label(&suffix.map_or_else(
                    || base_title.clone(),
                    |suffix| format!("{base_title} - {suffix}"),
                ));
            }
        }
        row = widget.next_sibling();
    }
}

pub(super) fn game_matches_library_filters(model: &AppModel, id: i64) -> bool {
    if (!model.show_hidden && model.hidden_products.contains(&id))
        || !organization::matches_tags(model.tags.get(&id), &model.tag_filters, model.tag_match_all)
    {
        return false;
    }
    let Some(game) = model.games.iter().find(|game| game.product_id == id) else {
        return false;
    };
    if model.favorites_only && !model.favorites.contains(&id) {
        return false;
    }
    if model.downloaded_only {
        let base_content = model.downloaded_products.contains(&game.product_id);
        let dlc_content = game
            .dlcs
            .iter()
            .any(|dlc| model.downloaded_products.contains(&dlc.product_id));
        if !base_content && !dlc_content {
            return false;
        }
    }
    if model.installed_only && !model.installed_products.contains(&id) {
        return false;
    }
    let played = model
        .product_activity
        .get(&id)
        .is_some_and(|activity| activity.last_played_at.is_some());
    if model.played_only && !model.unplayed_only && !played {
        return false;
    }
    if model.unplayed_only && !model.played_only && played {
        return false;
    }
    let has_os_filter = model.windows_only || model.linux_only || model.macos_only;
    if has_os_filter
        && !((model.windows_only && game.platforms.windows)
            || (model.linux_only && game.platforms.linux)
            || (model.macos_only && game.platforms.macos))
    {
        return false;
    }
    if let Some(language) = &model.language_filter
        && !game
            .languages
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(language))
    {
        return false;
    }
    if model.cloud_saves_only
        && metadata_ready(model, id)
        && !game
            .features
            .iter()
            .any(|feature| feature.to_lowercase().contains("cloud save"))
    {
        return false;
    }
    if model.achievements_only
        && metadata_ready(model, id)
        && !game
            .features
            .iter()
            .any(|feature| feature.to_lowercase().contains("achievement"))
    {
        return false;
    }
    if !model.genre_theme_filters.is_empty()
        && metadata_ready(model, id)
        && !game
            .metadata
            .genres
            .iter()
            .chain(&game.metadata.themes)
            .any(|term| {
                model
                    .genre_theme_filters
                    .iter()
                    .any(|selected| selected.eq_ignore_ascii_case(&term.name))
            })
    {
        return false;
    }
    if !model.game_mode_filters.is_empty()
        && metadata_ready(model, id)
        && !game.metadata.game_modes.iter().any(|term| {
            model
                .game_mode_filters
                .iter()
                .any(|selected| selected.eq_ignore_ascii_case(&term.name))
        })
    {
        return false;
    }
    if !model.property_filters.is_empty()
        && metadata_ready(model, id)
        && !game.metadata.properties.iter().any(|term| {
            model
                .property_filters
                .iter()
                .any(|selected| selected.eq_ignore_ascii_case(&term.name))
        })
    {
        return false;
    }
    let query = model.query.to_lowercase();
    query.is_empty()
        || game.title.to_lowercase().contains(&query)
        || game.slug.to_lowercase().contains(&query)
        || game
            .features
            .iter()
            .any(|feature| feature.to_lowercase().contains(&query))
        || game
            .metadata
            .tags
            .iter()
            .chain(&game.metadata.properties)
            .chain(&game.metadata.genres)
            .chain(&game.metadata.themes)
            .chain(&game.metadata.game_modes)
            .chain(&game.metadata.features)
            .any(|term| {
                term.name.to_lowercase().contains(&query)
                    || term.slug.to_lowercase().contains(&query)
            })
        || game
            .metadata
            .developers
            .iter()
            .chain(&game.metadata.publishers)
            .any(|company| company.name.to_lowercase().contains(&query))
        || game
            .metadata
            .series
            .as_ref()
            .is_some_and(|series| series.name.to_lowercase().contains(&query))
        || model
            .tags
            .get(&id)
            .is_some_and(|tags| tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
}

pub(super) fn game_matches_sidebar(model: &AppModel, id: i64) -> bool {
    game_matches_library_filters(model, id)
        && (!model.sidebar_playable_only || model.playable_products.contains(&id))
}

pub(super) fn sidebar_row_visible(model: &AppModel, row: &gtk::ListBoxRow) -> bool {
    if let Ok(id) = row.widget_name().parse::<i64>() {
        if !game_matches_sidebar(model, id) {
            return false;
        }
        if model.sidebar_sort_mode == SidebarSortMode::Alphabetical {
            return true;
        }
        return model
            .activity_sections
            .iter()
            .find(|section| section.members.contains(&id))
            .is_some_and(|section| !model.collapsed_activity_sections.contains(&section.key));
    }
    section_from_name(&row.widget_name()).is_some_and(|key| {
        model.sidebar_sort_mode == SidebarSortMode::LastPlayed
            && model
                .activity_sections
                .iter()
                .find(|section| section.key == key)
                .is_some_and(|section| {
                    section
                        .members
                        .iter()
                        .any(|id| game_matches_sidebar(model, *id))
                })
    })
}

pub(super) fn refresh_filters(w: &Widgets, model: &AppModel) {
    refresh_sidebar_visibility(w, model);
    w.home_grid.invalidate_filter();
    let count = model
        .games
        .iter()
        .filter(|game| game_matches_sidebar(model, game.product_id))
        .count();
    let incomplete = model
        .games
        .iter()
        .filter(|game| !metadata_ready(model, game.product_id))
        .count();
    let failed = model
        .games
        .iter()
        .filter(|game| {
            matches!(
                model
                    .section_states
                    .get(&(game.product_id, online::DetailSection::Metadata)),
                Some(SectionState::Failed(_))
            )
        })
        .count();
    let metadata_filter_active = model.cloud_saves_only
        || model.achievements_only
        || !model.genre_theme_filters.is_empty()
        || !model.game_mode_filters.is_empty()
        || !model.property_filters.is_empty();
    w.count
        .set_label(&if incomplete > 0 && metadata_filter_active {
            format!("{count} candidates · results incomplete")
        } else if incomplete > 0 && !model.query.is_empty() {
            format!("{count} games · metadata search incomplete")
        } else {
            format!("{count} games")
        });
    if let Some(popover) = w.filter_button.popover() {
        for name in [
            "cloud-filter-loading",
            "achievements-filter-loading",
            "genre-filter-loading",
            "modes-filter-loading",
            "properties-filter-loading",
            "language-filter-loading",
            "platform-filter-loading",
        ] {
            if let Some(spinner) =
                find_named_descendant(popover.upcast_ref(), name).and_downcast::<gtk::Spinner>()
            {
                let core = name == "language-filter-loading" || name == "platform-filter-loading";
                let loading = if core {
                    model.core_loading
                } else {
                    incomplete > failed
                };
                spinner.set_visible(loading);
                spinner.set_spinning(loading);
                spinner.set_tooltip_text(Some(if core {
                    "Loading platform and language data"
                } else {
                    "Filter metadata is incomplete; unknown games remain candidates"
                }));
            }
        }
        if let Some(label) = find_named_descendant(popover.upcast_ref(), "filter-data-status")
            .and_downcast::<gtk::Label>()
        {
            label.set_label(&if incomplete == 0 { String::new() } else if failed > 0 { format!("Filter data incomplete for {incomplete} games; {failed} failed. Unknown games remain candidates.") } else { format!("Loading filter data for {incomplete} games. Results are incomplete.") });
            label.set_visible(incomplete > 0);
        }
        if let Some(retry) = find_named_descendant(popover.upcast_ref(), "filter-data-retry")
            .and_downcast::<gtk::Button>()
        {
            retry.set_visible(failed > 0);
        }
    }
    let active_count = [
        model.favorites_only,
        model.downloaded_only,
        model.installed_only,
        model.played_only,
        model.unplayed_only,
        model.windows_only,
        model.linux_only,
        model.macos_only,
        model.cloud_saves_only,
        model.achievements_only,
        model.language_filter.is_some(),
    ]
    .into_iter()
    .filter(|active| *active)
    .count()
        + model.genre_theme_filters.len()
        + model.game_mode_filters.len()
        + model.property_filters.len()
        + model.tag_filters.len();
    for label in [
        &w.genre_theme_filter_label,
        &w.game_mode_filter_label,
        &w.property_filter_label,
    ] {
        label.set_visible(false);
    }
    w.filter_count.set_visible(false);
    w.clear_filters.set_visible(active_count > 0);
    if active_count > 0 {
        w.filter_button
            .add_css_class("sidebar-filter-button-active");
    } else {
        w.filter_button
            .remove_css_class("sidebar-filter-button-active");
    }
    rebuild_filter_chips(w, model);
    rebuild_property_filter_chips(w, model);
}

pub(super) fn metadata_ready(model: &AppModel, id: i64) -> bool {
    matches!(
        model
            .section_states
            .get(&(id, online::DetailSection::Metadata)),
        Some(SectionState::Ready)
    )
}

fn rebuild_property_filter_chips(w: &Widgets, model: &AppModel) {
    while let Some(child) = w.property_filter_chips.first_child() {
        w.property_filter_chips.remove(&child);
    }
    for value in &model.property_filters {
        let button = gtk::Button::new();
        button.set_action_name(Some("win.remove-library-filter"));
        button.set_action_target_value(Some(&format!("property:{value}").to_variant()));
        button.set_tooltip_text(Some(&format!("Remove {value} property")));
        button.add_css_class("active-filter-chip");
        button.set_hexpand(false);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        content.append(&gtk::Label::new(Some(value)));
        content.append(&gtk::Image::from_icon_name("window-close-symbolic"));
        button.set_child(Some(&content));
        w.property_filter_chips.insert(&button, -1);
    }
    w.property_filter_chips
        .set_visible(!model.property_filters.is_empty());
}

fn rebuild_filter_chips(w: &Widgets, model: &AppModel) {
    while let Some(child) = w.filter_chips.first_child() {
        w.filter_chips.remove(&child);
    }
    let mut chips = Vec::<(String, String)>::new();
    for (active, key, label) in [
        (model.favorites_only, "favorite", "Favorites"),
        (model.downloaded_only, "downloaded", "Downloaded"),
        (model.installed_only, "installed", "Installed"),
        (model.played_only, "played", "Played"),
        (model.unplayed_only, "unplayed", "Unplayed"),
        (model.windows_only, "windows", "Windows"),
        (model.linux_only, "linux", "Linux"),
        (model.macos_only, "macos", "macOS"),
        (model.cloud_saves_only, "cloud", "Cloud saves"),
        (model.achievements_only, "achievements", "Achievements"),
    ] {
        if active {
            chips.push((key.into(), label.into()));
        }
    }
    if let Some(language) = &model.language_filter {
        chips.push(("language".into(), language.clone()));
    }
    chips.extend(
        model
            .genre_theme_filters
            .iter()
            .map(|value| (format!("genre:{value}"), value.clone())),
    );
    chips.extend(
        model
            .game_mode_filters
            .iter()
            .map(|value| (format!("mode:{value}"), value.clone())),
    );
    chips.extend(
        model
            .property_filters
            .iter()
            .map(|value| (format!("property:{value}"), value.clone())),
    );
    let active = !chips.is_empty();
    for (key, label) in chips {
        let button = gtk::Button::new();
        button.set_action_name(Some("win.remove-library-filter"));
        button.set_action_target_value(Some(&key.to_variant()));
        button.set_tooltip_text(Some(&format!("Remove {label} filter")));
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Remove {label} filter"
        ))]);
        button.add_css_class("active-filter-chip");
        button.set_hexpand(false);
        button.set_halign(gtk::Align::Start);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let text = gtk::Label::new(Some(&label));
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        content.append(&text);
        content.append(&gtk::Image::from_icon_name("window-close-symbolic"));
        button.set_child(Some(&content));
        w.filter_chips.insert(&button, -1);
    }
    w.search.set_visible(!active);
    w.search.set_sensitive(!active);
    w.filter_chips.set_visible(active);
}

pub(super) fn update_language_options(w: &Widgets, model: &AppModel) {
    let mut languages: Vec<_> = model
        .games
        .iter()
        .flat_map(|game| game.languages.iter().cloned())
        .filter(|language| !language.trim().is_empty())
        .collect();
    languages.sort_by_key(|language| language.to_lowercase());
    languages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    while w.language_options.n_items() > 1 {
        w.language_options.remove(1);
    }
    for language in &languages {
        w.language_options.append(language);
    }
    let selected = model
        .language_filter
        .as_ref()
        .and_then(|current| {
            languages
                .iter()
                .position(|language| language.eq_ignore_ascii_case(current))
        })
        .map_or(0, |index| index as u32 + 1);
    w.language_filter.set_selected(selected);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MetadataFilterKind {
    GenreTheme,
    GameMode,
    Property,
}

pub(super) fn update_metadata_filter_options(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    let games = model.borrow().games.clone();
    // Keep the primary genre picker stable and familiar instead of exposing
    // every provider-specific genre/theme value GOG has ever returned.
    let genre_theme = [
        "Action",
        "Adventure",
        "Casual",
        "Indie",
        "Massively Multiplayer",
        "Racing",
        "RPG",
        "Simulation",
        "Sports",
        "Strategy",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut modes = games
        .iter()
        .flat_map(|game| game.metadata.game_modes.iter())
        .map(|term| term.name.clone())
        .collect::<Vec<_>>();
    let mut properties = games
        .iter()
        .flat_map(|game| game.metadata.properties.iter())
        .map(|term| term.name.clone())
        .collect::<Vec<_>>();
    for values in [&mut modes, &mut properties] {
        values.sort_by_key(|value| value.to_lowercase());
        values.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    }
    rebuild_metadata_filter_box(
        w,
        model,
        &w.genre_theme_filter_box,
        &genre_theme,
        MetadataFilterKind::GenreTheme,
    );
    rebuild_metadata_filter_box(
        w,
        model,
        &w.game_mode_filter_box,
        &modes,
        MetadataFilterKind::GameMode,
    );
    rebuild_metadata_filter_box(
        w,
        model,
        &w.property_filter_box,
        &properties,
        MetadataFilterKind::Property,
    );
}

pub(super) fn rebuild_metadata_filter_box(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    container: &gtk::Box,
    values: &[String],
    kind: MetadataFilterKind,
) {
    let mut child = container.first_child();
    if child
        .as_ref()
        .is_some_and(|widget| widget.is::<gtk::SearchEntry>())
    {
        child = child.and_then(|search| search.next_sibling());
    } else if container.has_css_class("inline-metadata-filter") {
        child = child.and_then(|heading| heading.next_sibling());
    }
    while let Some(widget) = child {
        let next = widget.next_sibling();
        container.remove(&widget);
        child = next;
    }
    for value in values {
        let check = gtk::CheckButton::with_label(value);
        let active = match kind {
            MetadataFilterKind::GenreTheme => model.borrow().genre_theme_filters.contains(value),
            MetadataFilterKind::GameMode => model.borrow().game_mode_filters.contains(value),
            MetadataFilterKind::Property => model.borrow().property_filters.contains(value),
        };
        check.set_active(active);
        let value = value.clone();
        let model = model.clone();
        let widgets = w.clone_refs();
        check.connect_toggled(move |check| {
            let mut state = model.borrow_mut();
            let set = match kind {
                MetadataFilterKind::GenreTheme => &mut state.genre_theme_filters,
                MetadataFilterKind::GameMode => &mut state.game_mode_filters,
                MetadataFilterKind::Property => &mut state.property_filters,
            };
            if check.is_active() {
                set.insert(value.clone());
            } else {
                set.remove(&value);
            }
            if check.is_active() {
                state.query.clear();
            }
            drop(state);
            if check.is_active() && !widgets.search.text().is_empty() {
                widgets.search.set_text("");
            }
            refresh_filters(&widgets, &model.borrow());
            if check.is_active() {
                request_filter_metadata(&widgets, &model, false);
            }
        });
        container.append(&check);
    }
}

pub(super) fn update_favorite_widgets(w: &Widgets, model: &AppModel, id: i64, favorite: bool) {
    let id_text = id.to_string();
    let mut row = w.game_list.first_child();
    while let Some(widget) = row {
        if widget.widget_name() == id_text {
            if let Some(star) =
                find_named_descendant(&widget, "favorite-star").and_downcast::<gtk::Image>()
            {
                star.set_visible(favorite);
            }
            break;
        }
        row = widget.next_sibling();
    }

    let platform = model
        .games
        .iter()
        .find(|game| game.product_id == id)
        .map(Game::platform_label)
        .unwrap_or_default();
    let mut child = w.home_grid.first_child();
    while let Some(wrapper) = child {
        if let Some(card) = wrapper.first_child()
            && card.widget_name() == id_text
        {
            if let Some(label) =
                find_named_descendant(&card, "card-meta").and_downcast::<gtk::Label>()
            {
                label.set_label(&format!("{platform}{}", if favorite { "  ★" } else { "" }));
            }
            break;
        }
        child = wrapper.next_sibling();
    }
}

pub(super) fn set_sidebar_icons_visible(w: &Widgets, visible: bool) {
    let mut row = w.game_list.first_child();
    while let Some(widget) = row {
        if let Some(icon) = find_named_descendant(&widget, "game-icon") {
            icon.set_visible(visible);
        }
        row = widget.next_sibling();
    }
}

pub(super) fn filter_heading(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("filter-heading");
    label
}

pub(super) fn inline_metadata_filter(title: &str) -> (gtk::Box, gtk::Label) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 3);
    section.add_css_class("inline-metadata-filter");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let title = filter_heading(title);
    title.set_hexpand(true);
    let count = gtk::Label::new(None);
    count.add_css_class("filter-count");
    count.set_visible(false);
    heading.append(&title);
    section.append(&heading);
    (section, count)
}

pub(super) fn game_row(game: &Game, favorite: bool, show_icon: bool) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_height_request(29);
    row.set_vexpand(false);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.set_height_request(29);
    content.set_vexpand(false);
    content.set_margin_top(1);
    content.set_margin_bottom(1);
    content.set_margin_start(8);
    content.set_margin_end(5);
    let icon = card_picture(game.icon.as_ref(), 23, 23);
    icon.set_widget_name("game-icon");
    icon.remove_css_class("hero-card");
    icon.add_css_class("game-icon");
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_visible(show_icon);
    content.append(&icon);
    let title = gtk::Label::new(Some(&game.title));
    title.set_widget_name("sidebar-game-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_hexpand(true);
    content.append(&title);
    let star = gtk::Image::from_icon_name("starred-symbolic");
    star.set_widget_name("favorite-star");
    star.set_visible(favorite);
    star.add_css_class("accent");
    content.append(&star);
    row.set_child(Some(&content));
    row
}

pub(super) fn game_card(game: &Game, favorite: bool, width: i32) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("game-card");
    let artwork_height = width * 9 / 16;
    let height = artwork_height + 62;
    card.set_size_request(width, height);
    card.set_halign(gtk::Align::Start);
    card.set_hexpand(false);
    card.set_overflow(gtk::Overflow::Hidden);

    let art = card_picture(game.artwork.as_ref(), width, artwork_height);
    art.set_widget_name("card-art");
    art.set_halign(gtk::Align::Center);
    art.set_valign(gtk::Align::Start);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    let status = gtk::Box::new(gtk::Orientation::Vertical, 4);
    status.set_widget_name("card-image-status");
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);
    status.set_can_target(false);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(20, 20);
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_max_width_chars(20);
    status.append(&spinner);
    status.append(&label);
    overlay.add_overlay(&status);
    if game.artwork.is_none() {
        set_picture_status(&art, "image-unavailable", "Image not cached");
    }
    let update = {
        let status = status.clone();
        move |art: &gtk::Picture| {
            let loading = art.has_css_class("image-loading") || art.has_css_class("image-pending");
            spinner.set_spinning(loading);
            spinner.set_visible(loading);
            label.set_label(art.tooltip_text().as_deref().unwrap_or("Image not cached"));
            status.set_visible(!art.has_css_class("image-ready"));
        }
    };
    update(&art);
    art.connect_notify_local(None, move |art, property| {
        if matches!(property.name(), "css-classes" | "tooltip-text") {
            update(art);
        }
    });
    card.append(&overlay);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.add_css_class("card-caption");
    text.set_halign(gtk::Align::Fill);
    text.set_valign(gtk::Align::Fill);
    text.set_vexpand(true);
    let title = gtk::Label::new(Some(&game.title));
    title.set_xalign(0.0);
    title.set_halign(gtk::Align::Fill);
    title.set_hexpand(true);
    title.set_lines(2);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("card-title");
    title.set_widget_name("card-title");
    let title_clamp = adw::Clamp::new();
    title_clamp.set_orientation(gtk::Orientation::Horizontal);
    title_clamp.set_maximum_size(width - 20);
    title_clamp.set_tightening_threshold(width - 20);
    title_clamp.set_child(Some(&title));
    let meta = gtk::Label::new(Some(&format!(
        "{}{}",
        game.platform_label(),
        if favorite { "  ★" } else { "" }
    )));
    meta.set_widget_name("card-meta");
    meta.set_xalign(0.0);
    meta.set_max_width_chars((width / 9).max(12));
    meta.add_css_class("dim-label");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_visible(false);
    text.append(&title_clamp);
    text.append(&meta);
    card.append(&text);
    card
}

pub(super) fn apply_card_cover_state(card: &gtk::Widget, state: Option<&CoverState>) {
    let Some(art) = find_named_descendant(card, "card-art").and_downcast::<gtk::Picture>() else {
        return;
    };
    match state {
        Some(CoverState::Pending) if art.paintable().is_none() => {
            set_picture_status(&art, "image-pending", "Image queued…")
        }
        Some(CoverState::Loading) if art.paintable().is_none() => {
            set_picture_status(&art, "image-loading", "Loading image…")
        }
        Some(CoverState::Unavailable) if art.paintable().is_none() => {
            set_picture_status(&art, "image-unavailable", "No image available")
        }
        Some(CoverState::Failed(error)) => set_picture_status(
            &art,
            "image-error",
            &format!("Image failed · {error}. Use Retry below."),
        ),
        _ => {}
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use chrono::FixedOffset;

    fn local(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        FixedOffset::west_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .unwrap()
    }

    #[test]
    fn buckets_recent_across_month_and_year_boundaries() {
        let may_four = local(2026, 5, 4);
        assert_eq!(
            activity_key(Some(local(2026, 5, 1).timestamp()), &may_four),
            ActivitySectionKey::Recent
        );
        assert_eq!(
            activity_key(Some(local(2026, 4, 28).timestamp()), &may_four),
            ActivitySectionKey::Recent
        );
        assert_eq!(
            activity_key(Some(local(2026, 4, 20).timestamp()), &may_four),
            ActivitySectionKey::Month {
                year: 2026,
                month: 4
            }
        );

        let january_three = local(2026, 1, 3);
        assert_eq!(
            activity_key(Some(local(2025, 12, 30).timestamp()), &january_three),
            ActivitySectionKey::Recent
        );
        assert_eq!(
            activity_key(Some(local(2025, 12, 1).timestamp()), &january_three),
            ActivitySectionKey::Year(2025)
        );
    }

    #[test]
    fn buckets_never_played_and_future_clock_skew() {
        let now = local(2026, 5, 4);
        assert_eq!(activity_key(None, &now), ActivitySectionKey::NeverPlayed);
        assert_eq!(
            activity_key(Some(local(2027, 1, 1).timestamp()), &now),
            ActivitySectionKey::Recent
        );
    }
}
