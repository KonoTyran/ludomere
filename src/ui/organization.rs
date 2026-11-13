use super::*;

pub(super) fn initialize(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    rebuild_filters(w, model);
    let action = gio::SimpleAction::new("hidden", Some(&i64::static_variant_type()));
    let widgets = w.clone();
    let state = model.clone();
    action.connect_activate(move |_, value| {
        let Some(id) = value.and_then(|v| v.get::<i64>()) else {
            return;
        };
        let (hidden, epoch) = {
            let mut model = state.borrow_mut();
            if model.logout_pending {
                return;
            }
            if !model.hidden_pending.insert(id) {
                show_progress(
                    &widgets,
                    "Saving this game's visibility. Try again when it finishes.",
                );
                return;
            }
            (!model.hidden_products.contains(&id), model.account_epoch)
        };
        let session = online::account_session();
        show_progress(
            &widgets,
            "Saving this game's visibility. Try again when it finishes.",
        );
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(save_hidden(session, id, hidden));
        });
        let widgets = widgets.clone();
        let state = state.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if state.borrow().account_epoch != epoch
                || state.borrow().logout_pending
                || online::account_session() != session
            {
                return glib::ControlFlow::Break;
            }
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err(anyhow::anyhow!("Visibility worker stopped unexpectedly"))
                }
            };
            match result {
                Ok(()) => {
                    if widgets.live_status.label()
                        == "Saving this game's visibility. Try again when it finishes."
                    {
                        show_progress(&widgets, "");
                    }
                    {
                        let mut model = state.borrow_mut();
                        model.hidden_pending.remove(&id);
                        if hidden {
                            model.hidden_products.insert(id);
                        } else {
                            model.hidden_products.remove(&id);
                        }
                    }
                    refresh_filters(&widgets, &state.borrow());
                    refresh_collection_metadata(&widgets, &state);
                    show_status(
                        &widgets,
                        if hidden {
                            "Game hidden locally. Use Show hidden to find it again."
                        } else {
                            "Game unhidden"
                        },
                    );
                    glib::ControlFlow::Break
                }
                Err(error) => {
                    if widgets.live_status.label()
                        == "Saving this game's visibility. Try again when it finishes."
                    {
                        show_progress(&widgets, "");
                    }
                    state.borrow_mut().hidden_pending.remove(&id);
                    show_status(
                        &widgets,
                        &super::notifications::failure_message(
                            "Could not save hidden state. Try again.",
                            &format!("{error:#}"),
                        ),
                    );
                    glib::ControlFlow::Break
                }
            }
        });
    });
    w.window.add_action(&action);
}

fn save_hidden(session: u64, id: i64, hidden: bool) -> anyhow::Result<()> {
    let _activity = crate::profile_reset::begin_activity("saving game visibility")?;
    online::with_account_session(session, || StateStore::open()?.set_hidden(id, hidden))
}

pub(super) fn matches_tags(
    tags: Option<&Vec<String>>,
    selected: &BTreeSet<String>,
    all: bool,
) -> bool {
    if selected.is_empty() {
        return true;
    }
    let contains = |selected: &String| {
        tags.is_some_and(|tags| tags.iter().any(|tag| tag.eq_ignore_ascii_case(selected)))
    };
    if all {
        selected.iter().all(contains)
    } else {
        selected.iter().any(contains)
    }
}

pub(super) fn rebuild_filters(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    while let Some(child) = w.organization_filters.first_child() {
        w.organization_filters.remove(&child);
    }
    let hidden = gtk::CheckButton::with_label("Show hidden");
    hidden.set_active(model.borrow().show_hidden);
    w.organization_filters.append(&hidden);
    hidden.connect_toggled({
        let w = w.clone_refs();
        let model = model.clone();
        move |button| {
            model.borrow_mut().show_hidden = button.is_active();
            refresh_filters(&w, &model.borrow());
            refresh_collection_metadata(&w, &model);
        }
    });
    let heading = gtk::Label::new(Some("Personal tags"));
    heading.set_xalign(0.0);
    w.organization_filters.append(&heading);
    let all = gtk::CheckButton::with_label("Match all selected tags");
    all.set_active(model.borrow().tag_match_all);
    all.set_tooltip_text(Some("Unchecked matches any selected tag"));
    w.organization_filters.append(&all);
    all.connect_toggled({
        let w = w.clone_refs();
        let model = model.clone();
        move |button| {
            model.borrow_mut().tag_match_all = button.is_active();
            refresh_filters(&w, &model.borrow());
        }
    });
    let mut tags = model
        .borrow()
        .tags
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    tags.sort_by_key(|tag| tag.to_lowercase());
    tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for tag in tags {
        let check = gtk::CheckButton::with_label(&tag);
        check.set_active(model.borrow().tag_filters.contains(&tag.to_lowercase()));
        w.organization_filters.append(&check);
        let w = w.clone_refs();
        let model = model.clone();
        check.connect_toggled(move |button| {
            if button.is_active() {
                model.borrow_mut().tag_filters.insert(tag.to_lowercase());
            } else {
                model.borrow_mut().tag_filters.remove(&tag.to_lowercase());
            }
            refresh_filters(&w, &model.borrow());
        });
    }
}

#[derive(Clone)]
enum Change {
    Add(String),
    Remove(String),
    Rename(String, String),
    Delete(String),
}

fn save_tags(session: u64, id: i64, change: Change) -> anyhow::Result<HashMap<i64, Vec<String>>> {
    let _activity = crate::profile_reset::begin_activity("saving personal tags")?;
    online::with_account_session(session, || {
        let store = StateStore::open()?;
        match change {
            Change::Add(tag) => store.add_tag(id, &tag)?,
            Change::Remove(tag) => store.remove_tag(id, &tag)?,
            Change::Rename(old, new) => store.rename_tag(&old, &new)?,
            Change::Delete(tag) => store.delete_tag(&tag)?,
        };
        store.tags()
    })
}

pub(super) fn tag_editor(w: &Widgets, model: &Rc<RefCell<AppModel>>, id: i64) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::new(Some("Personal tags"));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    root.append(&title);
    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    root.append(&chips);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::builder()
        .placeholder_text("Add a tag")
        .hexpand(true)
        .build();
    let add = gtk::Button::with_label("Add");
    add.set_sensitive(false);
    entry.connect_changed({
        let add = add.downgrade();
        move |entry| {
            if let Some(add) = add.upgrade() {
                add.set_sensitive(!entry.text().trim().is_empty());
            }
        }
    });
    entry.connect_activate({
        let add = add.downgrade();
        move |_| {
            if let Some(add) = add.upgrade()
                && add.is_sensitive()
            {
                add.emit_clicked();
            }
        }
    });
    let manage = gtk::Button::with_label("Manage tags…");
    row.append(&entry);
    row.append(&add);
    row.append(&manage);
    root.append(&row);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    root.append(&status);
    render_chips(&chips, w, model, id, &status);
    add.connect_clicked({
        let w = w.clone_refs();
        let model = model.clone();
        let chips = chips.clone();
        let status = status.clone();
        move |button| {
            if !button.is_sensitive() {
                return;
            }
            let tag = entry.text().trim().to_owned();
            if tag.is_empty() {
                return;
            }
            change(&w, &model, id, Change::Add(tag), &chips, &status);
        }
    });
    manage.connect_clicked({
        let w = w.clone_refs();
        let model = model.clone();
        let chips = chips.clone();
        let status = status.clone();
        move |_| {
            let mut tags = model.borrow().tags.values().flatten().cloned().collect::<Vec<_>>();
            tags.sort_by_key(|tag| tag.to_lowercase());
            tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            if tags.is_empty() {
                status.set_label("No personal tags to manage");
                return;
            }
            let dialog = adw::AlertDialog::builder()
                .heading("Manage personal tags")
                .body("Rename a tag everywhere, or delete the tag and all its assignments. Game files are unchanged.")
                .build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let choice = gtk::DropDown::from_strings(&tags.iter().map(String::as_str).collect::<Vec<_>>());
            let replacement = gtk::Entry::builder().placeholder_text("New tag name").build();
            content.append(&choice);
            content.append(&replacement);
            dialog.set_extra_child(Some(&content));
            dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename"), ("delete", "Delete tag")]);
            dialog.set_response_enabled("rename", false);
            replacement.connect_changed({
                let dialog = dialog.downgrade();
                move |entry| {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.set_response_enabled("rename", !entry.text().trim().is_empty());
                    }
                }
            });
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_close_response("cancel");
            let w = w.clone_refs();
            let model = model.clone();
            let chips = chips.clone();
            let status = status.clone();
            let epoch = model.borrow().account_epoch;
            dialog.choose(Some(&w.window.clone()), gio::Cancellable::NONE, move |response| {
                if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return;
                }
                let Some(tag) = tags.get(choice.selected() as usize) else { return };
                match response.as_str() {
                    "rename" if !replacement.text().trim().is_empty() => change(
                        &w, &model, id, Change::Rename(tag.clone(), replacement.text().trim().into()), &chips, &status,
                    ),
                    "delete" => change(&w, &model, id, Change::Delete(tag.clone()), &chips, &status),
                    _ => {}
                }
            });
        }
    });
    root
}

fn render_chips(
    chips: &gtk::FlowBox,
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    status: &gtk::Label,
) {
    while let Some(child) = chips.first_child() {
        chips.remove(&child);
    }
    for tag in model.borrow().tags.get(&id).cloned().unwrap_or_default() {
        let button = gtk::Button::with_label(&format!("{tag} ×"));
        button.set_tooltip_text(Some("Remove this tag from this game"));
        chips.insert(&button, -1);
        let w = w.clone_refs();
        let model = model.clone();
        let chips = chips.clone();
        let status = status.clone();
        button.connect_clicked(move |_| {
            change(&w, &model, id, Change::Remove(tag.clone()), &chips, &status)
        });
    }
}

fn change(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    change: Change,
    chips: &gtk::FlowBox,
    status: &gtk::Label,
) {
    let epoch = {
        let mut state = model.borrow_mut();
        if state.logout_pending {
            return;
        }
        if state.organization_pending {
            status.set_label("Wait for the current tag change to finish, then try again.");
            return;
        }
        state.organization_pending = true;
        state.account_epoch
    };
    status.set_label("Saving tags…");
    if let Some(editor) = chips.parent() {
        editor.set_sensitive(false);
    }
    let (sender, receiver) = mpsc::channel();
    let operation = change.clone();
    let session = online::account_session();
    std::thread::spawn(move || {
        let _ = sender.send(save_tags(session, id, operation));
    });
    let w = w.clone_refs();
    let model = model.clone();
    let chips = chips.downgrade();
    let status = status.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Tag worker stopped unexpectedly"))
            }
        };
        match result {
            Ok(tags) => {
                {
                    let mut state = model.borrow_mut();
                    state.organization_pending = false;
                    state.tags = tags;
                    match &change {
                        Change::Rename(old, new)
                            if state.tag_filters.remove(&old.to_lowercase()) =>
                        {
                            state.tag_filters.insert(new.to_lowercase());
                        }
                        Change::Delete(tag) => {
                            state.tag_filters.remove(&tag.to_lowercase());
                        }
                        _ => {}
                    }
                }
                rebuild_filters(&w, &model);
                refresh_filters(&w, &model.borrow());
                if let (Some(chips), Some(status)) = (chips.upgrade(), status.upgrade()) {
                    if let Some(editor) = chips.parent() {
                        editor.set_sensitive(true);
                    }
                    render_chips(&chips, &w, &model, id, &status);
                    status.set_label("Tags saved");
                }
                glib::ControlFlow::Break
            }
            Err(error) => {
                model.borrow_mut().organization_pending = false;
                if let Some(chips) = chips.upgrade()
                    && let Some(editor) = chips.parent()
                {
                    editor.set_sensitive(true);
                }
                if let Some(status) = status.upgrade() {
                    status.set_label(&super::notifications::failure_message(
                        "Could not save tags. Try again.",
                        &format!("{error:#}"),
                    ));
                }
                glib::ControlFlow::Break
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/all XDG and private GTK; only synthetic profile data and in-memory reset reservation"]
    fn organization_writes_reject_stale_sessions_and_reset_then_restore_controls() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p270-")
        );
        let session = online::account_session();
        let stale = session.wrapping_add(1);
        assert!(
            save_hidden(stale, 9270001, true)
                .unwrap_err()
                .to_string()
                .contains("account changed")
        );
        assert!(
            save_tags(stale, 9270001, Change::Add("Synthetic".into()))
                .unwrap_err()
                .to_string()
                .contains("account changed")
        );
        assert!(
            !crate::identity::database().exists(),
            "rejected workers must not create a profile"
        );
        let reservation = crate::profile_reset::reserve().unwrap();
        assert!(save_hidden(session, 9270001, true).is_err());
        assert!(save_tags(session, 9270001, Change::Add("Synthetic".into())).is_err());
        assert!(!crate::identity::database().exists());
        drop(reservation);
        save_hidden(session, 9270001, true).unwrap();
        assert!(
            StateStore::open()
                .unwrap()
                .hidden_product_ids()
                .unwrap()
                .contains(&9270001)
        );
        save_hidden(session, 9270001, false).unwrap();
        assert!(
            !StateStore::open()
                .unwrap()
                .hidden_product_ids()
                .unwrap()
                .contains(&9270001)
        );
        assert_eq!(
            save_tags(session, 9270001, Change::Add("Synthetic".into())).unwrap()[&9270001],
            ["Synthetic"]
        );
        assert_eq!(
            save_tags(
                session,
                9270001,
                Change::Rename("Synthetic".into(), "Renamed".into())
            )
            .unwrap()[&9270001],
            ["Renamed"]
        );
        assert!(
            save_tags(session, 9270001, Change::Remove("Renamed".into()))
                .unwrap()
                .is_empty()
        );
        save_tags(session, 9270001, Change::Add("Synthetic".into())).unwrap();
        assert!(
            save_tags(session, 9270001, Change::Delete("Synthetic".into()))
                .unwrap()
                .is_empty()
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
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.OrganizationTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let config = Config::default();
        let widgets = Rc::new(super::super::window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            ..AppModel::default()
        }));
        initialize(&widgets, &model);
        widgets.window.present();
        let hidden = widgets.window.lookup_action("hidden").unwrap();
        let reservation = crate::profile_reset::reserve().unwrap();
        hidden.activate(Some(&9270001i64.to_variant()));
        assert!(
            widgets
                .live_status
                .label()
                .contains("Saving this game's visibility")
        );
        wait_until(|| model.borrow().hidden_pending.is_empty());
        assert!(!model.borrow().hidden_products.contains(&9270001));
        assert!(widgets.live_status.label().is_empty());
        let editor = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let chips = gtk::FlowBox::new();
        let status = gtk::Label::new(None);
        editor.append(&chips);
        editor.append(&status);
        change(
            &widgets,
            &model,
            9270001,
            Change::Add("Synthetic".into()),
            &chips,
            &status,
        );
        assert_eq!(status.label(), "Saving tags…");
        assert!(!editor.is_sensitive());
        wait_until(|| !model.borrow().organization_pending);
        assert!(editor.is_sensitive());
        assert!(status.label().contains("Profile reset"));
        drop(reservation);
        change(
            &widgets,
            &model,
            9270001,
            Change::Add("Synthetic".into()),
            &chips,
            &status,
        );
        wait_until(|| status.label() == "Tags saved");
        assert_eq!(model.borrow().tags[&9270001], ["Synthetic"]);
        assert!(editor.is_sensitive());

        let tag_editor = tag_editor(&widgets, &model, 9270001);
        let manage = tag_editor
            .last_child()
            .unwrap()
            .prev_sibling()
            .unwrap()
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        assert_eq!(manage.label().as_deref(), Some("Manage tags…"));
        widgets.details.append(&tag_editor);
        widgets.content.set_visible_child_name("details");
        wait_until(|| manage.is_mapped());
        for rename in [false, true] {
            manage.emit_clicked();
            let dialog = widgets
                .window
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
                .unwrap();
            let replacement = dialog
                .extra_child()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::Entry>()
                .unwrap();
            wait_until(|| replacement.is_mapped());
            assert!(!dialog.is_response_enabled("rename"));
            for (text, enabled) in [(" \t ", false), (" Renamed in dialog ", true), ("", false)] {
                replacement.set_text(text);
                assert_eq!(dialog.is_response_enabled("rename"), enabled);
                assert!(dialog.is_response_enabled("cancel"));
                assert!(dialog.is_response_enabled("delete"));
                assert!(!model.borrow().organization_pending);
                assert_eq!(model.borrow().tags[&9270001], ["Synthetic"]);
            }
            replacement.set_text(" Renamed in dialog ");
            let mut pending = vec![dialog.clone().upcast::<gtk::Widget>()];
            let response = loop {
                let widget = pending.pop().expect("mapped dialog response button");
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && button.label().as_deref() == Some(if rename { "Rename" } else { "Cancel" })
                {
                    break button.clone();
                }
                pending.extend(std::iter::successors(widget.first_child(), |child| {
                    child.next_sibling()
                }));
            };
            wait_until(|| response.is_mapped());
            assert!(response.is_sensitive());
            response.emit_clicked();
            wait_until(|| widgets.window.visible_dialog().is_none());
            let expected = if rename {
                "Renamed in dialog"
            } else {
                "Synthetic"
            };
            wait_until(|| {
                let state = model.borrow();
                !state.organization_pending && state.tags[&9270001] == [expected]
            });
            assert_eq!(model.borrow().tags[&9270001], [expected]);
            assert_eq!(
                StateStore::open().unwrap().tags().unwrap()[&9270001],
                [expected]
            );
        }
        let add = manage
            .prev_sibling()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        let entry = add
            .prev_sibling()
            .unwrap()
            .downcast::<gtk::Entry>()
            .unwrap();
        let status = tag_editor
            .last_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(add.label().as_deref(), Some("Add"));
        wait_until(|| entry.is_mapped() && add.is_mapped());
        assert!(!add.is_sensitive());
        for (text, enabled) in [(" \t ", false), ("Candidate", true), ("", false)] {
            entry.set_text(text);
            assert_eq!(add.is_sensitive(), enabled);
        }
        entry.emit_by_name::<()>("activate", &[]);
        add.emit_clicked();
        assert!(!model.borrow().organization_pending);
        assert_eq!(model.borrow().tags[&9270001], ["Renamed in dialog"]);

        entry.set_text(" Added with Enter ");
        entry.emit_by_name::<()>("activate", &[]);
        assert!(model.borrow().organization_pending);
        assert_eq!(status.label(), "Saving tags…");
        assert!(!tag_editor.is_sensitive());
        // Programmatic signals and changed eligibility must not bypass the busy parent.
        entry.set_text("Must not be added while busy");
        assert!(!add.is_sensitive());
        entry.emit_by_name::<()>("activate", &[]);
        add.emit_clicked();
        assert_eq!(status.label(), "Saving tags…");
        wait_until(|| !model.borrow().organization_pending);
        assert!(tag_editor.is_sensitive());
        assert!(add.is_sensitive());
        assert_eq!(entry.text(), "Must not be added while busy");
        let saved = StateStore::open().unwrap().tags().unwrap();
        assert_eq!(saved[&9270001].len(), 2);
        assert!(saved[&9270001].iter().any(|tag| tag == "Added with Enter"));
        assert!(saved[&9270001].iter().any(|tag| tag == "Renamed in dialog"));
        assert_eq!(model.borrow().tags[&9270001], saved[&9270001]);

        let reservation = crate::profile_reset::reserve().unwrap();
        add.emit_clicked();
        assert!(!add.is_sensitive());
        wait_until(|| !model.borrow().organization_pending);
        assert!(status.label().contains("Profile reset"));
        assert!(add.is_sensitive());
        assert_eq!(StateStore::open().unwrap().tags().unwrap(), saved);
        entry.set_text(" \t ");
        assert!(!add.is_sensitive());
        drop(reservation);
        hidden.activate(Some(&9270001i64.to_variant()));
        wait_until(|| model.borrow().hidden_products.contains(&9270001));
        widgets.window.close();
    }

    #[test]
    fn any_all_filters_are_case_insensitive_and_missing_tags_do_not_match() {
        let tags = vec!["RPG".into(), "Co-op".into()];
        let selected = BTreeSet::from(["rpg".into(), "strategy".into()]);
        assert!(matches_tags(Some(&tags), &selected, false));
        assert!(!matches_tags(Some(&tags), &selected, true));
        assert!(!matches_tags(None, &selected, false));
        assert!(matches_tags(None, &BTreeSet::new(), true));
    }
}
