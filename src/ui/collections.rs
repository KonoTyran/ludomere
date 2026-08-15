use super::*;
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

    let ordered_groups = std::iter::once(("Favorites".to_owned(), favorites)).chain(groups);
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
