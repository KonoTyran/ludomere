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
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(StateStore::open().and_then(|store| store.set_hidden(id, hidden)));
        });
        let widgets = widgets.clone();
        let state = state.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if state.borrow().account_epoch != epoch || state.borrow().logout_pending {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(())) => {
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
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    if widgets.live_status.label()
                        == "Saving this game's visibility. Try again when it finishes."
                    {
                        show_progress(&widgets, "");
                    }
                    state.borrow_mut().hidden_pending.remove(&id);
                    show_status(&widgets, "Could not save hidden state. Try again.");
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            }
        });
    });
    w.window.add_action(&action);
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
        move |_| {
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
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let store = StateStore::open()?;
            match operation {
                Change::Add(tag) => store.add_tag(id, &tag)?,
                Change::Remove(tag) => store.remove_tag(id, &tag)?,
                Change::Rename(old, new) => store.rename_tag(&old, &new)?,
                Change::Delete(tag) => store.delete_tag(&tag)?,
            };
            store.tags()
        })();
        let _ = sender.send(result);
    });
    let w = w.clone_refs();
    let model = model.clone();
    let chips = chips.downgrade();
    let status = status.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(tags)) => {
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
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                model.borrow_mut().organization_pending = false;
                if let Some(chips) = chips.upgrade()
                    && let Some(editor) = chips.parent()
                {
                    editor.set_sensitive(true);
                }
                if let Some(status) = status.upgrade() {
                    status.set_label("Could not save tags. Try again.");
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
