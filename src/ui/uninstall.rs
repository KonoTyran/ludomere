use super::*;
use crate::installation::{GameResetResult, UninstallPreparation};
use std::cell::Cell;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Preview {
    window: adw::ApplicationWindow,
    model: Rc<RefCell<AppModel>>,
    game: DetailPageModel,
    epoch: u64,
    dialog: glib::WeakRef<adw::AlertDialog>,
    choice: RefCell<Option<UninstallPreparation>>,
    downloads: RefCell<Option<download::ManagedDownloads>>,
    prefix: RefCell<Option<std::path::PathBuf>>,
    cleanup: gtk::CheckButton,
    description: gtk::Label,
    status: gtk::Label,
    retry: gtk::Button,
    busy: Cell<bool>,
    closed: Cell<bool>,
    refresh: Rc<dyn Fn()>,
}

impl Preview {
    fn valid(&self) -> bool {
        let model = self.model.borrow();
        model.account_epoch == self.epoch && !model.logout_pending
    }

    fn load(self: &Rc<Self>) {
        if self.closed.get() || !self.valid() || self.busy.replace(true) {
            return;
        }
        self.choice.borrow_mut().take();
        self.downloads.borrow_mut().take();
        self.prefix.borrow_mut().take();
        self.cleanup.set_active(false);
        self.cleanup.set_sensitive(false);
        self.retry.set_sensitive(false);
        self.status
            .set_label("Checking installation, operations and downloaded files…");
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.set_response_enabled("uninstall", false);
        }
        let config = self.model.borrow().config.clone();
        let id = self.game.product_id;
        let slug = self.game.slug.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::installation::prepare_uninstall(&config, id, &slug)
                .and_then(|choice| {
                    let prefix = match &choice {
                        UninstallPreparation::Normal(installed) => {
                            crate::installation::uninstall_prefix(installed)?
                        }
                        UninstallPreparation::Recovery(_) => None,
                    };
                    let downloads = matches!(&choice, UninstallPreparation::Normal(_)).then(|| {
                        download::managed_downloads(id).map_err(|error| format!("{error:#}"))
                    });
                    Ok((choice, downloads, prefix))
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        let preview = self.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(dialog) = preview.dialog.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if preview.closed.get() {
                return glib::ControlFlow::Break;
            }
            if !preview.valid() {
                dialog.close();
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(result) => {
                    preview.busy.set(false);
                    preview.retry.set_sensitive(true);
                    match result {
                        Ok((choice, downloads, prefix)) => {
                            match &choice {
                                UninstallPreparation::Normal(installed) => {
                                    dialog.set_response_label("uninstall", "Uninstall");
                                    preview.description.set_label(&normal_description(
                                        &installed.installation_directory,
                                        prefix.as_deref(),
                                    ));
                                    match downloads {
                                        Some(Ok(files)) => {
                                            preview.cleanup.set_sensitive(files.count() > 0);
                                            preview.status.set_label(&format!("{} managed downloaded files ({})", files.count(), human_size(files.bytes())));
                                            *preview.downloads.borrow_mut() = Some(files);
                                        }
                                        Some(Err(error)) => preview.status.set_label(&notifications::failure_message("Downloaded files could not be checked. They will be kept; retry to enable optional cleanup.", &error)),
                                        None => {}
                                    }
                                }
                                UninstallPreparation::Recovery(plan) => {
                                    preview.description.set_label(&recovery_description(
                                        &plan.directories,
                                        &plan.prefixes,
                                    ));
                                    preview.cleanup.set_sensitive(true);
                                    preview.status.set_label(&format!("{} recorded downloaded files ({}). If selected, files finishing while work stops are included. External saves, other games' prefixes, Proton/runtime files, playtime and Ludomere preferences are kept.", plan.downloaded_files, human_size(plan.downloaded_bytes)));
                                    dialog
                                        .set_response_label("uninstall", "Remove files and reset");
                                }
                            }
                            *preview.prefix.borrow_mut() = prefix;
                            *preview.choice.borrow_mut() = Some(choice);
                            dialog.set_response_enabled("uninstall", true);
                        }
                        Err(error) => {
                            preview.description.set_label("Nothing has been removed. Resolve the problem below and retry the check.");
                            preview.status.set_label(&notifications::failure_message(
                                "Could not prepare removal",
                                &error,
                            ));
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    preview.busy.set(false);
                    preview.retry.set_sensitive(true);
                    preview.status.set_label(
                        "The removal check stopped. Nothing was removed. Retry the check.",
                    );
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn notice(&self, message: &str) {
        if self.valid() {
            let label =
                find_named_descendant(self.window.upcast_ref(), "application-status-message")
                    .and_downcast::<gtk::Label>();
            hold_status_notice(label.as_ref(), &format!("{}: {message}", self.game.title));
            (self.refresh)();
        }
    }
}

fn normal_description(directory: &std::path::Path, prefix: Option<&std::path::Path>) -> String {
    let mut description = format!(
        "Remove the installed game from {}? Downloaded files are kept unless selected below.",
        directory.display()
    );
    if let Some(prefix) = prefix {
        description.push_str(&format!(
            "\n\nAlso permanently delete this managed Windows prefix, including all saves and settings INSIDE it:\n{}\n\nExternal saves, other games' prefixes, Proton/runtime files, playtime and Ludomere preferences are kept.",
            prefix.display()
        ));
    }
    description
}

fn recovery_description(paths: &[std::path::PathBuf], prefixes: &[std::path::PathBuf]) -> String {
    let mut description = if paths.is_empty() {
        "Stop this game's downloads/install operations and reset their state? There is no verified game directory to remove. Downloaded installers and extras are kept unless selected below.".into()
    } else {
        format!(
            "Stop this game's downloads/install operations, delete all remaining files in the following game directories, and reset operation state? This includes untracked files and saves stored INSIDE these directories. Downloaded installers and extras are kept unless selected below.\n\nGame directories:\n{}",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    if !prefixes.is_empty() {
        description.push_str(&format!(
            "\n\nAlso permanently delete the following managed Windows prefixes, including all saves and settings INSIDE them. Saves OUTSIDE the listed game directories and prefixes are kept.\n\nWindows prefixes:\n{}",
            prefixes.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n")
        ));
    } else {
        description.push_str("\n\nSaves OUTSIDE the listed game directories are kept.");
    }
    description
}

pub(super) fn show_uninstall_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    refresh: Rc<dyn Fn()>,
) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Uninstall {}?", game.title))
        .body("Review the removal method and affected files below.")
        .build();
    dialog.set_widget_name("game-uninstall-confirmation");
    dialog.add_responses(&[("cancel", "Cancel"), ("uninstall", "Uninstall")]);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("uninstall", false);
    dialog.set_response_appearance("uninstall", adw::ResponseAppearance::Destructive);
    let cleanup = gtk::CheckButton::with_label(
        "Also delete downloaded installers, patches, extras and DLC backups",
    );
    cleanup.set_active(false);
    cleanup.set_sensitive(false);
    let description = gtk::Label::new(Some("Checking the current installation and active work…"));
    description.set_wrap(true);
    description.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    description.set_selectable(true);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_selectable(true);
    let retry = gtk::Button::with_label("Retry removal check");
    let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let information = gtk::Box::new(gtk::Orientation::Vertical, 8);
    information.append(&description);
    information.append(&status);
    extra.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(280)
            .child(&information)
            .build(),
    );
    extra.append(&cleanup);
    extra.append(&retry);
    dialog.set_extra_child(Some(&extra));
    let preview = Rc::new(Preview {
        window: window.clone(),
        model: model.clone(),
        game: game.clone(),
        epoch: model.borrow().account_epoch,
        dialog: dialog.downgrade(),
        choice: RefCell::new(None),
        downloads: RefCell::new(None),
        prefix: RefCell::new(None),
        cleanup,
        description,
        status,
        retry,
        busy: Cell::new(false),
        closed: Cell::new(false),
        refresh,
    });
    preview.retry.connect_clicked({
        let preview = Rc::downgrade(&preview);
        move |_| {
            if let Some(preview) = preview.upgrade() {
                preview.load();
            }
        }
    });
    preview.load();
    glib::timeout_add_local(Duration::from_millis(100), {
        let preview = Rc::downgrade(&preview);
        move || {
            let Some(preview) = preview.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if preview.closed.get() {
                return glib::ControlFlow::Break;
            }
            if !preview.valid() {
                if let Some(dialog) = preview.dialog.upgrade() {
                    dialog.close();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        }
    });
    dialog
        .clone()
        .choose(Some(window), gio::Cancellable::NONE, move |response| {
            preview.closed.set(true);
            if response != "uninstall" || !preview.valid() {
                return;
            }
            let Some(choice) = preview.choice.borrow_mut().take() else {
                return;
            };
            match choice {
                UninstallPreparation::Normal(installed) => queue_normal(preview, installed),
                UninstallPreparation::Recovery(plan) => run_recovery(preview, plan),
            }
        });
    dialog
}

fn queue_normal(preview: Rc<Preview>, installed: crate::domain::InstalledGame) {
    let windows = installed.installer_operating_system.as_deref() != Some("linux");
    let directory = installed.installation_directory.clone();
    let parent = preview.window.clone();
    let id = installed.product_id;
    let action = move || {
        if !preview.valid() {
            return;
        }
        let cleanup = if preview.cleanup.is_active() {
            preview.downloads.borrow_mut().take()
        } else {
            None
        };
        let config = preview.model.borrow().config.clone();
        let reviewed_prefix = preview.prefix.borrow().clone();
        let slug = preview.game.slug.clone();
        let session = online::account_session();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                anyhow::ensure!(
                    online::account_session() == session,
                    "Account changed; review removal again"
                );
                let fresh = crate::installation::prepare_uninstall(&config, id, &slug)?;
                let UninstallPreparation::Normal(fresh) = fresh else {
                    anyhow::bail!(
                        "The operation state changed. Reopen Uninstall to review recovery removal."
                    );
                };
                anyhow::ensure!(
                    fresh.installation_directory == installed.installation_directory,
                    "Installation location changed; review removal again"
                );
                anyhow::ensure!(
                    crate::installation::uninstall_prefix(&fresh)? == reviewed_prefix,
                    "Windows prefix location changed; review removal again"
                );
                anyhow::ensure!(
                    online::account_session() == session,
                    "Account changed; review removal again"
                );
                anyhow::ensure!(
                    crate::installation::enqueue_uninstallation_with_cleanup(fresh, cleanup),
                    "The game is busy. Wait for current work to stop, then retry."
                );
                Ok(())
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if !preview.valid() {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    preview.notice("Uninstallation queued");
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    preview.notice(&notifications::failure_message(
                        "Could not queue uninstallation",
                        &error,
                    ));
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    preview.notice("Uninstall preparation stopped. Reopen Uninstall to retry.");
                    glib::ControlFlow::Break
                }
            }
        });
    };
    if windows {
        with_windows_components(&parent, id, true, Some(directory), action);
    } else {
        action();
    }
}

enum ResetEvent {
    Progress(String),
    Complete(Result<GameResetResult, String>),
}

fn run_recovery(preview: Rc<Preview>, plan: crate::installation::GameResetPlan) {
    if !preview.valid() {
        return;
    }
    let dialog = adw::Dialog::builder()
        .title("Removing game files")
        .content_width(540)
        .build();
    dialog.set_widget_name("game-reset-progress");
    dialog.set_can_close(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&adw::HeaderBar::new());
    let status = gtk::Label::new(Some("Stopping affected work before removing files…"));
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_selectable(true);
    status.set_margin_start(20);
    status.set_margin_end(20);
    content.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(80)
            .max_content_height(300)
            .child(&status)
            .build(),
    );
    let cancel = gtk::Button::with_label("Cancel removal");
    cancel.set_margin_start(20);
    cancel.set_margin_end(20);
    cancel.set_margin_bottom(16);
    content.append(&cancel);
    let retry = gtk::Button::with_label("Review and retry");
    retry.set_visible(false);
    content.append(&retry);
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Rc::new(Cell::new(false));
    cancel.connect_clicked({
        let cancelled = cancelled.clone();
        let dialog = dialog.downgrade();
        let finished = finished.clone();
        move |button| {
            if finished.get() {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
            } else {
                cancelled.store(true, Ordering::Release);
                button.set_label("Stopping removal…");
                button.set_sensitive(false);
            }
        }
    });
    retry.connect_clicked({
        let dialog = dialog.downgrade();
        let preview = preview.clone();
        move |_| {
            if !preview.valid() {
                return;
            }
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            show_uninstall_dialog(
                &preview.window,
                &preview.model,
                &preview.game,
                preview.refresh.clone(),
            );
        }
    });
    dialog.set_child(Some(&content));
    let closed = Rc::new(Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    glib::timeout_add_local(Duration::from_millis(100), {
        let dialog = dialog.downgrade();
        let preview = preview.clone();
        let cancelled = cancelled.clone();
        move || {
            let Some(dialog) = dialog.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if closed.get() {
                return glib::ControlFlow::Break;
            }
            if !preview.valid() {
                cancelled.store(true, Ordering::Release);
                dialog.set_can_close(true);
                dialog.close();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        }
    });
    dialog.present(Some(&preview.window));
    let remove_downloads = preview.cleanup.is_active();
    let session = online::account_session();
    let (sender, receiver) = mpsc::sync_channel(16);
    let worker_cancel = cancelled.clone();
    std::thread::spawn(move || {
        let result = if online::account_session() != session {
            Err("Account changed; removal did not start".to_owned())
        } else {
            crate::installation::reset_game(plan, remove_downloads, &worker_cancel, |message| {
                let _ = sender.try_send(ResetEvent::Progress(message.to_owned()));
            })
            .map_err(|error| format!("{error:#}"))
        };
        let _ = sender.send(ResetEvent::Complete(result));
    });
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if !preview.valid() {
            cancelled.store(true, Ordering::Release);
            dialog.set_can_close(true);
            dialog.close();
            return glib::ControlFlow::Break;
        }
        loop {
            match receiver.try_recv() {
                Ok(ResetEvent::Progress(message)) => {
                    status.set_label(notifications::failure_message("", &message).trim_start())
                }
                Ok(ResetEvent::Complete(result)) => {
                    let failed = !matches!(&result, Ok(result) if result.failures.is_empty());
                    let message = match result {
                        Ok(result) if result.failures.is_empty() => format!(
                            "Removal finished. {} game directories and {} Windows prefixes removed; {} downloaded files kept.",
                            result.removed_directories,
                            result.removed_prefixes,
                            result.retained_downloads
                        ),
                        Ok(result) => notifications::failure_message(
                            "Removal was only partially completed. Some game or prefix files may remain. Review the remaining state before retrying.",
                            &result.failures.join("\n"),
                        ),
                        Err(error) => notifications::failure_message(
                            "Removal stopped. Some work may have been cancelled; review the current state before retrying.",
                            &error,
                        ),
                    };
                    status.set_label(&message);
                    preview.notice(&message);
                    finished.set(true);
                    cancel.set_label("Close");
                    cancel.set_sensitive(true);
                    retry.set_visible(failed);
                    dialog.set_can_close(true);
                    return glib::ControlFlow::Break;
                }
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.set_label("Removal worker stopped. Inspect the current state and review before retrying.");
                    preview.notice(&status.label());
                    finished.set(true);
                    cancel.set_label("Close");
                    cancel.set_sensitive(true);
                    retry.set_visible(true);
                    dialog.set_can_close(true);
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_warning_only_names_authoritative_windows_prefix() {
        let directory = std::path::Path::new("/games/Game");
        let prefix = std::path::Path::new("/games/.ludomere/compatibility/Game");
        let message = normal_description(directory, Some(prefix));
        assert!(message.contains(&prefix.display().to_string()));
        assert!(message.contains("all saves and settings INSIDE"));
        assert!(message.contains("External saves"));
        assert!(message.contains("kept unless selected"));
        assert!(!normal_description(directory, None).contains("prefix"));
    }
    #[test]
    fn recovery_warning_identifies_exact_scope_and_in_directory_save_loss() {
        let message = recovery_description(
            &["/games/Gungeon".into(), "/other/Game".into()],
            &["/games/.ludomere/compatibility/Gungeon".into()],
        );
        for required in [
            "/games/Gungeon",
            "/other/Game",
            "untracked",
            "INSIDE",
            "OUTSIDE",
            "Windows prefixes:",
            "/games/.ludomere/compatibility/Gungeon",
            "all saves and settings INSIDE",
            "kept unless selected",
        ] {
            assert!(message.contains(required), "{required}");
        }
        assert!(!message.contains("Prefixes and saves OUTSIDE"));
        let prefix_only =
            recovery_description(&[], &["/games/.ludomere/compatibility/Gungeon".into()]);
        assert!(prefix_only.contains("no verified game directory"));
        assert!(prefix_only.contains("permanently delete"));
        assert!(prefix_only.contains("/games/.ludomere/compatibility/Gungeon"));
        assert!(!recovery_description(&[], &[]).contains("Windows prefixes:"));
    }
}
