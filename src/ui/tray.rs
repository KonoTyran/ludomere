use super::*;
use ksni::blocking::TrayMethods;
use std::sync::{LazyLock, Mutex, atomic::AtomicBool, atomic::Ordering};

#[derive(Debug, Clone)]
struct RecentGame {
    product_id: i64,
    title: String,
}

#[derive(Debug, Clone, Copy)]
enum TrayCommand {
    Show,
    Settings,
    Launch(i64),
    Quit,
}

struct LudomereTray {
    commands: mpsc::Sender<TrayCommand>,
    recent_games: Vec<RecentGame>,
}

impl ksni::Tray for LudomereTray {
    fn id(&self) -> String {
        crate::identity::APP_ID.into()
    }

    fn title(&self) -> String {
        crate::identity::APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        crate::identity::APP_ID.into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.commands.send(TrayCommand::Show);
    }

    fn menu_about_to_show(&mut self) {
        self.recent_games = recent_played_games();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};

        let mut menu = Vec::new();

        if !self.recent_games.is_empty() {
            for game in &self.recent_games {
                let product_id = game.product_id;
                menu.push(
                    StandardItem {
                        label: game.title.clone(),
                        icon_name: "media-playback-start-symbolic".into(),
                        activate: Box::new(move |tray: &mut Self| {
                            let _ = tray.commands.send(TrayCommand::Launch(product_id));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            menu.push(MenuItem::Separator);
        }

        menu.push(
            StandardItem {
                label: "Open Ludomere".into(),
                icon_name: "window-new-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(TrayCommand::Show);
                }),
                ..Default::default()
            }
            .into(),
        );
        menu.push(
            StandardItem {
                label: "Settings".into(),
                icon_name: "preferences-system-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(TrayCommand::Settings);
                }),
                ..Default::default()
            }
            .into(),
        );
        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: "Close Ludomere".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        menu
    }
}

static TRAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
enum TrayLifecycle {
    Idle,
    Starting(mpsc::Sender<()>),
    Running(mpsc::Sender<()>),
    Stopped,
}

static TRAY_LIFECYCLE: LazyLock<Mutex<TrayLifecycle>> =
    LazyLock::new(|| Mutex::new(TrayLifecycle::Idle));

thread_local! {
    static PENDING_LAUNCHES: RefCell<HashSet<i64>> = RefCell::new(HashSet::new());
}

struct PendingLaunch(i64);

impl Drop for PendingLaunch {
    fn drop(&mut self) {
        PENDING_LAUNCHES.with(|pending| pending.borrow_mut().remove(&self.0));
    }
}

pub(super) fn start_tray(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let (stop, stopped) = mpsc::channel();
    {
        let mut lifecycle = TRAY_LIFECYCLE.lock().unwrap();
        if !matches!(*lifecycle, TrayLifecycle::Idle) || QUIT_REQUESTED.load(Ordering::Acquire) {
            return;
        }
        *lifecycle = TrayLifecycle::Starting(stop);
    }
    let (sender, receiver) = mpsc::channel();
    let (ready, registration) = mpsc::channel();
    let tray = LudomereTray {
        commands: sender,
        // ksni refreshes the menu via menu_about_to_show on its service thread.
        recent_games: Vec::new(),
    };
    std::thread::spawn(move || run_tray(tray, ready, stopped));

    let mut registration = Some(registration);
    let widgets = Rc::downgrade(w);
    let model = Rc::downgrade(model);
    #[cfg(test)]
    let poll_lifetime = {
        tests::POLLERS.fetch_add(1, Ordering::SeqCst);
        tests::PollLifetime
    };
    glib::timeout_add_local(Duration::from_millis(100), move || {
        #[cfg(test)]
        let _ = &poll_lifetime;
        if matches!(
            *TRAY_LIFECYCLE.lock().unwrap(),
            TrayLifecycle::Idle | TrayLifecycle::Stopped
        ) {
            return glib::ControlFlow::Break;
        }
        let (Some(widgets), Some(model)) = (widgets.upgrade(), model.upgrade()) else {
            stop_tray(false);
            return glib::ControlFlow::Break;
        };
        if let Some(pending) = registration.as_ref() {
            let result = match pending.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err(anyhow::anyhow!("tray registration stopped unexpectedly"))
                }
            };
            registration.take();
            if let Err(error) = result {
                stop_tray(false);
                tracing::warn!(?error, "system tray is unavailable");
                return glib::ControlFlow::Break;
            }
            let mut lifecycle = TRAY_LIFECYCLE.lock().unwrap();
            if !matches!(*lifecycle, TrayLifecycle::Starting(_)) {
                return glib::ControlFlow::Break;
            }
            let TrayLifecycle::Starting(stop) =
                std::mem::replace(&mut *lifecycle, TrayLifecycle::Idle)
            else {
                unreachable!();
            };
            *lifecycle = TrayLifecycle::Running(stop);
            TRAY_ACTIVE.store(true, Ordering::Release);
        }
        loop {
            if !TRAY_ACTIVE.load(Ordering::Acquire) || QUIT_REQUESTED.load(Ordering::Acquire) {
                return glib::ControlFlow::Break;
            }
            let command = match receiver.try_recv() {
                Ok(command) => command,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    stop_tray(false);
                    return glib::ControlFlow::Break;
                }
            };
            match command {
                TrayCommand::Show => show_main_window(&widgets),
                TrayCommand::Settings => show_settings(&widgets, &model),
                TrayCommand::Launch(product_id) => launch_recent_game(&widgets, &model, product_id),
                TrayCommand::Quit => {
                    QUIT_REQUESTED.store(true, Ordering::Release);
                    if let Some(application) = widgets.window.application() {
                        application.quit();
                    }
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

fn run_tray(
    tray: LudomereTray,
    ready: mpsc::Sender<anyhow::Result<()>>,
    stopped: mpsc::Receiver<()>,
) {
    #[cfg(test)]
    let script = tests::DRIVER.lock().unwrap().as_mut().map(|scripts| {
        scripts
            .pop_front()
            .expect("missing inert tray registration")
    });
    #[cfg(test)]
    if let Some(script) = script {
        assert!(tray.recent_games.is_empty());
        script.entered.send(tray.commands).unwrap();
        script.permit.recv_timeout(Duration::from_secs(10)).unwrap();
        match script.outcome {
            tests::Outcome::Ready => {
                finish_tray_registration(ready, stopped, || script.done.send(true).unwrap());
            }
            tests::Outcome::Failed => {
                let _ = ready.send(Err(anyhow::anyhow!("inert registration failure")));
                script.done.send(false).unwrap();
            }
            tests::Outcome::Disconnected => {
                drop(ready);
                script.done.send(false).unwrap();
            }
        }
        return;
    }
    match tray.spawn() {
        Ok(handle) => finish_tray_registration(ready, stopped, || handle.shutdown().wait()),
        Err(error) => {
            let _ = ready.send(Err(error.into()));
        }
    }
}

fn finish_tray_registration(
    ready: mpsc::Sender<anyhow::Result<()>>,
    stopped: mpsc::Receiver<()>,
    shutdown: impl FnOnce(),
) {
    if matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) && ready.send(Ok(())).is_ok() {
        #[cfg(test)]
        tests::PUBLISHED.fetch_add(1, Ordering::SeqCst);
        let _ = stopped.recv();
    }
    shutdown();
}

fn show_main_window(w: &Widgets) {
    w.window.set_visible(true);
    w.window.present();
}

fn recent_played_games() -> Vec<RecentGame> {
    let session = (online::account_session(), auth::session());
    let Ok(_activity) = crate::profile_reset::begin_activity("loading recent tray games") else {
        return Vec::new();
    };
    #[cfg(test)]
    let probe = tests::menu_probe();
    #[cfg(test)]
    if let Some(probe) = &probe {
        tests::wait_barrier(&probe.before);
    }
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    let Ok(store) = StateStore::open() else {
        return Vec::new();
    };
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    let config = Config::load_or_create().unwrap_or_default();
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    let installed = crate::installation::reconcile_installed_games(&store, &config.game_libraries)
        .unwrap_or_default();
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    let titles = store
        .normalized_games()
        .unwrap_or_default()
        .into_iter()
        .map(|game| (game.product_id, game.title))
        .collect::<HashMap<_, _>>();
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    let activity = store.all_product_activity().unwrap_or_default();
    let mut games = installed
        .into_iter()
        .filter_map(|game| {
            let played = activity.get(&game.product_id)?.last_played_at?;
            let title = titles.get(&game.product_id)?.clone();
            Some((
                played,
                RecentGame {
                    product_id: game.product_id,
                    title,
                },
            ))
        })
        .collect::<Vec<_>>();
    games.sort_by_key(|(played, _)| std::cmp::Reverse(*played));
    #[cfg(test)]
    if let Some(probe) = &probe {
        tests::wait_barrier(&probe.after);
    }
    if !tray_session_is_current(session) {
        return Vec::new();
    }
    games.into_iter().take(5).map(|(_, game)| game).collect()
}

fn tray_session_is_current(session: (u64, u64)) -> bool {
    session == (online::account_session(), auth::session())
}

fn launch_recent_game(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>, product_id: i64) {
    show_main_window(w);
    if model.borrow().logout_pending {
        show_status(
            w,
            "Sign-out is in progress. Wait for it to finish before launching a game.",
        );
        return;
    }
    if crate::installation::is_game_running(product_id) {
        show_status(w, "That game is already running.");
        return;
    }
    if !PENDING_LAUNCHES.with(|pending| pending.borrow_mut().insert(product_id)) {
        show_status(w, "That game is already being prepared for launch.");
        return;
    }
    let mut pending = Some(PendingLaunch(product_id));
    let libraries = model.borrow().config.game_libraries.clone();
    let epoch = model.borrow().account_epoch;
    let launch_generation = model.borrow().detail_generation;
    let session = online::account_session();
    let auth_session = auth::session();
    show_status(w, "Preparing game launch — checking installed files…");
    if !tray_session_is_current((session, auth_session))
        || model.borrow().account_epoch != epoch
        || model.borrow().logout_pending
    {
        return;
    }
    let activity = match crate::profile_reset::begin_activity("preparing recent tray game launch") {
        Ok(activity) => activity,
        Err(error) => {
            show_status(
                w,
                &notifications::failure_message(
                    "Could not prepare game launch",
                    &error.to_string(),
                ),
            );
            return;
        }
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _activity = activity;
        #[cfg(test)]
        let probe = tests::preflight_probe();
        #[cfg(test)]
        if let Some(probe) = &probe {
            tests::wait_barrier(&probe.before);
        }
        #[cfg(test)]
        let disconnected = probe.as_ref().is_some_and(|probe| probe.disconnect);
        let result = (|| {
            anyhow::ensure!(
                tray_session_is_current((session, auth_session)),
                "The account changed before inspecting installed files."
            );
            #[cfg(test)]
            if let Some(probe) = probe {
                if probe.disconnect {
                    return Ok(None);
                }
                tests::PREFLIGHT_ENTRIES.fetch_add(1, Ordering::SeqCst);
                return probe.result;
            }
            let store = StateStore::open()?;
            anyhow::ensure!(
                tray_session_is_current((session, auth_session)),
                "The account changed before inspecting installed files."
            );
            crate::installation::reconcile_installed_games(&store, &libraries)
                .map(|games| games.into_iter().find(|game| game.product_id == product_id))
        })();
        #[cfg(test)]
        if disconnected {
            return;
        }
        if !tray_session_is_current((session, auth_session)) {
            return;
        }
        let _ = sender.send(result);
    });
    let widgets = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(40), move || {
        if online::account_session() != session
            || auth::session() != auth_session
            || model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
        {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(Some(game))) => {
                if crate::installation::is_game_running(product_id) {
                    show_status(&widgets, "That game is already running.");
                } else if game.installer_operating_system.as_deref() != Some("linux")
                    && (model.borrow().detail_generation != launch_generation
                        || !widgets.window.is_visible()
                        || !widgets.window.is_active())
                {
                    show_status(
                        &widgets,
                        "Launch preparation finished. Select the game again when you are ready to continue Windows setup.",
                    );
                } else {
                    show_status(&widgets, "Starting game…");
                    if !tray_session_is_current((session, auth_session))
                        || model.borrow().account_epoch != epoch
                        || model.borrow().logout_pending
                    {
                        return glib::ControlFlow::Break;
                    }
                    if game.installer_operating_system.as_deref() != Some("linux")
                        && (model.borrow().detail_generation != launch_generation
                            || !widgets.window.is_visible()
                            || !widgets.window.is_active())
                    {
                        show_status(
                            &widgets,
                            "Launch preparation finished. Select the game again when you are ready to continue Windows setup.",
                        );
                        return glib::ControlFlow::Break;
                    }
                    start_recent_game(
                        &widgets,
                        &model,
                        game,
                        launch_generation,
                        pending.take().unwrap(),
                    );
                }
            }
            Ok(Ok(None)) => show_status(&widgets, "That game is no longer installed."),
            Ok(Err(error)) => show_status(
                &widgets,
                &notifications::failure_message(
                    "Could not inspect installed game files. Try launching again.",
                    &format!("{error:#}"),
                ),
            ),
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => show_status(
                &widgets,
                "Game preparation stopped unexpectedly. Try launching again.",
            ),
        }
        glib::ControlFlow::Break
    });
}

fn start_recent_game(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    game: crate::domain::InstalledGame,
    launch_generation: u64,
    pending: PendingLaunch,
) {
    #[cfg(test)]
    if tests::HANDOFFS.with(|handoffs| {
        if let Some(handoffs) = handoffs.borrow_mut().as_mut() {
            handoffs.push(game.clone());
            true
        } else {
            false
        }
    }) {
        return;
    }
    let product_id = game.product_id;
    let session = online::account_session();
    let auth_session = auth::session();
    let epoch = model.borrow().account_epoch;
    let receiver = launch_with_components(&w.window, game);
    let mut pending = Some(pending);
    let widgets = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if online::account_session() != session
            || auth::session() != auth_session
            || model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
        {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(
                event @ (crate::installation::LaunchEvent::EnablementRequired { .. }
                | crate::installation::LaunchEvent::PreLaunchConflict { .. }
                | crate::installation::LaunchEvent::LaunchWithoutSyncRequired { .. }
                | crate::installation::LaunchEvent::SyncWarning(_)
                | crate::installation::LaunchEvent::PostExitSync(_)
                | crate::installation::LaunchEvent::PostExitConflict(_)),
            ) => {
                present_cloud_launch_event(&widgets.window, event);
                glib::ControlFlow::Continue
            }
            Ok(crate::installation::LaunchEvent::Started) => {
                pending.take();
                show_status(
                    &widgets,
                    "Game started. Launch output is available in the game's Logs tab.",
                );
                let now = chrono::Utc::now().timestamp();
                model
                    .borrow_mut()
                    .product_activity
                    .entry(product_id)
                    .or_default()
                    .last_played_at = Some(now);
                update_sidebar_download_styles(&widgets, &model.borrow());
                glib::ControlFlow::Continue
            }
            Ok(crate::installation::LaunchEvent::CloudSyncStarted(_)) => {
                glib::ControlFlow::Continue
            }
            Ok(crate::installation::LaunchEvent::Exited { .. }) => glib::ControlFlow::Break,
            Ok(crate::installation::LaunchEvent::PrefixRecoveryRequired {
                message,
                game,
                setup_required,
            }) => {
                let title = model
                    .borrow()
                    .games
                    .iter()
                    .find(|entry| entry.product_id == product_id)
                    .map(|game| game.title.clone())
                    .unwrap_or_else(|| format!("Game {product_id}"));
                offer_prefix_recovery(
                    &widgets.window,
                    &model,
                    *game,
                    &title,
                    &message,
                    setup_required,
                    launch_generation,
                );
                glib::ControlFlow::Break
            }
            Ok(crate::installation::LaunchEvent::Failed(error)) => {
                let message = notifications::failure_message("Could not run game", &error);
                show_status(&widgets, &message);
                if model.borrow().detail_generation == launch_generation
                    && widgets.window.is_visible()
                    && widgets.window.is_active()
                {
                    let dialog = adw::AlertDialog::builder()
                        .heading("Could not run game")
                        .body(message)
                        .build();
                    dialog.add_response("close", "Close");
                    dialog.present(Some(&widgets.window));
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if pending.is_some() {
                    show_status(
                        &widgets,
                        "Game launch ended before the game started. Select it again to retry.",
                    );
                }
                glib::ControlFlow::Break
            }
        }
    });
}

pub(super) fn should_hide_on_close() -> bool {
    TRAY_ACTIVE.load(Ordering::Acquire) && !QUIT_REQUESTED.load(Ordering::Acquire)
}

pub(crate) fn shutdown_tray() {
    stop_tray(true);
}

fn stop_tray(shutdown: bool) {
    let previous = {
        let mut lifecycle = TRAY_LIFECYCLE.lock().unwrap();
        if shutdown {
            QUIT_REQUESTED.store(true, Ordering::Release);
        }
        TRAY_ACTIVE.store(false, Ordering::Release);
        let next = if shutdown || matches!(*lifecycle, TrayLifecycle::Stopped) {
            TrayLifecycle::Stopped
        } else {
            TrayLifecycle::Idle
        };
        std::mem::replace(&mut *lifecycle, next)
    };
    if let TrayLifecycle::Starting(stop) | TrayLifecycle::Running(stop) = previous {
        // Dropping the sole sender wakes the owning worker, including after late registration.
        drop(stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::VecDeque, sync::atomic::AtomicUsize};

    pub(super) enum Outcome {
        Ready,
        Failed,
        Disconnected,
    }

    pub(super) struct Script {
        pub(super) outcome: Outcome,
        pub(super) entered: mpsc::Sender<mpsc::Sender<TrayCommand>>,
        pub(super) permit: mpsc::Receiver<()>,
        pub(super) done: mpsc::Sender<bool>,
    }

    pub(super) static DRIVER: Mutex<Option<VecDeque<Script>>> = Mutex::new(None);
    pub(super) static POLLERS: AtomicUsize = AtomicUsize::new(0);
    pub(super) static PUBLISHED: AtomicUsize = AtomicUsize::new(0);

    pub(super) type Barrier = (mpsc::Sender<()>, mpsc::Receiver<()>);
    pub(super) struct MenuProbe {
        pub(super) before: Option<Barrier>,
        pub(super) after: Option<Barrier>,
    }
    pub(super) struct PreflightProbe {
        pub(super) before: Option<Barrier>,
        pub(super) result: anyhow::Result<Option<crate::domain::InstalledGame>>,
        pub(super) disconnect: bool,
    }
    static MENU_PROBES: Mutex<Option<VecDeque<MenuProbe>>> = Mutex::new(None);
    static PREFLIGHT_PROBES: Mutex<Option<VecDeque<PreflightProbe>>> = Mutex::new(None);
    pub(super) static PREFLIGHT_ENTRIES: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        pub(super) static HANDOFFS: RefCell<Option<Vec<crate::domain::InstalledGame>>> = const { RefCell::new(None) };
    }

    pub(super) fn menu_probe() -> Option<MenuProbe> {
        MENU_PROBES
            .lock()
            .unwrap()
            .as_mut()
            .map(|probes| probes.pop_front().expect("missing inert tray menu probe"))
    }

    pub(super) fn preflight_probe() -> Option<PreflightProbe> {
        PREFLIGHT_PROBES
            .lock()
            .unwrap()
            .as_mut()
            .map(|probes| probes.pop_front().expect("missing inert tray preflight"))
    }

    pub(super) fn wait_barrier(barrier: &Option<Barrier>) {
        if let Some((entered, proceed)) = barrier {
            entered.send(()).unwrap();
            proceed.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    }

    fn barrier() -> (Barrier, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, entry) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        ((entered, proceed), entry, release)
    }

    fn private_profile() {
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
                    .starts_with("/tmp/ludomere-p392-"),
                "{key}"
            );
        }
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
    }

    fn installed_game(product_id: i64, windows: bool) -> crate::domain::InstalledGame {
        let slug = format!("tray-{product_id}");
        crate::domain::InstalledGame {
            product_id,
            library_id: "tray-test".into(),
            installed_version: None,
            installation_directory: std::env::temp_dir().join(&slug),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some(if windows { "windows" } else { "linux" }.into()),
            installer_language: None,
            compatibility: windows.then(|| crate::compatibility::GameCompatibilityPreferences {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                prefix_slug: slug,
                profile: crate::compatibility::UmuProfile::fallback(),
                pending_profile: None,
            }),
            primary_executable: None,
            launch_arguments: Vec::new(),
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: Some(1),
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    #[ignore = "requires fresh private HOME/all XDG/TMP; actual menu inspection uses only synthetic files, no bus"]
    fn recent_menu_admits_profile_work_and_keeps_local_ordering() {
        use ksni::Tray;
        use std::os::unix::fs::PermissionsExt;
        private_profile();
        *MENU_PROBES.lock().unwrap() = Some(VecDeque::new());
        let inspect = || {
            std::thread::spawn(|| {
                let (commands, _) = mpsc::channel();
                let mut tray = LudomereTray {
                    commands,
                    recent_games: Vec::new(),
                };
                tray.menu_about_to_show();
                assert_eq!(
                    tray.menu().len(),
                    if tray.recent_games.is_empty() {
                        4
                    } else {
                        tray.recent_games.len() + 5
                    }
                );
                tray.recent_games
            })
        };
        let frozen = crate::profile_reset::reserve().unwrap();
        assert!(inspect().join().unwrap().is_empty());
        assert!(MENU_PROBES.lock().unwrap().as_ref().unwrap().is_empty());
        drop(frozen);
        for invalidate in [
            auth::invalidate_session as fn(),
            online::invalidate_library_session as fn(),
        ] {
            let (before, entered, release) = barrier();
            MENU_PROBES
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .push_back(MenuProbe {
                    before: Some(before),
                    after: None,
                });
            let worker = inspect();
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(crate::profile_reset::reserve().is_err());
            invalidate();
            release.send(()).unwrap();
            assert!(worker.join().unwrap().is_empty());
            drop(crate::profile_reset::reserve().unwrap());
            assert!(!crate::identity::database().exists());
            assert!(!Config::path().exists());
        }
        let root = tempfile::tempdir().unwrap();
        let library = crate::config::GameLibrary {
            id: "tray-test".into(),
            name: "Tray".into(),
            path: root.path().join("games"),
            default: true,
        };
        Config {
            game_libraries: vec![library.clone()],
            offline_libraries: Vec::new(),
            extras_libraries: Vec::new(),
            ..Config::default()
        }
        .save()
        .unwrap();
        let store = StateStore::open().unwrap();
        let mut catalog = Vec::new();
        for product_id in 1..=9 {
            let marker = crate::installation::installation_marker_from_game(
                &installed_game(product_id, product_id == 7),
                Vec::new(),
            );
            catalog.push(Game {
                product_id,
                slug: marker.slug.clone(),
                title: format!("Local {product_id}"),
                ..Game::default()
            });
            if product_id != 9 {
                let directory = library.path.join(&marker.slug);
                std::fs::create_dir_all(&directory).unwrap();
                let executable = directory.join(if product_id == 7 {
                    "game.exe"
                } else {
                    "start.sh"
                });
                std::fs::write(&executable, b"inert payload; never execute").unwrap();
                std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                crate::installation::write_installation_marker(&marker, &directory).unwrap();
            }
            if product_id != 8 {
                store
                    .record_game_session(product_id, product_id * 100, 1)
                    .unwrap();
            }
        }
        store.upsert_normalized_library(&catalog).unwrap();
        drop(store);
        assert!(!library.path.join(".ludomere/compatibility/tray-7").exists());
        MENU_PROBES
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push_back(MenuProbe {
                before: None,
                after: None,
            });
        let games = inspect().join().unwrap();
        assert_eq!(
            games.iter().map(|game| game.product_id).collect::<Vec<_>>(),
            vec![7, 6, 5, 4, 3]
        );
        assert_eq!(games[0].title, "Local 7");
        let (after, entered, release) = barrier();
        MENU_PROBES
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push_back(MenuProbe {
                before: None,
                after: Some(after),
            });
        let worker = inspect();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(crate::profile_reset::reserve().is_err());
        online::invalidate_library_session();
        release.send(()).unwrap();
        assert!(worker.join().unwrap().is_empty());
        drop(crate::profile_reset::reserve().unwrap());
        assert!(MENU_PROBES.lock().unwrap().take().unwrap().is_empty());
    }

    #[test]
    #[ignore = "requires fresh private HOME/all XDG/TMP and active private GTK; preflight and launch handoff are fail-closed inert"]
    fn recent_launch_preflight_is_responsive_and_origin_guarded() {
        private_profile();
        *PREFLIGHT_PROBES.lock().unwrap() = Some(VecDeque::new());
        HANDOFFS.with(|handoffs| *handoffs.borrow_mut() = Some(Vec::new()));
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.TrayProfileTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let config = Config::default();
        let widgets = Rc::new(super::super::window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            ..AppModel::default()
        }));
        widgets.window.present();
        wait_until(|| widgets.window.is_mapped());
        let heartbeat = Rc::new(Cell::new(0));
        let clock = glib::timeout_add_local(Duration::from_millis(10), {
            let heartbeat = heartbeat.clone();
            move || {
                heartbeat.set(heartbeat.get() + 1);
                glib::ControlFlow::Continue
            }
        });
        let game = |windows| installed_game(42, windows);
        let enqueue = |result, disconnect| {
            let (before, entered, release) = barrier();
            PREFLIGHT_PROBES
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .push_back(PreflightProbe {
                    before: Some(before),
                    result,
                    disconnect,
                });
            (entered, release)
        };
        let pending = || PENDING_LAUNCHES.with(|pending| pending.borrow().contains(&42));
        let launches = || HANDOFFS.with(|handoffs| handoffs.borrow().as_ref().unwrap().len());
        let drained = || {
            wait_until(|| !pending());
            wait_until(|| crate::profile_reset::reserve().is_ok());
        };

        let frozen = crate::profile_reset::reserve().unwrap();
        launch_recent_game(&widgets, &model, 42);
        assert!(!pending());
        assert!(
            widgets
                .status
                .text()
                .contains("Could not prepare game launch")
        );
        assert!(
            PREFLIGHT_PROBES
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_empty()
        );
        drop(frozen);
        // Revocation from preparation feedback occurs before activity/spawn.
        let invalidate = widgets
            .status
            .connect_notify_local(Some("label"), |label, _| {
                if label.text().starts_with("Preparing game launch") {
                    auth::invalidate_session();
                }
            });
        launch_recent_game(&widgets, &model, 42);
        widgets.status.disconnect(invalidate);
        assert!(!pending());
        assert!(
            PREFLIGHT_PROBES
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_empty()
        );

        // Admission is already registered while the worker is held; duplicate clicks
        // cannot consume a second script or block GTK.
        let (entered, release) = enqueue(Err(anyhow::anyhow!("inert inspection error")), false);
        launch_recent_game(&widgets, &model, 42);
        assert!(crate::profile_reset::reserve().is_err());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let ticks = heartbeat.get();
        wait_until(|| heartbeat.get() >= ticks + 3);
        launch_recent_game(&widgets, &model, 42);
        assert!(widgets.status.text().contains("already being prepared"));
        release.send(()).unwrap();
        drained();
        assert!(widgets.status.text().contains("inert inspection error"));
        let (entered, release) = enqueue(Ok(None), true);
        launch_recent_game(&widgets, &model, 42);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        drained();
        assert!(widgets.status.text().contains("stopped unexpectedly"));

        for invalidate in [
            auth::invalidate_session as fn(),
            online::invalidate_library_session as fn(),
        ] {
            let count = PREFLIGHT_ENTRIES.load(Ordering::SeqCst);
            let (entered, release) = enqueue(Ok(Some(game(false))), false);
            launch_recent_game(&widgets, &model, 42);
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            invalidate();
            release.send(()).unwrap();
            drained();
            assert_eq!(PREFLIGHT_ENTRIES.load(Ordering::SeqCst), count);
            assert_eq!(launches(), 0);
        }
        for logout in [false, true] {
            let (entered, release) = enqueue(Ok(Some(game(false))), false);
            launch_recent_game(&widgets, &model, 42);
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            if logout {
                model.borrow_mut().logout_pending = true;
            } else {
                model.borrow_mut().account_epoch += 1;
            }
            release.send(()).unwrap();
            drained();
            assert_eq!(launches(), 0);
            model.borrow_mut().logout_pending = false;
        }
        // Starting feedback must not let the next helper recapture a revoked origin.
        for invalidate in [
            auth::invalidate_session as fn(),
            online::invalidate_library_session as fn(),
        ] {
            let hook = widgets
                .status
                .connect_notify_local(Some("label"), move |label, _| {
                    if label.text() == "Starting game…" {
                        invalidate();
                    }
                });
            let (entered, release) = enqueue(Ok(Some(game(false))), false);
            launch_recent_game(&widgets, &model, 42);
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            release.send(()).unwrap();
            drained();
            widgets.status.disconnect(hook);
            assert_eq!(launches(), 0);
        }
        // Native handoff remains allowed after a view/window change; no real
        // launch function can run while the capture slot is armed.
        let hook = widgets.status.connect_notify_local(Some("label"), {
            let model = Rc::downgrade(&model);
            let window = widgets.window.downgrade();
            move |label, _| {
                if label.text() == "Starting game…" {
                    model.upgrade().unwrap().borrow_mut().detail_generation += 1;
                    window.upgrade().unwrap().set_visible(false);
                }
            }
        });
        let (entered, release) = enqueue(Ok(Some(game(false))), false);
        launch_recent_game(&widgets, &model, 42);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        drained();
        widgets.status.disconnect(hook);
        assert_eq!(launches(), 1);
        assert!(!widgets.window.is_visible());

        // Windows keeps its existing foreground policy both before and after
        // Starting feedback, including changes caused by that feedback itself.
        let (entered, release) = enqueue(Ok(Some(game(true))), false);
        launch_recent_game(&widgets, &model, 42);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        model.borrow_mut().detail_generation += 1;
        release.send(()).unwrap();
        drained();
        assert!(widgets.status.text().contains("continue Windows setup"));
        assert_eq!(launches(), 1);
        widgets.window.present();
        wait_until(|| widgets.window.is_active());
        let reached = Rc::new(Cell::new(false));
        let hook = widgets.status.connect_notify_local(Some("label"), {
            let model = Rc::downgrade(&model);
            let reached = reached.clone();
            move |label, _| {
                if label.text() == "Starting game…" {
                    reached.set(true);
                    model.upgrade().unwrap().borrow_mut().detail_generation += 1;
                }
            }
        });
        let (entered, release) = enqueue(Ok(Some(game(true))), false);
        launch_recent_game(&widgets, &model, 42);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        drained();
        widgets.status.disconnect(hook);
        assert!(reached.get());
        assert_eq!(launches(), 1);
        assert!(widgets.status.text().contains("continue Windows setup"));
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
        assert!(PREFLIGHT_PROBES.lock().unwrap().take().unwrap().is_empty());
        HANDOFFS.with(|handoffs| {
            handoffs.borrow_mut().take();
        });
        clock.remove();
        widgets.window.close();
    }

    pub(super) struct PollLifetime;

    impl Drop for PollLifetime {
        fn drop(&mut self) {
            POLLERS.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[track_caller]
    fn wait_until(check: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !check() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(check());
    }

    fn script(
        outcome: Outcome,
    ) -> (
        mpsc::Sender<()>,
        mpsc::Receiver<mpsc::Sender<TrayCommand>>,
        mpsc::Receiver<bool>,
    ) {
        let (entered, registration) = mpsc::channel();
        let (permit, proceed) = mpsc::channel();
        let (done, completion) = mpsc::channel();
        DRIVER.lock().unwrap().as_mut().unwrap().push_back(Script {
            outcome,
            entered,
            permit: proceed,
            done,
        });
        (permit, registration, completion)
    }

    fn reset_after_worker() {
        wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
        assert!(!TRAY_ACTIVE.load(Ordering::Acquire));
        let mut lifecycle = TRAY_LIFECYCLE.lock().unwrap();
        assert!(matches!(
            *lifecycle,
            TrayLifecycle::Idle | TrayLifecycle::Stopped
        ));
        *lifecycle = TrayLifecycle::Idle;
        QUIT_REQUESTED.store(false, Ordering::Release);
    }

    fn finished(done: &mpsc::Receiver<bool>, disposed: bool) {
        assert_eq!(done.recv_timeout(Duration::from_secs(5)).unwrap(), disposed);
        wait_until(|| match done.try_recv() {
            Err(mpsc::TryRecvError::Disconnected) => true,
            Err(mpsc::TryRecvError::Empty) => false,
            Ok(_) => panic!("tray worker completed more than once"),
        });
    }

    #[test]
    #[ignore = "requires private HOME/all XDG/TMP and private GTK; tray registration is fail-closed inert"]
    fn tray_registration_keeps_gtk_responsive_and_disposes_late_handles() {
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
                    .starts_with("/tmp/ludomere-p389-")
            );
        }
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
        *DRIVER.lock().unwrap() = Some(VecDeque::new());
        // A lost result receiver still requires explicit disposal, even before a stop signal.
        let (ready, registration) = mpsc::channel();
        drop(registration);
        let (stop, stopped) = mpsc::channel();
        let (disposed, disposal) = mpsc::channel();
        std::thread::spawn(move || {
            finish_tray_registration(ready, stopped, || disposed.send(()).unwrap());
        });
        disposal.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(stop);
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.TrayStartupTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let config = Config::default();
        let widgets = Rc::new(super::super::window::create_widgets(&app, &config));
        let model = Rc::new(RefCell::new(AppModel {
            config,
            ..AppModel::default()
        }));
        widgets.window.present();
        wait_until(|| widgets.window.is_mapped());
        let content = widgets.content.visible_child_name();
        let heartbeat = Rc::new(Cell::new(0));
        let heartbeat_source = glib::timeout_add_local(Duration::from_millis(10), {
            let heartbeat = heartbeat.clone();
            move || {
                heartbeat.set(heartbeat.get() + 1);
                glib::ControlFlow::Continue
            }
        });

        // Pending and empty channels are normal: duplicate admission must not consume a script.
        let (permit, entered, done) = script(Outcome::Ready);
        start_tray(&widgets, &model);
        let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        start_tray(&widgets, &model);
        assert_eq!(POLLERS.load(Ordering::SeqCst), 1);
        assert!(DRIVER.lock().unwrap().as_ref().unwrap().is_empty());
        wait_until(|| heartbeat.get() >= 15);
        assert!(!should_hide_on_close());
        assert_eq!(POLLERS.load(Ordering::SeqCst), 1);
        assert_eq!(widgets.content.visible_child_name(), content);
        assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        widgets.window.set_visible(false);
        permit.send(()).unwrap();
        wait_until(should_hide_on_close);
        assert!(
            !widgets.window.is_visible(),
            "registration must not present the window"
        );
        assert_eq!(widgets.content.visible_child_name(), content);
        start_tray(&widgets, &model);
        assert_eq!(POLLERS.load(Ordering::SeqCst), 1);
        let before = heartbeat.get();
        wait_until(|| heartbeat.get() >= before + 15);
        assert!(
            should_hide_on_close(),
            "hidden window remains a valid tray owner"
        );
        commands.send(TrayCommand::Show).unwrap();
        wait_until(|| widgets.window.is_mapped());
        assert_eq!(widgets.content.visible_child_name(), content);
        shutdown_tray();
        assert!(!should_hide_on_close());
        assert!(QUIT_REQUESTED.load(Ordering::Acquire));
        finished(&done, true);
        drop(commands);
        start_tray(&widgets, &model);
        assert!(matches!(
            *TRAY_LIFECYCLE.lock().unwrap(),
            TrayLifecycle::Stopped
        ));
        reset_after_worker();

        // Failed registration/disconnected result retire the attempt and permit an explicit retry.
        for outcome in [Outcome::Failed, Outcome::Disconnected] {
            let (permit, entered, done) = script(outcome);
            start_tray(&widgets, &model);
            let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
            permit.send(()).unwrap();
            finished(&done, false);
            wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
            assert!(!should_hide_on_close());
            assert!(!QUIT_REQUESTED.load(Ordering::Acquire));
            assert!(matches!(
                *TRAY_LIFECYCLE.lock().unwrap(),
                TrayLifecycle::Idle
            ));
            drop(commands);
        }

        // Shutdown does not wait for a held registration; a late successful handle is disposed.
        let (permit, entered, done) = script(Outcome::Ready);
        start_tray(&widgets, &model);
        let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        widgets.window.set_visible(false);
        commands.send(TrayCommand::Show).unwrap();
        let published = PUBLISHED.load(Ordering::SeqCst);
        shutdown_tray();
        assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(!should_hide_on_close());
        start_tray(&widgets, &model);
        permit.send(()).unwrap();
        finished(&done, true);
        wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
        assert_eq!(PUBLISHED.load(Ordering::SeqCst), published);
        assert!(!widgets.window.is_visible());
        assert!(QUIT_REQUESTED.load(Ordering::Acquire));
        drop(commands);
        reset_after_worker();

        // Publication can race shutdown before GTK consumes readiness; it still cannot reactivate.
        let (permit, entered, done) = script(Outcome::Ready);
        start_tray(&widgets, &model);
        let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let published = PUBLISHED.load(Ordering::SeqCst);
        permit.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while PUBLISHED.load(Ordering::SeqCst) == published && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(PUBLISHED.load(Ordering::SeqCst), published + 1);
        assert!(!TRAY_ACTIVE.load(Ordering::Acquire));
        shutdown_tray();
        finished(&done, true);
        wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
        assert!(!should_hide_on_close());
        assert!(!widgets.window.is_visible());
        assert!(QUIT_REQUESTED.load(Ordering::Acquire));
        drop(commands);
        reset_after_worker();

        // Losing the service's command sender clears close-to-tray and explicitly stops its owner.
        let (permit, entered, done) = script(Outcome::Ready);
        start_tray(&widgets, &model);
        let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        permit.send(()).unwrap();
        wait_until(should_hide_on_close);
        drop(commands);
        wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
        finished(&done, true);
        assert!(!should_hide_on_close());
        assert!(matches!(
            *TRAY_LIFECYCLE.lock().unwrap(),
            TrayLifecycle::Idle
        ));

        // Release actual Rc owners, not just GtkWindow.destroy with retained Rc<Widgets>.
        let (permit, entered, done) = script(Outcome::Ready);
        start_tray(&widgets, &model);
        let commands = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let weak_widgets = Rc::downgrade(&widgets);
        let weak_model = Rc::downgrade(&model);
        widgets.window.close();
        drop(widgets);
        drop(model);
        assert!(weak_widgets.upgrade().is_none());
        assert!(weak_model.upgrade().is_none());
        wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
        assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        permit.send(()).unwrap();
        finished(&done, true);
        drop(commands);
        assert!(!should_hide_on_close());
        heartbeat_source.remove();
        shutdown_tray();
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
        assert!(DRIVER.lock().unwrap().take().unwrap().is_empty());
    }
}
