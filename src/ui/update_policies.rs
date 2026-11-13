use super::*;

pub(super) fn check_updates(w: &Widgets, model: &Rc<RefCell<AppModel>>, manual: bool) {
    start_update_check(w, model, manual, None);
}

fn start_update_check(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    manual: bool,
    feedback: Option<(glib::WeakRef<gtk::Button>, glib::WeakRef<gtk::Label>)>,
) {
    let finish = move |message: &str| {
        if let Some((button, status)) = &feedback {
            if let Some(status) = status.upgrade() {
                status.set_label(message);
                status.set_visible(true);
            }
            // Restore last: a sensitivity observer may immediately start a fresh request.
            if let Some(button) = button.upgrade() {
                button.set_sensitive(true);
            }
        }
    };
    let (config, games, token, epoch, session, auth_session) = {
        let state = model.borrow();
        if state.logout_pending || !state.network_available {
            drop(state);
            if manual {
                show_status(w, "Go online and sign in to check for updates.");
            }
            finish("Go online and sign in to check for updates.");
            return;
        }
        let Some(token) = state.account_token.clone() else {
            drop(state);
            if manual {
                show_status(w, "Sign in to check for updates.");
            }
            finish("Sign in to check for updates.");
            return;
        };
        (
            state.config.clone(),
            state.games.clone(),
            token,
            state.account_epoch,
            online::account_session(),
            auth::session(),
        )
    };
    let receiver = update_check_request(
        config,
        games,
        token,
        if manual {
            crate::updates::CheckMode::Manual
        } else {
            crate::updates::CheckMode::Automatic
        },
        session,
    );
    let mut receiver = Some(receiver);
    let w = w.clone_refs();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let stale = {
            let state = model.borrow();
            state.account_epoch != epoch
                || state.logout_pending
                || online::account_session() != session
                || auth::session() != auth_session
        };
        if stale {
            receiver.take();
            finish("Account/session changed. Check again after signing in.");
            return glib::ControlFlow::Break;
        }
        let Some(current) = receiver.as_ref() else {
            return glib::ControlFlow::Break;
        };
        let result = current.try_recv();
        if !matches!(&result, Err(mpsc::TryRecvError::Empty)) {
            receiver.take();
        }
        match result {
            Ok(Ok(report)) => {
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
                    let message = if failures.is_empty() {
                        message
                    } else {
                        notifications::failure_message(&message, &failures)
                    };
                    hold_status_notice(Some(&w.status), &message);
                    finish(&message);
                } else if manual && report.already_running {
                    show_status(&w, "Another update check is already running.");
                    finish("Another update check is already running.");
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                let message = notifications::failure_message(
                    "Game update check failed. Retry from Settings.",
                    &format!("{error:#}"),
                );
                hold_status_notice(Some(&w.status), &message);
                finish(&message);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                hold_status_notice(
                    Some(&w.status),
                    "Game update check stopped unexpectedly. Retry from Settings.",
                );
                finish("Game update check stopped unexpectedly. Retry from Settings.");
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });
}

#[cfg(test)]
type TestCheckRequest = (
    crate::updates::CheckMode,
    mpsc::Sender<anyhow::Result<crate::updates::UpdateCheckReport>>,
);

#[cfg(test)]
thread_local! {
    static TEST_CHECK_REQUESTS: RefCell<Option<mpsc::Sender<TestCheckRequest>>> = const { RefCell::new(None) };
}

fn update_check_request(
    config: Config,
    games: Vec<Game>,
    token: auth::Token,
    mode: crate::updates::CheckMode,
    session: u64,
) -> mpsc::Receiver<anyhow::Result<crate::updates::UpdateCheckReport>> {
    let (sender, receiver) = mpsc::channel();
    #[cfg(test)]
    if let Some(requests) = TEST_CHECK_REQUESTS.with(|requests| requests.borrow().clone()) {
        let _ = requests.send((mode, sender));
        return receiver;
    }
    std::thread::spawn(move || {
        let _ = sender.send(crate::updates::check_and_queue(
            &config, &games, &token, mode, session,
        ));
    });
    receiver
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
    button.set_widget_name("settings-check-updates");
    button.set_valign(gtk::Align::Center);
    row.add_suffix(&button);
    group.add(&row);
    let status = gtk::Label::new(None);
    status.set_widget_name("settings-update-check-status");
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_max_width_chars(70);
    status.set_xalign(0.0);
    status.set_selectable(true);
    status.set_visible(false);
    group.add(&status);
    let w = w.clone();
    let model = model.clone();
    button.connect_clicked({
        let status = status.downgrade();
        move |button| {
            if !button.is_sensitive() {
                return;
            }
            let Some(status) = status.upgrade() else {
                return;
            };
            button.set_sensitive(false);
            status.set_label("Checking and queuing updates…");
            status.set_visible(true);
            start_update_check(
                &w,
                &model,
                true,
                Some((button.downgrade(), status.downgrade())),
            );
        }
    });
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

#[cfg(test)]
mod feedback_tests {
    use super::*;
    use crate::updates::{CheckMode, UpdateCheckReport};

    #[test]
    #[ignore = "requires private HOME/all XDG/TMP, GTK and D-Bus; injected update outcomes only"]
    fn settings_update_check_feedback_tracks_its_request_and_releases_the_page() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p364-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn pump(duration: Duration) {
            let deadline = std::time::Instant::now() + duration;
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.UpdateFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let config = Config::default();
        let w = Rc::new(window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            network_available: true,
            account_token: Some(auth::Token {
                access_token: "synthetic-never-used".into(),
                refresh_token: "synthetic-never-used".into(),
                user_id: "synthetic".into(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
            }),
            // Exercise completion's existing refresh request without starting profile IO.
            local_refresh_running: true,
            ..Default::default()
        }));
        let (dispatch, requests) = mpsc::channel();
        TEST_CHECK_REQUESTS.with(|target| *target.borrow_mut() = Some(dispatch));
        let take = |mode| {
            let (actual, response) = requests.try_recv().unwrap();
            assert!(actual == mode);
            response
        };
        let settings = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(940)
            .default_height(650)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let focus = gtk::Entry::new();
        content.append(&focus);
        let page = adw::PreferencesPage::new();
        let group = global_group(&w, &model);
        page.add(&group);
        content.append(&page);
        settings.set_content(Some(&content));
        let button = find_named_descendant(group.upcast_ref(), "settings-check-updates")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let status = find_named_descendant(group.upcast_ref(), "settings-update-check-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        settings.present();
        wait(|| button.is_mapped());
        assert!(focus.grab_focus());
        let original_focus = gtk::prelude::GtkWindowExt::focus(&settings);
        assert!(original_focus.is_some());
        let original_page = w.content.visible_child_name();
        assert!(
            !w.window.is_visible(),
            "only Settings is presented in this fixture"
        );
        show_progress(&w, "Unrelated progress must survive");
        let assert_unchanged = || {
            assert_eq!(w.live_status.label(), "Unrelated progress must survive");
            assert!(w.live_status.get_visible());
            assert_eq!(w.content.visible_child_name(), original_page);
            assert_eq!(gtk::prelude::GtkWindowExt::focus(&settings), original_focus);
            assert!(settings.visible_dialog().is_none());
            assert_eq!(settings.content().as_ref(), Some(content.upcast_ref()));
            assert_eq!(button.label().as_deref(), Some("Check and queue updates"));
        };

        // Automatic owner first: its silent success must not leave phantom main progress.
        check_updates(&w, &model, false);
        let automatic = take(CheckMode::Automatic);
        button.emit_clicked();
        assert!(!button.is_sensitive());
        assert_eq!(status.label(), "Checking and queuing updates…");
        assert!(status.is_visible());
        let duplicate = take(CheckMode::Manual);
        button.emit_clicked();
        assert!(requests.try_recv().is_err());
        let heartbeat = Rc::new(std::cell::Cell::new(false));
        glib::idle_add_local_once({
            let heartbeat = heartbeat.clone();
            move || heartbeat.set(true)
        });
        wait(|| heartbeat.get());
        assert!(!button.is_sensitive());
        duplicate
            .send(Ok(UpdateCheckReport {
                already_running: true,
                ..Default::default()
            }))
            .unwrap();
        wait(|| button.is_sensitive());
        assert_eq!(status.label(), "Another update check is already running.");
        assert!(!model.borrow().local_refresh_pending);
        let previous_notice = w.status.label();
        automatic.send(Ok(UpdateCheckReport::default())).unwrap();
        wait(|| model.borrow().local_refresh_pending);
        assert_eq!(w.status.label(), previous_notice);
        assert_unchanged();

        // Manual owner first: an automatic duplicate neither unlocks it nor changes progress.
        model.borrow_mut().local_refresh_pending = false;
        button.emit_clicked();
        let manual = take(CheckMode::Manual);
        check_updates(&w, &model, false);
        take(CheckMode::Automatic)
            .send(Ok(UpdateCheckReport {
                already_running: true,
                ..Default::default()
            }))
            .unwrap();
        pump(Duration::from_millis(250));
        assert!(!button.is_sensitive());
        assert_eq!(status.label(), "Checking and queuing updates…");
        assert!(!model.borrow().local_refresh_pending);
        assert_unchanged();
        manual
            .send(Ok(UpdateCheckReport {
                galaxy_updates_queued: 2,
                offline_installers_queued: 3,
                extras_queued: 1,
                skipped_running: 4,
                skipped_busy: 5,
                failures: vec![(
                    7,
                    "ExampleDependency failed access_token=REPORT_CANARY".into(),
                )],
                ..Default::default()
            }))
            .unwrap();
        wait(|| button.is_sensitive());
        assert!(status.label().contains("Queued 2 Depot update(s), 3 installer update(s), 1 extras update(s); 9 busy/running; 1 failed"));
        assert!(status.label().contains("ExampleDependency"));
        assert!(!status.label().contains("CANARY"));
        assert_eq!(status.label(), w.status.label());
        assert!(model.borrow().local_refresh_pending);
        assert_unchanged();

        button.emit_clicked();
        take(CheckMode::Manual)
            .send(Err(anyhow::anyhow!(format!(
                "{}\nTerminalCause access_token=ERROR_CANARY https://example.invalid/signed?secret=URL_CANARY",
                "Diagnostic context. ".repeat(200)
            ))))
            .unwrap();
        wait(|| button.is_sensitive());
        assert!(status.label().contains("TerminalCause"));
        assert!(!status.label().contains("CANARY"));
        assert!(status.wraps());
        assert!(status.is_selectable());
        wait(|| status.width() > 0);
        assert!(status.width() <= settings.width());
        assert_unchanged();
        button.emit_clicked();
        drop(take(CheckMode::Manual));
        wait(|| button.is_sensitive());
        assert!(status.label().contains("stopped unexpectedly"));

        model.borrow_mut().network_available = false;
        button.emit_clicked();
        assert!(button.is_sensitive());
        assert!(status.label().contains("Go online"));
        assert!(requests.try_recv().is_err());
        model.borrow_mut().network_available = true;
        let token = model.borrow_mut().account_token.take();
        button.emit_clicked();
        assert!(button.is_sensitive());
        assert_eq!(status.label(), "Sign in to check for updates.");
        assert!(requests.try_recv().is_err());
        model.borrow_mut().account_token = token;
        model.borrow_mut().logout_pending = true;
        button.emit_clicked();
        assert!(button.is_sensitive());
        assert!(status.label().contains("Go online"));
        assert!(requests.try_recv().is_err());
        model.borrow_mut().logout_pending = false;

        for change in 0..4 {
            button.emit_clicked();
            let old = take(CheckMode::Manual);
            match change {
                0 => model.borrow_mut().account_epoch += 1,
                1 => online::invalidate_library_session(),
                2 => auth::invalidate_session(),
                _ => model.borrow_mut().logout_pending = true,
            }
            wait(|| button.is_sensitive());
            assert!(status.label().contains("Account/session changed"));
            assert!(old.send(Ok(UpdateCheckReport::default())).is_err());
            model.borrow_mut().logout_pending = false;
            button.emit_clicked();
            take(CheckMode::Manual)
                .send(Ok(UpdateCheckReport::default()))
                .unwrap();
            wait(|| button.is_sensitive());
            assert!(status.label().contains("Queued 0 Depot update(s)"));
            assert_unchanged();
        }

        // A fresh click during the old request's unlock must see an already-retired receiver.
        button.emit_clicked();
        let old = take(CheckMode::Manual);
        let retired = Rc::new(std::cell::Cell::new(false));
        let retried = Rc::new(std::cell::Cell::new(false));
        let handler = button.connect_sensitive_notify({
            let retired = retired.clone();
            let retried = retried.clone();
            move |button| {
                if button.is_sensitive() && !retried.replace(true) {
                    retired.set(old.send(Ok(UpdateCheckReport::default())).is_err());
                    button.emit_clicked();
                }
            }
        });
        model.borrow_mut().account_epoch += 1;
        wait(|| retried.get());
        assert!(retired.get());
        assert!(!button.is_sensitive());
        assert_eq!(status.label(), "Checking and queuing updates…");
        button.disconnect(handler);
        take(CheckMode::Manual)
            .send(Ok(UpdateCheckReport::default()))
            .unwrap();
        wait(|| button.is_sensitive());
        assert!(status.label().contains("Queued 0 Depot update(s)"));
        assert_unchanged();

        button.emit_clicked();
        let pending = take(CheckMode::Manual);
        let weak_button = button.downgrade();
        let weak_status = status.downgrade();
        settings.set_content(None::<&gtk::Widget>);
        content.remove(&page);
        page.remove(&group);
        settings.destroy();
        drop(button);
        drop(status);
        drop(group);
        drop(page);
        wait(|| weak_button.upgrade().is_none() && weak_status.upgrade().is_none());
        model.borrow_mut().local_refresh_pending = false;
        pending
            .send(Ok(UpdateCheckReport {
                extras_queued: 6,
                ..Default::default()
            }))
            .unwrap();
        wait(|| model.borrow().local_refresh_pending);
        assert!(w.status.label().contains("6 extras update(s)"));
        assert!(weak_button.upgrade().is_none() && weak_status.upgrade().is_none());
        assert_eq!(w.live_status.label(), "Unrelated progress must survive");
        assert!(w.live_status.get_visible());
        assert_eq!(w.content.visible_child_name(), original_page);
        assert!(!settings.is_visible());
        assert!(!crate::identity::database().exists());
        TEST_CHECK_REQUESTS.with(|target| target.borrow_mut().take());
        w.window.destroy();
    }
}
