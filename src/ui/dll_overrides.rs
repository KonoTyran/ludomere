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
    preferences_generation: Rc<std::cell::Cell<u64>>,
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
        let preferences_generation = self.preferences_generation.clone();
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
            let generation = preferences_generation.get();
            if generation != crate::compatibility::proton_preferences_generation() {
                status.set_label("Proton preferences were reset. Reopen Properties before editing DLL overrides.");
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
                    if generation != crate::compatibility::proton_preferences_generation() {
                        if let Some(status) = status.upgrade() {
                            status.set_label("Proton preferences were reset. Reopen Properties before editing DLL overrides.");
                        }
                        return;
                    }
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
                        anyhow::ensure!(
                            generation == crate::compatibility::proton_preferences_generation(),
                            "Proton preferences were reset. Reopen Properties before editing DLL overrides."
                        );
                        online::with_account_session(session, || {
                            set_game_dll_overrides(product_id, values.clone())
                        })?;
                        Ok(values)
                    });
                    glib::timeout_add_local(Duration::from_millis(50), move || {
                        if generation != crate::compatibility::proton_preferences_generation() {
                            if let Some(status) = status.upgrade() {
                                status.set_label("Proton preferences were reset. Reopen Properties before editing DLL overrides.");
                            }
                            return glib::ControlFlow::Break;
                        }
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
        preferences_generation: Rc::new(std::cell::Cell::new(
            crate::compatibility::proton_preferences_generation(),
        )),
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
    let reset = gtk::Button::with_label("Reset DLL Overrides…");
    let recover = gtk::Button::with_label("Reset Proton Preferences…");
    recover.set_visible(false);
    let reload = gtk::Button::with_label("Retry Loading");
    reload.set_visible(false);
    controls.append(&add);
    controls.append(&reset);
    controls.set_sensitive(false);
    group.add(&editor.rows);
    group.add(&controls);
    group.add(&editor.status);
    group.add(&reload);
    group.add(&recover);
    reset.connect_clicked({
        let editor = editor.clone();
        let controls = controls.clone();
        move |button| {
            let Some(window) = button.root().and_downcast::<adw::ApplicationWindow>() else { return; };
            if online::account_session() != editor.session { return; }
            let dialog = adw::AlertDialog::builder().heading("Reset this game's DLL overrides?")
                .body("Remove all DLL overrides for this game and use the existing defaults. Its Proton choice, compatibility-fix switches, files and saves will not change.").build();
            dialog.add_responses(&[("cancel", "Cancel"), ("reset", "Reset DLL Overrides")]);
            dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let editor = editor.clone();
            let controls = controls.clone();
            dialog.choose(Some(&window.clone()), gio::Cancellable::NONE, move |response| {
                if response != "reset" || !window.is_visible() || online::account_session() != editor.session { return; }
                // Invalidate pending text timers; FIFO puts reset after already submitted saves.
                editor.revision.set(editor.revision.get().wrapping_add(1));
                editor.rows.set_sensitive(false);
                controls.set_sensitive(false);
                editor.status.set_label("Resetting DLL overrides…");
                let session = editor.session;
                let generation = editor.preferences_generation.get();
                let receiver = update_policies::policy_request(move || {
                    anyhow::ensure!(generation == crate::compatibility::proton_preferences_generation(), "Proton preferences were reset. Reopen Properties before editing DLL overrides.");
                    online::with_account_session(session, || set_game_dll_overrides(product_id, BTreeMap::new()))
                });
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    let result = match receiver.try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(_) => Err(anyhow::anyhow!("DLL reset worker stopped. Reopen Properties before retrying.")),
                    };
                    if online::account_session() != session {
                        editor.status.set_label("The account changed. Reopen Properties before changing DLL overrides.");
                        return glib::ControlFlow::Break;
                    }
                    controls.set_sensitive(true);
                    editor.rows.set_sensitive(true);
                    match result {
                        Ok(()) => {
                            editor.saved.borrow_mut().clear();
                            editor.restore();
                            editor.status.set_label("DLL overrides reset. Defaults apply to the next launch.");
                        }
                        Err(error) => editor.status.set_label(&format!("Could not reset DLL overrides: {error}. Try again.")),
                    }
                    glib::ControlFlow::Break
                });
            });
        }
    });
    add.connect_clicked({
        let editor = editor.clone();
        move |_| editor.add("", DllLoadOrder::NativeThenBuiltin)
    });
    reload.connect_clicked({
        let editor = editor.clone();
        let controls = controls.clone();
        let recover = recover.clone();
        move |reload| load(product_id, &editor, &controls, reload, &recover)
    });
    proton::connect_preferences_recovery(
        &recover,
        &editor.status,
        Rc::new({
            let editor = editor.clone();
            let controls = controls.clone();
            let reload = reload.clone();
            let recover = recover.clone();
            move || load(product_id, &editor, &controls, &reload, &recover)
        }),
    );
    load(product_id, &editor, &controls, &reload, &recover);
    group
}

fn load(
    product_id: i64,
    editor: &Editor,
    controls: &gtk::Box,
    reload: &gtk::Button,
    recover: &gtk::Button,
) {
    let generation = crate::compatibility::proton_preferences_generation();
    editor.preferences_generation.set(generation);
    editor.revision.set(editor.revision.get().wrapping_add(1));
    editor.rows.set_sensitive(false);
    controls.set_sensitive(false);
    reload.set_visible(false);
    editor.status.set_label("Loading DLL overrides…");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender.send(game_dll_overrides(product_id)).ok();
    });
    let editor = editor.clone();
    let controls = controls.clone();
    let reload = reload.clone();
    let recover = recover.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if generation != crate::compatibility::proton_preferences_generation() {
            editor.status.set_label(
                "Proton preferences were reset. Reopen Properties before editing DLL overrides.",
            );
            reload.set_visible(true);
            return glib::ControlFlow::Break;
        }
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
                recover.set_visible(false);
                *editor.saved.borrow_mut() = values;
                editor.restore();
                editor.rows.set_sensitive(true);
                controls.set_sensitive(true);
            }
            Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                editor.status.set_label("Could not load DLL overrides. Retry loading, or back up and reset unreadable Proton preferences. Existing preferences were not changed.");
                reload.set_visible(true);
                recover.set_visible(true);
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
    #[ignore = "requires private HOME/all XDG, GTK display and D-Bus"]
    fn confirmed_resets_preserve_other_games_cancel_and_pending_edit_order() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p247-")
        );
        adw::init().unwrap();
        fn pump() {
            let deadline = std::time::Instant::now() + Duration::from_millis(750);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut all = vec![root.clone()];
            let mut child = root.first_child();
            while let Some(widget) = child {
                all.extend(widgets(&widget));
                child = widget.next_sibling();
            }
            all
        }
        let application = adw::Application::builder()
            .application_id("io.github.ludomere.PreferenceRecoveryTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        application.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&application)
            .build();
        let overrides = BTreeMap::from([("dinput8".to_owned(), DllLoadOrder::Builtin)]);
        set_game_dll_overrides(7, overrides.clone()).unwrap();
        set_game_dll_overrides(8, overrides.clone()).unwrap();
        let group = group(7);
        window.set_content(Some(&group));
        window.present();
        pump();
        let button = |label: &str| {
            widgets(group.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some(label))
                .unwrap()
        };
        let respond = |response: &str| {
            window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap()
                .emit_by_name::<()>("response", &[&response])
        };
        button("Reset DLL Overrides…").emit_clicked();
        respond("cancel");
        pump();
        assert_eq!(game_dll_overrides(7).unwrap(), overrides);
        let entry = widgets(group.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
            .unwrap();
        entry.set_text("pending_new_name");
        button("Reset DLL Overrides…").emit_clicked();
        respond("reset");
        pump();
        assert!(game_dll_overrides(7).unwrap().is_empty());
        assert_eq!(game_dll_overrides(8).unwrap(), overrides);
        assert!(
            widgets(group.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .any(|label| label.label().contains("DLL overrides reset"))
        );

        let path = crate::identity::config_root().join("proton.json");
        let other = super::group(8);
        let other_window = adw::ApplicationWindow::builder()
            .application(&application)
            .build();
        other_window.set_content(Some(&other));
        other_window.present();
        pump();
        let other_entry = widgets(other.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
            .unwrap();
        let valid = std::fs::read(&path).unwrap();
        let malformed = b"{ unreadable preference fixture";
        std::fs::write(&path, malformed).unwrap();
        button("Retry Loading").emit_clicked();
        pump();
        assert!(button("Reset Proton Preferences…").is_visible());
        button("Reset Proton Preferences…").emit_clicked();
        respond("cancel");
        pump();
        assert_eq!(std::fs::read(&path).unwrap(), malformed);
        // A stale recovery offer must never discard settings repaired in another view.
        std::fs::write(&path, &valid).unwrap();
        button("Reset Proton Preferences…").emit_clicked();
        respond("reset");
        pump();
        assert_eq!(std::fs::read(&path).unwrap(), valid);
        assert!(
            widgets(group.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .any(|label| label.label().contains("readable now"))
        );
        std::fs::write(&path, malformed).unwrap();
        other_entry.set_text("must_not_survive_global_reset");
        button("Reset Proton Preferences…").emit_clicked();
        respond("reset");
        pump();
        assert!(
            crate::compatibility::proton_preferences()
                .unwrap()
                .dll_overrides
                .is_empty()
        );
        assert!(!button("Reset Proton Preferences…").is_visible());
        assert!(
            widgets(other.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .any(|label| label.label().contains("preferences were reset"))
        );
        let backup = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("proton-recovery-")
            })
            .unwrap()
            .path();
        assert_eq!(std::fs::read(&backup).unwrap(), malformed);
        let done = window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        assert!(done.body().contains(backup.to_str().unwrap()));
        respond("close");
        other_window.close();
        window.close();
    }

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
            preferences_generation: Rc::new(std::cell::Cell::new(
                crate::compatibility::proton_preferences_generation(),
            )),
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
            preferences_generation: Rc::new(std::cell::Cell::new(
                crate::compatibility::proton_preferences_generation(),
            )),
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
