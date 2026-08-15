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

pub(super) fn start_tray(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    if TRAY_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let tray = LudomereTray {
        commands: sender,
        recent_games: recent_played_games(),
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
    let libraries = model.borrow().config.game_libraries.clone();
    let installed = StateStore::open().ok().and_then(|store| {
        crate::installation::reconcile_installed_games(&store, &libraries)
            .ok()?
            .into_iter()
            .find(|game| game.product_id == product_id)
    });
    let Some(game) = installed else {
        show_main_window(w);
        show_status(w, "That game is no longer installed");
        return;
    };
    if crate::installation::is_game_running(product_id) {
        return;
    }
    let receiver = crate::installation::launch_game(game);
    let widgets = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(crate::installation::LaunchEvent::Started) => {
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
            Ok(crate::installation::LaunchEvent::Exited { .. }) => glib::ControlFlow::Break,
            Ok(crate::installation::LaunchEvent::Failed(error)) => {
                show_main_window(&widgets);
                let dialog = adw::AlertDialog::builder()
                    .heading("Could not run game")
                    .body(error)
                    .build();
                dialog.add_response("close", "Close");
                dialog.present(Some(&widgets.window));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
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
