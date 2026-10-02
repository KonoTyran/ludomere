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
    alternatives: gtk::Box,
    exact_library: RefCell<Option<String>>,
    prepare_recovery: gtk::Button,
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
        self.prepare_recovery.set_visible(false);
        while let Some(child) = self.alternatives.first_child() {
            self.alternatives.remove(&child);
        }
        self.status
            .set_label("Checking installation, operations and downloaded files…");
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.set_response_enabled("uninstall", false);
        }
        let config = self.model.borrow().config.clone();
        let id = self.game.product_id;
        let slug = self.game.slug.clone();
        let exact_library = self.exact_library.borrow().clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = match &exact_library {
                Some(library) => crate::installation::recovery::prepare_game_directory_reset(
                    &config, id, &slug, library,
                )
                .map(UninstallPreparation::Recovery),
                None => crate::installation::prepare_uninstall(&config, id, &slug),
            }
            .and_then(|choice| {
                let prefix = match &choice {
                    UninstallPreparation::Normal(installed) => {
                        crate::installation::uninstall_prefix(installed)?
                    }
                    UninstallPreparation::Recovery(_) => None,
                };
                let downloads = matches!(&choice, UninstallPreparation::Normal(_))
                    .then(|| download::managed_downloads(id).map_err(|error| format!("{error:#}")));
                Ok((choice, downloads, prefix))
            });
            let recovery_needed = result.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<crate::installation::recovery::DamagedOperationRecord>()
                    .is_some()
            });
            let result = result.map_err(|error| format!("{error:#}"));
            let alternatives = if exact_library.is_none() {
                crate::storage::game_directories_for_browsing(&config, &slug)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|path| {
                        config
                            .game_libraries
                            .iter()
                            .find(|library| path.parent() == Some(library.path.as_path()))
                            .map(|library| (library.id.clone(), path))
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let _ = sender.send((result, alternatives, recovery_needed));
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
                Ok((result, alternatives, recovery_needed)) => {
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
                                    if preview.exact_library.borrow().is_some() {
                                        preview.description.set_label(&format!("{}\n\nYou chose a file-only reset instead of the game's uninstaller. The reset removes ALL contents of this exact folder after confirmation, including unrecognized files or saves you placed there. Existing Windows prefixes are kept; no Wine or Proton helper is needed for this file-only reset.", preview.description.label()));
                                    }
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
                            preview.description.set_label("Nothing has been removed. Browse the files to inspect or back up saves, then review the available recovery options below.");
                            preview.status.set_label(&notifications::failure_message(
                                "Could not prepare removal",
                                &error,
                            ));
                            preview.prepare_recovery.set_visible(
                                preview.exact_library.borrow().is_some() && recovery_needed,
                            );
                        }
                    }
                    for (library, path) in alternatives {
                        let review = gtk::Button::with_label(&format!(
                            "Review File Reset — {}",
                            path.display()
                        ));
                        review.set_tooltip_text(Some("Review an explicit reset of this exact game folder. No files are deleted until you confirm the affected paths."));
                        let weak = Rc::downgrade(&preview);
                        review.connect_clicked(move |_| {
                            if let Some(preview) = weak.upgrade() {
                                *preview.exact_library.borrow_mut() = Some(library.clone());
                                preview.load();
                            }
                        });
                        preview.alternatives.append(&review);
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

fn prepare_directory_recovery(preview: &Rc<Preview>) {
    if preview.closed.get() || !preview.valid() || preview.busy.get() {
        return;
    }
    let Some(library) = preview.exact_library.borrow().clone() else {
        return;
    };
    let confirmation = adw::AlertDialog::builder()
        .heading("Prepare damaged operation data for recovery?")
        .body("If an operation record is corrupt, Ludomere cannot safely tell whether its installer is still running. Preparation records this folder's identity and may require restarting your computer. After restart, return here and choose Prepare Recovery again. Unchanged damaged operation data will be retained as a recovery copy, then you can review file removal separately. Game files are not deleted by this step.")
        .build();
    confirmation.add_responses(&[("cancel", "Cancel"), ("prepare", "Prepare Recovery")]);
    confirmation.set_default_response(Some("cancel"));
    confirmation.set_close_response("cancel");
    let preview = preview.clone();
    confirmation.choose(Some(&preview.window.clone()), gio::Cancellable::NONE, move |response| {
        if response != "prepare" || !preview.valid() || preview.closed.get() || preview.busy.replace(true) { return; }
        preview.prepare_recovery.set_sensitive(false);
        preview.retry.set_sensitive(false);
        preview.status.set_label("Preparing safe recovery of the damaged operation record…");
        let config = preview.model.borrow().config.clone();
        let id = preview.game.product_id;
        let slug = preview.game.slug.clone();
        let session = online::account_session();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                anyhow::ensure!(online::account_session() == session, "The account changed. Review recovery again.");
                crate::installation::recovery::prepare_game_directory_recovery(&config, id, &slug, &library)
            })();
            let _ = sender.send(result);
        });
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if preview.closed.get() || !preview.valid() { return glib::ControlFlow::Break; }
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(_) => Err(anyhow::anyhow!("Recovery preparation stopped. Retry preparation.")),
            };
            preview.busy.set(false);
            preview.prepare_recovery.set_sensitive(true);
            preview.retry.set_sensitive(true);
            match result {
                Ok(crate::installation::recovery::RecoveryReadiness::Ready) => preview.load(),
                Ok(crate::installation::recovery::RecoveryReadiness::RestartRequired) => preview.status.set_label("Recovery is prepared. Restart your computer, then return to this game's Uninstall → Review File Reset → Prepare Recovery. No game files have been deleted."),
                Err(error) => preview.status.set_label(&format!("Recovery preparation did not complete: {error:#}. No game files have been deleted.")),
            }
            glib::ControlFlow::Break
        });
    });
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
    show_removal_dialog(window, model, game, refresh, None)
}

pub(super) fn show_game_directory_reset_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    library_id: String,
    refresh: Rc<dyn Fn()>,
) -> adw::AlertDialog {
    show_removal_dialog(window, model, game, refresh, Some(library_id))
}

fn show_removal_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
    refresh: Rc<dyn Fn()>,
    exact_library: Option<String>,
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
    let alternatives = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let prepare_recovery = gtk::Button::with_label("Prepare Recovery…");
    prepare_recovery.set_visible(false);
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
    let browse = gtk::Button::with_label("Browse Local Files Before Removing");
    browse.connect_clicked({
        let window = window.clone();
        let model = model.clone();
        let game = game.clone();
        let epoch = model.borrow().account_epoch;
        move |_| {
            if model.borrow().account_epoch == epoch && !model.borrow().logout_pending {
                browse_game_files(&window, &model, &game);
            }
        }
    });
    extra.append(&browse);
    extra.append(&retry);
    extra.append(&alternatives);
    extra.append(&prepare_recovery);
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
        alternatives,
        exact_library: RefCell::new(exact_library),
        prepare_recovery,
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
    preview.prepare_recovery.connect_clicked({
        let preview = Rc::downgrade(&preview);
        move |_| {
            if let Some(preview) = preview.upgrade() {
                prepare_directory_recovery(&preview);
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
    let install_again = gtk::Button::with_label("Install Again…");
    install_again.set_visible(false);
    content.append(&install_again);
    install_again.connect_clicked({
        let preview = preview.clone();
        let dialog = dialog.downgrade();
        move |_| {
            if !preview.valid() {
                return;
            }
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            show_install_dialog(&preview.window, &preview.model, &preview.game);
        }
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Rc::new(Cell::new(false));
    let progress = gtk::ProgressBar::new();
    progress.set_margin_start(20);
    progress.set_margin_end(20);
    content.append(&progress);
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
            show_removal_dialog(
                &preview.window,
                &preview.model,
                &preview.game,
                preview.refresh.clone(),
                preview.exact_library.borrow().clone(),
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
        let finished = finished.clone();
        move || {
            let Some(dialog) = dialog.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if closed.get() {
                return glib::ControlFlow::Break;
            }
            progress.set_visible(!finished.get());
            if !finished.get() {
                progress.pulse();
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
                    install_again.set_visible(!failed);
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
    #[ignore = "requires private HOME/all XDG, D-Bus and GTK display; removes only synthetic fixtures"]
    fn recovery_controls_review_cancel_reset_and_session_change() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p246-")
        );
        adw::init().unwrap();
        fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut result = vec![widget.clone()];
            let mut next = widget.first_child();
            while let Some(child) = next {
                result.extend(children(&child));
                next = child.next_sibling();
            }
            result
        }
        fn button(widget: &gtk::Widget, prefix: &str) -> Option<gtk::Button> {
            children(widget)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| {
                    button
                        .label()
                        .is_some_and(|label| label.starts_with(prefix))
                })
        }
        fn text(widget: &gtk::Widget, value: &str) -> bool {
            children(widget)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .any(|label| label.text().contains(value))
        }
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("games");
        let directory = library.join("synthetic-game");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("unrecognized-save.bin"), "synthetic save").unwrap();
        let config = Config {
            game_libraries: vec![crate::config::GameLibrary {
                id: "synthetic".into(),
                name: "Test".into(),
                path: library.clone(),
                default: true,
            }],
            ..Config::default()
        };
        config.save().unwrap();
        let game = Game {
            product_id: 9246001,
            slug: "synthetic-game".into(),
            title: "Synthetic Game".into(),
            ..Game::default()
        };
        StateStore::open()
            .unwrap()
            .upsert_normalized_library(std::slice::from_ref(&game))
            .unwrap();
        let detail = DetailPageModel::game(game.clone(), false);
        let model = Rc::new(RefCell::new(AppModel {
            config,
            games: vec![game],
            ..AppModel::default()
        }));
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.RecoveryControlsTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.set_content(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
        window.present();

        let dialog = show_uninstall_dialog(&window, &model, &detail, Rc::new(|| {}));
        wait_until(|| button(dialog.upcast_ref(), "Review File Reset —").is_some());
        assert!(directory.join("unrecognized-save.bin").exists());
        button(dialog.upcast_ref(), "Review File Reset —")
            .unwrap()
            .emit_clicked();
        wait_until(|| dialog.is_response_enabled("uninstall"));
        assert!(text(dialog.upcast_ref(), &directory.display().to_string()));
        assert!(text(dialog.upcast_ref(), "ALL contents"));
        assert!(text(dialog.upcast_ref(), "prefixes are kept"));
        button(dialog.upcast_ref(), "Browse Local Files Before Removing")
            .unwrap()
            .emit_clicked();
        wait_until(|| {
            window
                .visible_dialog()
                .is_some_and(|visible| visible != dialog)
        });
        let browse = window.visible_dialog().unwrap();
        wait_until(|| text(browse.upcast_ref(), &directory.display().to_string()));
        browse.close();
        wait_until(|| {
            window
                .visible_dialog()
                .is_some_and(|visible| visible == dialog)
        });
        dialog.close();
        wait_until(|| window.visible_dialog().is_none());
        assert!(
            directory.join("unrecognized-save.bin").exists(),
            "cancelling review must not delete files"
        );

        browse_recovery_directory(&window, &model, root.path().join("outside"));
        let invalid = window.visible_dialog().unwrap();
        wait_until(|| text(invalid.upcast_ref(), "Could not inspect game files"));
        invalid.close();
        wait_until(|| window.visible_dialog().is_none());
        let mut missing = detail.clone();
        missing.slug = "not-downloaded".into();
        browse_game_files(&window, &model, &missing);
        let empty = window.visible_dialog().unwrap();
        wait_until(|| text(empty.upcast_ref(), "No game files were found"));
        empty.close();
        wait_until(|| window.visible_dialog().is_none());

        let stale = show_uninstall_dialog(&window, &model, &detail, Rc::new(|| {}));
        model.borrow_mut().account_epoch += 1;
        wait_until(|| window.visible_dialog().is_none());
        assert!(!stale.is_response_enabled("uninstall"));
        assert!(directory.exists());

        let other_library = root.path().join("other-games");
        let other_copy = other_library.join("synthetic-game");
        std::fs::create_dir_all(&other_copy).unwrap();
        std::fs::write(other_copy.join("keep-this-copy"), "other game copy").unwrap();
        model
            .borrow_mut()
            .config
            .game_libraries
            .push(crate::config::GameLibrary {
                id: "other".into(),
                name: "Other".into(),
                path: other_library,
                default: false,
            });
        model.borrow().config.save().unwrap();
        show_directory_repair_dialog(&window, &model, &detail, directory.clone());
        let repair = window.visible_dialog().unwrap();
        wait_until(|| {
            text(
                repair.upcast_ref(),
                "This folder has no recognized Depot installation",
            )
        });
        button(repair.upcast_ref(), "Review File Reset…")
            .unwrap()
            .emit_clicked();
        wait_until(|| {
            window
                .visible_dialog()
                .is_some_and(|dialog| dialog != repair)
        });
        let reset = window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        wait_until(|| reset.is_response_enabled("uninstall"));
        assert!(text(reset.upcast_ref(), &directory.display().to_string()));
        assert!(
            !text(reset.upcast_ref(), &other_copy.display().to_string()),
            "selected-directory recovery must not target another copy"
        );
        assert!(directory.exists());
        button(reset.upcast_ref(), "Remove files and reset")
            .unwrap()
            .emit_clicked();
        wait_until(|| !directory.exists());
        assert!(other_copy.join("keep-this-copy").exists());
        wait_until(|| {
            window.visible_dialog().is_some_and(|dialog| {
                button(dialog.upcast_ref(), "Install Again…")
                    .is_some_and(|button| button.is_visible())
            })
        });
        let progress = window.visible_dialog().unwrap();
        assert!(text(progress.upcast_ref(), "Removal finished"));
        assert!(
            crate::installation::depot_operation_snapshots().is_empty(),
            "recovery must not queue a new installation automatically"
        );
        button(progress.upcast_ref(), "Install Again…")
            .unwrap()
            .emit_clicked();
        wait_until(|| {
            window
                .visible_dialog()
                .is_some_and(|dialog| dialog != progress)
        });
        assert!(
            crate::installation::depot_operation_snapshots().is_empty(),
            "opening installation choices must not enqueue a download"
        );
        window.visible_dialog().unwrap().close();
        window.close();
    }
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
