use super::*;
use crate::saved_view::{SavedViewQuery, SavedViewSort};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn rebuild_collections_index(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    clear(&w.collections);
    w.collections.add_css_class("collections-page");

    let heading = gtk::Label::new(Some("YOUR COLLECTIONS"));
    heading.set_xalign(0.0);
    heading.add_css_class("collections-heading");
    w.collections.append(&heading);

    let state = model.borrow();
    let favorites = state
        .games
        .iter()
        .filter(|game| state.favorites.contains(&game.product_id))
        .map(|game| game.product_id)
        .collect::<BTreeSet<_>>();
    let hidden = state.hidden_games.iter().copied().collect::<BTreeSet<_>>();
    let mut groups = BTreeMap::<String, BTreeSet<i64>>::new();
    for game in &state.games {
        for term in game.metadata.genres.iter().chain(&game.metadata.themes) {
            if !term.name.trim().is_empty() {
                groups
                    .entry(term.name.trim().to_owned())
                    .or_default()
                    .insert(game.product_id);
            }
        }
    }
    let games = state.games.clone();
    drop(state);

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .column_spacing(18)
        .row_spacing(18)
        .max_children_per_line(20)
        .min_children_per_line(1)
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Fill)
        .build();
    grid.add_css_class("collections-grid");

    let ordered_groups = [
        ("Favorites".to_owned(), favorites),
        ("Hidden".to_owned(), hidden),
    ]
    .into_iter()
    .chain(groups);
    for (name, ids) in ordered_groups {
        let ids = ids.into_iter().collect::<Vec<_>>();
        let artwork = ids.iter().find_map(|id| {
            games
                .iter()
                .find(|game| game.product_id == *id)
                .and_then(|game| game.artwork.as_ref())
        });
        let button = gtk::Button::new();
        button.add_css_class("collection-card");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&card_picture(artwork, 174, 174)));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
        copy.set_halign(gtk::Align::Fill);
        copy.set_valign(gtk::Align::Fill);
        copy.add_css_class("collection-card-overlay");
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        copy.append(&spacer);
        let title = gtk::Label::new(Some(&name.to_uppercase()));
        title.set_wrap(true);
        title.set_justify(gtk::Justification::Center);
        title.add_css_class("collection-card-title");
        copy.append(&title);
        let count = gtk::Label::new(Some(&format!("( {} )", ids.len())));
        count.add_css_class("collection-card-count");
        copy.append(&count);
        overlay.add_overlay(&copy);
        button.set_child(Some(&overlay));
        let w = w.clone();
        let model = model.clone();
        button.connect_clicked(move |_| show_collection(&w, &model, &name, &ids));
        grid.insert(&button, -1);
    }
    w.collections.append(&grid);

    let saved_heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    saved_heading.set_margin_top(24);
    let label = gtk::Label::new(Some("SAVED VIEWS"));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("collections-heading");
    saved_heading.append(&label);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("Save the current library filters"));
    let w_add = w.clone();
    let model_add = model.clone();
    add.connect_clicked(move |_| present_create_saved_view(&w_add, &model_add));
    saved_heading.append(&add);
    w.collections.append(&saved_heading);

    let saved_grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(18)
        .row_spacing(18)
        .max_children_per_line(20)
        .min_children_per_line(1)
        .valign(gtk::Align::Start)
        .build();
    saved_grid.add_css_class("collections-grid");
    let views = model.borrow().saved_views.clone();
    for (index, view) in views.iter().enumerate() {
        saved_grid.insert(&saved_view_card(w, model, view, index, views.len()), -1);
    }
    w.collections.append(&saved_grid);
}

fn saved_view_card(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    view: &crate::saved_view::SavedView,
    index: usize,
    count: usize,
) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let ids = saved_view_ids(&model.borrow(), &view.query);
    let open = gtk::Button::with_label(&format!("{}\n( {} )", view.name.to_uppercase(), ids.len()));
    open.add_css_class("collection-card");
    let w_open = w.clone();
    let model_open = model.clone();
    let name = view.name.clone();
    open.connect_clicked(move |_| show_collection(&w_open, &model_open, &name, &ids));
    container.append(&open);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    controls.set_halign(gtk::Align::Center);
    for (icon, tooltip, enabled, operation) in [
        ("go-up-symbolic", "Move earlier", index > 0, "up"),
        ("go-down-symbolic", "Move later", index + 1 < count, "down"),
        ("document-edit-symbolic", "Rename", true, "rename"),
        (
            "view-refresh-symbolic",
            "Update from current filters",
            true,
            "update",
        ),
        ("user-trash-symbolic", "Delete", true, "delete"),
    ] {
        let button = gtk::Button::from_icon_name(icon);
        button.add_css_class("flat");
        button.set_tooltip_text(Some(tooltip));
        button.set_sensitive(enabled);
        let w = w.clone();
        let model = model.clone();
        let view = view.clone();
        button.connect_clicked(move |_| manage_saved_view(&w, &model, &view, operation));
        controls.append(&button);
    }
    container.append(&controls);
    container
}

fn current_saved_view_query(model: &AppModel) -> SavedViewQuery {
    let mut operating_systems = Vec::new();
    if model.windows_only {
        operating_systems.push("windows".into());
    }
    if model.linux_only {
        operating_systems.push("linux".into());
    }
    if model.macos_only {
        operating_systems.push("macos".into());
    }
    SavedViewQuery {
        text: (!model.query.trim().is_empty()).then(|| model.query.trim().to_owned()),
        installed: model.installed_only.then_some(true),
        downloaded: model.downloaded_only.then_some(true),
        favorite: model.favorites_only.then_some(true),
        played: match (model.played_only, model.unplayed_only) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        },
        include_hidden: model.show_hidden,
        tags: model.tag_filters.iter().cloned().collect(),
        all_tags: model.all_tag_filters,
        operating_systems,
        sort: match model.sidebar_sort_mode {
            crate::config::SidebarSortMode::Alphabetical => SavedViewSort::Title,
            crate::config::SidebarSortMode::LastPlayed => SavedViewSort::LastPlayed,
        },
        ..Default::default()
    }
}

fn saved_view_ids(model: &AppModel, query: &SavedViewQuery) -> Vec<i64> {
    let mut games = model
        .games
        .iter()
        .filter(|game| {
            let id = game.product_id;
            if query.owned == Some(false) {
                return false;
            }
            if !query.include_hidden && model.hidden_games.contains(&id) {
                return false;
            }
            if query
                .favorite
                .is_some_and(|expected| model.favorites.contains(&id) != expected)
                || query
                    .installed
                    .is_some_and(|expected| model.installed_products.contains(&id) != expected)
                || query.downloaded.is_some_and(|expected| {
                    let downloaded = model.downloaded_products.contains(&id)
                        || game
                            .installers
                            .iter()
                            .chain(&game.patches)
                            .chain(&game.extras)
                            .any(|file| file.path.is_file())
                        || game.dlcs.iter().any(|dlc| {
                            dlc.installers
                                .iter()
                                .chain(&dlc.extras)
                                .any(|file| file.path.is_file())
                                || model.downloaded_products.contains(&dlc.product_id)
                        });
                    downloaded != expected
                })
            {
                return false;
            }
            let played = model
                .product_activity
                .get(&id)
                .is_some_and(|activity| activity.last_played_at.is_some());
            if query.played.is_some_and(|expected| played != expected) {
                return false;
            }
            if let Some(text) = &query.text
                && !game.title.to_lowercase().contains(&text.to_lowercase())
                && !game.slug.to_lowercase().contains(&text.to_lowercase())
            {
                return false;
            }
            if !query.operating_systems.is_empty()
                && !query.operating_systems.iter().any(|os| match os.as_str() {
                    "windows" => game.platforms.windows,
                    "linux" => game.platforms.linux,
                    "macos" => game.platforms.macos,
                    _ => false,
                })
            {
                return false;
            }
            if !query.tags.is_empty() {
                let assigned = model.tags.get(&id).map(Vec::as_slice).unwrap_or_default();
                let matches = |tag: &String| {
                    assigned
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                };
                if (query.all_tags && !query.tags.iter().all(matches))
                    || (!query.all_tags && !query.tags.iter().any(matches))
                {
                    return false;
                }
            }
            true
        })
        .collect::<Vec<_>>();
    games.sort_by(|left, right| match query.sort {
        SavedViewSort::Title => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
        SavedViewSort::LastPlayed => model
            .product_activity
            .get(&right.product_id)
            .and_then(|v| v.last_played_at)
            .cmp(
                &model
                    .product_activity
                    .get(&left.product_id)
                    .and_then(|v| v.last_played_at),
            ),
        SavedViewSort::Playtime => model
            .product_activity
            .get(&right.product_id)
            .map_or(0, |v| v.playtime_seconds)
            .cmp(
                &model
                    .product_activity
                    .get(&left.product_id)
                    .map_or(0, |v| v.playtime_seconds),
            ),
        SavedViewSort::ReleaseDate => right.release_date.cmp(&left.release_date),
    });
    games.into_iter().map(|game| game.product_id).collect()
}

fn present_create_saved_view(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Save current view")
        .body("Name this combination of search and library filters.")
        .build();
    let entry = gtk::Entry::builder().placeholder_text("View name").build();
    entry.set_activates_default(true);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let w = w.clone();
    let model = model.clone();
    let window = w.window.clone();
    dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
        if response != "save" {
            return;
        }
        let query = current_saved_view_query(&model.borrow());
        match StateStore::open().and_then(|store| store.create_saved_view(&entry.text(), &query)) {
            Ok(id) => {
                let position = model.borrow().saved_views.len() as i64;
                model
                    .borrow_mut()
                    .saved_views
                    .push(crate::saved_view::SavedView {
                        id,
                        name: entry.text().trim().to_owned(),
                        query,
                        position,
                    });
                rebuild_collections_index(&w, &model);
            }
            Err(error) => show_saved_view_error(&w.window, &error.to_string()),
        }
    });
}

fn manage_saved_view(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    view: &crate::saved_view::SavedView,
    operation: &str,
) {
    if matches!(operation, "up" | "down") {
        let mut state = model.borrow_mut();
        let Some(index) = state
            .saved_views
            .iter()
            .position(|candidate| candidate.id == view.id)
        else {
            return;
        };
        let target = if operation == "up" {
            index.saturating_sub(1)
        } else {
            index + 1
        };
        if target >= state.saved_views.len() {
            return;
        }
        state.saved_views.swap(index, target);
        let ids = state
            .saved_views
            .iter()
            .map(|view| view.id)
            .collect::<Vec<_>>();
        drop(state);
        if let Err(error) = StateStore::open().and_then(|store| store.reorder_saved_views(&ids)) {
            show_saved_view_error(&w.window, &error.to_string());
        }
        rebuild_collections_index(w, model);
        return;
    }
    if operation == "update" {
        let query = current_saved_view_query(&model.borrow());
        match StateStore::open()
            .and_then(|store| store.update_saved_view(view.id, &view.name, &query))
        {
            Ok(()) => {
                if let Some(saved) = model
                    .borrow_mut()
                    .saved_views
                    .iter_mut()
                    .find(|saved| saved.id == view.id)
                {
                    saved.query = query;
                }
                rebuild_collections_index(w, model);
            }
            Err(error) => show_saved_view_error(&w.window, &error.to_string()),
        }
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading(if operation == "delete" {
            "Delete saved view?"
        } else {
            "Rename saved view"
        })
        .build();
    let entry = gtk::Entry::new();
    entry.set_text(&view.name);
    if operation == "rename" {
        dialog.set_extra_child(Some(&entry));
    }
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        (
            operation,
            if operation == "delete" {
                "Delete"
            } else {
                "Rename"
            },
        ),
    ]);
    dialog.set_close_response("cancel");
    let w = w.clone();
    let model = model.clone();
    let view = view.clone();
    let operation = operation.to_owned();
    let window = w.window.clone();
    dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
        if response != operation {
            return;
        }
        let result = StateStore::open().and_then(|store| {
            if operation == "delete" {
                store.delete_saved_view(view.id)
            } else {
                store.update_saved_view(view.id, &entry.text(), &view.query)
            }
        });
        match result {
            Ok(()) => {
                let mut state = model.borrow_mut();
                if operation == "delete" {
                    state.saved_views.retain(|saved| saved.id != view.id);
                } else if let Some(saved) = state
                    .saved_views
                    .iter_mut()
                    .find(|saved| saved.id == view.id)
                {
                    saved.name = entry.text().trim().to_owned();
                }
                drop(state);
                rebuild_collections_index(&w, &model);
            }
            Err(error) => show_saved_view_error(&w.window, &error.to_string()),
        }
    });
}

fn show_saved_view_error(window: &adw::ApplicationWindow, message: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading("Could not save view")
        .body(message)
        .build();
    dialog.add_response("close", "Close");
    dialog.present(Some(window));
}

fn show_collection(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>, name: &str, ids: &[i64]) {
    clear(&w.collections);
    let heading_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    heading_row.add_css_class("collection-games-heading");
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back to Collections"));
    let w_back = w.clone();
    let model_back = model.clone();
    back.connect_clicked(move |_| rebuild_collections_index(&w_back, &model_back));
    heading_row.append(&back);
    let heading = gtk::Label::new(Some(&format!("{}  ({})", name.to_uppercase(), ids.len())));
    heading.set_xalign(0.0);
    heading.add_css_class("collections-heading");
    heading_row.append(&heading);
    w.collections.append(&heading_row);

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .column_spacing(18)
        .row_spacing(18)
        .max_children_per_line(20)
        .min_children_per_line(1)
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Fill)
        .build();
    let state = model.borrow();
    for game in state
        .games
        .iter()
        .filter(|game| ids.contains(&game.product_id))
    {
        let card = game_card(
            game,
            state.favorites.contains(&game.product_id),
            state.card_width,
        );
        let id = game.product_id;
        let w = w.clone();
        let model = model.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| show_game(&w, &model, id, None));
        card.add_controller(click);
        grid.insert(&card, -1);
    }
    drop(state);
    w.collections.append(&grid);
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
