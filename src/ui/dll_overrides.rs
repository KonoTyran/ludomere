use super::*;
use crate::compatibility::{
    DllLoadOrder, game_dll_overrides, normalize_dll_overrides, set_game_dll_overrides,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct Editor {
    product_id: i64,
    session: u64,
    revision: Rc<std::cell::Cell<u64>>,
    group: glib::WeakRef<adw::PreferencesGroup>,
    rows: gtk::ListBox,
    entries: Rc<RefCell<Vec<(adw::ActionRow, gtk::Entry, gtk::DropDown)>>>,
    saved: Rc<RefCell<BTreeMap<String, DllLoadOrder>>>,
    status: gtk::Label,
}

impl Editor {
    fn add(&self, name: &str, order: DllLoadOrder) {
        let row = adw::ActionRow::new();
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("DLL name, e.g. dinput8"));
        entry.set_tooltip_text(Some("One DLL name, with or without .dll; no paths"));
        entry.set_text(name);
        entry.set_hexpand(true);
        entry.set_valign(gtk::Align::Center);
        entry.set_max_length(132);
        let order_row = gtk::DropDown::from_strings(&DllLoadOrder::ALL.map(DllLoadOrder::label));
        order_row.set_selected(
            DllLoadOrder::ALL
                .iter()
                .position(|value| *value == order)
                .unwrap() as u32,
        );
        order_row.set_tooltip_text(Some("DLL load order"));
        order_row.set_valign(gtk::Align::Center);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_tooltip_text(Some("Remove this DLL override"));
        remove.set_valign(gtk::Align::Center);
        remove.add_css_class("flat");
        row.add_prefix(&entry);
        row.add_suffix(&order_row);
        row.add_suffix(&remove);
        self.rows.append(&row);
        self.entries
            .borrow_mut()
            .push((row.clone(), entry.clone(), order_row.clone()));
        let changed = self.change_handler();
        entry.connect_changed({
            let changed = changed.clone();
            move |_| changed(true)
        });
        entry.connect_activate({
            let changed = changed.clone();
            move |_| changed(false)
        });
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave({
            let changed = changed.clone();
            move |_| changed(false)
        });
        entry.add_controller(focus);
        order_row.connect_selected_notify({
            let changed = changed.clone();
            move |_| changed(false)
        });
        let rows = self.rows.downgrade();
        let entries = Rc::downgrade(&self.entries);
        let row = row.downgrade();
        remove.connect_clicked(move |_| {
            if let (Some(rows), Some(row), Some(entries)) =
                (rows.upgrade(), row.upgrade(), entries.upgrade())
            {
                entries
                    .borrow_mut()
                    .retain(|(candidate, _, _)| *candidate != row);
                rows.remove(&row);
                changed(false);
            }
        });
    }

    fn restore(&self) {
        for (row, _, _) in self.entries.borrow_mut().drain(..) {
            self.rows.remove(&row);
        }
        for (name, order) in self.saved.borrow().iter() {
            self.add(name, *order);
        }
        self.status
            .set_label("Changes apply to the next Windows game launch.");
    }

    #[cfg(test)]
    fn values(&self) -> anyhow::Result<BTreeMap<String, DllLoadOrder>> {
        normalize_dll_overrides(self.entries.borrow().iter().map(|(_, name, order)| {
            (
                name.text().to_string(),
                DllLoadOrder::ALL[order.selected() as usize],
            )
        }))
    }

    fn change_handler(&self) -> Rc<dyn Fn(bool)> {
        let entries = Rc::downgrade(&self.entries);
        let status = self.status.downgrade();
        let saved = self.saved.clone();
        let revision = self.revision.clone();
        let product_id = self.product_id;
        let session = self.session;
        Rc::new(move |debounce| {
            let (Some(entries), Some(status)) = (entries.upgrade(), status.upgrade()) else {
                return;
            };
            if online::account_session() != session {
                status.set_label(
                    "The account changed. Reopen Properties before changing DLL overrides.",
                );
                return;
            }
            revision.set(revision.get().wrapping_add(1));
            let current = revision.get();
            let values =
                normalize_dll_overrides(entries.borrow().iter().map(|(_, name, order)| {
                    (
                        name.text().to_string(),
                        DllLoadOrder::ALL[order.selected() as usize],
                    )
                }));
            status.set_label("Checking DLL overrides…");
            let status = status.downgrade();
            let saved = saved.clone();
            let revision = revision.clone();
            glib::timeout_add_local_once(
                Duration::from_millis(if debounce { 400 } else { 0 }),
                move || {
                    if online::account_session() != session {
                        if let Some(status) = status.upgrade() {
                            status.set_label("The account changed. Reopen Properties before changing DLL overrides.");
                        }
                        return;
                    }
                    if revision.get() != current {
                        return;
                    }
                    let values = match values {
                        Ok(values) => values,
                        Err(error) => {
                            if let Some(status) = status.upgrade() {
                                status.set_label(&format!(
                                    "Not saved: {error}. Correct the invalid row or remove it."
                                ));
                            }
                            return;
                        }
                    };
                    if let Some(status) = status.upgrade() {
                        status.set_label("Saving DLL overrides…");
                    }
                    let receiver = update_policies::policy_request(move || {
                        online::with_account_session(session, || {
                            set_game_dll_overrides(product_id, values.clone())
                        })?;
                        Ok(values)
                    });
                    glib::timeout_add_local(Duration::from_millis(50), move || {
                        let result = match receiver.try_recv() {
                            Ok(result) => result,
                            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                            Err(_) => Err(anyhow::anyhow!("DLL preference worker stopped")),
                        };
                        if online::account_session() != session {
                            if let Some(status) = status.upgrade() {
                                status.set_label("The account changed. Reopen Properties before changing DLL overrides.");
                            }
                            return glib::ControlFlow::Break;
                        }
                        if let Ok(values) = &result {
                            *saved.borrow_mut() = values.clone();
                        }
                        if revision.get() == current
                            && let Some(status) = status.upgrade()
                        {
                            status.set_label(&match result {
                            Ok(_) => "Saved automatically. Changes apply to the next Windows game launch.".into(),
                            Err(error) => format!("Could not save DLL overrides: {error}. Edit a row to retry; the last saved overrides are unchanged."),
                        });
                        }
                        glib::ControlFlow::Break
                    });
                },
            );
        })
    }
}

pub(super) fn group(product_id: i64) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("DLL overrides");
    group.set_description(Some("Changes save automatically for the next Windows launch. Native uses a Windows DLL; Builtin uses Wine's implementation. Choices override matching Ludomere and inherited settings; Proton may apply its own runtime policy. Removing a row restores existing defaults. No DLLs are downloaded."));
    let editor = Editor {
        product_id,
        session: online::account_session(),
        revision: Rc::new(std::cell::Cell::new(0)),
        group: group.downgrade(),
        rows: gtk::ListBox::new(),
        entries: Rc::new(RefCell::new(Vec::new())),
        saved: Rc::new(RefCell::new(BTreeMap::new())),
        status: gtk::Label::new(Some("Loading DLL overrides…")),
    };
    editor.rows.set_selection_mode(gtk::SelectionMode::None);
    editor.rows.add_css_class("boxed-list");
    editor.status.set_wrap(true);
    editor.status.set_xalign(0.0);
    editor.status.set_selectable(true);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let add = gtk::Button::with_label("Add DLL");
    let reload = gtk::Button::with_label("Retry Loading");
    reload.set_visible(false);
    controls.append(&add);
    controls.set_sensitive(false);
    group.add(&editor.rows);
    group.add(&controls);
    group.add(&editor.status);
    group.add(&reload);
    add.connect_clicked({
        let editor = editor.clone();
        move |_| editor.add("", DllLoadOrder::NativeThenBuiltin)
    });
    reload.connect_clicked({
        let editor = editor.clone();
        let controls = controls.clone();
        move |reload| load(product_id, &editor, &controls, reload)
    });
    load(product_id, &editor, &controls, &reload);
    group
}

fn load(product_id: i64, editor: &Editor, controls: &gtk::Box, reload: &gtk::Button) {
    reload.set_visible(false);
    editor.status.set_label("Loading DLL overrides…");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender.send(game_dll_overrides(product_id)).ok();
    });
    let editor = editor.clone();
    let controls = controls.clone();
    let reload = reload.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if editor.group.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        if online::account_session() != editor.session {
            editor
                .status
                .set_label("The account changed. Reopen Properties before changing DLL overrides.");
            controls.set_sensitive(false);
            reload.set_sensitive(false);
            editor.rows.set_sensitive(false);
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(values)) => {
                *editor.saved.borrow_mut() = values;
                editor.restore();
                controls.set_sensitive(true);
            }
            Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                editor.status.set_label("Could not load DLL overrides. Check Proton configuration access and syntax, then retry; existing preferences were not changed.");
                reload.set_visible(true);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
        }
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires private HOME/all XDG and an isolated GTK display"]
    fn dll_autosave_preserves_invalid_drafts_and_orders_reverted_choices() {
        assert!(std::env::var("HOME").unwrap().starts_with("/tmp/ludomere-"));
        gtk::init().unwrap();
        let group = adw::PreferencesGroup::new();
        let editor = Editor {
            product_id: 997,
            session: online::account_session(),
            revision: Rc::new(std::cell::Cell::new(0)),
            group: group.downgrade(),
            rows: gtk::ListBox::new(),
            entries: Rc::new(RefCell::new(Vec::new())),
            saved: Rc::new(RefCell::new(BTreeMap::new())),
            status: gtk::Label::new(None),
        };
        fn wait_until(condition: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !condition() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(condition());
        }
        editor.add("dinput8", DllLoadOrder::Builtin);
        let (name, order) = {
            let rows = editor.entries.borrow();
            (rows[0].1.clone(), rows[0].2.clone())
        };
        name.set_text("dinput9");
        wait_until(|| editor.status.label().starts_with("Saved automatically"));
        let valid = game_dll_overrides(997).unwrap();
        name.set_text("../invalid");
        wait_until(|| editor.status.label().starts_with("Not saved:"));
        assert_eq!(game_dll_overrides(997).unwrap(), valid);
        name.set_text("dinput9");
        name.emit_by_name::<()>("activate", &[]);
        wait_until(|| editor.status.label().starts_with("Saved automatically"));
        order.set_selected(4);
        while glib::MainContext::default().iteration(false) {}
        order.set_selected(1);
        wait_until(|| editor.status.label().starts_with("Saved automatically"));
        assert_eq!(
            game_dll_overrides(997).unwrap(),
            valid,
            "reverting during a queued write must persist the final choice"
        );
        name.set_text("dinput10");
        drop(editor);
        wait_until(|| game_dll_overrides(997).unwrap().contains_key("dinput10"));
    }

    #[test]
    #[ignore = "requires an isolated GTK display"]
    fn dll_editor_keeps_invalid_rows_and_restores_saved_modes() {
        gtk::init().unwrap();
        let group = adw::PreferencesGroup::new();
        let editor = Editor {
            product_id: 7,
            session: online::account_session(),
            revision: Rc::new(std::cell::Cell::new(0)),
            group: group.downgrade(),
            rows: gtk::ListBox::new(),
            entries: Rc::new(RefCell::new(Vec::new())),
            saved: Rc::new(RefCell::new(BTreeMap::from([(
                "dinput8".into(),
                DllLoadOrder::Builtin,
            )]))),
            status: gtk::Label::new(None),
        };
        editor.restore();
        editor.add("DINPUT8.DLL", DllLoadOrder::Disabled);
        assert!(editor.values().is_err());
        assert_eq!(editor.entries.borrow().len(), 2);
        editor.restore();
        assert_eq!(editor.values().unwrap(), *editor.saved.borrow());
        editor.entries.borrow()[0].2.set_selected(4);
        assert_eq!(editor.values().unwrap()["dinput8"], DllLoadOrder::Disabled);
        let row = editor.entries.borrow()[0].0.clone();
        editor.rows.remove(&row);
        editor.entries.borrow_mut().clear();
        assert!(editor.values().unwrap().is_empty());
    }
}
