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
static TRAY_HANDLE: LazyLock<Mutex<Option<ksni::blocking::Handle<LudomereTray>>>> =
    LazyLock::new(|| Mutex::new(None));

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
    if TRAY_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let tray = LudomereTray {
        commands: sender,
        // ksni refreshes the menu via menu_about_to_show on its service thread.
        recent_games: Vec::new(),
    };
    let handle = match tray.spawn() {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(?error, "system tray is unavailable");
            return;
        }
    };
    *TRAY_HANDLE.lock().unwrap() = Some(handle);
    QUIT_REQUESTED.store(false, Ordering::Release);
    TRAY_ACTIVE.store(true, Ordering::Release);

    let widgets = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(command) = receiver.try_recv() {
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
        glib::ControlFlow::Continue
    });
}

fn show_main_window(w: &Widgets) {
    w.window.set_visible(true);
    w.window.present();
}

fn recent_played_games() -> Vec<RecentGame> {
    let Ok(store) = StateStore::open() else {
        return Vec::new();
    };
    let config = Config::load_or_create().unwrap_or_default();
    let installed = crate::installation::reconcile_installed_games(&store, &config.game_libraries)
        .unwrap_or_default();
    let titles = store
        .normalized_games()
        .unwrap_or_default()
        .into_iter()
        .map(|game| (game.product_id, game.title))
        .collect::<HashMap<_, _>>();
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
    games.into_iter().take(5).map(|(_, game)| game).collect()
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
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = StateStore::open()
            .and_then(|store| crate::installation::reconcile_installed_games(&store, &libraries))
            .map(|games| games.into_iter().find(|game| game.product_id == product_id));
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
    TRAY_ACTIVE.store(false, Ordering::Release);
    if let Some(handle) = TRAY_HANDLE.lock().unwrap().take() {
        handle.shutdown().wait();
    }
}
