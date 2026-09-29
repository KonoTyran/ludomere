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
                        || report.galaxy_updates_queued + report.offline_installers_queued > 0)
                {
                    let message = format!(
                        "Queued {} Depot update(s), {} offline backup(s); {} busy/running; {} failed",
                        report.galaxy_updates_queued,
                        report.offline_installers_queued,
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
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                if w.live_status.label() == "Checking and queuing game updates…" {
                    show_progress(&w, "");
                }
                hold_status_notice(
                    Some(&w.status),
                    "Game update check failed. Retry from Settings.",
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
            1,
            "Automatically download offline backups",
            "Opt in to new offline installer revisions",
            model.borrow().config.auto_download_offline_installers,
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
                1 => state.config.auto_download_offline_installers = row.is_active(),
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

pub(super) fn game_group(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    game: &DetailPageModel,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Update policy and Depot language");
    let status = gtk::Label::new(Some("Loading saved policies…"));
    status.set_wrap(true);
    status.set_xalign(0.0);
    group.add(&status);
    let mut selectors = Vec::new();
    for title in [
        "Depot updates",
        "Offline backup downloads",
        "Old-installer cleanup",
    ] {
        let row = adw::ActionRow::builder().title(title).build();
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
    let apply = gtk::Button::with_label("Save policies");
    apply.set_sensitive(false);
    group.add(&apply);
    let reconcile = gtk::Button::with_label("Apply Depot language…");
    reconcile.set_sensitive(false);
    group.add(&reconcile);
    let id = game.product_id;
    let depot_installed = Rc::new(std::cell::Cell::new(false));
    let installed = model.borrow().installed_games.get(&id).cloned();
    let epoch = model.borrow().account_epoch;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let depot = installed.as_ref().is_some_and(|game| {
            crate::installation::load_installation_marker(&game.installation_directory)
                .ok()
                .flatten()
                .is_some_and(|marker| {
                    marker.source == crate::domain::InstallationSource::GalaxyDepot
                })
        });
        let _ = sender.send(
            StateStore::open()
                .and_then(|s| s.game_preferences(id))
                .map(|preferences| (preferences, depot)),
        );
    });
    {
        let model = model.clone();
        let group = group.downgrade();
        let selectors = selectors.clone();
        let language = language.clone();
        let status = status.clone();
        let apply = apply.clone();
        let reconcile = reconcile.clone();
        let depot_installed = depot_installed.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if group.upgrade().is_none()
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
            {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok((preferences, depot))) => {
                    depot_installed.set(depot);
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
                    apply.set_sensitive(true);
                    reconcile.set_sensitive(depot);
                    reconcile
                        .set_tooltip_text((!depot).then_some(
                            "Install this game from Depot before applying its language",
                        ));
                    status.set_label("Policies inherit global values unless overridden. Save stores the language override for future updates; Apply Depot language reconciles it now.");
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    status.set_label("Could not load preferences. Close and reopen to retry.");
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            }
        });
    }
    let save: Rc<dyn Fn(bool)> = Rc::new({
        let model = model.clone();
        let status = status.downgrade();
        let language = language.clone();
        let selectors = selectors.clone();
        let window = window.downgrade();
        let apply = apply.downgrade();
        let reconcile_button = reconcile.downgrade();
        move |reconcile| {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                return;
            }
            let Some(window) = window.upgrade() else {
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
            let (config, game, token, session) = {
                let state = model.borrow();
                (
                    state.config.clone(),
                    state.games.iter().find(|g| g.product_id == id).cloned(),
                    state.account_token.clone(),
                    online::account_session(),
                )
            };
            let status = status.clone();
            let completion_model = model.clone();
            let apply = apply.clone();
            let reconcile_button = reconcile_button.clone();
            let depot_installed = depot_installed.clone();
            let run = move || {
                let (Some(apply), Some(reconcile_button)) =
                    (apply.upgrade(), reconcile_button.upgrade())
                else {
                    return;
                };
                if !completion_model.borrow_mut().policy_saving.insert(id) {
                    if let Some(status) = status.upgrade() {
                        status.set_label(
                            "Wait for the current policy save to finish, then try again.",
                        );
                    }
                    return;
                }
                apply.set_sensitive(false);
                reconcile_button.set_sensitive(false);
                if let Some(status) = status.upgrade() {
                    status.set_label("Saving preferences…");
                }
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<String> {
                        online::with_account_session(session, || {
                            StateStore::open()?.set_game_update_preferences(
                                id,
                                policies[0],
                                policies[1],
                                policies[2],
                                (!language.is_empty()).then_some(language.as_str()),
                            )
                        })?;
                        if reconcile {
                            let token = token.ok_or_else(|| {
                                anyhow::anyhow!("Sign in to apply the Depot language")
                            })?;
                            let game = game.ok_or_else(|| {
                                anyhow::anyhow!("Reopen the game to apply its language")
                            })?;
                            let queued = crate::updates::queue_language_reconciliation(
                                &config, &game, &token, session,
                            )?;
                            Ok(if queued {
                                "Language reconciliation queued"
                            } else {
                                "No language change was required"
                            }
                            .into())
                        } else {
                            Ok("Policies saved".into())
                        }
                    })();
                    let _ = sender.send(result);
                });
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    if completion_model.borrow().account_epoch != epoch
                        || completion_model.borrow().logout_pending
                    {
                        return glib::ControlFlow::Break;
                    }
                    let result = match receiver.try_recv() {
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        result => result,
                    };
                    completion_model.borrow_mut().policy_saving.remove(&id);
                    let Some(status) = status.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    apply.set_sensitive(true);
                    reconcile_button.set_sensitive(depot_installed.get());
                    match result {
                        Ok(Ok(message)) => {
                            status.set_label(&message);
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            status.set_label(&error.to_string());
                            glib::ControlFlow::Break
                        }
                        Err(_) => {
                            status.set_label("Saving stopped. Try again.");
                            glib::ControlFlow::Break
                        }
                    }
                });
            };
            if reconcile {
                let dialog = adw::AlertDialog::builder()
                    .heading("Apply Depot language?")
                    .body("Save these preferences and reconcile the Depot language now. This may download language files and update to the latest available build on the same branch.")
                    .build();
                dialog.add_responses(&[("cancel", "Cancel"), ("apply", "Apply language")]);
                dialog.set_close_response("cancel");
                let model = model.clone();
                dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                    if response == "apply"
                        && model.borrow().account_epoch == epoch
                        && !model.borrow().logout_pending
                    {
                        run();
                    }
                });
            } else {
                run();
            }
        }
    });
    apply.connect_clicked({
        let save = save.clone();
        move |_| save(false)
    });
    reconcile.connect_clicked(move |_| save(true));
    group
}
