use super::*;

pub(super) fn check_updates(w: &Widgets, model: &Rc<RefCell<AppModel>>, manual: bool) {
    let (config, games, token, epoch, session) = {
        let state = model.borrow();
        if state.logout_pending || !state.network_available {
            if manual {
                show_status(w, "Go online and sign in to check for updates.");
            }
            return;
        }
        let Some(token) = state.account_token.clone() else {
            if manual {
                show_status(w, "Sign in to check for updates.");
            }
            return;
        };
        (
            state.config.clone(),
            state.games.clone(),
            token,
            state.account_epoch,
            online::account_session(),
        )
    };
    if manual {
        show_progress(w, "Checking and queuing game updates…");
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mode = if manual {
            crate::updates::CheckMode::Manual
        } else {
            crate::updates::CheckMode::Automatic
        };
        let _ = sender.send(crate::updates::check_and_queue(
            &config, &games, &token, mode, session,
        ));
    });
    let w = w.clone_refs();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(report)) => {
                if w.live_status.label() == "Checking and queuing game updates…" {
                    show_progress(&w, "");
                }
                if !report.already_running {
                    refresh_local_action_state(&w, &model);
                }
                if !report.already_running
                    && (manual
                        || !report.failures.is_empty()
                        || report.galaxy_updates_queued
                            + report.offline_installers_queued
                            + report.extras_queued
                            > 0)
                {
                    let message = format!(
                        "Queued {} Depot update(s), {} installer update(s), {} extras update(s); {} busy/running; {} failed",
                        report.galaxy_updates_queued,
                        report.offline_installers_queued,
                        report.extras_queued,
                        report.skipped_running + report.skipped_busy,
                        report.failures.len()
                    );
                    let failures = report
                        .failures
                        .iter()
                        .map(|(id, error)| format!("{id}: {error}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    hold_status_notice(
                        Some(&w.status),
                        &if failures.is_empty() {
                            message
                        } else {
                            format!("{message}\n{failures}")
                        },
                    );
                } else if manual && report.already_running {
                    show_status(&w, "An update check is already running");
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                if w.live_status.label() == "Checking and queuing game updates…" {
                    show_progress(&w, "");
                }
                hold_status_notice(
                    Some(&w.status),
                    &super::notifications::failure_message(
                        "Game update check failed. Retry from Settings.",
                        &format!("{error:#}"),
                    ),
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if w.live_status.label() == "Checking and queuing game updates…" {
                    show_progress(&w, "");
                }
                hold_status_notice(
                    Some(&w.status),
                    "Game update check stopped unexpectedly. Retry from Settings.",
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });
}

pub(super) fn global_group(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Game updates and installer retention");
    group.set_description(Some("Checked after library synchronization and every six hours online. Running and busy games are skipped. Hidden games retain their update policies."));
    for (index, title, subtitle, active) in [
        (
            0,
            "Automatically update Depot installations",
            "Download and apply available updates to installed generation-two Windows Depot games",
            model.borrow().config.auto_update_galaxy_installations,
        ),
        (
            2,
            "Clean up superseded offline installers",
            "Move old managed revisions to Trash only after their replacements are verified",
            model.borrow().config.prune_superseded_offline_installers,
        ),
    ] {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(active)
            .build();
        group.add(&row);
        let model = model.clone();
        let w = w.clone();
        row.connect_active_notify(move |row| {
            let mut state = model.borrow_mut();
            match index {
                0 => state.config.auto_update_galaxy_installations = row.is_active(),
                _ => state.config.prune_superseded_offline_installers = row.is_active(),
            }
            if state.config.save().is_err() {
                show_status(&w, "Could not save update policy. Try again.");
            }
        });
    }
    let row = adw::ActionRow::builder()
        .title("Check and queue updates")
        .subtitle(
            "Apply the selected update policies now; this can queue game downloads and updates",
        )
        .build();
    let button = gtk::Button::with_label("Check and queue updates");
    button.set_valign(gtk::Align::Center);
    row.add_suffix(&button);
    group.add(&row);
    let w = w.clone();
    let model = model.clone();
    button.connect_clicked(move |_| check_updates(&w, &model, true));
    group
}

// Reads share the write queue so reopening Properties cannot observe an older pending save.
pub(super) fn policy_request<T: Send + 'static>(
    operation: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> mpsc::Receiver<anyhow::Result<T>> {
    static REQUESTS: std::sync::LazyLock<mpsc::Sender<Box<dyn FnOnce() + Send>>> =
        std::sync::LazyLock::new(|| {
            let (sender, receiver) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
            std::thread::spawn(move || {
                for request in receiver {
                    request();
                }
            });
            sender
        });
    let (sender, receiver) = mpsc::channel();
    let _ = REQUESTS.send(Box::new(move || {
        let _ = sender.send(operation());
    }));
    receiver
}

pub(super) fn game_group(
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Update policy and Depot language");
    group.set_description(Some("Inherit uses the global setting; On or Off overrides it for this game. Checks run after library synchronization and every six hours while signed in and online. Changes save automatically for future checks without starting downloads."));
    let status = gtk::Label::new(Some("Loading saved policies…"));
    status.set_wrap(true);
    status.set_xalign(0.0);
    group.add(&status);
    let mut selectors = Vec::new();
    for (title, subtitle) in [
        (
            "Depot updates",
            "Download and apply updates to supported installed Depot games. Running and busy games are skipped.",
        ),
        (
            "Keep downloaded installers up to date",
            "Update existing installer copies in their current Offline Installers libraries.",
        ),
        (
            "Old-installer cleanup",
            "Move superseded installers to Trash after a replacement is verified in the same library.",
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .build();
        let choice = gtk::DropDown::from_strings(&["Inherit global setting", "On", "Off"]);
        choice.set_valign(gtk::Align::Center);
        choice.set_sensitive(false);
        row.add_suffix(&choice);
        group.add(&row);
        selectors.push(choice);
    }
    let language = adw::EntryRow::builder()
        .title("Depot language code, e.g. en (blank inherits default)")
        .build();
    language.set_sensitive(false);
    group.add(&language);
    let id = game.product_id;
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let loaded = Rc::new(std::cell::Cell::new(false));
    let receiver = policy_request(move || {
        let _activity = crate::profile_reset::begin_activity("loading update preferences")?;
        online::with_account_session(session, || StateStore::open()?.game_preferences(id))
    });
    {
        let model = model.clone();
        let group = group.downgrade();
        let selectors = selectors.clone();
        let language = language.clone();
        let status = status.clone();
        let loaded = loaded.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if group.upgrade().is_none()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
            {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(preferences)) => {
                    let p = preferences.unwrap_or_default();
                    for (choice, value) in selectors.iter().zip([
                        p.auto_update_galaxy,
                        p.auto_download_offline_installer,
                        p.prune_superseded_installers,
                    ]) {
                        choice.set_selected(match value {
                            None => 0,
                            Some(true) => 1,
                            Some(false) => 2,
                        });
                        choice.set_sensitive(true);
                    }
                    language.set_text(p.galaxy_language.as_deref().unwrap_or(""));
                    language.set_sensitive(true);
                    loaded.set(true);
                    status.set_label("Changes save automatically. Blank language inherits the default; editing it does not start a download.");
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    status.set_label(&super::notifications::failure_message(
                        "Could not load update preferences. Close and reopen Properties to retry.",
                        &format!("{error:#}"),
                    ));
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.set_label("Loading update preferences stopped unexpectedly. Close and reopen Properties to retry.");
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            }
        });
    }
    let save: Rc<dyn Fn()> = Rc::new({
        let model = model.clone();
        let status = status.downgrade();
        let language = language.downgrade();
        let selectors = selectors.iter().map(|s| s.downgrade()).collect::<Vec<_>>();
        let revision = Rc::new(std::cell::Cell::new(0_u64));
        move || {
            if !loaded.get()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
            {
                return;
            }
            let Some(language) = language.upgrade() else {
                return;
            };
            let Some(selectors) = selectors
                .iter()
                .map(|s| s.upgrade())
                .collect::<Option<Vec<_>>>()
            else {
                return;
            };
            let policies = selectors
                .iter()
                .map(|s| match s.selected() {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let language = language.text().trim().to_owned();
            revision.set(revision.get() + 1);
            let saved_revision = revision.get();
            if let Some(status) = status.upgrade() {
                status.set_label("Saving preferences…");
            }
            let receiver = policy_request(move || {
                let _activity = crate::profile_reset::begin_activity("saving update preferences")?;
                online::with_account_session(session, || {
                    StateStore::open()?.set_game_update_preferences(
                        id,
                        policies[0],
                        policies[1],
                        policies[2],
                        (!language.is_empty()).then_some(language.as_str()),
                    )
                })
            });
            let status = status.clone();
            let completion_model = model.clone();
            let revision = revision.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if completion_model.borrow().account_epoch != epoch
                    || completion_model.borrow().logout_pending
                    || revision.get() != saved_revision
                {
                    return glib::ControlFlow::Break;
                }
                let Some(status) = status.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match receiver.try_recv() {
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Ok(Ok(())) => status.set_label("Preferences saved. No download was started."),
                    Ok(Err(error)) => status.set_label(&format!(
                        "Could not save preferences: {error}. Change the option again to retry."
                    )),
                    Err(_) => status.set_label("Saving stopped. Reopen Properties and try again."),
                }
                glib::ControlFlow::Break
            });
        }
    });
    for choice in selectors {
        let save = save.clone();
        choice.connect_selected_notify(move |_| save());
    }
    language.connect_changed(move |_| save());
    group
}
