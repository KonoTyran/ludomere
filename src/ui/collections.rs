use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn rebuild_collections_index(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    clear(&w.collections);
    w.collections.set_widget_name("collections-index");
    w.collections.add_css_class("collections-page");
    let heading = gtk::Label::new(Some("YOUR COLLECTIONS"));
    heading.set_xalign(0.0);
    heading.add_css_class("collections-heading");
    w.collections.append(&heading);
    append_collection_loading(w, model);
    let grid = collection_grid();
    grid.add_css_class("collections-grid");
    w.collections.append(&grid);
    refresh_collection_metadata(w, model);
    request_filter_metadata(w, model, false);
}

fn collection_grid() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .column_spacing(18)
        .row_spacing(18)
        .max_children_per_line(20)
        .min_children_per_line(1)
        .valign(gtk::Align::Start)
        .halign(gtk::Align::Fill)
        .name("collection-grid")
        .build()
}

fn append_collection_loading(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let spinner = gtk::Spinner::new();
    spinner.set_widget_name("collection-loading");
    row.append(&spinner);
    let label = gtk::Label::new(None);
    label.set_widget_name("collection-load-status");
    label.set_wrap(true);
    row.append(&label);
    let retry = gtk::Button::with_label("Retry");
    retry.set_widget_name("collection-retry");
    let widgets = w.clone_refs();
    let model = model.clone();
    retry.connect_clicked(move |_| request_filter_metadata(&widgets, &model, true));
    row.append(&retry);
    w.collections.append(&row);
}

/// Apply membership and count changes in place without changing the selected collection.
pub(super) fn refresh_collection_metadata(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    let Some(grid) = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
    else {
        return;
    };
    let state = model.borrow();
    let missing = state
        .games
        .iter()
        .filter(|game| !metadata_ready(&state, game.product_id))
        .count();
    let failed = state
        .games
        .iter()
        .filter(|game| {
            matches!(
                state
                    .section_states
                    .get(&(game.product_id, online::DetailSection::Metadata)),
                Some(SectionState::Failed(_))
            )
        })
        .count();
    let selected = w.collections.widget_name();
    let name = selected.strip_prefix("collection:");
    let incomplete = missing > 0 && name != Some("Favorites");
    if let Some(spinner) = find_named_descendant(w.collections.upcast_ref(), "collection-loading")
        .and_downcast::<gtk::Spinner>()
    {
        spinner.set_visible(incomplete && missing > failed);
        spinner.set_spinning(incomplete && missing > failed);
    }
    if let Some(label) = find_named_descendant(w.collections.upcast_ref(), "collection-load-status")
        .and_downcast::<gtk::Label>()
    {
        label.set_visible(incomplete);
        label.set_label(&if failed > 0 {
            format!("Collection data incomplete; {failed} games failed to load.")
        } else {
            "Loading collection data; results are incomplete.".into()
        });
    }
    if let Some(retry) = find_named_descendant(w.collections.upcast_ref(), "collection-retry")
        .and_downcast::<gtk::Button>()
    {
        retry.set_visible(incomplete && failed > 0);
    }
    let mut existing = HashMap::new();
    let mut child = grid.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(card) = widget.first_child() {
            existing.insert(card.widget_name().to_string(), widget);
        }
    }
    if let Some(name) = name {
        let games = state
            .games
            .iter()
            .filter(|game| state.show_hidden || !state.hidden_products.contains(&game.product_id))
            .filter(|game| {
                if name == "Favorites" {
                    state.favorites.contains(&game.product_id)
                } else {
                    !metadata_ready(&state, game.product_id)
                        || game
                            .metadata
                            .genres
                            .iter()
                            .chain(&game.metadata.themes)
                            .any(|term| term.name.trim() == name)
                }
            })
            .collect::<Vec<_>>();
        if let Some(heading) =
            find_named_descendant(w.collections.upcast_ref(), "collection-heading")
                .and_downcast::<gtk::Label>()
        {
            heading.set_label(&format!(
                "{}  ({}{})",
                name.to_uppercase(),
                games.len(),
                if incomplete { " candidates" } else { "" }
            ));
        }
        for game in games {
            if existing.remove(&game.product_id.to_string()).is_some() {
                continue;
            }
            let card = game_card(
                game,
                state.favorites.contains(&game.product_id),
                state.card_width,
            );
            card.set_widget_name(&game.product_id.to_string());
            let id = game.product_id;
            let widgets = w.clone_refs();
            attach_game_context_menu(&card, w, model, id);
            let model = model.clone();
            let click = gtk::GestureClick::new();
            click.set_button(gtk::gdk::BUTTON_PRIMARY);
            click.connect_released(move |_, _, _, _| show_game(&widgets, &model, id, None));
            card.add_controller(click);
            grid.insert(&card, -1);
        }
    } else {
        let mut groups = BTreeMap::<String, BTreeSet<i64>>::new();
        groups.insert(
            "Favorites".into(),
            state
                .games
                .iter()
                .filter(|game| {
                    state.show_hidden || !state.hidden_products.contains(&game.product_id)
                })
                .filter(|game| state.favorites.contains(&game.product_id))
                .map(|game| game.product_id)
                .collect(),
        );
        for game in &state.games {
            if !state.show_hidden && state.hidden_products.contains(&game.product_id) {
                continue;
            }
            for term in game.metadata.genres.iter().chain(&game.metadata.themes) {
                if !term.name.trim().is_empty() {
                    groups
                        .entry(term.name.trim().to_owned())
                        .or_default()
                        .insert(game.product_id);
                }
            }
        }
        for (name, ids) in groups {
            if let Some(widget) = existing.remove(&name) {
                if let Some(count) =
                    find_named_descendant(&widget, "collection-count").and_downcast::<gtk::Label>()
                {
                    count.set_label(&format!(
                        "( {}{} )",
                        ids.len(),
                        if incomplete && name != "Favorites" {
                            "+"
                        } else {
                            ""
                        }
                    ));
                }
                continue;
            }
            let artwork = state
                .games
                .iter()
                .filter(|game| ids.contains(&game.product_id))
                .find_map(|game| game.artwork.as_ref());
            let button = gtk::Button::new();
            button.set_widget_name(&name);
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
            let count = gtk::Label::new(Some(&format!(
                "( {}{} )",
                ids.len(),
                if incomplete && name != "Favorites" {
                    "+"
                } else {
                    ""
                }
            )));
            count.set_widget_name("collection-count");
            count.add_css_class("collection-card-count");
            copy.append(&count);
            overlay.add_overlay(&copy);
            button.set_child(Some(&overlay));
            let widgets = w.clone_refs();
            let model = model.clone();
            button.connect_clicked(move |_| show_collection(&widgets, &model, &name));
            grid.insert(&button, -1);
        }
    }
    for widget in existing.into_values() {
        grid.remove(&widget);
    }
}

fn show_collection(w: &Widgets, model: &Rc<RefCell<AppModel>>, name: &str) {
    clear(&w.collections);
    w.collections.set_widget_name(&format!("collection:{name}"));
    let heading_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    heading_row.add_css_class("collection-games-heading");
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back to Collections"));
    let w_back = w.clone_refs();
    let model_back = model.clone();
    back.connect_clicked(move |_| rebuild_collections_index(&w_back, &model_back));
    heading_row.append(&back);
    let heading = gtk::Label::new(None);
    heading.set_widget_name("collection-heading");
    heading.set_xalign(0.0);
    heading.add_css_class("collections-heading");
    heading_row.append(&heading);
    w.collections.append(&heading_row);
    append_collection_loading(w, model);
    w.collections.append(&collection_grid());
    refresh_collection_metadata(w, model);
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
