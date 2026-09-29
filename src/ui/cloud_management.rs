use super::*;
use crate::cloud_saves::{
    api::RemoteObject,
    backup::{self, ManagementSession},
};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) fn cloud_management_group(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    installed: &crate::domain::InstalledGame,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Export and manage remote saves");
    group.set_description(Some("Exports preserve original paths and include a checksum manifest. Remote deletion keeps a verified recovery copy and leaves local saves unchanged. Full profile reset preserves recovery copies but clears deletion tracking, so local saves may upload again afterward."));
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    let export_row = adw::ActionRow::builder()
        .title("Export remote saves")
        .subtitle("Choose a folder for a verified copy of the current cloud revisions")
        .build();
    let export = gtk::Button::with_label("Export…");
    export.set_valign(gtk::Align::Center);
    export_row.add_suffix(&export);
    group.add(&export_row);
    let parent = window.clone();
    let state = model.clone();
    let game = installed.clone();
    let export_status = status.clone();
    export.connect_clicked(move |button| {
        let Some(session) = session(&state, &game, &export_status) else {
            return;
        };
        let epoch = state.borrow().account_epoch;
        let picker = gtk::FileDialog::builder()
            .title("Export GOG cloud saves to folder")
            .modal(true)
            .build();
        let parent = parent.clone();
        let state = state.clone();
        let status = export_status.clone();
        let button = button.clone();
        button.set_sensitive(false);
        picker.select_folder(
            Some(&parent.clone()),
            gio::Cancellable::NONE,
            move |result| {
                button.set_sensitive(true);
                if state.borrow().account_epoch != epoch || !parent.is_visible() {
                    return;
                }
                let Ok(folder) = result else { return };
                let Some(destination) = folder.path() else {
                    status.set_label("Choose a local filesystem directory.");
                    return;
                };
                button.set_sensitive(false);
                status.set_label("Exporting and verifying cloud saves…");
                run_worker(
                    parent.upcast_ref(),
                    &state,
                    session,
                    move |session| backup::export(&session, &destination),
                    move |result| {
                        button.set_sensitive(true);
                        match result {
                            Ok(path) => status
                                .set_label(&format!("Verified export saved to {}", path.display())),
                            Err(error) => status.set_label(&format!("Export failed: {error}")),
                        }
                    },
                );
            },
        );
    });
    let manage_row = adw::ActionRow::builder()
        .title("Delete selected remote saves")
        .subtitle("Requires fresh revisions, a recovery copy and explicit confirmation")
        .build();
    let manage = gtk::Button::with_label("Manage…");
    manage.set_valign(gtk::Align::Center);
    manage_row.add_suffix(&manage);
    group.add(&manage_row);
    let parent = window.clone();
    let model = model.clone();
    let game = installed.clone();
    let manage_status = status.clone();
    manage.connect_clicked(move |_| {
        if let Some(session) = session(&model, &game, &manage_status) {
            show_remote_files(&parent, &model, session);
        }
    });
    group.add(&status);
    group
}

fn session(
    model: &Rc<RefCell<AppModel>>,
    game: &crate::domain::InstalledGame,
    status: &gtk::Label,
) -> Option<ManagementSession> {
    if model.borrow().logout_pending {
        status.set_label("Finish signing out before managing cloud saves.");
        return None;
    }
    let Some(account_id) = model
        .borrow()
        .account_token
        .as_ref()
        .map(|token| token.user_id.clone())
    else {
        status.set_label("Sign in to GOG to manage cloud saves.");
        return None;
    };
    Some(ManagementSession {
        game: game.clone(),
        account_id,
        account_session: online::account_session(),
        cancelled: Arc::new(AtomicBool::new(false)),
    })
}

fn run_worker<T: Send + 'static>(
    owner: &gtk::Window,
    model: &Rc<RefCell<AppModel>>,
    session: ManagementSession,
    work: impl FnOnce(ManagementSession) -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(anyhow::Result<T>) + 'static,
) {
    let epoch = model.borrow().account_epoch;
    let cancelled = session.cancelled.clone();
    let close_cancel = cancelled.clone();
    let handler = owner.connect_close_request(move |_| {
        close_cancel.store(true, Ordering::Release);
        glib::Propagation::Proceed
    });
    let owner = owner.downgrade();
    let model = model.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work(session));
    });
    let mut done = Some(done);
    let mut handler = Some(handler);
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let current = owner.upgrade().filter(|owner| owner.is_visible());
        if current.is_none() || model.borrow().account_epoch != epoch {
            cancelled.store(true, Ordering::Release);
            if let Some(owner) = owner.upgrade()
                && let Some(handler) = handler.take()
            {
                owner.disconnect(handler);
            }
            if current.is_some() {
                done.take().unwrap()(Err(anyhow::anyhow!(
                    "The account session changed; close this window and reopen cloud-save management."
                )));
            }
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!(
                "cloud-save worker stopped; refresh before retrying"
            )),
        };
        if let Some(handler) = handler.take() {
            current.unwrap().disconnect(handler);
        }
        done.take().unwrap()(result);
        glib::ControlFlow::Break
    });
}

fn show_remote_files(
    parent: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    session: ManagementSession,
) {
    let dialog = gtk::Window::builder()
        .title("Remote cloud saves")
        .transient_for(parent)
        .modal(true)
        .default_width(640)
        .default_height(540)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    let explanation = gtk::Label::new(Some(
        "Stop games and other cloud clients before deleting. Ludomere first verifies a recovery copy and rechecks remote revisions, but another client's upload after the final recheck could be deleted without a recovery copy. Local saves are kept; unchanged copies are not uploaded again unless you use Force upload, change them, or reset the profile.",
    ));
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    content.append(&explanation);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    content.append(&status);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(
        &gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&list)
            .build(),
    );
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let select_all = gtk::CheckButton::with_label("Select all");
    let refresh = gtk::Button::with_label("Refresh");
    let delete = gtk::Button::with_label("Delete selected…");
    delete.add_css_class("destructive-action");
    delete.set_sensitive(false);
    actions.append(&select_all);
    actions.append(&refresh);
    actions.append(&delete);
    content.append(&actions);
    dialog.set_child(Some(&content));
    let rows = Rc::new(RefCell::new(Vec::<(gtk::CheckButton, RemoteObject)>::new()));
    let busy = Rc::new(Cell::new(false));
    {
        let rows = rows.clone();
        select_all.connect_toggled(move |select| {
            for (check, _) in rows.borrow().iter() {
                check.set_active(select.is_active());
            }
        });
    }
    {
        let dialog = dialog.clone();
        let model = model.clone();
        let rows = rows.clone();
        let busy = busy.clone();
        let status = status.clone();
        let select_all = select_all.clone();
        let delete = delete.clone();
        let session = session.clone();
        refresh.connect_clicked(move |button| {
            if busy.replace(true) {
                return;
            }
            button.set_sensitive(false);
            delete.set_sensitive(false);
            select_all.set_sensitive(false);
            status.set_label("Loading remote files…");
            rows.borrow_mut().clear();
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let button = button.clone();
            let rows = rows.clone();
            let busy = busy.clone();
            let status = status.clone();
            let select_all = select_all.clone();
            let delete = delete.clone();
            let list = list.clone();
            run_worker(
                &dialog,
                &model,
                session.clone(),
                |session| backup::inventory(&session),
                move |result| {
                    busy.set(false);
                    button.set_sensitive(true);
                    select_all.set_active(false);
                    match result {
                        Ok(objects) => {
                            status.set_label(&format!(
                                "{} remote files. Select files to delete.",
                                objects.len()
                            ));
                            for object in objects {
                                let check = gtk::CheckButton::with_label(&format!(
                                    "{}/{}  ({})",
                                    object.namespace,
                                    object.path,
                                    human_size(object.size)
                                ));
                                let checks = rows.clone();
                                let delete = delete.clone();
                                let busy = busy.clone();
                                check.connect_toggled(move |_| {
                                    delete.set_sensitive(
                                        !busy.get()
                                            && checks
                                                .borrow()
                                                .iter()
                                                .any(|(check, _)| check.is_active()),
                                    );
                                });
                                list.append(&check);
                                rows.borrow_mut().push((check, object));
                            }
                            select_all.set_sensitive(!rows.borrow().is_empty());
                        }
                        Err(error) => status.set_label(&format!(
                            "Could not load remote files: {error}. Use Refresh to retry."
                        )),
                    }
                },
            );
        });
    }
    {
        let dialog = dialog.clone();
        let model = model.clone();
        let busy = busy.clone();
        let refresh = refresh.clone();
        delete.connect_clicked(move |button| {
            if busy.get() { return; }
            let selected = rows.borrow().iter().filter(|(check, _)| check.is_active()).map(|(_, object)| object.clone()).collect::<Vec<_>>();
            if selected.is_empty() { return; }
            let confirmation = adw::AlertDialog::builder()
                .heading("Delete selected cloud saves?")
                .body(format!("Delete {} remote files after verifying a recovery copy and rechecking their revisions. Stop games and other cloud clients first: an upload after the final recheck could be deleted without recovery. Local saves remain unchanged. Ludomere cannot automatically undo this deletion. Type DELETE to confirm.", selected.len()))
                .build();
            let entry = gtk::Entry::new(); entry.set_placeholder_text(Some("DELETE"));
            confirmation.set_extra_child(Some(&entry));
            confirmation.add_responses(&[("cancel", "Cancel"), ("delete", "Delete remote files")]);
            confirmation.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            confirmation.set_default_response(Some("cancel")); confirmation.set_close_response("cancel"); confirmation.set_response_enabled("delete", false);
            let weak = confirmation.downgrade();
            entry.connect_changed(move |entry| { if let Some(dialog) = weak.upgrade() { dialog.set_response_enabled("delete", entry.text() == "DELETE"); } });
            let epoch = model.borrow().account_epoch;
            let dialog = dialog.clone(); let model = model.clone(); let session = session.clone(); let busy = busy.clone();
            let refresh = refresh.clone(); let select_all = select_all.clone(); let rows = rows.clone(); let status = status.clone(); let button = button.clone();
            confirmation.choose(Some(&dialog.clone()), gio::Cancellable::NONE, move |response| {
                if response != "delete" || entry.text() != "DELETE" || model.borrow().account_epoch != epoch || !dialog.is_visible() { return; }
                busy.set(true); button.set_sensitive(false); refresh.set_sensitive(false); select_all.set_sensitive(false);
                for (check, _) in rows.borrow().iter() { check.set_sensitive(false); }
                status.set_label("Verifying a recovery copy, then deleting the selected revisions…");
                run_worker(&dialog, &model, session, move |session| backup::delete(&session, &selected), move |result| {
                    busy.set(false); refresh.set_sensitive(true);
                    match result {
                        Ok(report) => status.set_label(&format!("Confirmed deletion of {} of {} files. Recovery copy: {}. {}", report.deleted, report.requested, report.recovery_snapshot.display(), report.error.map_or_else(|| "Use Refresh to check current storage.".into(), |error| format!("Stopped: {error}. Some requests may have completed; refresh before retrying.")))),
                        Err(error) => status.set_label(&format!("Deletion was not started: {error}. Use Refresh before retrying.")),
                    }
                });
            });
        });
    }
    dialog.present();
    refresh.emit_clicked();
}
