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

fn game_matches_collection(state: &AppModel, game: &Game, name: &str) -> bool {
    (state.show_hidden || !state.hidden_products.contains(&game.product_id))
        && if name == "Favorites" {
            state.favorites.contains(&game.product_id)
        } else {
            !metadata_ready(state, game.product_id)
                || game
                    .metadata
                    .genres
                    .iter()
                    .chain(&game.metadata.themes)
                    .any(|term| term.name.trim() == name)
        }
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
            .filter(|game| game_matches_collection(&state, game, name))
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
            if let Some(widget) = existing.remove(&game.product_id.to_string()) {
                widget.update_property(&[gtk::accessible::Property::Label(&game.title)]);
                if let Some(title) =
                    find_named_descendant(&widget, "card-title").and_downcast::<gtk::Label>()
                {
                    title.set_label(&game.title);
                }
                continue;
            }
            let card = game_card(
                game,
                state.favorites.contains(&game.product_id),
                state.card_width,
            );
            card.set_widget_name(&game.product_id.to_string());
            attach_game_context_menu(&card, w, model, game.product_id);
            let child = gtk::FlowBoxChild::builder().child(&card).build();
            child.update_property(&[gtk::accessible::Property::Label(&game.title)]);
            grid.insert(&child, -1);
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
    let grid = collection_grid();
    grid.set_activate_on_single_click(true);
    grid.add_css_class("game-grid");
    grid.connect_child_activated({
        let widgets = w.clone_refs();
        let model = model.clone();
        let epoch = model.borrow().account_epoch;
        let name = name.to_owned();
        move |grid, child| {
            if !child.is_mapped()
                || !child.is_child_visible()
                || child.parent().as_ref() != Some(grid.upcast_ref())
                || grid.parent().as_ref() != Some(widgets.collections.upcast_ref())
                || find_named_descendant(widgets.collections.upcast_ref(), "collection-grid")
                    .as_ref()
                    != Some(grid.upcast_ref())
                || widgets.collections.widget_name() != format!("collection:{name}")
            {
                return;
            }
            let Some(id) = child
                .child()
                .and_then(|card| card.widget_name().parse::<i64>().ok())
            else {
                return;
            };
            if model.try_borrow().is_ok_and(|state| {
                state.account_epoch == epoch
                    && !state.logout_pending
                    && state
                        .games
                        .iter()
                        .find(|game| game.product_id == id)
                        .is_some_and(|game| game_matches_collection(&state, game, &name))
            }) {
                show_game(&widgets, &model, id, None);
            }
        }
    });
    w.collections.append(&grid);
    refresh_collection_metadata(w, model);
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[test]
#[ignore = "requires isolated HOME/all XDG, private GTK display/D-Bus and GTK_A11Y=test"]
fn collection_games_use_native_activation_and_current_membership() {
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
                .starts_with("/tmp/ludomere-p351-")
        );
    }
    assert_eq!(std::env::var("GTK_A11Y").unwrap(), "test");
    adw::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    install_css();
    fn wait(check: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !check() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(check());
    }
    fn assert_title(child: &gtk::FlowBoxChild, title: &str) {
        let title = std::ffi::CString::new(title).unwrap();
        // GTK's private test backend returns null for an equal accessibility property.
        let mismatch: Option<glib::GString> = unsafe {
            glib::translate::from_glib_full(gtk::ffi::gtk_test_accessible_check_property(
                child.as_ptr().cast(),
                gtk::ffi::GTK_ACCESSIBLE_PROPERTY_LABEL,
                title.as_ptr(),
            ))
        };
        assert_eq!(mismatch, None);
        assert_eq!(child.accessible_role(), gtk::AccessibleRole::GridCell);
    }
    fn tile(grid: &gtk::FlowBox, id: i64) -> gtk::FlowBoxChild {
        let mut child = grid.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget
                .first_child()
                .is_some_and(|card| card.widget_name() == id.to_string())
            {
                return widget.downcast().unwrap();
            }
        }
        panic!("missing collection game {id}");
    }
    let app = adw::Application::builder()
        .application_id("io.github.ludomere.CollectionKeyboardTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let w = Rc::new(window::create_widgets(&app, &Config::default()));
    let model = Rc::new(RefCell::new(AppModel {
        games: [
            (1, "Genre game"),
            (2, "Theme game"),
            (3, "Favorite RPG"),
            (4, "Unknown candidate"),
        ]
        .into_iter()
        .map(|(product_id, title)| {
            let mut game = Game {
                product_id,
                title: title.into(),
                ..Game::default()
            };
            let term = crate::domain::MetadataTerm {
                provider_id: None,
                name: if product_id < 3 { " Action " } else { "RPG" }.into(),
                slug: "synthetic".into(),
                source: crate::domain::MetadataSource::GamesDb,
            };
            if product_id == 2 {
                game.metadata.themes.push(term);
            } else if product_id != 4 {
                game.metadata.genres.push(term);
            }
            game
        })
        .collect(),
        favorites: HashSet::from([3]),
        favorites_only: true,
        query: "Home query must not filter Collections".into(),
        section_states: (1..=4)
            .flat_map(|id| {
                [
                    online::DetailSection::Product,
                    online::DetailSection::Metadata,
                    online::DetailSection::Artwork,
                    online::DetailSection::Acquisition,
                    online::DetailSection::Builds,
                ]
                .into_iter()
                .map(move |section| {
                    (
                        (id, section),
                        if id == 4 && section == online::DetailSection::Metadata {
                            SectionState::Failed(
                                "Synthetic incomplete metadata; no request required".into(),
                            )
                        } else {
                            SectionState::Ready
                        },
                    )
                })
            })
            .collect(),
        ..AppModel::default()
    }));
    window::connect_actions(&w, &model);
    rebuild_collections_index(&w, &model);
    w.content.set_visible_child_name("collections");
    w.window.present();
    let index = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
        .unwrap();
    let action = find_named_descendant(index.upcast_ref(), "Action")
        .and_downcast::<gtk::Button>()
        .unwrap();
    wait(|| action.is_mapped());
    assert!(!index.has_css_class("game-grid"));
    assert!(gtk::prelude::WidgetExt::activate(&action));
    wait(|| w.collections.widget_name() == "collection:Action");
    let grid = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
        .unwrap();
    assert!(grid.activates_on_single_click() && grid.has_css_class("game-grid"));
    let children = [tile(&grid, 1), tile(&grid, 2), tile(&grid, 4)];
    assert!(grid.child_at_index(3).is_none());
    assert!(!game_matches_library_filters(&model.borrow(), 1));
    for (child, id) in children.iter().zip([1, 2, 4]) {
        let state = model.borrow();
        let game = state
            .games
            .iter()
            .find(|game| game.product_id == id)
            .unwrap();
        assert_title(child, &game.title);
        let card = child.child().unwrap();
        assert!(!card.is_focusable());
        let controllers = card.observe_controllers();
        let gestures = (0..controllers.n_items())
            .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
            .collect::<Vec<_>>();
        assert_eq!(gestures.len(), 1);
        assert_eq!(gestures[0].button(), gtk::gdk::BUTTON_SECONDARY);
    }
    let allocated = Rc::new(std::cell::Cell::new(false));
    grid.add_tick_callback({
        let allocated = allocated.clone();
        move |_, _| {
            allocated.set(true);
            glib::ControlFlow::Break
        }
    });
    wait(|| {
        allocated.get()
            && children
                .iter()
                .all(|child| child.is_mapped() && child.width() > 0)
    });
    let activated = Rc::new(std::cell::Cell::new(0));
    grid.connect_child_activated({
        let activated = activated.clone();
        move |_, _| activated.set(activated.get() + 1)
    });
    assert!(grid.child_focus(gtk::DirectionType::TabForward));
    assert!(children[0].has_focus());
    grid.emit_by_name::<bool>(
        "move-cursor",
        &[&gtk::MovementStep::VisualPositions, &1_i32, &false, &false],
    );
    assert!(children[1].has_focus());
    let generation = model.borrow().detail_generation;
    assert!(gtk::prelude::WidgetExt::activate(&children[1]));
    assert_eq!(activated.get(), 1);
    assert_eq!(model.borrow().detail_generation, generation + 1);
    assert_eq!(model.borrow().selected, Some(2));
    assert_eq!(w.content.visible_child_name().as_deref(), Some("details"));
    assert!(w.window.visible_dialog().is_none());

    w.content.set_visible_child_name("collections");
    wait(|| children[0].is_mapped());
    assert!(children[0].grab_focus());
    let focus = gtk::prelude::GtkWindowExt::focus(&w.window);
    let generation = model.borrow().detail_generation;
    model.borrow_mut().games[0].title =
        "Genre game with a complete updated accessible title".into();
    refresh_collection_metadata(&w, &model);
    assert_eq!(tile(&grid, 1), children[0]);
    assert_title(&children[0], &model.borrow().games[0].title);
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&w.window), focus);
    assert_eq!(
        w.content.visible_child_name().as_deref(),
        Some("collections")
    );
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().hidden_products.insert(1);
    assert!(gtk::prelude::WidgetExt::activate(&children[0]));
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().hidden_products.remove(&1);
    let genre = model.borrow_mut().games[0].metadata.genres.pop().unwrap();
    assert!(gtk::prelude::WidgetExt::activate(&children[0]));
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().games[0].metadata.genres.push(genre);
    model.borrow_mut().logout_pending = true;
    assert!(gtk::prelude::WidgetExt::activate(&children[1]));
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().logout_pending = false;
    {
        let _updating = model.borrow_mut();
        grid.emit_by_name::<()>("child-activated", &[&children[1]]);
    }
    assert_eq!(model.borrow().detail_generation, generation);

    // Even a reattached, mapped prior grid with the same collection name cannot activate.
    show_collection(&w, &model, "Action");
    let current = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
        .unwrap();
    w.collections.append(&grid);
    wait(|| children[1].is_mapped());
    grid.emit_by_name::<()>("child-activated", &[&children[1]]);
    assert_eq!(model.borrow().detail_generation, generation);
    w.collections.remove(&grid);
    assert!(!children[1].is_mapped());
    let current_one = tile(&current, 1);
    wait(|| current_one.is_mapped());
    model.borrow_mut().account_epoch += 1;
    assert!(gtk::prelude::WidgetExt::activate(&current_one));
    assert_eq!(model.borrow().detail_generation, generation);

    show_collection(&w, &model, "Action");
    let current = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
        .unwrap();
    let removed = tile(&current, 1);
    wait(|| removed.is_mapped());
    model.borrow_mut().games.remove(0);
    assert!(gtk::prelude::WidgetExt::activate(&removed));
    assert_eq!(model.borrow().detail_generation, generation);
    let retained = tile(&current, 2);
    assert!(retained.grab_focus());
    let focus = gtk::prelude::GtkWindowExt::focus(&w.window);
    refresh_collection_metadata(&w, &model);
    assert!(removed.parent().is_none());
    assert_eq!(tile(&current, 2), retained);
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&w.window), focus);
    assert_eq!(model.borrow().detail_generation, generation);

    show_collection(&w, &model, "Favorites");
    let favorites = find_named_descendant(w.collections.upcast_ref(), "collection-grid")
        .and_downcast::<gtk::FlowBox>()
        .unwrap();
    let favorite = tile(&favorites, 3);
    wait(|| favorite.is_mapped());
    assert!(
        favorites.child_at_index(1).is_none(),
        "unknown metadata is not Favorites membership"
    );
    model.borrow_mut().favorites.remove(&3);
    assert!(gtk::prelude::WidgetExt::activate(&favorite));
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().favorites.insert(3);
    model.borrow_mut().hidden_products.insert(3);
    assert!(gtk::prelude::WidgetExt::activate(&favorite));
    assert_eq!(model.borrow().detail_generation, generation);
    model.borrow_mut().show_hidden = true;
    assert!(gtk::prelude::WidgetExt::activate(&favorite));
    assert_eq!(model.borrow().selected, Some(3));
    assert_eq!(model.borrow().detail_generation, generation + 1);
    assert_eq!(w.content.visible_child_name().as_deref(), Some("details"));
    assert_eq!(
        model.borrow().query,
        "Home query must not filter Collections"
    );
    assert!(model.borrow().favorites_only);
    assert!(w.window.visible_dialog().is_none());
    w.window.destroy();
}
