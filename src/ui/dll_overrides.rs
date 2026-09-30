use super::*;
use crate::compatibility::{
    DllLoadOrder, game_dll_overrides, normalize_dll_overrides, set_game_dll_overrides,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct Editor {
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
            .push((row.clone(), entry, order_row));
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

    fn values(&self) -> anyhow::Result<BTreeMap<String, DllLoadOrder>> {
        normalize_dll_overrides(self.entries.borrow().iter().map(|(_, name, order)| {
            (
                name.text().to_string(),
                DllLoadOrder::ALL[order.selected() as usize],
            )
        }))
    }
}

pub(super) fn group(product_id: i64) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("DLL overrides");
    group.set_description(Some("Windows game launches only. Native uses a Windows DLL; Builtin uses Wine's implementation. Choices override matching Ludomere and inherited settings; Proton may apply its own runtime policy. Removing a row restores existing defaults. No DLLs are downloaded."));
    let editor = Editor {
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
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel Changes");
    let reload = gtk::Button::with_label("Retry Loading");
    reload.set_visible(false);
    for button in [&add, &save, &cancel] {
        controls.append(button);
    }
    controls.set_sensitive(false);
    group.add(&editor.rows);
    group.add(&controls);
    group.add(&editor.status);
    group.add(&reload);
    add.connect_clicked({
        let editor = editor.clone();
        move |_| editor.add("", DllLoadOrder::NativeThenBuiltin)
    });
    cancel.connect_clicked({
        let editor = editor.clone();
        move |_| editor.restore()
    });
    save.connect_clicked({
        let editor = editor.clone();
        let controls = controls.downgrade();
        move |_| {
            let values = match editor.values() {
                Ok(values) => values,
                Err(error) => { editor.status.set_label(&error.to_string()); return; }
            };
            let Some(controls) = controls.upgrade() else { return; };
            controls.set_sensitive(false);
            editor.rows.set_sensitive(false);
            editor.status.set_label("Saving DLL overrides…");
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = set_game_dll_overrides(product_id, values.clone()).map(|()| values);
                sender.send(result).ok();
            });
            let editor = editor.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if editor.group.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok(values)) => {
                        *editor.saved.borrow_mut() = values;
                        editor.restore();
                        editor.status.set_label("Saved. Changes apply to the next Windows game launch.");
                    }
                    Ok(Err(error)) => {
                        if let Some(crate::compatibility::CompatibilityFailure::PreferencesTooLarge) = error.downcast_ref::<crate::compatibility::CompatibilityFailure>() {
                            editor.status.set_label(&crate::compatibility::CompatibilityFailure::PreferencesTooLarge.to_string());
                        } else {
                            editor.status.set_label("Could not save DLL overrides. Your changes are still here; check configuration access and retry Save.");
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => editor.status.set_label("DLL preference worker stopped. Retry Save."),
                }
                controls.set_sensitive(true);
                editor.rows.set_sensitive(true);
                glib::ControlFlow::Break
            });
        }
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
    #[ignore = "requires an isolated GTK display"]
    fn dll_editor_keeps_invalid_rows_and_cancel_restores_saved_modes() {
        gtk::init().unwrap();
        let group = adw::PreferencesGroup::new();
        let editor = Editor {
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
