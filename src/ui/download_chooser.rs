use super::*;
use crate::config::{GameLibrary, LibraryKind};

pub(super) fn offline_installation_plan(
    product_id: i64,
    library: &GameLibrary,
    slug: &str,
    candidate: &crate::installation::InstallerCandidate,
) -> crate::domain::InstalledGame {
    let now = chrono::Utc::now().timestamp();
    crate::domain::InstalledGame {
        product_id,
        library_id: library.id.clone(),
        installed_version: candidate.version.clone(),
        installation_directory: library.path.join(slug),
        installer_revision_id: candidate.revision_id,
        installer_job_id: None,
        installer_files: candidate.paths.clone(),
        installer_complete: candidate.complete,
        installer_operating_system: candidate.operating_system.clone(),
        installer_language: candidate.language.clone(),
        compatibility: None,
        primary_executable: None,
        launch_arguments: Vec::new(),
        state: crate::domain::InstallationState::Pending,
        error: None,
        installed_at: None,
        verified_at: None,
        last_played_at: None,
        playtime_seconds: 0,
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn retain_offline_launch_preferences(
    plan: &mut crate::domain::InstalledGame,
    preferences: Option<&crate::domain::GamePreferences>,
    previous: Option<&crate::domain::InstalledGame>,
) {
    plan.launch_arguments = preferences.map_or_else(
        || previous.map_or_else(Vec::new, |game| game.launch_arguments.clone()),
        |preferences| preferences.launch_arguments.clone(),
    );
    plan.compatibility = if plan
        .installer_operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        preferences.map_or_else(
            || previous.and_then(|game| game.compatibility.clone()),
            |preferences| preferences.compatibility.clone(),
        )
    } else {
        None
    };
}

fn prepare_cached_offline_installation<T: Send + 'static>(
    mut plan: crate::domain::InstalledGame,
    fresh: bool,
    session: (u64, u64),
    enqueue: impl FnOnce(crate::domain::InstalledGame) -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<mpsc::Receiver<anyhow::Result<T>>> {
    anyhow::ensure!(
        session == (online::account_session(), auth::session()),
        "Account changed before setup"
    );
    let activity = crate::profile_reset::begin_activity("preparing offline installation")?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let _activity = activity;
            anyhow::ensure!(
                session == (online::account_session(), auth::session()),
                "Account changed before setup"
            );
            let store = StateStore::open()?;
            if fresh {
                let preferences = store.game_preferences(plan.product_id)?;
                retain_offline_launch_preferences(&mut plan, preferences.as_ref(), None);
            }
            online::with_account_session(session.0, || {
                anyhow::ensure!(auth::session() == session.1, "Account changed before setup");
                crate::installation::save_game_preferences(&store, &plan)
            })?;
            anyhow::ensure!(
                session == (online::account_session(), auth::session()),
                "Account changed before setup"
            );
            enqueue(plan)
        })();
        let _ = sender.send(result);
    });
    Ok(receiver)
}

#[cfg(test)]
type CapturedDownloads = (
    Vec<download::DownloadRequest>,
    Option<download::AutoInstallRequest>,
);
#[cfg(test)]
pub(super) static TEST_DOWNLOAD_QUEUE: std::sync::Mutex<Option<Vec<CapturedDownloads>>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
pub(super) struct DownloadQueueCapture;
#[cfg(test)]
impl DownloadQueueCapture {
    pub(super) fn start() -> Self {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p296-")
        );
        let mut capture = TEST_DOWNLOAD_QUEUE.lock().unwrap();
        assert!(capture.is_none());
        *capture = Some(Vec::new());
        Self
    }
}
#[cfg(test)]
impl Drop for DownloadQueueCapture {
    fn drop(&mut self) {
        *TEST_DOWNLOAD_QUEUE.lock().unwrap() = None;
    }
}
#[cfg(test)]
pub(super) fn capture_queued_downloads(
    requests: Vec<download::DownloadRequest>,
    install: Option<download::AutoInstallRequest>,
) -> Result<usize, Box<CapturedDownloads>> {
    let mut capture = TEST_DOWNLOAD_QUEUE.lock().unwrap();
    if let Some(capture) = capture.as_mut() {
        let count = requests.len();
        for request in &requests {
            let _ = request.events.send(download::DownloadEvent::Cancelled);
        }
        capture.push((requests, install));
        Ok(count)
    } else {
        Err(Box::new((requests, install)))
    }
}

pub(super) fn choose_download_libraries(
    window: &adw::ApplicationWindow,
    kinds: Vec<LibraryKind>,
    chosen: impl FnOnce(Vec<(LibraryKind, GameLibrary)>) + 'static,
) {
    let session = online::account_session();
    let auth_session = auth::session();
    let dialog = adw::Dialog::builder()
        .title("Choose download libraries")
        .content_width(620)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.append(&adw::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_bottom(20);
    let status = gtk::Label::new(Some("Checking configured libraries…"));
    status.set_wrap(true);
    status.set_xalign(0.0);
    body.append(&status);
    let selectors = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&selectors);
    let settings = gtk::Button::with_label("Storage settings");
    settings.connect_clicked({
        let window = window.clone();
        move |_| {
            let _ = gtk::prelude::WidgetExt::activate_action(
                &window,
                "win.settings-page",
                Some(&"storage".to_variant()),
            );
        }
    });
    let confirm = gtk::Button::with_label("Download");
    confirm.add_css_class("suggested-action");
    confirm.set_sensitive(false);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&settings);
    actions.append(&confirm);
    body.append(&actions);
    root.append(&body);
    dialog.set_child(Some(&root));
    let active = Rc::new(std::cell::Cell::new(true));
    dialog.connect_closed({
        let active = active.clone();
        move |_| active.set(false)
    });
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let _activity = crate::profile_reset::begin_activity("loading download libraries")?;
            anyhow::ensure!(
                session == online::account_session() && auth_session == auth::session(),
                "The account changed. Reopen download choices."
            );
            let config = crate::storage::read_config()?;
            let statuses = crate::storage::inspect_libraries(&config)?;
            Ok((config, statuses))
        })();
        let _ = sender.send(result);
    });
    let pending_choice = Rc::new(RefCell::new(Some(chosen)));
    let dialog_for_result = dialog.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if !active.get() || online::account_session() != session || auth::session() != auth_session
        {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok((config, statuses))) => {
                let mut choices = Vec::new();
                let mut missing = Vec::new();
                for kind in [
                    LibraryKind::GameFiles,
                    LibraryKind::OfflineInstallers,
                    LibraryKind::Extras,
                ] {
                    if !kinds.contains(&kind) {
                        continue;
                    }
                    let libraries = config.libraries(kind).to_vec();
                    let labels = libraries
                        .iter()
                        .map(|library| {
                            let reason = statuses
                                .iter()
                                .find(|status| {
                                    status.kind == kind && status.library_id == library.id
                                })
                                .map(|status| match &status.compatibility {
                                    crate::storage::LibraryCompatibility::Compatible => {
                                        String::new()
                                    }
                                    crate::storage::LibraryCompatibility::Incompatible(reason) => {
                                        format!(" — Incompatible: {reason}")
                                    }
                                    crate::storage::LibraryCompatibility::Unavailable(reason) => {
                                        format!(" — Unavailable: {reason}")
                                    }
                                })
                                .unwrap_or_else(|| " — Unavailable".into());
                            format!("{} — {}{reason}", library.name, library.path.display())
                        })
                        .collect::<Vec<_>>();
                    let label = gtk::Label::new(Some(kind.label()));
                    label.set_xalign(0.0);
                    selectors.append(&label);
                    if libraries.is_empty() {
                        missing.push(kind.label());
                        selectors.append(&gtk::Label::new(Some("No library configured. Add one in Storage settings, then reopen this chooser.")));
                        continue;
                    }
                    let short_labels = libraries
                        .iter()
                        .map(|library| library.name.chars().take(64).collect::<String>())
                        .collect::<Vec<_>>();
                    let selector = gtk::DropDown::from_strings(
                        &short_labels.iter().map(String::as_str).collect::<Vec<_>>(),
                    );
                    selector.set_selected(
                        libraries
                            .iter()
                            .position(|library| library.default)
                            .unwrap_or(0) as u32,
                    );
                    selectors.append(&selector);
                    let selected_path = gtk::Label::new(
                        labels.get(selector.selected() as usize).map(String::as_str),
                    );
                    selected_path.set_wrap(true);
                    selected_path.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                    selected_path.set_max_width_chars(65);
                    selected_path.set_xalign(0.0);
                    selected_path.set_selectable(true);
                    selectors.append(&selected_path);
                    selector.connect_selected_notify(move |selector| {
                        selected_path.set_label(
                            labels
                                .get(selector.selected() as usize)
                                .map(String::as_str)
                                .unwrap_or("No library selected"),
                        );
                    });
                    choices.push((kind, libraries, selector));
                }
                status.set_label(if missing.is_empty() {
                    "Each file type uses its own library. Existing downloads keep their recorded destination; selecting another library creates a separate download."
                } else {
                    "Configure the missing library types in Storage settings before downloading."
                });
                let choices = Rc::new(choices);
                let can_confirm: Rc<dyn Fn()> = Rc::new({
                    let choices = choices.clone();
                    let confirm = confirm.clone();
                    move || {
                        confirm.set_sensitive(missing.is_empty() && !choices.is_empty() && choices.iter().all(|(kind,libraries,selector)| {
                        libraries.get(selector.selected() as usize).is_some_and(|library| statuses.iter().any(|status| status.kind == *kind && status.library_id == library.id && matches!(status.compatibility, crate::storage::LibraryCompatibility::Compatible)))
                    }))
                    }
                });
                for (_, _, selector) in choices.iter() {
                    let can_confirm = can_confirm.clone();
                    selector.connect_selected_notify(move |_| can_confirm());
                }
                can_confirm();
                confirm.connect_clicked({
                    let dialog = dialog_for_result.clone();
                    let status = status.clone();
                    let active = active.clone();
                    let pending_choice = pending_choice.clone();
                    move |button| {
                        if online::account_session() != session || auth::session() != auth_session {
                            return;
                        }
                        let selected = choices
                            .iter()
                            .filter_map(|(kind, libraries, selector)| {
                                libraries
                                    .get(selector.selected() as usize)
                                    .map(|library| (*kind, library.clone()))
                            })
                            .collect::<Vec<_>>();
                        if selected.len() != choices.len() {
                            return;
                        }
                        button.set_sensitive(false);
                        status.set_label("Validating selected libraries…");
                        let (sender, receiver) = mpsc::channel();
                        std::thread::spawn(move || {
                            let result = (|| -> anyhow::Result<_> {
                                let _activity = crate::profile_reset::begin_activity(
                                    "validating download libraries",
                                )?;
                                anyhow::ensure!(
                                    session == online::account_session()
                                        && auth_session == auth::session(),
                                    "The account changed. Reopen download choices."
                                );
                                let config = crate::storage::read_config()?;
                                selected
                                    .into_iter()
                                    .map(|(kind, selected)| {
                                        let library = crate::storage::validate_library(
                                            &config,
                                            kind,
                                            &selected.id,
                                        )?;
                                        anyhow::ensure!(
                                            library.path == selected.path,
                                            "The selected library path changed. Reopen the chooser."
                                        );
                                        Ok((kind, library))
                                    })
                                    .collect::<anyhow::Result<Vec<_>>>()
                            })();
                            let _ = sender.send(result);
                        });
                        let button = button.clone();
                        let status = status.clone();
                        let active = active.clone();
                        let dialog = dialog.clone();
                        let pending_choice = pending_choice.clone();
                        glib::timeout_add_local(Duration::from_millis(50), move || {
                            if !active.get()
                                || online::account_session() != session
                                || auth::session() != auth_session
                            {
                                return glib::ControlFlow::Break;
                            }
                            match receiver.try_recv() {
                                Ok(Ok(selected)) => {
                                    let chosen = pending_choice.borrow_mut().take();
                                    dialog.close();
                                    if let Some(chosen) = chosen {
                                        chosen(selected);
                                    }
                                    glib::ControlFlow::Break
                                }
                                Ok(Err(error)) => {
                                    status.set_label(&format!("Cannot use this library: {error}"));
                                    button.set_sensitive(true);
                                    glib::ControlFlow::Break
                                }
                                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                                Err(_) => {
                                    status.set_label("Library validation stopped. Try again.");
                                    button.set_sensitive(true);
                                    glib::ControlFlow::Break
                                }
                            }
                        });
                    }
                });
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                status.set_label(&format!("Could not inspect libraries: {error}"));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                status.set_label("Library inspection stopped. Reopen this chooser.");
                glib::ControlFlow::Break
            }
        }
    });
    dialog.present(Some(window));
}

pub(super) type ManagedArtifactIdentity = (i64, String, Option<String>);

pub(super) fn current_depot_session(
    model: &Rc<RefCell<AppModel>>,
) -> anyhow::Result<crate::gog::depot_service::DepotSession> {
    let state = model.borrow();
    anyhow::ensure!(
        !state.logout_pending,
        "Sign-out is in progress. Sign in again to continue."
    );
    crate::gog::depot_service::DepotSession::new(
        state
            .account_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Sign in to GOG before starting a Depot operation."))?,
        (online::account_session(), auth::session()),
    )
}

pub(super) fn review_depot_resume(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    operation_id: String,
    token: String,
) {
    let epoch = model.borrow().account_epoch;
    let session = crate::online::account_session();
    let authentication = current_depot_session(model);
    let dialog = adw::Dialog::builder()
        .title("Preparing Resume")
        .content_width(560)
        .content_height(320)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&adw::HeaderBar::new());
    let status = gtk::Label::builder()
        .label("Checking the saved build and its complete prerequisite plan…")
        .wrap(true)
        .selectable(true)
        .build();
    body.append(
        &gtk::ScrolledWindow::builder()
            .child(&status)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    dialog.set_child(Some(&body));
    let closed = Rc::new(std::cell::Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    dialog.present(Some(window));
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let authentication = authentication?;
            authentication.validate()?;
            anyhow::ensure!(
                token == authentication.token.access_token
                    && session == crate::online::account_session(),
                "Account changed before Resume preparation"
            );
            let request = crate::installation::prepare_depot_resume(operation_id, token)?;
            authentication.validate()?;
            Ok((request, authentication))
        })();
        let _ = sender.send(result);
    });
    let window = window.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if model.borrow().account_epoch != epoch || closed.get() {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok((request, authentication))) => { confirm_depot_plan(&window,&model,request,authentication,&dialog); }
            Ok(Err(error)) => status.set_label(&notifications::failure_message("Could not prepare Resume. Close and retry, or use Offline installers and extras from Manage game.",&format!("{error:#}"))),
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => status.set_label("Preparation stopped. Close and retry Resume."),
        }
        glib::ControlFlow::Break
    });
}

fn confirm_depot_plan(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    request: crate::installation::DepotOperationRequest,
    authentication: crate::gog::depot_service::DepotSession,
    dialog: &adw::Dialog,
) {
    if model.borrow().logout_pending
        || request.account_session != crate::online::account_session()
        || !crate::installation::recovery::current(request.product_id, request.recovery_generation)
    {
        return;
    }
    let epoch = model.borrow().account_epoch;
    let session = crate::online::account_session();
    let product_id = request.product_id;
    let needs_review = request
        .dependency_plan
        .as_ref()
        .is_some_and(|plan| !plan.entries.is_empty());
    let description = request
        .dependency_plan
        .as_ref()
        .map(crate::installation::dependency_setup::describe)
        .unwrap_or_else(|| "This native installation requires no Windows setup.".into());
    let dialog = dialog.clone();
    dialog.set_title("Required game components");
    dialog.set_content_width(600);
    dialog.set_content_height(440);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_widget_name("component-consent-header");
    root.append(&header);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_vexpand(true);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    let introduction = gtk::Label::new(Some(
        "Install the components this game needs to run. Missing files will be downloaded and setup will run in this game's Windows environment.",
    ));
    introduction.set_widget_name("component-consent-introduction");
    introduction.set_xalign(0.0);
    introduction.set_wrap(true);
    introduction.set_visible(needs_review);
    body.append(&introduction);
    let label = gtk::Label::builder()
        .name("component-consent-description")
        .label(&description)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .yalign(0.0)
        .valign(gtk::Align::Start)
        .build();
    body.append(
        &gtk::ScrolledWindow::builder()
            .name("component-consent-description-scroll")
            .child(&label)
            .vexpand(true)
            .min_content_height(32)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    let status = gtk::Label::builder()
        .name("component-consent-status")
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();
    body.append(
        &gtk::ScrolledWindow::builder()
            .name("component-consent-status-scroll")
            .child(&status)
            .max_content_height(140)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    let admission_progress = gtk::ProgressBar::builder()
        .name("component-consent-admission-progress")
        .visible(false)
        .build();
    body.append(&admission_progress);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_widget_name("component-consent-cancel");
    let offline = gtk::Button::with_label("Offline installers…");
    offline.set_widget_name("component-consent-offline");
    let confirm = gtk::Button::with_label("Install required components");
    confirm.set_widget_name("component-consent-confirm");
    confirm.add_css_class("suggested-action");
    if !needs_review {
        confirm.set_label("Retry");
    }
    for button in [&cancel, &offline, &confirm] {
        actions.append(button);
    }
    body.append(&actions);
    actions.set_visible(needs_review);
    root.append(&body);
    dialog.set_focus(None::<&gtk::Widget>);
    dialog.set_child(Some(&root));
    if needs_review {
        dialog.set_focus(Some(&cancel));
    }
    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });
    offline.connect_clicked({
        let dialog = dialog.clone();
        let window = window.clone();
        let model = model.clone();
        move |_| {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                return;
            }
            dialog.close();
            let _ = gtk::prelude::WidgetExt::activate_action(
                &window,
                "win.offline-download",
                Some(&product_id.to_variant()),
            );
        }
    });
    let model = model.clone();
    let pending = Rc::new(std::cell::Cell::new(false));
    confirm.connect_clicked(move |button| {
        if model.borrow().account_epoch != epoch || model.borrow().logout_pending || pending.replace(true) {
            return;
        }
        button.set_sensitive(false);
        offline.set_sensitive(false);
        cancel.set_sensitive(false);
        dialog.set_can_close(false);
        status.set_label("Saving operation…");
        admission_progress.set_visible(true);
        admission_progress.pulse();
        let request = request.clone();
        let authentication = authentication.clone();
        let operation_id = request.operation_id.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                authentication.validate()?;
                anyhow::ensure!(crate::online::account_session()==session,"Account changed before queuing setup");
                anyhow::ensure!(crate::installation::enqueue_depot_operation(request),
                    "Operation conflicts with active work or could not be saved. Reopen installation choices to retry.");
                Ok(())
            })();
            let _ = sender.send(result);
        });
        let model = model.clone();
        let dialog = dialog.clone();
        let status = status.clone();
        let button = button.clone();
        let offline = offline.clone();
        let cancel = cancel.clone();
        let pending = pending.clone();
        let actions = actions.clone();
        let admission_progress = admission_progress.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if model.borrow().account_epoch != epoch {
                dialog.set_can_close(true);
                dialog.close();
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Err(error)) => {
                    status.set_label(&super::notifications::failure_message("Could not start Depot setup", &format!("{error:#}")));
                }
                Ok(Ok(())) => {
                    dialog.set_can_close(true);
                    monitor_setup(&dialog, &model, SetupOperation::Depot(operation_id.clone()));
                    return glib::ControlFlow::Break;
                }
                Err(mpsc::TryRecvError::Disconnected) => status.set_label("Preparation worker stopped; retry or choose offline installers."),
                Err(mpsc::TryRecvError::Empty) => { admission_progress.pulse(); return glib::ControlFlow::Continue; }
            }
            admission_progress.set_visible(false);
            pending.set(false);
            actions.set_visible(true);
            button.set_sensitive(true);
            offline.set_sensitive(true);
            cancel.set_sensitive(true);
            dialog.set_can_close(true);
            glib::ControlFlow::Break
        });
    });
    if !needs_review {
        confirm.emit_clicked();
    }
}

enum SetupOperation {
    Depot(String),
    Offline(crate::installation::TrackedInstallation),
    #[cfg(test)]
    Fixture(Result<(), String>, String),
}

fn depot_setup_details(snapshot: &crate::installation::DepotOperationSnapshot) -> String {
    let mut details = format!("Stage: {}", snapshot.state.replace('_', " "));
    let component_phase = snapshot.state == "dependencies";
    let phase_progress = snapshot.total_bytes > 0
        && matches!(
            snapshot.state.as_str(),
            "verifying" | "verifying_existing" | "dependencies" | "extracting"
        );
    if phase_progress {
        details.push_str(&format!(
            "\n{}: {} / {}",
            if component_phase {
                "Component data processed"
            } else {
                "Processed"
            },
            human_size(snapshot.bytes_completed),
            human_size(snapshot.total_bytes)
        ));
    }
    if (snapshot.bytes_downloaded > 0 || snapshot.download_total_bytes.is_some())
        && !(component_phase
            && phase_progress
            && snapshot.bytes_downloaded == snapshot.bytes_completed
            && snapshot.download_total_bytes.is_none())
    {
        if !component_phase
            && snapshot.bytes_downloaded == 0
            && snapshot.download_total_bytes == Some(0)
        {
            details.push_str("\nNo Depot file download required.");
        } else {
            details.push_str(&format!(
                "\n{}: {}{}",
                if component_phase {
                    "Component data processed"
                } else if snapshot.download_total_bytes.is_none() {
                    "Data processed"
                } else {
                    "Depot files downloaded"
                },
                human_size(snapshot.bytes_downloaded),
                snapshot
                    .download_total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| format!(" / {}", human_size(total)))
                    .unwrap_or_default()
            ));
        }
    }
    if snapshot.bytes_written > 0 || snapshot.total_write_bytes > 0 {
        details.push_str(&format!(
            "\nPayload data written this run: {}",
            human_size(snapshot.bytes_written)
        ));
        if snapshot.total_write_bytes > 0 {
            details.push_str(&format!(
                "\nFull write estimate: {}\nExisting files can be reused without rewriting them.",
                human_size(snapshot.total_write_bytes)
            ));
        }
    }
    details
}

fn setup_failure_summary(error: &str) -> String {
    let safe = notifications::failure_message("", error);
    let cause = safe
        .split_once(". Log:")
        .map_or(safe.as_str(), |(cause, _)| cause);
    let summary = cause.split_whitespace().collect::<Vec<_>>().join(" ");
    if summary.is_empty() {
        "Setup could not finish. Review Details before retrying.".into()
    } else if summary.chars().count() > 320 {
        format!("{}…", summary.chars().take(320).collect::<String>())
    } else {
        summary
    }
}

fn monitor_setup(dialog: &adw::Dialog, model: &Rc<RefCell<AppModel>>, operation: SetupOperation) {
    let operation_name = match &operation {
        SetupOperation::Depot(id) => format!("Depot operation: {id}"),
        SetupOperation::Offline(_) => "Offline installer setup (current attempt)".to_owned(),
        #[cfg(test)]
        SetupOperation::Fixture(..) => "Synthetic setup attempt".to_owned(),
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.append(&adw::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_bottom(20);
    let status = gtk::Label::builder()
        .name("setup-status")
        .label("Waiting to start setup…")
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .width_chars(1)
        .lines(3)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .xalign(0.0)
        .build();
    let failure = gtk::Label::builder()
        .name("setup-failure-summary")
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .width_chars(1)
        .lines(3)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .selectable(true)
        .xalign(0.0)
        .visible(false)
        .build();
    let progress = gtk::ProgressBar::new();
    let components = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();
    let details = gtk::Label::builder()
        .name("setup-details")
        .label(format!(
            "{operation_name}\nWaiting for this operation to start."
        ))
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();
    let expander = gtk::Expander::builder()
        .label("Details")
        .child(&details)
        .build();
    expander.set_margin_start(20);
    expander.set_margin_end(20);
    let explanation = gtk::Label::builder().label("Closing this dialog lets setup continue in the background. The game will not start automatically.").wrap(true).xalign(0.0).build();
    for widget in [
        status.upcast_ref::<gtk::Widget>(),
        failure.upcast_ref(),
        progress.upcast_ref(),
        components.upcast_ref(),
        explanation.upcast_ref(),
    ] {
        body.append(widget);
    }
    root.append(&body);
    root.append(
        &gtk::ScrolledWindow::builder()
            .name("setup-details-scroll")
            .child(&expander)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_start(20);
    actions.set_margin_end(20);
    actions.set_margin_bottom(20);
    let stop = gtk::Button::with_label("Stop setup");
    let close = gtk::Button::with_label("Run in background");
    close.set_widget_name("setup-close");
    actions.append(&stop);
    actions.append(&close);
    root.append(&actions);
    dialog.set_title("Game setup progress");
    dialog.set_content_height(400);
    dialog.set_child(Some(&root));
    let closed = Rc::new(std::cell::Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    close.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    let stop_setup: Box<dyn Fn()> = match &operation {
        SetupOperation::Depot(id) => {
            let id = id.clone();
            Box::new(move || {
                let id = id.clone();
                std::thread::spawn(move || {
                    crate::installation::cancel_depot_operation(&id);
                });
            })
        }
        SetupOperation::Offline(tracked) => {
            let control = tracked.control();
            Box::new(move || {
                let control = control.clone();
                std::thread::spawn(move || {
                    control.cancel();
                });
            })
        }
        #[cfg(test)]
        SetupOperation::Fixture(..) => Box::new(|| {}),
    };
    let stopping = Rc::new(std::cell::Cell::new(false));
    stop.connect_clicked({
        let status = status.downgrade();
        let stopping = stopping.clone();
        move |button| {
            stopping.set(true);
            button.set_sensitive(false);
            if let Some(status) = status.upgrade() {
                status.set_label("Stopping setup…");
            }
            stop_setup();
        }
    });
    let epoch = model.borrow().account_epoch;
    let session = online::account_session();
    let model = model.clone();
    let dialog = dialog.downgrade();
    let mut fraction = None;
    let mut finished = false;
    let mut stage_details = "Waiting for this operation to start.".to_owned();
    let mut recent_stages = std::collections::VecDeque::<String>::new();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if closed.get() {
            return glib::ControlFlow::Break;
        }
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || online::account_session() != session
        {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            return glib::ControlFlow::Break;
        }
        if finished {
            return glib::ControlFlow::Continue;
        }
        let mut terminal = None;
        let mut terminal_stopped = false;
        match &operation {
            #[cfg(test)]
            SetupOperation::Fixture(result, details) => {
                stage_details = details.clone();
                terminal = Some(result.clone());
            }
            SetupOperation::Offline(tracked) => {
                loop {
                    match tracked.events.try_recv() {
                        Ok(crate::installation::InstallationEvent::Starting { message }) => {
                            status.set_label(&message);
                            stage_details = message;
                            fraction = None;
                        }
                        Ok(crate::installation::InstallationEvent::Running {
                            message,
                            percentage,
                            ..
                        }) => {
                            status.set_label(&message);
                            stage_details = if let Some(value) = percentage {
                                format!("{message}\nReported progress: {value}%")
                            } else {
                                format!("{message}\nThe installer has not reported a percentage.")
                            };
                            fraction = percentage.map(|value| f64::from(value) / 100.0);
                        }
                        Ok(crate::installation::InstallationEvent::Prompt { text, .. }) => {
                            status.set_label(&text);
                            stage_details = format!("Waiting for an installer response:\n{text}");
                            fraction = None;
                        }
                        Ok(crate::installation::InstallationEvent::Complete { .. }) => {
                            terminal = Some(Ok(()));
                            break;
                        }
                        Ok(crate::installation::InstallationEvent::Failed(error)) => {
                            terminal = Some(Err(error));
                            break;
                        }
                        Ok(crate::installation::InstallationEvent::Cancelled) => {
                            terminal_stopped = true;
                            terminal=Some(Err("Setup was stopped. Any retained backup remains safe; setup may still be required.".into()));
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            terminal=Some(Err("Setup progress stopped unexpectedly. Review the game before retrying.".into()));
                            break;
                        }
                    }
                }
            }
            SetupOperation::Depot(id) => {
                if let Some(snapshot) = crate::installation::depot_operation_snapshot(id) {
                    stage_details = depot_setup_details(&snapshot);
                    fraction = None;
                    components.set_visible(false);
                    match snapshot.state.as_str() {
                        "complete" => terminal = Some(Ok(())),
                        "failed" => {
                            terminal = Some(Err(snapshot.error.unwrap_or_else(|| {
                                "Setup could not finish. Review the game before retrying.".into()
                            })))
                        }
                        "cancelled" | "abandoned" | "interrupted" | "paused" => {
                            terminal_stopped = true;
                            terminal=Some(Err(snapshot.error.unwrap_or_else(|| "Setup was stopped. Any retained backup remains safe; setup may still be required.".into())));
                        }
                        "setup" => {
                            if let Some(setup) = snapshot.setup {
                                stage_details
                                    .push_str(&format!("\nCurrent step: {}", setup.component));
                                status.set_label(&if setup.total == 0 {
                                    setup.component.clone()
                                } else {
                                    format!("Installing: {}", setup.component)
                                });
                                if setup.total > 0 {
                                    stage_details.push_str(&format!(
                                        "\nComponents completed: {} of {}",
                                        setup.completed, setup.total
                                    ));
                                    components.set_visible(true);
                                    components
                                        .set_fraction(setup.completed as f64 / setup.total as f64);
                                    components.set_text(Some(&format!(
                                        "{} of {} components completed",
                                        setup.completed, setup.total
                                    )));
                                }
                            } else {
                                status.set_label("Applying game setup…");
                            }
                        }
                        phase => {
                            status.set_label(match phase {
                                "queued" => "Waiting to start setup…",
                                "preparing" => "Reading game download information…",
                                "calculating" => "Calculating required downloads…",
                                "dependencies" => "Preparing required components…",
                                "downloading" | "materializing" => "Downloading game files…",
                                "extracting" => "Extracting game files…",
                                "verifying" | "verifying_existing" => "Checking installed files…",
                                "committing" => "Saving game files…",
                                "finalizing" => "Finishing game installation…",
                                _ => "Preparing game setup…",
                            });
                            if matches!(
                                phase,
                                "verifying" | "verifying_existing" | "dependencies" | "extracting"
                            ) && snapshot.total_bytes > 0
                            {
                                fraction = Some(
                                    snapshot.bytes_completed as f64 / snapshot.total_bytes as f64,
                                );
                            } else if matches!(phase, "downloading" | "materializing")
                                && let Some(total) =
                                    snapshot.download_total_bytes.filter(|total| *total > 0)
                            {
                                fraction = Some(snapshot.bytes_downloaded as f64 / total as f64);
                            }
                        }
                    }
                }
            }
        }
        if stopping.get() && terminal.is_none() {
            status.set_label("Stopping setup…");
        }
        let stage = status.label().chars().take(240).collect::<String>();
        if recent_stages.back() != Some(&stage) {
            recent_stages.push_back(stage);
            if recent_stages.len() > 8 {
                recent_stages.pop_front();
            }
        }
        let current_details = format!(
            "{operation_name}\n{stage_details}\n\nRecent stages:\n{}",
            recent_stages.iter().cloned().collect::<Vec<_>>().join("\n")
        );
        details.set_label(notifications::failure_message("", &current_details).trim_start());
        if let Some(result) = terminal {
            stop.set_visible(false);
            close.set_label("Close");
            components.set_visible(false);
            progress.set_visible(false);
            explanation.set_label("The game has not been launched.");
            match result {
                Ok(()) => {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.set_title("Setup complete");
                    }
                    status.set_label("Game setup completed.");
                    details.set_label(notifications::failure_message("", &format!("Result: Game setup completed. The game was not launched.\n\n{current_details}")).trim_start());
                }
                Err(error) => {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.set_title(if terminal_stopped {
                            "Setup stopped"
                        } else {
                            "Setup failed"
                        });
                    }
                    status.set_label(if terminal_stopped {
                        "Game setup was stopped."
                    } else {
                        "Game setup did not complete."
                    });
                    failure.set_label(&setup_failure_summary(&error));
                    failure.set_visible(true);
                    details.set_label(
                        notifications::failure_message(
                            "",
                            &format!("Result: {error}\n\n{current_details}"),
                        )
                        .trim_start(),
                    );
                    expander.set_expanded(true);
                }
            }
            finished = true;
            return glib::ControlFlow::Continue;
        }
        if let Some(fraction) = fraction {
            progress.set_fraction(fraction.clamp(0.0, 1.0));
        } else {
            progress.pulse();
        }
        glib::ControlFlow::Continue
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum InstallerCoverage {
    #[default]
    None,
    Partial,
    Complete,
}

pub(super) fn installer_backup_coverage_from(
    game: &DetailPageModel,
    config: &Config,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
) -> InstallerCoverage {
    let mut required = 0_usize;
    let mut downloaded = 0_usize;
    for artifacts in std::iter::once(game.remote_artifacts.as_slice()).chain(
        game.dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| dlc.remote_artifacts.as_slice()),
    ) {
        let groups = preferred_installer_groups(artifacts, config);
        required += groups.len();
        downloaded += groups
            .iter()
            .filter(|group| {
                dialog_artifact_state(group, managed_paths) == DialogArtifactState::Downloaded
            })
            .count();
    }
    if required > 0 && downloaded == required {
        InstallerCoverage::Complete
    } else if downloaded > 0 {
        InstallerCoverage::Partial
    } else {
        InstallerCoverage::None
    }
}

fn preferred_installer_groups(artifacts: &[RemoteArtifact], config: &Config) -> Vec<ArtifactGroup> {
    let groups = download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    let languages = download_selection::available_languages(groups.iter());
    let selected_languages =
        download_selection::default_languages(&languages, config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if config.installer_windows {
        selected_os.insert("windows".into());
    }
    if config.installer_linux {
        selected_os.insert("linux".into());
    }
    if config.installer_macos {
        selected_os.insert("macos".into());
    }
    groups
        .iter()
        .filter(|group| {
            download_selection::matches_preferences(group, &selected_os, &selected_languages)
        })
        .filter(|group| preferred_artifact_group(group, groups.iter()))
        .cloned()
        .collect()
}

pub(super) fn required_owned_dlc_ids(game: &DetailPageModel, config: &Config) -> HashSet<i64> {
    game.dlcs
        .iter()
        .filter(|dlc| {
            dlc.owned && !preferred_installer_groups(&dlc.remote_artifacts, config).is_empty()
        })
        .map(|dlc| dlc.product_id)
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DlcActionState {
    pub(super) missing_download: bool,
    pub(super) missing_install: bool,
}

pub(super) fn owned_dlc_action_state_from(
    game: &DetailPageModel,
    config: &Config,
    base_installed: bool,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
    installed_ids: &HashSet<i64>,
) -> DlcActionState {
    if game.parent_id.is_some() {
        return DlcActionState::default();
    }
    let mut state = DlcActionState::default();
    for dlc in game.dlcs.iter().filter(|dlc| {
        dlc.owned
            && dlc
                .remote_artifacts
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::Installer)
    }) {
        let downloaded = product_default_installers_are_downloaded(
            &dlc.remote_artifacts,
            config,
            managed_paths,
            false,
        );
        state.missing_download |= !downloaded;
        state.missing_install |= base_installed && !installed_ids.contains(&dlc.product_id);
    }
    state
}

pub(super) fn product_default_installers_are_downloaded(
    artifacts: &[RemoteArtifact],
    config: &Config,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
    allow_no_installers: bool,
) -> bool {
    let groups = download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    if groups.is_empty() {
        return allow_no_installers;
    }
    let languages = download_selection::available_languages(groups.iter());
    let selected_languages =
        download_selection::default_languages(&languages, config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if config.installer_windows {
        selected_os.insert("windows".into());
    }
    if config.installer_linux {
        selected_os.insert("linux".into());
    }
    if config.installer_macos {
        selected_os.insert("macos".into());
    }
    let preferred = groups
        .iter()
        .filter(|group| {
            download_selection::matches_preferences(group, &selected_os, &selected_languages)
        })
        .filter(|group| preferred_artifact_group(group, groups.iter()))
        .collect::<Vec<_>>();
    if preferred.is_empty() {
        return false;
    }
    preferred
        .into_iter()
        .all(|group| dialog_artifact_state(group, managed_paths) == DialogArtifactState::Downloaded)
}

pub(super) fn managed_artifact_paths() -> HashSet<ManagedArtifactIdentity> {
    StateStore::open()
        .and_then(|store| store.managed_files())
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.present && file.matched)
        .filter_map(|file| {
            file.provider_file_id
                .or(file.artifact_path)
                .map(|identity| (file.product_id, identity, file.version))
        })
        .collect()
}

pub(super) fn show_install_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    show_install_dialog_with_mode(window, model, detail, false, false);
}

pub(super) fn show_offline_install_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    show_install_dialog_with_mode(window, model, detail, false, true);
}

pub(super) fn cached_galaxy_available(
    detail: &DetailPageModel,
    config: &Config,
) -> Result<(), String> {
    let store =
        StateStore::open().map_err(|error| format!("could not open metadata cache: {error}"))?;
    cached_galaxy_selection_available(
        &store,
        detail,
        &default_galaxy_selection(detail, config, None),
    )
}

fn cached_galaxy_selection_available(
    store: &StateStore,
    detail: &DetailPageModel,
    selection: &crate::gog::depot_acquisition::Selection,
) -> Result<(), String> {
    let build = newest_master_windows_build(detail)
        .ok_or_else(|| "no current Master build is advertised".to_owned())?;
    crate::installation::depot_planner::cached_acquisition_available(store, &build, selection)
        .map_err(|error| format!("cached metadata is invalid: {error}"))?
        .then_some(())
        .ok_or_else(|| "Galaxy installation data has not been loaded".to_owned())
}

pub(super) fn newest_master_windows_build(
    detail: &DetailPageModel,
) -> Option<crate::domain::GalaxyBuild> {
    detail
        .galaxy_builds
        .iter()
        .filter(|build| {
            build.generation == 2
                && build.currently_returned
                && build.operating_system.eq_ignore_ascii_case("windows")
                && build.branch.is_none()
        })
        .max_by_key(|build| (build.published_at.unwrap_or_default(), build.last_seen_at))
        .cloned()
}

pub(super) fn default_galaxy_selection(
    detail: &DetailPageModel,
    config: &Config,
    preferences: Option<&crate::domain::GamePreferences>,
) -> crate::gog::depot_acquisition::Selection {
    let override_language =
        preferences.and_then(|preferences| preferences.galaxy_language.as_deref());
    let configured_language = override_language.or(config.installer_language.as_deref());
    let language = detail
        .metadata
        .localizations
        .iter()
        .find(|localization| {
            configured_language.is_some_and(|configured| {
                localization.name.eq_ignore_ascii_case(configured)
                    || localization.language_code.eq_ignore_ascii_case(configured)
            })
        })
        .map(|localization| localization.language_code.clone())
        .or_else(|| override_language.map(str::to_owned))
        .or_else(|| {
            detail
                .metadata
                .localizations
                .iter()
                .find(|localization| localization.language_code.starts_with("en"))
                .map(|localization| localization.language_code.clone())
        })
        .unwrap_or_else(|| "en".into());
    let owned_dlc = detail
        .dlcs
        .iter()
        .filter(|dlc| dlc.owned)
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    crate::gog::depot_acquisition::Selection {
        language,
        bitness: Some("64".into()),
        owned_dlc: owned_dlc.clone(),
        selected_dlc: owned_dlc,
    }
}

pub(super) fn show_primary_download(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    let fresh_base = {
        let state = model.borrow();
        detail.parent_id.is_none() && !state.installed_products.contains(&detail.product_id)
    };
    if fresh_base {
        show_install_dialog(&w.window, model, detail);
    } else {
        show_download_selector(w, model, detail);
    }
}

pub(super) fn show_repair_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    start_existing_depot_operation_dialog(
        window,
        model,
        detail,
        crate::domain::DepotOperationKind::Repair,
        None,
    );
}

pub(super) fn show_directory_repair_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    directory: std::path::PathBuf,
) {
    start_existing_depot_operation_dialog(
        window,
        model,
        detail,
        crate::domain::DepotOperationKind::Repair,
        Some(directory),
    );
}

pub(super) fn show_update_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    start_existing_depot_operation_dialog(
        window,
        model,
        detail,
        crate::domain::DepotOperationKind::Update,
        None,
    );
}

#[cfg(test)]
type TestDepotInspectionResult = anyhow::Result<
    Option<(
        crate::domain::InstalledGame,
        crate::installation::InstallationMarker,
    )>,
>;

#[cfg(test)]
thread_local! {
    static TEST_DEPOT_INSPECTION_RESULT: RefCell<Option<TestDepotInspectionResult>> = const { RefCell::new(None) };
    static TEST_DEPOT_INSPECTION_DELAYS: RefCell<VecDeque<mpsc::Receiver<()>>> = const { RefCell::new(VecDeque::new()) };
}

fn start_existing_depot_operation_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    kind: crate::domain::DepotOperationKind,
    directory: Option<std::path::PathBuf>,
) {
    let (config, epoch, session) = {
        let state = model.borrow();
        if state.logout_pending {
            return;
        }
        (
            state.config.clone(),
            state.account_epoch,
            (online::account_session(), auth::session()),
        )
    };
    let pending = adw::Dialog::builder()
        .title(match kind {
            crate::domain::DepotOperationKind::Update => "Update game",
            _ => "Repair game",
        })
        .content_width(420)
        .content_height(180)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_widget_name("repair-inspection-header");
    root.append(&header);
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 12);
    shell.set_margin_start(18);
    shell.set_margin_end(18);
    shell.set_margin_top(18);
    shell.set_margin_bottom(18);
    let spinner = gtk::Spinner::new();
    spinner.set_widget_name("repair-inspection-spinner");
    spinner.set_spinning(true);
    shell.append(&spinner);
    let label = gtk::Label::new(Some("Checking the installed game…"));
    label.set_widget_name("repair-inspection-message");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    shell.append(&label);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&shell)
        .build();
    scroll.set_widget_name("repair-inspection-scroll");
    root.append(&scroll);
    pending.set_child(Some(&root));
    pending.present(Some(window));
    let closed = Rc::new(std::cell::Cell::new(false));
    pending.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    let product_id = detail.product_id;
    let exact_library = directory
        .as_ref()
        .and_then(|directory| {
            config
                .game_libraries
                .iter()
                .find(|library| directory.parent() == Some(library.path.as_path()))
        })
        .map(|library| library.id.clone());
    let expected_directory = directory.clone();
    let (sender, receiver) = mpsc::channel();
    match crate::profile_reset::begin_activity("inspecting installed game") {
        Err(error) => {
            let _ = sender.send(Err(error));
        }
        Ok(activity) => {
            #[cfg(test)]
            let test_result =
                TEST_DEPOT_INSPECTION_RESULT.with(|result| result.borrow_mut().take());
            #[cfg(test)]
            let delay = TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow_mut().pop_front());
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<_> {
                    let _activity = activity;
                    #[cfg(test)]
                    if let Some(delay) = delay {
                        delay.recv_timeout(Duration::from_secs(10))?;
                    }
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed; inspect the installation again."
                    );
                    #[cfg(test)]
                    if let Some(result) = test_result {
                        return result;
                    }
                    let store = StateStore::open()?;
                    let libraries = config
                        .game_libraries
                        .iter()
                        .filter(|library| {
                            expected_directory.as_ref().is_none_or(|directory| {
                                directory.parent() == Some(library.path.as_path())
                            })
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed; inspect the installation again."
                    );
                    let installed =
                        crate::installation::reconcile_installed_games(&store, &libraries)?
                            .into_iter()
                            .find(|game| {
                                game.product_id == product_id
                                    && expected_directory.as_ref().is_none_or(|directory| {
                                        &game.installation_directory == directory
                                    })
                            });
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed; inspect the installation again."
                    );
                    let Some(installed) = installed else {
                        return Ok(None);
                    };
                    let marker = crate::installation::load_installation_marker(
                        &installed.installation_directory,
                    )?;
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed; inspect the installation again."
                    );
                    Ok(marker
                        .filter(|marker| {
                            marker.source == crate::domain::InstallationSource::GalaxyDepot
                        })
                        .map(|marker| (installed, marker)))
                })();
                let _ = sender.send(result);
            });
        }
    }
    let window = window.clone();
    let model = model.clone();
    let detail = detail.clone();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if closed.get() {
            return glib::ControlFlow::Break;
        }
        if model.borrow().account_epoch != epoch
            || model.borrow().logout_pending
            || session != (online::account_session(), auth::session())
        {
            pending.close();
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Ok(Ok(Some((installed, marker)))) => {
                pending.close();
                present_existing_depot_operation_dialog(
                    &window, &model, &detail, kind, installed, marker,
                );
                return glib::ControlFlow::Break;
            }
            Ok(Ok(None)) => {
                spinner.set_spinning(false);
                spinner.set_visible(false);
                label.set_label(match kind {
                    crate::domain::DepotOperationKind::Update => "This update tool checks Galaxy Depot installations. Review reinstallation options for other installation sources. Nothing has been changed.",
                    _ => "This repair tool checks Galaxy Depot installations. For a game installed from an offline installer, review reinstallation options. Nothing has been changed.",
                });
                if directory.is_some() {
                    label.set_label("Depot repair is unavailable for this folder. Browse its files or review a reset of this folder, then choose Install Again. Other installed copies are not changed.");
                }
                let reinstall = gtk::Button::with_label("Review Reinstallation…");
                reinstall.set_widget_name("repair-review-reinstallation");
                reinstall.set_visible(directory.is_none());
                reinstall.connect_clicked({
                    let pending = pending.clone();
                    let window = window.clone();
                    let model = model.clone();
                    let detail = detail.clone();
                    move |_| {
                        if model.borrow().account_epoch == epoch && !model.borrow().logout_pending {
                            load_install_choices(&pending, &window, &model, &detail, true, false);
                        }
                    }
                });
                shell.append(&reinstall);
            }
            result => {
                spinner.set_spinning(false);
                spinner.set_visible(false);
                label.set_label(&format!(
                    "Could not inspect the installation: {}",
                    match result {
                        Ok(Err(error)) => error.to_string(),
                        _ => "Worker stopped".into(),
                    }
                ));
                let retry = gtk::Button::with_label("Retry");
                retry.set_widget_name("repair-inspection-retry");
                retry.connect_clicked({
                    let pending = pending.clone();
                    let window = window.clone();
                    let model = model.clone();
                    let detail = detail.clone();
                    let directory = directory.clone();
                    move |_| {
                        if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                            return;
                        }
                        pending.close();
                        start_existing_depot_operation_dialog(
                            &window,
                            &model,
                            &detail,
                            kind,
                            directory.clone(),
                        )
                    }
                });
                shell.append(&retry);
            }
        }
        let browse = gtk::Button::with_label("Browse Local Files");
        browse.set_widget_name("repair-inspection-browse");
        browse.connect_clicked({
            let window = window.clone();
            let model = model.clone();
            let detail = detail.clone();
            let directory = directory.clone();
            move |button| {
                if model.borrow().account_epoch == epoch && !model.borrow().logout_pending {
                    if let Some(directory) = &directory {
                        browse_recovery_directory(&window, &model, directory.clone(), button);
                    } else {
                        browse_game_files(&window, &model, &detail, None, button);
                    }
                }
            }
        });
        shell.append(&browse);
        let reset = gtk::Button::with_label("Review File Reset…");
        reset.set_widget_name("repair-inspection-reset");
        reset.connect_clicked({
            let pending = pending.clone();
            let window = window.clone();
            let model = model.clone();
            let detail = detail.clone();
            let exact_library = exact_library.clone();
            move |_| {
                if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                    return;
                }
                pending.close();
                // Completion publishes the existing uninstallation event for global UI refresh.
                if let Some(library) = &exact_library {
                    super::uninstall::show_game_directory_reset_dialog(
                        &window,
                        &model,
                        &detail,
                        library.clone(),
                        Rc::new(|| {}),
                    );
                } else {
                    super::uninstall::show_uninstall_dialog(
                        &window,
                        &model,
                        &detail,
                        Rc::new(|| {}),
                    );
                }
            }
        });
        shell.append(&reset);
        pending.set_content_height(400);
        glib::ControlFlow::Break
    });
}

fn present_existing_depot_operation_dialog(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    kind: crate::domain::DepotOperationKind,
    installed: crate::domain::InstalledGame,
    marker: crate::installation::InstallationMarker,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(match kind {
            crate::domain::DepotOperationKind::Update => "Update Galaxy installation?",
            _ => "Continue game repair?",
        })
        .body(match kind {
            crate::domain::DepotOperationKind::Update => {
                "Install the newest build from the currently selected branch."
            }
            _ => {
                "Check the installed game's files and finish required setup. Missing or damaged files may be downloaded. Your current Depot version is kept."
            }
        })
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("start", "Start")]);
    dialog.set_default_response(Some("start"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("start", adw::ResponseAppearance::Suggested);
    let branches = marker
        .galaxy_depot
        .as_ref()
        .and_then(|depot| depot.branch.clone());
    let library_root = installed
        .installation_directory
        .parent()
        .map(std::path::Path::to_path_buf);
    let selected_dlc = marker
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<BTreeSet<_>>();
    let language = marker
        .galaxy_depot
        .as_ref()
        .and_then(|depot| depot.language.clone())
        .unwrap_or_else(|| "en".into());
    let bitness = marker
        .galaxy_depot
        .as_ref()
        .and_then(|depot| depot.architecture.clone());
    let product_id = detail.product_id;
    let slug = detail.slug.clone();
    let library_id = installed.library_id;
    let epoch = model.borrow().account_epoch;
    let model = model.clone();
    let result_window = window.clone();
    dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if response != "start" || model.borrow().account_epoch != epoch {
            return;
        }
        let Some(library_root) = library_root else {
            return;
        };
        let pending = adw::Dialog::builder().title("Preparing required components").content_width(560).content_height(320).build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&adw::HeaderBar::new());
        let status = gtk::Label::builder().label("Checking which files and components this game needs…").wrap(true).selectable(true).build();
        content.append(&gtk::ScrolledWindow::builder().child(&status).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
        let progress = gtk::ProgressBar::new();
        content.append(&progress);
        progress.pulse();
        let offline = gtk::Button::with_label("Offline installers…");
        content.append(&offline);
        offline.connect_clicked({ let pending=pending.clone(); let window=result_window.clone(); let model=model.clone(); move |_| {
            if model.borrow().account_epoch != epoch { return; }
            pending.close();
            let _=gtk::prelude::WidgetExt::activate_action(&window,"win.offline-download",Some(&product_id.to_variant()));
        }});
        pending.set_child(Some(&content));
        let closed=Rc::new(std::cell::Cell::new(false));
        pending.connect_closed({let closed=closed.clone(); move |_|closed.set(true)});
        pending.present(Some(&result_window));
        let session=crate::online::account_session();
        let authentication = current_depot_session(&model);
        let (sender,receiver)=mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let authentication = authentication?;
                authentication.validate()?;
                let store = StateStore::open()?;
                let client = reqwest::blocking::Client::new();
                let builds = crate::gog::depot_service::list_builds(
                    &store,
                    &client,
                    &authentication,
                    &crate::gog::depot_service::BuildRequest {
                        user_id: authentication.token.user_id.clone(),
                        product_id,
                        platform: "windows".into(),
                        generation: 2,
                        branch: branches.clone(),
                        supplied_password: None,
                    },
                )?;
                let build = crate::gog::depot_service::resolve_operation_build(
                    &builds, &marker, kind, None,
                )?
                .clone();
                let request=crate::gog::depot_service::prepare_operation(
                    &store,
                    &client,
                    &authentication,
                    crate::gog::depot_service::PrepareOperationRequest {
                        build,
                        selection: crate::gog::depot_acquisition::Selection {
                            language,
                            bitness,
                            owned_dlc: selected_dlc.clone(),
                            selected_dlc,
                        },
                        operation_id: format!(
                            "{}-{}",
                            product_id,
                            chrono::Utc::now().timestamp_millis()
                        ),
                        kind,
                        library_id,
                        library_root,
                        slug,
                    },
                )?;
                anyhow::ensure!(crate::online::account_session()==session,"Account changed during preparation");
                Ok((request, authentication))
            })();
            let _=sender.send(result);
        });
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if closed.get() { return glib::ControlFlow::Break; }
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending || crate::online::account_session()!=session { pending.close(); return glib::ControlFlow::Break; }
            match receiver.try_recv() {
                Ok(Ok((request, authentication))) => { confirm_depot_plan(&result_window,&model,request,authentication,&pending); }
                Ok(Err(error)) => status.set_label(&notifications::failure_message("Could not prepare required components. Close and retry, or choose offline installers.", &format!("{error:#}"))),
                Err(mpsc::TryRecvError::Empty) => { progress.pulse(); return glib::ControlFlow::Continue; }
                Err(mpsc::TryRecvError::Disconnected) => status.set_label("Preparation stopped. Close and retry, or choose offline installers."),
            }
            progress.set_visible(false);
            glib::ControlFlow::Break
        });
    });
}

struct InstallPreparation {
    local_only: bool,
    config: Config,
    existing_installation: Option<crate::domain::InstalledGame>,
    installed_dlc_ids: HashSet<i64>,
    candidates: crate::installation::InstallerCandidates,
    dlc_candidates: HashMap<i64, crate::installation::InstallerCandidates>,
    galaxy_preflight: Result<(), String>,
    galaxy_selection: crate::gog::depot_acquisition::Selection,
    library_statuses: Vec<crate::storage::LibraryStatus>,
    remote_installers: Vec<download_selection::ArtifactGroup>,
    offline_error: Option<String>,
}

#[derive(Clone, Copy)]
enum InstallSource {
    GalaxyWindows,
    OfflineInstaller(usize),
    RemoteOffline(usize),
}

impl InstallSource {
    fn size(
        self,
        candidates: &[crate::installation::InstallerCandidate],
        remote: &[download_selection::ArtifactGroup],
    ) -> u64 {
        match self {
            Self::GalaxyWindows => 0,
            Self::OfflineInstaller(index) => candidates[index].total_size,
            Self::RemoteOffline(index) => remote[index].total_size.unwrap_or(0),
        }
    }
}

fn show_install_dialog_with_mode(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    repair: bool,
    local_only: bool,
) {
    let dialog = adw::Dialog::builder()
        .content_width(680)
        .content_height(620)
        .build();
    load_install_choices(&dialog, window, model, detail, repair, local_only);
    dialog.present(Some(window));
}

fn load_install_choices(
    dialog: &adw::Dialog,
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    repair: bool,
    local_only: bool,
) {
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &detail.title,
        "Preparing installation choices",
    )));
    shell.append(&header);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    shell.append(&spinner);
    let status = gtk::Label::new(Some("Checking downloaded installers and installed files…"));
    status.set_wrap(true);
    shell.append(&status);
    dialog.set_child(Some(&shell));
    let (config, epoch) = {
        let state = model.borrow();
        (state.config.clone(), state.account_epoch)
    };
    let prepared_detail = detail.clone();
    let product_id = detail.product_id;
    let session = online::account_session();
    let auth_session = auth::session();
    let acquisition = {
        let state = model.borrow();
        state
            .games
            .iter()
            .find(|game| game.product_id == product_id)
            .cloned()
            .zip(
                state
                    .account_token
                    .as_ref()
                    .map(|token| token.access_token.clone()),
            )
    };
    let preferences = super::update_policies::policy_request(move || {
        online::with_account_session(session, || StateStore::open()?.game_preferences(product_id))
    });
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<InstallPreparation> {
            let _activity = crate::profile_reset::begin_activity("loading installation choices")?;
            anyhow::ensure!(
                online::account_session() == session && auth::session() == auth_session,
                "The account changed. Reopen installation choices."
            );
            let preferences = preferences
                .recv()
                .map_err(|_| anyhow::anyhow!("Game language preferences stopped loading"))??;
            let store = StateStore::open()?;
            let library_statuses = crate::storage::inspect_libraries(&config)?;
            let managed_files = store
                .managed_files()?
                .into_iter()
                .filter(|file| {
                    library_statuses.iter().any(|status| {
                        status.kind == LibraryKind::OfflineInstallers
                            && status.compatibility
                                == crate::storage::LibraryCompatibility::Compatible
                            && file.path.starts_with(&status.path)
                    })
                })
                .collect::<Vec<_>>();
            let id = prepared_detail
                .parent_id
                .unwrap_or(prepared_detail.product_id);
            let existing_installation =
                crate::installation::reconcile_installed_games(&store, &config.game_libraries)?
                    .into_iter()
                    .find(|game| {
                        game.product_id == id
                            && game.state == crate::domain::InstallationState::Installed
                    });
            let installed_dlc_ids = crate::installation::installed_dlc_ids(&store, id)?;
            let revisions = store.load_all_download_revisions(id)?;
            let mut candidates = crate::installation::detect_installer_candidates(
                id,
                &revisions,
                &managed_files,
                &config,
            );
            candidates.usable.retain(|candidate| {
                candidate.method != crate::installation::InstallationMethod::Unsupported
                    && (!local_only || candidate.complete)
            });
            let mut dlc_candidates = HashMap::new();
            for child in std::iter::once(prepared_detail.product_id).chain(
                prepared_detail
                    .dlcs
                    .iter()
                    .filter(|dlc| dlc.owned)
                    .map(|dlc| dlc.product_id),
            ) {
                dlc_candidates.insert(
                    child,
                    crate::installation::detect_installer_candidates(
                        child,
                        &store.load_all_download_revisions(child)?,
                        &managed_files,
                        &config,
                    ),
                );
            }
            let galaxy_selection =
                default_galaxy_selection(&prepared_detail, &config, preferences.as_ref());
            let galaxy_preflight =
                cached_galaxy_selection_available(&store, &prepared_detail, &galaxy_selection);
            let mut artifacts = prepared_detail.remote_artifacts.clone();
            let mut offline_error = None;
            if !repair
                && !local_only
                && existing_installation.is_none()
                && prepared_detail.parent_id.is_none()
                && artifacts.is_empty()
                && let Some((game, token)) = acquisition
            {
                match online::fetch_product_section(
                    &game,
                    online::DetailSection::Acquisition,
                    Some(&token),
                    Some(&galaxy_selection.language),
                    session,
                ) {
                    Ok(game) => artifacts = game.remote_artifacts,
                    Err(error) => {
                        offline_error = Some(notifications::failure_message(
                            "Could not load offline installer choices",
                            &format!("{error:#}"),
                        ))
                    }
                }
            }
            let remote_installers = download_selection::group_artifacts(&artifacts)
                .into_iter()
                .filter(|group| {
                    group.kind == ArtifactKind::Installer
                        && group.product_id == product_id
                        && matches!(group.operating_system.as_deref(), Some("windows" | "linux"))
                })
                .filter(|group| {
                    !candidates.usable.iter().any(|candidate| {
                        candidate.complete
                            && revisions.iter().any(|revision| {
                                Some(revision.revision_id) == candidate.revision_id
                                    && revision.parts.len() == group.artifacts.len()
                                    && group.artifacts.iter().all(|artifact| {
                                        revision.parts.iter().any(|part| {
                                            artifact.provider_group_id.as_deref()
                                                == Some(revision.provider_group_id.as_str())
                                                && artifact.provider_file_id.as_deref()
                                                    == Some(part.provider_file_id.as_str())
                                                && artifact.download_path == part.downlink
                                                && artifact.version == revision.version
                                        })
                                    })
                            })
                    })
                })
                .collect();
            Ok(InstallPreparation {
                local_only,
                config,
                existing_installation,
                installed_dlc_ids,
                candidates,
                dlc_candidates,
                galaxy_preflight,
                galaxy_selection,
                library_statuses,
                remote_installers,
                offline_error,
            })
        })();
        let _ = sender.send(result);
    });
    let closed = Rc::new(std::cell::Cell::new(false));
    dialog.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    let dialog = dialog.clone();
    let window = window.clone();
    let model = model.clone();
    let detail = detail.clone();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if closed.get()
            || model.borrow().account_epoch != epoch
            || online::account_session() != session
            || auth::session() != auth_session
        {
            dialog.close();
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err(anyhow::anyhow!("Installer preparation stopped")),
        };
        match result {
            Ok(preparation) => {
                populate_install_dialog(&dialog, &window, &model, &detail, repair, preparation)
            }
            Err(error) => {
                spinner.set_spinning(false);
                status.set_label(&format!("Could not load installation choices: {error}"));
                let retry = gtk::Button::with_label("Retry");
                retry.connect_clicked({
                    let dialog = dialog.clone();
                    let window = window.clone();
                    let model = model.clone();
                    let detail = detail.clone();
                    move |_| {
                        load_install_choices(&dialog, &window, &model, &detail, repair, local_only)
                    }
                });
                shell.append(&retry);
            }
        }
        glib::ControlFlow::Break
    });
}

fn populate_install_dialog(
    dialog: &adw::Dialog,
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    repair: bool,
    preparation: InstallPreparation,
) {
    dialog.set_content_width(680);
    dialog.set_content_height(620);
    dialog.set_title(if repair {
        "Repair game"
    } else {
        "Install game"
    });
    let InstallPreparation {
        local_only,
        config,
        existing_installation,
        installed_dlc_ids,
        candidates,
        mut dlc_candidates,
        galaxy_preflight,
        galaxy_selection,
        library_statuses,
        remote_installers,
        offline_error,
    } = preparation;
    let galaxy_preflight = (!local_only).then_some(galaxy_preflight);
    let galaxy_request = {
        let state = model.borrow();
        (!repair && !local_only && existing_installation.is_none() && detail.parent_id.is_none())
            .then(|| {
                state
                    .games
                    .iter()
                    .find(|game| {
                        game.product_id == detail.product_id
                            && (game.platforms.windows
                                || (!game.platforms.linux && !game.platforms.macos))
                    })
                    .cloned()
            })
            .flatten()
            .zip(
                state
                    .account_token
                    .as_ref()
                    .map(|token| token.access_token.clone()),
            )
            .map(|(game, token)| {
                (
                    game,
                    token,
                    Some(galaxy_selection.language.clone()),
                    online::account_session(),
                    state.account_epoch,
                )
            })
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &format!(
            "{} {}",
            if repair { "Repair" } else { "Install" },
            detail.title
        ),
        if repair {
            "Reinstall the game and its DLC"
        } else {
            "Create an installation plan"
        },
    )));
    root.append(&header);
    let body = adw::PreferencesPage::new();
    let installer_group = adw::PreferencesGroup::new();
    installer_group.set_title("Installer");
    let galaxy_feedback = gtk::Box::new(gtk::Orientation::Vertical, 6);
    galaxy_feedback.set_widget_name("install-galaxy-feedback");
    let galaxy_preflight_label = gtk::Label::new(None);
    galaxy_preflight_label.set_widget_name("install-galaxy-preflight");
    galaxy_preflight_label.set_xalign(0.0);
    galaxy_preflight_label.set_wrap(true);
    galaxy_preflight_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    galaxy_preflight_label.set_selectable(true);
    galaxy_preflight_label.set_visible(false);
    galaxy_feedback.append(&galaxy_preflight_label);
    installer_group.add(&galaxy_feedback);
    let galaxy_builds = detail
        .galaxy_builds
        .iter()
        .filter(|_| !local_only)
        .filter(|build| {
            build.generation == 2
                && build.currently_returned
                && build.operating_system.eq_ignore_ascii_case("windows")
        })
        .filter(|_| galaxy_preflight.as_ref().is_none_or(Result::is_ok))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(Err(error)) = &galaxy_preflight {
        galaxy_preflight_label.set_label(&if galaxy_request.is_some() {
            "Downloaded installers remain available while Galaxy data loads.".into()
        } else {
            format!("Galaxy build unavailable: {error}")
        });
        galaxy_preflight_label.set_visible(true);
    }
    let galaxy_ready = Rc::new(std::cell::Cell::new(!galaxy_builds.is_empty()));
    let mut ranked_sources = if repair || existing_installation.is_some() {
        Vec::new()
    } else {
        crate::installation::rank_fresh_install_sources(
            &config,
            &candidates.usable,
            !galaxy_builds.is_empty() || galaxy_request.is_some(),
        )
        .into_iter()
        .map(|source| match source {
            crate::installation::FreshInstallSource::GalaxyWindows => InstallSource::GalaxyWindows,
            crate::installation::FreshInstallSource::OfflineInstaller(index) => {
                InstallSource::OfflineInstaller(index)
            }
        })
        .collect::<Vec<_>>()
    };
    if !repair && !local_only && existing_installation.is_none() {
        ranked_sources.extend((0..remote_installers.len()).map(InstallSource::RemoteOffline));
        ranked_sources.sort_by_key(|source| {
            use crate::config::PreferredInstallationSource::*;
            let preferred = match source {
                InstallSource::GalaxyWindows => WindowsGalaxy,
                InstallSource::OfflineInstaller(index)
                    if candidates.usable[*index].method
                        == crate::installation::InstallationMethod::NativeLinux =>
                {
                    LinuxOffline
                }
                InstallSource::RemoteOffline(index)
                    if remote_installers[*index].operating_system.as_deref() == Some("linux") =>
                {
                    LinuxOffline
                }
                _ => WindowsOffline,
            };
            let language = match source {
                InstallSource::RemoteOffline(index) => {
                    remote_installers[*index].language.as_deref()
                }
                InstallSource::OfflineInstaller(index) => {
                    candidates.usable[*index].language.as_deref()
                }
                InstallSource::GalaxyWindows => None,
            }
            .unwrap_or("");
            (
                config
                    .installation_source_order
                    .iter()
                    .position(|value| *value == preferred)
                    .unwrap_or(usize::MAX),
                if config
                    .installer_language
                    .as_deref()
                    .is_some_and(|preferred| preferred.eq_ignore_ascii_case(language))
                {
                    0
                } else if language.eq_ignore_ascii_case("english")
                    || language.eq_ignore_ascii_case("en")
                {
                    1
                } else {
                    2
                },
            )
        });
    }
    let mut source_values = Vec::new();
    for source in &ranked_sources {
        let label = match source {
            InstallSource::GalaxyWindows => "Windows · Depot".to_owned(),
            InstallSource::OfflineInstaller(index) => {
                let candidate = &candidates.usable[*index];
                format!(
                    "{} · Downloaded installer · {} · {}",
                    if candidate.method == crate::installation::InstallationMethod::NativeLinux {
                        "Linux"
                    } else {
                        "Windows"
                    },
                    candidate.version.as_deref().unwrap_or("Unknown version"),
                    candidate.language.as_deref().unwrap_or("Any language")
                )
            }
            InstallSource::RemoteOffline(index) => {
                let group = &remote_installers[*index];
                format!(
                    "{} · Offline installer · {} · {}",
                    group.operating_system.as_deref().unwrap_or("Any OS"),
                    group.version.as_deref().unwrap_or("Unknown version"),
                    group.language.as_deref().unwrap_or("Any language")
                )
            }
        };
        source_values.push((label, *source));
    }
    let source_list = gtk::StringList::new(
        &source_values
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>(),
    );
    let source = gtk::DropDown::new(Some(source_list), gtk::Expression::NONE);
    let galaxy_selected =
        Rc::new(std::cell::Cell::new(source_values.first().is_some_and(
            |(_, value)| matches!(value, InstallSource::GalaxyWindows),
        )));
    let candidate_labels = candidates
        .usable
        .iter()
        .map(|candidate| {
            format!(
                "{} · {} · {}",
                candidate.operating_system.as_deref().unwrap_or("Any OS"),
                candidate.language.as_deref().unwrap_or("Any language"),
                candidate.version.as_deref().unwrap_or("Unknown version"),
            )
        })
        .collect::<Vec<_>>();
    let candidate_list = gtk::StringList::new(
        &candidate_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let candidate = gtk::DropDown::new(Some(candidate_list), gtk::Expression::NONE);
    let preferred_candidate = source_values
        .first()
        .and_then(|(_, source)| match source {
            InstallSource::OfflineInstaller(index) => Some(*index),
            InstallSource::GalaxyWindows | InstallSource::RemoteOffline(_) => None,
        })
        .or(candidates.preferred)
        .unwrap_or(0);
    candidate.set_selected(preferred_candidate as u32);
    let candidate_menu = gtk::MenuButton::new();
    candidate_menu.set_widget_name("install-source-menu");
    candidate_menu.set_hexpand(true);
    candidate_menu.set_sensitive(if existing_installation.is_none() && !repair {
        !source_values.is_empty()
    } else {
        !candidates.usable.is_empty()
    });
    candidate_menu.add_css_class("install-choice-menu");
    let candidate_popover = gtk::Popover::new();
    let candidate_choices = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let mut candidate_choice_buttons = Vec::new();
    let mut source_choice_buttons = Vec::new();
    if existing_installation.is_none() && !repair {
        for (index, (label, source_value)) in source_values.iter().enumerate() {
            let size = source_value.size(&candidates.usable, &remote_installers);
            let choice = gtk::Button::new();
            choice.set_widget_name(&format!("install-source-{index}"));
            if matches!(source_value, InstallSource::GalaxyWindows) {
                choice.set_sensitive(galaxy_ready.get());
            }
            choice.add_css_class("flat");
            choice.add_css_class("install-choice-row");
            choice.set_child(Some(&install_choice_content(
                detail.icon.as_deref(),
                &detail.title,
                label,
                size,
                index == source.selected() as usize,
                false,
            )));
            choice.connect_clicked({
                let source = source.clone();
                let popover = candidate_popover.clone();
                move |_| {
                    source.set_selected(index as u32);
                    popover.popdown();
                }
            });
            candidate_choices.append(&choice);
            source_choice_buttons.push(choice);
        }
    } else {
        for (index, installer) in candidates.usable.iter().enumerate() {
            let choice = gtk::Button::new();
            choice.add_css_class("flat");
            choice.add_css_class("install-choice-row");
            choice.set_child(Some(&install_choice_content(
                detail.icon.as_deref(),
                &detail.title,
                &candidate_labels[index],
                installer.total_size,
                index == candidate.selected() as usize,
                false,
            )));
            choice.connect_clicked({
                let candidate = candidate.clone();
                let popover = candidate_popover.clone();
                move |_| {
                    candidate.set_selected(index as u32);
                    popover.popdown();
                }
            });
            candidate_choices.append(&choice);
            candidate_choice_buttons.push(choice);
        }
    }
    candidate_popover.set_child(Some(&candidate_choices));
    candidate_menu.set_popover(Some(&candidate_popover));
    if let Some((label, source_value)) = source_values.first()
        && existing_installation.is_none()
        && !repair
    {
        let size = source_value.size(&candidates.usable, &remote_installers);
        candidate_menu.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            label,
            size,
            false,
            true,
        )));
    } else if let Some(installer) = candidates.usable.get(candidate.selected() as usize) {
        candidate_menu.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            &candidate_labels[candidate.selected() as usize],
            installer.total_size,
            false,
            true,
        )));
    }
    installer_group.add(&candidate_menu);
    let mut branches = galaxy_builds
        .iter()
        .map(|build| build.branch.clone())
        .collect::<Vec<_>>();
    branches.sort_by(|left, right| match (left, right) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        _ => left.cmp(right),
    });
    branches.dedup();
    let branch_labels = branches
        .iter()
        .map(|branch| branch.as_deref().unwrap_or("Master"))
        .collect::<Vec<_>>();
    let branch_list = gtk::StringList::new(&branch_labels);
    let branch = gtk::DropDown::new(Some(branch_list), gtk::Expression::NONE);
    branch.set_selected(0);
    let branch_row = adw::ActionRow::new();
    branch_row.set_title("Galaxy branch");
    branch_row.add_suffix(&branch);
    branch_row.set_visible(false);
    installer_group.add(&branch_row);
    let branch_password = adw::PasswordEntryRow::new();
    branch_password.set_title("Protected branch password");
    branch_password.set_show_apply_button(false);
    branch_password
        .set_visible(galaxy_selected.get() && branches.first().is_some_and(Option::is_some));
    installer_group.add(&branch_password);
    let branches = Rc::new(RefCell::new(branches));
    let galaxy_builds = Rc::new(RefCell::new(galaxy_builds));
    if let Some(installed_game) = &existing_installation
        && !repair
    {
        candidate_menu.set_sensitive(false);
        let installed_label = format!(
            "{} · {} · {}",
            installed_game
                .installer_operating_system
                .as_deref()
                .unwrap_or("Unknown OS"),
            installed_game
                .installer_language
                .as_deref()
                .unwrap_or("Unknown language"),
            installed_game
                .installed_version
                .as_deref()
                .unwrap_or("Unknown version")
        );
        candidate_menu.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            &installed_label,
            0,
            false,
            false,
        )));
        candidate_menu.set_tooltip_text(Some(
            "Installed base game; no base installer file is required to add DLC",
        ));
    }
    let dlc_products = if detail.parent_id.is_some() {
        vec![(detail.product_id, detail.title.clone())]
    } else {
        detail
            .dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| (dlc.product_id, dlc.title.clone()))
            .collect()
    };
    let selected_base_version = candidates
        .usable
        .get(candidate.selected() as usize)
        .and_then(|installer| installer.version.as_deref())
        .or_else(|| {
            existing_installation
                .as_ref()
                .and_then(|installed| installed.installed_version.as_deref())
        });
    let mut dlc_choices = Vec::new();
    for (dlc_product_id, dlc_title) in dlc_products {
        let choices = dlc_candidates.remove(&dlc_product_id).unwrap_or_default();
        let installers = choices
            .usable
            .into_iter()
            .filter(|installer| {
                installer.complete
                    && installer.method != crate::installation::InstallationMethod::Unsupported
                    && existing_installation.as_ref().is_none_or(|installed| {
                        installer_matches_installed_game(installer, installed)
                    })
            })
            .collect::<Vec<_>>();
        let selected = installers
            .iter()
            .find(|installer| {
                versions_match(installer.version.as_deref(), selected_base_version)
                    && candidates
                        .usable
                        .get(candidate.selected() as usize)
                        .is_none_or(|base| {
                            config.offline_libraries.iter().any(|library| {
                                base.paths
                                    .iter()
                                    .chain(&installer.paths)
                                    .all(|path| path.starts_with(&library.path))
                            })
                        })
            })
            .cloned();
        let check = gtk::CheckButton::with_label(&dlc_title);
        let already_installed = installed_dlc_ids.contains(&dlc_product_id);
        check.set_active(selected.is_some());
        check.set_sensitive(selected.is_some() && (repair || !already_installed));
        if already_installed && !repair {
            check.set_tooltip_text(Some("Already installed"));
        }
        dlc_choices.push(DlcInstallerChoice {
            product_id: dlc_product_id,
            title: dlc_title,
            installers,
            selected: Rc::new(RefCell::new(selected)),
            check,
            already_installed,
        });
    }
    let dlc_summary = gtk::Label::new(None);
    let dlc_menu = gtk::MenuButton::new();
    if !dlc_choices.is_empty() {
        dlc_menu.set_hexpand(true);
        dlc_menu.add_css_class("install-dlc-menu");
        dlc_summary.set_xalign(0.0);
        dlc_summary.set_hexpand(true);
        let dlc_menu_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        dlc_menu_content.append(&gtk::Image::from_icon_name("package-x-generic-symbolic"));
        dlc_menu_content.append(&dlc_summary);
        dlc_menu_content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
        dlc_menu.set_child(Some(&dlc_menu_content));
        let dlc_popover = gtk::Popover::new();
        let dlc_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        dlc_list.set_margin_start(12);
        dlc_list.set_margin_end(12);
        dlc_list.set_margin_top(10);
        dlc_list.set_margin_bottom(10);
        let update_summary: Rc<dyn Fn()> = {
            let choices = dlc_choices.clone();
            let summary = dlc_summary.clone();
            Rc::new(move || update_dlc_summary(&summary, &choices, repair))
        };
        for choice in &dlc_choices {
            let check = &choice.check;
            dlc_list.append(check);
            let update_summary = update_summary.clone();
            check.connect_toggled(move |_| update_summary());
        }
        update_summary();
        dlc_popover.set_child(Some(&dlc_list));
        dlc_menu.set_popover(Some(&dlc_popover));
        installer_group.add(&dlc_menu);
    }
    if existing_installation.is_none() && !repair {
        candidate_menu.set_sensitive(!source_values.is_empty());
        branch_row.set_visible(galaxy_selected.get() && branches.borrow().len() > 1);
        for choice in &dlc_choices {
            if galaxy_selected.get() {
                choice.check.set_active(true);
                choice.check.set_sensitive(true);
            }
        }
        let galaxy_selected_state = galaxy_selected.clone();
        let candidate_menu_state = candidate_menu.clone();
        let candidate_state = candidate.clone();
        let branch_row_state = branch_row.clone();
        let choices = dlc_choices.clone();
        let source_values_state = source_values.clone();
        let branch_password_state = branch_password.clone();
        let branches_state = branches.clone();
        let branch_state = branch.clone();
        let source_buttons_state = source_choice_buttons.clone();
        let icon = detail.icon.clone();
        let title = detail.title.clone();
        let source_candidates = candidates.usable.clone();
        let remote_installers = remote_installers.clone();
        source.connect_selected_notify(move |selector| {
            let selected_index = selector.selected() as usize;
            let Some((label, selected)) = source_values_state.get(selected_index) else {
                return;
            };
            let is_galaxy = matches!(selected, InstallSource::GalaxyWindows);
            galaxy_selected_state.set(is_galaxy);
            branch_row_state.set_visible(is_galaxy && branches_state.borrow().len() > 1);
            branch_password_state.set_visible(
                is_galaxy
                    && branches_state
                        .borrow()
                        .get(branch_state.selected() as usize)
                        .is_some_and(Option::is_some),
            );
            if let InstallSource::OfflineInstaller(index) = selected {
                candidate_state.set_selected(*index as u32);
            }
            for choice in &choices {
                if is_galaxy {
                    choice.check.set_active(true);
                    choice.check.set_sensitive(true);
                }
            }
            let size = selected.size(&source_candidates, &remote_installers);
            candidate_menu_state.set_child(Some(&install_choice_content(
                icon.as_deref(),
                &title,
                label,
                size,
                false,
                true,
            )));
            for (index, button) in source_buttons_state.iter().enumerate() {
                if let Some((label, source)) = source_values_state.get(index) {
                    let size = source.size(&source_candidates, &remote_installers);
                    button.set_child(Some(&install_choice_content(
                        icon.as_deref(),
                        &title,
                        label,
                        size,
                        index == selected_index,
                        false,
                    )));
                }
            }
        });
        let galaxy_selected_state = galaxy_selected.clone();
        let branch_password_state = branch_password.clone();
        let branches_state = branches.clone();
        branch.connect_selected_notify(move |selector| {
            branch_password_state.set_visible(
                galaxy_selected_state.get()
                    && branches_state
                        .borrow()
                        .get(selector.selected() as usize)
                        .is_some_and(Option::is_some),
            );
        });
    }
    let interactive_prompts = adw::SwitchRow::new();
    interactive_prompts.set_title("Interactive install");
    interactive_prompts.set_subtitle(
        "Show the installer and let you choose optional settings; Ludomere still supplies the install directory",
    );
    interactive_prompts.set_tooltip_text(Some(
        "When disabled, Windows installers run fully unattended. Enable this to choose options such as the installer language yourself.",
    ));
    interactive_prompts.set_active(config.interactive_installer_prompts);
    interactive_prompts.set_visible(!galaxy_selected.get());
    {
        let interactive_prompts = interactive_prompts.clone();
        let values = source_values.clone();
        source.connect_selected_notify(move |selector| {
            interactive_prompts.set_visible(
                !values
                    .get(selector.selected() as usize)
                    .is_some_and(|(_, source)| matches!(source, InstallSource::GalaxyWindows)),
            );
        });
    }
    {
        let window = window.clone();
        let reverting = Rc::new(std::cell::Cell::new(false));
        interactive_prompts.connect_active_notify(move |row| {
            if reverting.replace(false) || !row.is_active() {
                return;
            }
            let config = Config::load_or_create().unwrap_or_default();
            if config.interactive_installer_explanation_dismissed {
                return;
            }
            let never_show = gtk::CheckButton::with_label("Don't show this explanation again");
            let explanation = adw::AlertDialog::builder()
                .heading("Enable interactive install?")
                .body("Windows installers will open normally so you can choose optional settings such as language. Ludomere will still provide the required game directory. Linux installers will ask you for responses they cannot handle automatically.")
                .extra_child(&never_show)
                .build();
            explanation.add_responses(&[("cancel", "Cancel"), ("enable", "Enable")]);
            explanation.set_default_response(Some("enable"));
            explanation.set_close_response("cancel");
            explanation.set_response_appearance("enable", adw::ResponseAppearance::Suggested);
            let row = row.clone();
            let reverting = reverting.clone();
            explanation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if never_show.is_active()
                    && let Ok(mut config) = Config::load_or_create()
                {
                    config.interactive_installer_explanation_dismissed = true;
                    let _ = config.save();
                }
                if response != "enable" {
                    reverting.set(true);
                    row.set_active(false);
                }
            });
        });
    }
    let installer_detail = adw::ActionRow::new();
    installer_detail.set_use_markup(false);
    body.add(&installer_group);
    if let Some(error) = offline_error {
        let row = adw::ActionRow::builder()
            .title("Offline installer choices unavailable")
            .subtitle(&error)
            .build();
        let retry = gtk::Button::with_label("Retry");
        row.add_suffix(&retry);
        installer_group.add(&row);
        let dialog = dialog.downgrade();
        let window = window.downgrade();
        let model = model.clone();
        let detail = detail.clone();
        let epoch = model.borrow().account_epoch;
        retry.connect_clicked(move |_| {
            if model.borrow().account_epoch == epoch
                && !model.borrow().logout_pending
                && let (Some(dialog), Some(window)) = (dialog.upgrade(), window.upgrade())
            {
                load_install_choices(&dialog, &window, &model, &detail, repair, local_only);
            }
        });
    }

    let archive_group = adw::PreferencesGroup::builder().title("Save offline installers to").description("All required parts are saved here, then installed automatically into Game Files. Optional DLC can be installed separately. Automatic installation is unattended.").build();
    let archive_library = gtk::DropDown::from_strings(
        &config
            .offline_libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect::<Vec<_>>(),
    );
    archive_library.set_widget_name("install-archive-library");
    archive_library.set_selected(
        config
            .offline_libraries
            .iter()
            .position(|library| library.default)
            .unwrap_or(0) as u32,
    );
    let archive_path = gtk::Label::new(None);
    archive_path.set_wrap(true);
    archive_path.set_selectable(true);
    archive_group.add(&archive_library);
    archive_group.add(&archive_path);
    body.add(&archive_group);

    let destination_group = adw::PreferencesGroup::new();
    destination_group.set_title("INSTALL TO GAME FILES:");
    let storage_settings = gtk::Button::from_icon_name("emblem-system-symbolic");
    storage_settings.set_tooltip_text(Some("Open Storage settings"));
    destination_group.set_header_suffix(Some(&storage_settings));
    let library_labels = config
        .game_libraries
        .iter()
        .map(|library| library.name.as_str())
        .collect::<Vec<_>>();
    let library_list = gtk::StringList::new(&library_labels);
    let library = gtk::DropDown::new(Some(library_list), gtk::Expression::NONE);
    let default_library = existing_installation
        .as_ref()
        .and_then(|installed| {
            config.game_libraries.iter().position(|library| {
                library.id == installed.library_id
                    || installed.installation_directory.parent() == Some(library.path.as_path())
            })
        })
        .or_else(|| {
            config
                .game_libraries
                .iter()
                .position(|library| library.default)
        })
        .unwrap_or(0);
    library.set_selected(default_library as u32);
    let library_choices = gtk::ListBox::new();
    library_choices.set_widget_name("install-game-libraries");
    library_choices.set_selection_mode(gtk::SelectionMode::Single);
    library_choices.add_css_class("install-library-list");
    let storage_locked = existing_installation.is_some();
    for (index, game_library) in config.game_libraries.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
        let path = gtk::Label::new(Some(&game_library.path.display().to_string()));
        path.set_xalign(0.0);
        path.set_hexpand(true);
        path.set_wrap(true);
        path.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        path.add_css_class("install-library-path");
        content.append(&path);
        if game_library.default {
            let star = gtk::Image::from_icon_name("starred-symbolic");
            star.add_css_class("storage-default-library");
            star.set_tooltip_text(Some("Default game library"));
            content.append(&star);
        }
        let free = gtk::Label::new(Some("Calculating…"));
        free.add_css_class("install-library-free");
        content.append(&free);
        row.set_child(Some(&content));
        if storage_locked && index != default_library {
            row.add_css_class("dim-label");
            row.set_tooltip_text(Some(
                "Use Storage settings to move an installed game to another library.",
            ));
        } else if storage_locked {
            row.set_tooltip_text(Some("This game is installed in this library."));
        }
        library_choices.append(&row);
        match library_statuses
            .iter()
            .find(|status| {
                status.kind == LibraryKind::GameFiles && status.library_id == game_library.id
            })
            .map(|status| &status.compatibility)
        {
            Some(crate::storage::LibraryCompatibility::Compatible) => {
                update_install_library_free_space(&game_library.path, &free);
            }
            Some(
                crate::storage::LibraryCompatibility::Incompatible(reason)
                | crate::storage::LibraryCompatibility::Unavailable(reason),
            ) => {
                row.set_sensitive(false);
                row.set_tooltip_text(Some(reason));
                free.set_label("Unavailable");
            }
            None => free.set_label("Unavailable"),
        }
        if index == default_library {
            library_choices.select_row(Some(&row));
        }
    }
    library_choices.connect_row_selected({
        let library = library.clone();
        let locked_index = storage_locked.then_some(default_library as i32);
        move |list, row| {
            if let Some(row) = row {
                if let Some(locked) = locked_index {
                    if row.index() != locked
                        && let Some(installed_row) = list.row_at_index(locked)
                    {
                        list.select_row(Some(&installed_row));
                    }
                    library.set_selected(locked as u32);
                } else {
                    library.set_selected(row.index() as u32);
                }
            }
        }
    });
    destination_group.add(&library_choices);
    let path_row = adw::ActionRow::new();
    path_row.set_widget_name("install-destination");
    path_row.set_title("Installation folder");
    path_row.set_use_markup(false);
    path_row.set_subtitle_selectable(true);
    destination_group.add(&path_row);
    body.add(&destination_group);
    let update_destination = {
        let libraries = config.game_libraries.clone();
        let existing = existing_installation
            .as_ref()
            .map(|installed| installed.installation_directory.clone());
        let slug = detail.slug.clone();
        move |library: &gtk::DropDown| {
            path_row.set_subtitle(
                &existing
                    .clone()
                    .or_else(|| {
                        libraries
                            .get(library.selected() as usize)
                            .map(|library| library.path.join(&slug))
                    })
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "No game library configured".into()),
            );
        }
    };
    update_destination(&library);
    library.connect_selected_notify(update_destination);
    update_install_plan_preview(
        &installer_detail,
        &candidates.usable,
        candidate.selected() as usize,
    );
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&body)
        .build();
    scroll.set_widget_name("install-choices-scroll");
    root.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    footer.add_css_class("download-selector-footer");
    interactive_prompts.set_title("Interactive install");
    interactive_prompts.set_subtitle("");
    interactive_prompts.set_hexpand(false);
    interactive_prompts.add_css_class("install-footer-switch");
    interactive_prompts.set_activatable(true);
    footer.append(&interactive_prompts);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    let status = gtk::Label::new(None);
    status.set_widget_name("install-status");
    let galaxy_status = gtk::Label::new(None);
    galaxy_status.set_widget_name("install-galaxy-status");
    let statuses = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for label in [&status, &galaxy_status] {
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_selectable(true);
        statuses.append(label);
    }
    let status_scroll = gtk::ScrolledWindow::builder()
        .child(&statuses)
        .max_content_height(160)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    status_scroll.set_widget_name("install-status-scroll");
    let refresh_status: Rc<dyn Fn()> = Rc::new({
        let status_scroll = status_scroll.downgrade();
        let status = status.downgrade();
        let galaxy_status = galaxy_status.downgrade();
        let galaxy_selected = galaxy_selected.clone();
        move || {
            let (Some(status_scroll), Some(status), Some(galaxy_status)) = (
                status_scroll.upgrade(),
                status.upgrade(),
                galaxy_status.upgrade(),
            ) else {
                return;
            };
            status.set_visible(!status.label().is_empty());
            galaxy_status.set_visible(galaxy_selected.get() && !galaxy_status.label().is_empty());
            status_scroll.set_visible(status.get_visible() || galaxy_status.get_visible());
        }
    });
    for label in [&status, &galaxy_status] {
        let refresh = refresh_status.clone();
        label.connect_label_notify(move |_| refresh());
    }
    source.connect_selected_notify({
        let refresh = refresh_status.clone();
        move |_| refresh()
    });
    refresh_status();
    root.append(&status_scroll);
    let close = gtk::Button::with_label("Cancel");
    close.set_widget_name("install-cancel");
    footer.append(&close);
    let install = gtk::Button::new();
    install.set_widget_name("install-confirm");
    let queue_spinner = gtk::Spinner::new();
    queue_spinner.set_visible(false);
    footer.append(&queue_spinner);
    install.add_css_class("suggested-action");
    install.set_sensitive(
        !config.game_libraries.is_empty()
            && ((galaxy_selected.get()
                && galaxy_ready.get()
                && existing_installation.is_none()
                && !repair)
                || (!candidates.usable.is_empty() && (repair || existing_installation.is_none()))
                || (existing_installation.is_some()
                    && dlc_choices.iter().any(|choice| choice.check.is_sensitive()))),
    );
    update_install_action(
        &install,
        &candidates.usable,
        candidate.selected() as usize,
        existing_installation.is_some() && !repair,
        &dlc_choices,
    );
    if galaxy_selected.get() {
        install.set_sensitive(galaxy_ready.get() && !config.game_libraries.is_empty());
        install.set_label("Download and install");
    }
    if config.game_libraries.is_empty() {
        install.set_sensitive(false);
    }
    {
        let install = install.clone();
        let galaxy_selected = galaxy_selected.clone();
        let galaxy_ready = galaxy_ready.clone();
        let has_library = !config.game_libraries.is_empty();
        source.connect_selected_notify(move |_| {
            if galaxy_selected.get() {
                install.set_sensitive(galaxy_ready.get() && has_library);
                install.set_label("Download and install");
            }
        });
    }
    let target_guard = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    target_guard.append(&install);
    footer.append(&target_guard);
    let usable_targets = config
        .game_libraries
        .iter()
        .map(|library| {
            library_statuses.iter().any(|status| {
                status.kind == LibraryKind::GameFiles
                    && status.library_id == library.id
                    && matches!(
                        status.compatibility,
                        crate::storage::LibraryCompatibility::Compatible
                    )
            })
        })
        .collect::<Vec<_>>();
    target_guard.set_sensitive(
        usable_targets
            .get(library.selected() as usize)
            .copied()
            .unwrap_or(false),
    );
    library.connect_selected_notify(move |library| {
        target_guard.set_sensitive(
            usable_targets
                .get(library.selected() as usize)
                .copied()
                .unwrap_or(false),
        )
    });
    for choice in &dlc_choices {
        let check = &choice.check;
        let install = install.clone();
        let candidates = candidates.usable.clone();
        let candidate = candidate.clone();
        let dlc_choices = dlc_choices.clone();
        let base_installed = existing_installation.is_some() && !repair;
        let galaxy_selected = galaxy_selected.clone();
        let galaxy_ready = galaxy_ready.clone();
        let has_library = !config.game_libraries.is_empty();
        check.connect_toggled(move |_| {
            update_install_action(
                &install,
                &candidates,
                candidate.selected() as usize,
                base_installed,
                &dlc_choices,
            );
            if galaxy_selected.get() {
                install.set_sensitive(galaxy_ready.get() && has_library);
                install.set_label("Download and install");
            }
        });
    }
    root.append(&footer);
    if let Some((game, token, language, session, epoch)) = galaxy_request {
        let row = adw::ActionRow::new();
        row.set_title("Galaxy builds");
        let spinner = gtk::Spinner::new();
        row.add_suffix(&spinner);
        let retry = gtk::Button::with_label("Retry");
        row.add_suffix(&retry);
        galaxy_feedback.append(&row);
        let closed = Rc::new(std::cell::Cell::new(false));
        dialog.connect_closed({
            let closed = closed.clone();
            move |_| closed.set(true)
        });
        let model = model.clone();
        let builds = galaxy_builds.clone();
        let branches = branches.clone();
        let branch = branch.clone();
        let branch_row = branch_row.clone();
        let branch_password = branch_password.clone();
        let galaxy_selected = galaxy_selected.clone();
        let galaxy_ready = galaxy_ready.clone();
        let install = install.clone();
        let buttons = source_choice_buttons.clone();
        let values = source_values.clone();
        let icon = detail.icon.clone();
        let title = detail.title.clone();
        let has_library = !config.game_libraries.is_empty();
        let galaxy_preflight_label = galaxy_preflight_label.clone();
        retry.connect_clicked(move |retry| {
            if closed.get() || model.borrow().account_epoch!=epoch {return;}
            retry.set_sensitive(false);spinner.set_spinning(true);row.set_subtitle("Checking Galaxy installation data…");
            let game=game.clone();let token=token.clone();let language=language.clone();
            let (sender,receiver)=mpsc::channel();
            std::thread::spawn(move || {let _=sender.send(online::fetch_product_section(&game,online::DetailSection::Builds,Some(&token),language.as_deref(),session).map_err(|error|error.to_string()));});
            let model=model.clone();let builds=builds.clone();let branches=branches.clone();let closed=closed.clone();
            let spinner=spinner.clone();let row=row.clone();let retry=retry.clone();let branch=branch.clone();let branch_row=branch_row.clone();let branch_password=branch_password.clone();
            let galaxy_selected=galaxy_selected.clone();let galaxy_ready=galaxy_ready.clone();let install=install.clone();let buttons=buttons.clone();let values=values.clone();let icon=icon.clone();let title=title.clone();let galaxy_preflight_label=galaxy_preflight_label.clone();
            glib::timeout_add_local(Duration::from_millis(32),move || {
                if closed.get() || model.borrow().account_epoch!=epoch {return glib::ControlFlow::Break;}
                let result=match receiver.try_recv(){Ok(result)=>result,Err(mpsc::TryRecvError::Empty)=>return glib::ControlFlow::Continue,Err(_)=>Err("Galaxy metadata loading stopped".into())};
                spinner.set_spinning(false);retry.set_sensitive(true);
                match result {
                    Ok(game)=>{
                        galaxy_preflight_label.set_visible(false);
                        let id=game.product_id;
                        let fresh=game.galaxy_builds.iter().filter(|build|build.generation==2 && build.currently_returned && build.operating_system.eq_ignore_ascii_case("windows")).cloned().collect::<Vec<_>>();
                        if let Some(current)=model.borrow_mut().games.iter_mut().find(|current|current.product_id==id){online::apply_product_section(current,game,online::DetailSection::Builds);}
                        let selected=branches.borrow().get(branch.selected() as usize).cloned();
                        let mut fresh_branches=fresh.iter().map(|build|build.branch.clone()).collect::<Vec<_>>();fresh_branches.sort();fresh_branches.dedup();
                        let selected=selected.and_then(|selected|fresh_branches.iter().position(|value|*value==selected)).unwrap_or(0);
                        branch.set_model(Some(&gtk::StringList::new(&fresh_branches.iter().map(|value|value.as_deref().unwrap_or("Master")).collect::<Vec<_>>())));
                        *branches.borrow_mut()=fresh_branches;branch.set_selected(selected as u32);
                        *builds.borrow_mut()=fresh;galaxy_ready.set(!builds.borrow().is_empty());
                        branch_row.set_visible(galaxy_selected.get() && branches.borrow().len()>1);
                        branch_password.set_visible(galaxy_selected.get() && branches.borrow().get(selected).is_some_and(Option::is_some));
                        for (index,(_,value)) in values.iter().enumerate(){if matches!(value,InstallSource::GalaxyWindows) && let Some(button)=buttons.get(index){button.set_sensitive(galaxy_ready.get());button.set_child(Some(&install_choice_content(icon.as_deref(),&title,"Windows · Galaxy build",0,false,false)));}}
                        if galaxy_selected.get(){install.set_sensitive(galaxy_ready.get() && has_library);}
                        row.set_subtitle(if galaxy_ready.get(){"Galaxy installation choices are ready"}else{"No current Galaxy build is available"});
                        retry.set_visible(!galaxy_ready.get());
                    }
                    Err(error)=>row.set_subtitle(&format!("Could not load Galaxy choices: {error}. Downloaded installers remain available.")),
                }
                glib::ControlFlow::Break
            });
        });
        retry.emit_clicked();
    }
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let window = window.clone();
        storage_settings.connect_clicked(move |_| {
            dialog.close();
            if let Err(error) = gtk::prelude::WidgetExt::activate_action(
                &window,
                "win.settings-page",
                Some(&"storage".to_variant()),
            ) {
                tracing::warn!(%error, "could not open Storage settings");
            }
        });
    }
    {
        let detail_row = installer_detail.clone();
        let candidates = candidates.usable.clone();
        let install = install.clone();
        let candidate_menu = candidate_menu.clone();
        let candidate_labels = candidate_labels.clone();
        let candidate_choice_buttons = candidate_choice_buttons.clone();
        let icon = detail.icon.clone();
        let title = detail.title.clone();
        let existing_installation = existing_installation.is_some() && !repair;
        let dlc_choices = dlc_choices.clone();
        let dlc_summary = dlc_summary.clone();
        let offline_libraries = config.offline_libraries.clone();
        candidate.connect_selected_notify(move |candidate| {
            let selected = candidate.selected() as usize;
            let base_version = candidates
                .get(selected)
                .and_then(|installer| installer.version.as_deref());
            for choice in &dlc_choices {
                let matching = choice
                    .installers
                    .iter()
                    .find(|installer| versions_match(installer.version.as_deref(), base_version)
                        && candidates.get(selected).is_none_or(|base| offline_libraries.iter().any(|library|
                            base.paths.iter().chain(&installer.paths).all(|path| path.starts_with(&library.path)))))
                    .cloned();
                *choice.selected.borrow_mut() = matching.clone();
                choice.check.set_active(matching.is_some());
                choice
                    .check
                    .set_sensitive(matching.is_some() && (repair || !choice.already_installed));
                choice.check.set_tooltip_text(if matching.is_none() {
                    Some("A complete DLC installer matching the selected game version is not downloaded")
                } else {
                    None
                });
            }
            update_dlc_summary(&dlc_summary, &dlc_choices, repair);
            update_install_plan_preview(&detail_row, &candidates, selected);
            update_install_action(
                &install,
                &candidates,
                selected,
                existing_installation,
                &dlc_choices,
            );
            if let Some(installer) = candidates.get(selected) {
                candidate_menu.set_child(Some(&install_choice_content(
                    icon.as_deref(),
                    &title,
                    &candidate_labels[selected],
                    installer.total_size,
                    false,
                    true,
                )));
            }
            for (index, button) in candidate_choice_buttons.iter().enumerate() {
                if let Some(installer) = candidates.get(index) {
                    button.set_child(Some(&install_choice_content(
                        icon.as_deref(),
                        &title,
                        &candidate_labels[index],
                        installer.total_size,
                        index == selected,
                        false,
                    )));
                }
            }
        });
    }
    let refresh_source: Rc<dyn Fn()> = Rc::new({
        let source = source.clone();
        let values = source_values.clone();
        let archive_library = archive_library.clone();
        let archive_group = archive_group.clone();
        let archive_path = archive_path.clone();
        let libraries = config.offline_libraries.clone();
        let statuses = library_statuses.clone();
        let install = install.clone();
        let galaxy_ready = galaxy_ready.clone();
        let galaxy_feedback = galaxy_feedback.clone();
        let interactive = interactive_prompts.clone();
        let dlc_menu = dlc_menu.clone();
        let candidates = candidates.usable.clone();
        let candidate = candidate.clone();
        let dlcs = dlc_choices.clone();
        let has_target = !config.game_libraries.is_empty();
        let base_installed = existing_installation.is_some() && !repair;
        let signed_in = model.borrow().account_token.is_some();
        move || {
            let selected = values
                .get(source.selected() as usize)
                .map(|(_, source)| *source);
            let remote = matches!(selected, Some(InstallSource::RemoteOffline(_)));
            galaxy_feedback.set_visible(matches!(selected, Some(InstallSource::GalaxyWindows)));
            archive_group.set_visible(remote);
            dlc_menu.set_visible(!remote && !dlcs.is_empty());
            interactive
                .set_visible(!remote && !matches!(selected, Some(InstallSource::GalaxyWindows)));
            if remote {
                let library = libraries.get(archive_library.selected() as usize);
                let usable = library.is_some_and(|library| {
                    statuses.iter().any(|status| {
                        status.kind == LibraryKind::OfflineInstallers
                            && status.library_id == library.id
                            && matches!(
                                status.compatibility,
                                crate::storage::LibraryCompatibility::Compatible
                            )
                    })
                });
                archive_path.set_label(&match library {
                    Some(library) if usable => library.path.display().to_string(),
                    Some(library) => format!("{} is unavailable or incompatible. Choose another library or correct it in Storage settings.", library.path.display()),
                    None => "Configure an Offline Installers library in Storage settings before downloading an installer.".into(),
                });
                install.set_label("Download and install");
                install.set_sensitive(usable && has_target && signed_in);
            } else if matches!(selected, Some(InstallSource::GalaxyWindows)) {
                install.set_label("Download and install");
                install.set_sensitive(galaxy_ready.get() && has_target);
            } else {
                update_install_action(
                    &install,
                    &candidates,
                    candidate.selected() as usize,
                    base_installed,
                    &dlcs,
                );
                if !has_target {
                    install.set_sensitive(false);
                }
            }
        }
    });
    for selector in [&source, &archive_library] {
        let refresh = refresh_source.clone();
        selector.connect_selected_notify(move |_| refresh());
    }
    refresh_source();
    {
        let candidates = candidates.usable;
        let dlc_choices = dlc_choices.clone();
        let existing_installation = existing_installation.clone();
        let libraries = config.game_libraries.clone();
        let product_id = detail.product_id;
        let slug = detail.slug.clone();
        let candidate = candidate.clone();
        let library = library.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        let interactive_prompts = interactive_prompts.clone();
        let galaxy_selected = galaxy_selected.clone();
        let galaxy_builds = galaxy_builds.clone();
        let branches = branches.clone();
        let branch = branch.clone();
        let language = galaxy_selection.language;
        let language_detail = detail.clone();
        let owned_dlc = detail
            .dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| dlc.product_id)
            .collect::<BTreeSet<_>>();
        let branch_password = branch_password.clone();
        let action_epoch = model.borrow().account_epoch;
        let action_model = model.clone();
        let dependency_window = window.clone();
        let preparation_closed = Rc::new(std::cell::Cell::new(false));
        dialog.connect_closed({
            let closed = preparation_closed.clone();
            move |_| closed.set(true)
        });
        let preparing = Rc::new(std::cell::Cell::new(false));
        let preparation_button = install.clone();
        let remote_sources = source_values.clone();
        let selected_source = source.clone();
        let windows_product = {
            let model = action_model.clone();
            let galaxy_selected = galaxy_selected.clone();
            let candidates = candidates.clone();
            let candidate = candidate.clone();
            let existing = existing_installation.clone();
            let source = source.clone();
            let sources = source_values.clone();
            move || {
                if model.borrow().account_epoch != action_epoch {
                    return None;
                }
                if sources
                    .get(source.selected() as usize)
                    .is_some_and(|(_, source)| matches!(source, InstallSource::RemoteOffline(_)))
                {
                    return None;
                }
                (galaxy_selected.get()
                    || if let Some(game) = existing.as_ref().filter(|_| !repair) {
                        game.installer_operating_system.as_deref() != Some("linux")
                    } else {
                        candidates
                            .get(candidate.selected() as usize)
                            .is_some_and(|candidate| {
                                candidate.method
                                    == crate::installation::InstallationMethod::WindowsCompatibility
                            })
                    })
                .then_some(product_id)
            }
        };
        let archive_libraries = config.offline_libraries.clone();
        let title = detail.title.clone();
        let action_session = (online::account_session(), auth::session());
        let operation_controls = body.clone();
        connect_windows_action(&install, window, true, windows_product, move |button| {
            if action_model.borrow().account_epoch != action_epoch {
                status.set_label("Account changed; reopen installation choices.");
                status.add_css_class("error");
                return;
            }
            if let Some((_, InstallSource::RemoteOffline(index))) =
                remote_sources.get(selected_source.selected() as usize)
            {
                if preparing.replace(true) {
                    return;
                }
                let Some(archive) = archive_libraries
                    .get(archive_library.selected() as usize)
                    .cloned()
                else {
                    preparing.set(false);
                    return;
                };
                let Some(target) = libraries.get(library.selected() as usize).cloned() else {
                    preparing.set(false);
                    return;
                };
                let Some(token) = action_model
                    .borrow()
                    .account_token
                    .as_ref()
                    .map(|token| token.access_token.clone())
                else {
                    preparing.set(false);
                    status.set_label("Sign in before downloading this installer.");
                    return;
                };
                let group = remote_installers[*index].clone();
                let slug = slug.clone();
                let title = title.clone();
                let mut config = action_model.borrow().config.clone();
                config.interactive_installer_prompts = false;
                status.set_label("Queuing every required installer part; installation will follow the completed download…");
                button.set_sensitive(false);
                operation_controls.set_sensitive(false);
                queue_spinner.set_visible(true);
                queue_spinner.start();
                dialog.set_can_close(false);
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<usize> {
                        let _activity =
                            crate::profile_reset::begin_activity("queuing offline installation")?;
                        anyhow::ensure!(
                            action_session == (online::account_session(), auth::session()),
                            "The account changed. Reopen installation choices."
                        );
                        let current = crate::storage::read_config()?;
                        for (kind, selected) in [
                            (LibraryKind::OfflineInstallers, &archive),
                            (LibraryKind::GameFiles, &target),
                        ] {
                            let fresh =
                                crate::storage::validate_library(&current, kind, &selected.id)?;
                            anyhow::ensure!(
                                fresh.path == selected.path,
                                "A selected library changed. Reopen installation choices."
                            );
                        }
                        let destination = download::destination(
                            &archive.path,
                            &slug,
                            None,
                            &group.artifacts.iter().collect::<Vec<_>>(),
                        );
                        let (events, _) = mpsc::channel();
                        let requests = vec![download::DownloadRequest {
                            artifacts: group.artifacts,
                            title: title.clone(),
                            access_token: token,
                            destination,
                            library_id: archive.id,
                            events,
                        }];
                        let install = Some(download::AutoInstallRequest {
                            product_id,
                            slug,
                            title,
                            config,
                            library_id: target.id,
                        });
                        #[cfg(test)]
                        let (requests, install) = match capture_queued_downloads(requests, install)
                        {
                            Ok(count) => return Ok(count),
                            Err(request) => *request,
                        };
                        download::enqueue_with_install(requests, install, action_session.0)
                    })();
                    let _ = sender.send(result);
                });
                let dialog = dialog.clone();
                let status = status.clone();
                let button = button.downgrade();
                let model = action_model.clone();
                let preparing = preparing.clone();
                let operation_controls = operation_controls.clone();
                let queue_spinner = queue_spinner.clone();
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    if model.borrow().account_epoch != action_epoch
                        || model.borrow().logout_pending
                        || action_session != (online::account_session(), auth::session())
                    {
                        dialog.set_can_close(true);
                        dialog.close();
                        return glib::ControlFlow::Break;
                    }
                    let result = match receiver.try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(_) => Err(anyhow::anyhow!("Queue preparation stopped. Try again.")),
                    };
                    dialog.set_can_close(true);
                    preparing.set(false);
                    operation_controls.set_sensitive(true);
                    queue_spinner.stop();
                    queue_spinner.set_visible(false);
                    match result {
                        Ok(_) => dialog.close(),
                        Err(error) => {
                            status.set_label(&notifications::failure_message(
                                "Could not queue installation",
                                &format!("{error:#}"),
                            ));
                            if let Some(button) = button.upgrade() {
                                button.set_sensitive(true);
                            }
                            false
                        }
                    };
                    glib::ControlFlow::Break
                });
                return;
            }
            if galaxy_selected.get() {
                let status = galaxy_status.clone();
                if preparing.get() {
                    return;
                }
                let authentication = match current_depot_session(&action_model) {
                    Ok(authentication) => authentication,
                    Err(error) => {
                        status.set_label(&error.to_string());
                        status.add_css_class("error");
                        return;
                    }
                };
                let selected_branch = branches
                    .borrow()
                    .get(branch.selected() as usize)
                    .cloned()
                    .flatten();
                let Some(mut build) = galaxy_builds
                    .borrow()
                    .iter()
                    .filter(|build| build.branch == selected_branch)
                    .max_by_key(|build| build.published_at)
                    .cloned()
                else {
                    status.set_label("The selected Galaxy branch has no available build");
                    status.add_css_class("error");
                    return;
                };
                let Some(library) = libraries.get(library.selected() as usize).cloned() else {
                    return;
                };
                let selected_dlc = dlc_choices
                    .iter()
                    .filter(|choice| choice.check.is_active())
                    .map(|choice| choice.product_id)
                    .collect();
                let request = crate::gog::depot_service::PrepareOperationRequest {
                    build: build.clone(),
                    selection: crate::gog::depot_acquisition::Selection {
                        language: language.clone(),
                        bitness: Some("64".into()),
                        owned_dlc: owned_dlc.clone(),
                        selected_dlc,
                    },
                    operation_id: format!(
                        "{}-{}",
                        product_id,
                        chrono::Utc::now().timestamp_millis()
                    ),
                    kind: crate::domain::DepotOperationKind::Install,
                    library_id: library.id,
                    library_root: library.path,
                    slug: slug.clone(),
                };
                status.remove_css_class("error");
                status.set_label("Preparing Galaxy installation…");
                preparing.set(true);
                preparation_button.set_sensitive(false);
                let preparation_pending = preparing.clone();
                let preparation_button_result = preparation_button.clone();
                let session = crate::online::account_session();
                let (sender, receiver) = mpsc::channel();
                let password = (!branch_password.text().is_empty()).then(|| {
                    crate::gog::depot_service::BranchPassword::new(
                        branch_password.text().to_string(),
                    )
                });
                let build_request = crate::gog::depot_service::BuildRequest {
                    user_id: authentication.token.user_id.clone(),
                    product_id,
                    platform: "windows".into(),
                    generation: 2,
                    branch: selected_branch.clone(),
                    supplied_password: password,
                };
                let language_detail = language_detail.clone();
                let preferences = super::update_policies::policy_request(move || {
                    online::with_account_session(session, || {
                        StateStore::open()?.game_preferences(product_id)
                    })
                });
                std::thread::spawn(move || {
                    let result = StateStore::open().and_then(|store| {
                        authentication.validate()?;
                        let preferences = preferences.recv().map_err(|_| {
                            anyhow::anyhow!("Game language preferences stopped loading")
                        })??;
                        let client = reqwest::blocking::Client::new();
                        if selected_branch.is_some() {
                            let builds = crate::gog::depot_service::list_builds(
                                &store,
                                &client,
                                &authentication,
                                &build_request,
                            )?;
                            build = builds
                                .into_iter()
                                .filter(|candidate| candidate.branch == selected_branch)
                                .max_by_key(|candidate| candidate.published_at)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("selected Galaxy branch is unavailable")
                                })?;
                        }
                        let mut request = request;
                        request.build = build;
                        request.selection.language = default_galaxy_selection(
                            &language_detail,
                            &crate::storage::read_config()?,
                            preferences.as_ref(),
                        )
                        .language;
                        let operation = crate::gog::depot_service::prepare_operation(
                            &store,
                            &client,
                            &authentication,
                            request,
                        )?;
                        anyhow::ensure!(
                            crate::online::account_session() == session,
                            "Account changed during preparation"
                        );
                        Ok((operation, authentication))
                    });
                    let _ = sender.send(result);
                });
                let status_result = status.clone();
                let dialog_result = dialog.clone();
                let plan_window = dependency_window.clone();
                let plan_model = action_model.clone();
                let preparation_closed = preparation_closed.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if plan_model.borrow().account_epoch != action_epoch || preparation_closed.get()
                    {
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(Ok((request, authentication))) => {
                            confirm_depot_plan(
                                &plan_window,
                                &plan_model,
                                request,
                                authentication,
                                &dialog_result,
                            );
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            preparation_pending.set(false);
                            preparation_button_result.set_sensitive(true);
                            status_result.set_label(&super::notifications::failure_message("", &format!(
                                "Could not prepare required Depot components: {error:#}\nRetry preparation or select an offline installer from the source menu."
                            )));
                            status_result.add_css_class("error");
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            preparation_pending.set(false);
                            preparation_button_result.set_sensitive(true);
                            status_result.set_label("Preparation stopped. Retry or select an offline installer from the source menu.");
                            glib::ControlFlow::Break
                        }
                    }
                });
                return;
            }
            let candidate = candidates.get(candidate.selected() as usize);
            if existing_installation.is_none() && candidate.is_none() {
                return;
            }
            let Some(library) = libraries.get(library.selected() as usize) else {
                return;
            };
            let plan = if let Some(mut installed) = existing_installation.clone() {
                if repair {
                    let Some(candidate) = candidate else { return };
                    installed.installed_version = candidate.version.clone();
                    installed.installer_revision_id = candidate.revision_id;
                    installed.installer_files = candidate.paths.clone();
                    installed.installer_complete = candidate.complete;
                    installed.installer_operating_system = candidate.operating_system.clone();
                    installed.installer_language = candidate.language.clone();
                }
                installed
            } else {
                let candidate = candidate.expect("a new installation requires an installer");
                offline_installation_plan(product_id, library, &slug, candidate)
            };
            {
                status.remove_css_class("error");
                let windows = candidate.is_some_and(|candidate| {
                    candidate.method
                        == crate::installation::InstallationMethod::WindowsCompatibility
                });
                status.set_label(if windows {
                    "Preparing UMU compatibility environment…"
                } else {
                    "Starting Linux installer…"
                });
                status.remove_css_class("success");
                let additional_installers = dlc_choices
                    .iter()
                    .filter(|choice| {
                        choice.check.is_active() && (repair || choice.check.is_sensitive())
                    })
                    .filter_map(|choice| {
                        let installer = choice.selected.borrow().clone()?;
                        versions_match(
                            installer.version.as_deref(),
                            candidate
                                .and_then(|base| base.version.as_deref())
                                .or(plan.installed_version.as_deref()),
                        )
                        .then_some((choice, installer))
                    })
                    .map(
                        |(choice, installer)| crate::installation::AdditionalInstaller {
                            product_id: choice.product_id,
                            revision_id: installer.revision_id,
                            version: installer.version.clone(),
                            title: choice.title.clone(),
                            files: installer.paths.clone(),
                        },
                    )
                    .collect::<Vec<_>>();
                let install_base = repair || existing_installation.is_none();
                let interactive = interactive_prompts.is_active();
                let session = action_session;
                let receiver = match prepare_cached_offline_installation(
                    plan,
                    existing_installation.is_none(),
                    session,
                    move |plan| {
                        crate::installation::enqueue_installation_tracked(
                            plan, additional_installers, install_base, interactive,
                        ).ok_or_else(|| anyhow::anyhow!("An installation operation is already active, or setup could not be saved"))
                    },
                ) {
                    Ok(receiver) => receiver,
                    Err(error) => {
                        status.set_label(&format!("Could not start setup: {error:#}"));
                        status.add_css_class("error");
                        return;
                    }
                };
                button.set_sensitive(false);
                dialog.set_can_close(false);
                let model = action_model.clone();
                let dialog = dialog.clone();
                let status = status.clone();
                let button = button.downgrade();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    if model.borrow().account_epoch != action_epoch
                        || model.borrow().logout_pending
                        || session != (online::account_session(), auth::session())
                    {
                        dialog.set_can_close(true);
                        dialog.close();
                        return glib::ControlFlow::Break;
                    }
                    let result = match receiver.try_recv() {
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        result => result,
                    };
                    dialog.set_can_close(true);
                    match result {
                        Ok(Ok(tracked)) => {
                            monitor_setup(&dialog, &model, SetupOperation::Offline(tracked))
                        }
                        result => {
                            status.set_label(&match result {
                                Ok(Err(error)) => format!("Could not start setup: {error:#}"),
                                _ => "Setup worker stopped; retry installation.".into(),
                            });
                            status.add_css_class("error");
                            if let Some(button) = button.upgrade() {
                                button.set_sensitive(true);
                            }
                        }
                    }
                    glib::ControlFlow::Break
                });
            }
        });
    }
    dialog.set_child(Some(&root));
}

pub(super) fn present_installer_prompt(
    window: &adw::ApplicationWindow,
    product_id: i64,
    prompt: &str,
    choices: &[String],
    context: &str,
) {
    let dialog = adw::Dialog::builder()
        .content_width(760)
        .content_height(480)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Installer needs your response",
        prompt,
    )));
    root.append(&header);
    let prompt_details = gtk::Box::new(gtk::Orientation::Vertical, 8);
    prompt_details.set_margin_start(20);
    prompt_details.set_margin_end(20);
    prompt_details.set_vexpand(true);
    let context_heading = gtk::Label::new(Some("Recent installer output"));
    context_heading.set_xalign(0.0);
    context_heading.add_css_class("heading");
    prompt_details.append(&context_heading);
    let context_label = gtk::Label::new(Some(context));
    context_label.set_xalign(0.0);
    context_label.set_wrap(true);
    context_label.set_selectable(true);
    context_label.add_css_class("monospace");
    context_label.add_css_class("installer-prompt-context");
    let context_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(220)
        .child(&context_label)
        .build();
    prompt_details.append(&context_scroll);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Type the response to send to the installer"));
    let actions = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .margin_start(20)
        .margin_end(20)
        .margin_bottom(16)
        .build();
    let cancel = gtk::Button::with_label("Cancel Installation");
    cancel.add_css_class("destructive-action");
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        crate::installation::cancel_operation(product_id);
        dialog_for_cancel.close();
    });
    if choices.is_empty() {
        prompt_details.append(&entry);
        let send = gtk::Button::with_label("Send");
        send.add_css_class("suggested-action");
        let dialog_for_send = dialog.clone();
        let entry_for_send = entry.clone();
        send.connect_clicked(move |_| {
            crate::installation::respond_to_installation(
                product_id,
                entry_for_send.text().to_string(),
            );
            dialog_for_send.close();
        });
        actions.attach(&cancel, 0, 0, 1, 1);
        actions.attach(&send, 1, 0, 1, 1);
    } else {
        for (index, choice) in choices.iter().enumerate() {
            let response = gtk::Button::with_label(choice);
            response.set_hexpand(true);
            let dialog = dialog.clone();
            let choice = choice.clone();
            response.connect_clicked(move |_| {
                crate::installation::respond_to_installation(product_id, choice.clone());
                dialog.close();
            });
            actions.attach(&response, (index % 2) as i32, (index / 2) as i32, 1, 1);
        }
        actions.attach(&cancel, 0, choices.len().div_ceil(2) as i32, 2, 1);
    }
    root.append(&prompt_details);
    root.append(&actions);
    dialog.set_child(Some(&root));
    dialog.present(Some(window));
}

#[derive(Clone)]
struct DlcInstallerChoice {
    product_id: i64,
    title: String,
    installers: Vec<crate::installation::InstallerCandidate>,
    selected: Rc<RefCell<Option<crate::installation::InstallerCandidate>>>,
    check: gtk::CheckButton,
    already_installed: bool,
}

fn update_dlc_summary(summary: &gtk::Label, choices: &[DlcInstallerChoice], repair: bool) {
    let selected = choices
        .iter()
        .filter(|choice| choice.check.is_active() && choice.check.is_sensitive())
        .count();
    let unavailable = choices
        .iter()
        .filter(|choice| !choice.check.is_sensitive())
        .count();
    summary.set_label(&dlc_summary_text(selected, unavailable, repair));
}

fn dlc_summary_text(selected: usize, unavailable: usize, repair: bool) -> String {
    let suffix = if unavailable == 0 {
        String::new()
    } else if repair {
        format!(" · {unavailable} unavailable")
    } else {
        format!(" · {unavailable} installed")
    };
    format!("DLC · {selected} selected{suffix}")
}

fn update_install_action(
    button: &gtk::Button,
    candidates: &[crate::installation::InstallerCandidate],
    selected: usize,
    base_installed: bool,
    dlc_choices: &[DlcInstallerChoice],
) {
    let selected_dlc = dlc_choices
        .iter()
        .filter(|choice| choice.check.is_active() && choice.check.is_sensitive())
        .collect::<Vec<_>>();
    let supported = if base_installed {
        !selected_dlc.is_empty()
            && selected_dlc.iter().all(|choice| {
                choice.selected.borrow().as_ref().is_some_and(|installer| {
                    installer.method != crate::installation::InstallationMethod::Unsupported
                })
            })
    } else {
        candidates.get(selected).is_some_and(|candidate| {
            candidate.method != crate::installation::InstallationMethod::Unsupported
        }) && selected_dlc.iter().all(|choice| {
            choice.selected.borrow().as_ref().is_some_and(|installer| {
                installer.method != crate::installation::InstallationMethod::Unsupported
            })
        })
    };
    button.set_label("Install");
    button.set_sensitive(supported);
    button.set_tooltip_text(Some(if supported {
        "Start installation"
    } else {
        "The selected installer is unsupported"
    }));
}

fn installer_matches_installed_game(
    candidate: &crate::installation::InstallerCandidate,
    installed: &crate::domain::InstalledGame,
) -> bool {
    let operating_system_matches =
        installed
            .installer_operating_system
            .as_deref()
            .is_none_or(|installed_os| {
                candidate
                    .operating_system
                    .as_deref()
                    .is_some_and(|candidate_os| candidate_os.eq_ignore_ascii_case(installed_os))
            });
    let language_matches =
        installed
            .installer_language
            .as_deref()
            .is_none_or(|installed_language| {
                candidate
                    .language
                    .as_deref()
                    .is_some_and(|candidate_language| {
                        candidate_language.eq_ignore_ascii_case(installed_language)
                    })
            });
    operating_system_matches && language_matches
}

fn versions_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.trim().eq_ignore_ascii_case(right.trim()),
        (None, None) => true,
        _ => false,
    }
}

fn install_choice_content(
    icon: Option<&std::path::Path>,
    title: &str,
    choice: &str,
    size: u64,
    selected: bool,
    dropdown: bool,
) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let artwork = gtk::Picture::new();
    artwork.set_width_request(52);
    artwork.set_height_request(52);
    artwork.set_content_fit(gtk::ContentFit::Cover);
    if let Some(icon) = icon {
        widgets::media::set_card_picture(&artwork, icon, 52, 52);
    }
    content.append(&artwork);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("file-name");
    labels.append(&title);
    let choice = gtk::Label::new(Some(choice));
    choice.set_xalign(0.0);
    choice.add_css_class("dim-label");
    labels.append(&choice);
    content.append(&labels);
    let size_label = gtk::Label::new(Some(&human_size(size)));
    size_label.add_css_class("install-choice-size");
    size_label.set_visible(size > 0);
    content.append(&size_label);
    if selected || dropdown {
        let indicator = gtk::Image::from_icon_name(if selected {
            "object-select-symbolic"
        } else {
            "pan-down-symbolic"
        });
        content.append(&indicator);
    }
    content
}

fn update_install_library_free_space(path: &std::path::Path, label: &gtk::Label) {
    let path = path.to_owned();
    let label = label.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(install_library_free_space(&path).ok());
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(Some(free)) => {
                label.set_label(&format!("{} free", human_size(free)));
                glib::ControlFlow::Break
            }
            Ok(None) => {
                label.set_label("Unavailable");
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                label.set_label("Unavailable");
                glib::ControlFlow::Break
            }
        }
    });
}

fn install_library_free_space(path: &std::path::Path) -> std::io::Result<u64> {
    if !std::fs::symlink_metadata(path)?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "The library path is not a real directory",
        ));
    }
    fs2::available_space(path)
}

fn update_install_plan_preview(
    method_row: &adw::ActionRow,
    candidates: &[crate::installation::InstallerCandidate],
    candidate_index: usize,
) {
    if let Some(candidate) = candidates.get(candidate_index) {
        let (title, method_description) = match candidate.method {
            crate::installation::InstallationMethod::WindowsCompatibility => (
                "Windows through UMU",
                "A dedicated compatibility environment will use L: for the selected library",
            ),
            crate::installation::InstallationMethod::NativeLinux => (
                "Native Linux installer",
                "The installer will target the selected host library directly",
            ),
            crate::installation::InstallationMethod::Unsupported => (
                "Unsupported on Arch Linux",
                "This installer cannot be executed on the current system",
            ),
        };
        let completeness = if candidate.complete {
            ""
        } else {
            "; missing parts may be reported when the installer runs"
        };
        method_row.set_subtitle(&format!(
            "{method_description} · {} local file{} selected{completeness}",
            candidate.paths.len(),
            if candidate.paths.len() == 1 { "" } else { "s" },
        ));
        method_row.set_title(title);
    }
}

pub(super) fn show_download_selector(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    let name = format!("download-chooser-{}", detail.product_id);
    if let Some(window) = w.window.application().and_then(|app| {
        app.windows()
            .into_iter()
            .find(|window| window.widget_name() == name)
    }) {
        window.present();
        return;
    }
    let dialog = gtk::Window::builder()
        .title(format!("Download {}", detail.title))
        .transient_for(&w.window)
        .modal(true)
        .default_width(920)
        .default_height(720)
        .build();
    if let Some(app) = w.window.application() {
        dialog.set_application(Some(&app));
    }
    dialog.set_widget_name(&name);
    let loading = gtk::Box::new(gtk::Orientation::Vertical, 12);
    loading.set_margin_top(24);
    loading.set_margin_start(24);
    loading.set_margin_end(24);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(16, 16);
    spinner.start();
    let label = gtk::Label::new(Some("Loading available downloads…"));
    label.set_wrap(true);
    row.append(&spinner);
    row.append(&label);
    loading.append(&row);
    let retry = gtk::Button::with_label("Retry");
    retry.set_visible(false);
    loading.append(&retry);
    dialog.set_child(Some(&loading));
    dialog.present();
    if model.borrow().account_token.is_none() {
        spinner.stop();
        spinner.set_visible(false);
        label.set_label("Sign in to load and download this game's offline installers.");
        let sign_in = gtk::Button::with_label("Sign in");
        loading.append(&sign_in);
        let reconnect = w.reconnect.clone();
        sign_in.connect_clicked(move |_| {
            dialog.close();
            reconnect.emit_clicked();
        });
        return;
    }
    let id = detail.product_id;
    let parent = detail.parent_id;
    let request_id = parent.unwrap_or(id);
    let epoch = model.borrow().account_epoch;
    {
        let dialog = dialog.downgrade();
        let model = model.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(dialog) = dialog.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !dialog.is_visible() {
                return glib::ControlFlow::Break;
            }
            if model.borrow().account_epoch != epoch {
                dialog.close();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let w = w.clone();
        let model = model.clone();
        retry.connect_clicked(move |_| {
            request_product_section(
                &w,
                &model,
                request_id,
                online::DetailSection::Acquisition,
                true,
            )
        });
    }
    request_product_section(
        w,
        model,
        request_id,
        online::DetailSection::Acquisition,
        false,
    );
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if !dialog.is_visible() || model.borrow().account_epoch != epoch {
            return glib::ControlFlow::Break;
        }
        let state = model
            .borrow()
            .section_states
            .get(&(request_id, online::DetailSection::Acquisition))
            .cloned();
        match state {
            Some(SectionState::Ready) => {
                let detail = current_detail(&model.borrow(), id, parent);
                if let Some(detail) = detail {
                    populate_download_selector(&w, &model, &detail, &dialog, None);
                }
                glib::ControlFlow::Break
            }
            Some(SectionState::Failed(error)) => {
                spinner.stop();
                label.set_label(&error);
                retry.set_visible(true);
                let detail = current_detail(&model.borrow(), id, parent);
                if let Some(detail) = detail
                    && !detail.remote_artifacts.is_empty()
                {
                    populate_download_selector(
                        &w,
                        &model,
                        &detail,
                        &dialog,
                        Some("Using cached downloads; refresh could not complete."),
                    );
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            }
            _ => {
                spinner.start();
                label.set_label("Loading available downloads…");
                retry.set_visible(false);
                glib::ControlFlow::Continue
            }
        }
    });
}

fn populate_download_selector(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
    dialog: &gtk::Window,
    note: Option<&str>,
) {
    let app = model.borrow();
    let Some(token) = app
        .account_token
        .as_ref()
        .map(|token| token.access_token.clone())
    else {
        let message = gtk::Label::new(Some(
            "Sign in to download this game. Close this window and try again after signing in.",
        ));
        message.set_wrap(true);
        message.set_margin_top(24);
        message.set_margin_start(24);
        message.set_margin_end(24);
        dialog.set_child(Some(&message));
        return;
    };
    let mut products = vec![DownloadDialogProduct {
        product_id: detail.product_id,
        slug: detail.slug.clone(),
        parent_slug: detail.parent_slug.clone(),
        title: detail.title.clone(),
        artwork: detail
            .artwork
            .clone()
            .or_else(|| detail.icon.clone())
            .or_else(|| detail.detail_artwork.clone()),
        groups: download_selection::group_artifacts(&detail.remote_artifacts),
        is_primary: true,
    }];
    if detail.parent_id.is_none() {
        products.extend(detail.dlcs.iter().filter(|dlc| dlc.owned).map(|dlc| {
            DownloadDialogProduct {
                product_id: dlc.product_id,
                slug: dlc.slug.clone(),
                parent_slug: Some(detail.slug.clone()),
                title: dlc.title.clone(),
                artwork: dlc.artwork.clone().or_else(|| dlc.icon.clone()),
                groups: download_selection::group_artifacts(&dlc.remote_artifacts),
                is_primary: false,
            }
        }));
    }
    let base_installers = products[0]
        .groups
        .iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    let available_base_os = base_installers
        .iter()
        .filter_map(|group| group.operating_system.as_deref())
        .map(normalize_os)
        .collect::<BTreeSet<_>>();
    let languages = download_selection::available_languages(base_installers.into_iter());
    let selected_languages =
        download_selection::default_languages(&languages, app.config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if app.config.installer_windows {
        selected_os.insert("windows".into());
    }
    if app.config.installer_linux {
        selected_os.insert("linux".into());
    }
    if app.config.installer_macos {
        selected_os.insert("macos".into());
    }
    selected_os.retain(|os| available_base_os.contains(os));
    let online = app.network_available;
    let include_extras = app.config.download_extras_by_default;
    let installed_update = app.installed_games.contains_key(&detail.product_id);
    let include_patches = app.config.download_patches_by_default
        || (installed_update && app.config.prefer_patch_updates);
    drop(app);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(note) = note {
        let label = gtk::Label::new(Some(note));
        label.set_wrap(true);
        root.append(&label);
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.set_margin_start(22);
    body.set_margin_end(22);
    body.set_margin_top(18);
    body.set_margin_bottom(24);

    let preferences = gtk::Box::new(gtk::Orientation::Vertical, 10);
    preferences.add_css_class("download-selector-card");
    preferences.add_css_class("compact-download-preferences");
    let title = gtk::Label::new(Some("Installer preferences"));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    preferences.append(&title);
    let os_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let os_label = gtk::Label::new(Some("Platforms"));
    os_label.set_xalign(0.0);
    os_row.append(&os_label);
    let mut os_buttons = [
        ("windows", "Windows"),
        ("linux", "Linux"),
        ("macos", "macOS"),
    ]
    .into_iter()
    .filter(|(os, _)| available_base_os.contains(*os))
    .map(|(os, label)| {
        let button = gtk::ToggleButton::with_label(label);
        button.set_active(selected_os.contains(os));
        button.add_css_class("compact-preference-toggle");
        (os, button)
    })
    .collect::<Vec<_>>();
    os_buttons.sort_by_key(|(os, _)| download_selection::operating_system_rank(Some(os)));
    for (_, button) in &os_buttons {
        os_row.append(button);
    }
    let language_popover = gtk::Popover::new();
    let language_flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(10)
        .row_spacing(6)
        .max_children_per_line(3)
        .build();
    language_flow.set_margin_start(12);
    language_flow.set_margin_end(12);
    language_flow.set_margin_top(12);
    language_flow.set_margin_bottom(12);
    let mut language_buttons = Vec::new();
    for language in &languages {
        let button = gtk::CheckButton::with_label(language);
        button.set_active(selected_languages.contains(language));
        language_flow.insert(&button, -1);
        language_buttons.push((language.clone(), button));
    }
    if languages.is_empty() {
        let empty = gtk::Label::new(Some("No language-specific installers are listed"));
        empty.add_css_class("dim-label");
        language_flow.insert(&empty, -1);
    }
    language_popover.set_child(Some(&language_flow));
    let language_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let language_label = gtk::Label::new(Some("Languages"));
    language_label.set_xalign(0.0);
    language_row.append(&language_label);
    let language_summary = gtk::Label::new(None);
    let language_menu_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    language_menu_content.append(&language_summary);
    language_menu_content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    let language_menu = gtk::MenuButton::builder()
        .popover(&language_popover)
        .child(&language_menu_content)
        .build();
    language_menu.add_css_class("compact-preference-menu");
    language_row.append(&language_menu);
    let selection_row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    selection_row.append(&os_row);
    selection_row.append(&language_row);
    preferences.append(&selection_row);

    let optional_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let optional_heading = gtk::Label::new(Some("Optional content"));
    optional_heading.set_xalign(0.0);
    optional_row.append(&optional_heading);
    let extras_label = gtk::Label::new(Some("Extras"));
    extras_label.set_xalign(0.0);
    extras_label.add_css_class("dim-label");
    let include_extras_toggle = gtk::Switch::new();
    include_extras_toggle.set_active(include_extras);
    optional_row.append(&extras_label);
    optional_row.append(&include_extras_toggle);
    let patches_label = gtk::Label::new(Some("Compatible patches"));
    patches_label.set_xalign(0.0);
    patches_label.add_css_class("dim-label");
    let include_patches_toggle = gtk::Switch::new();
    include_patches_toggle.set_active(include_patches);
    optional_row.append(&patches_label);
    optional_row.append(&include_patches_toggle);
    let optional_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    optional_spacer.set_hexpand(true);
    optional_row.append(&optional_spacer);
    let view_file_details = gtk::Button::with_label("View file details…");
    view_file_details.add_css_class("flat");
    optional_row.append(&view_file_details);
    preferences.append(&optional_row);
    body.append(&preferences);

    let expert_dialog = gtk::Window::builder()
        .title(format!("File details — {}", detail.title))
        .transient_for(dialog)
        .modal(true)
        .default_width(960)
        .default_height(720)
        .build();
    let expert_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let expert_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    expert_body.set_margin_start(18);
    expert_body.set_margin_end(18);
    expert_body.set_margin_top(16);
    expert_body.set_margin_bottom(20);
    let expert_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&expert_body)
        .build();
    expert_root.append(&expert_scroll);
    let expert_footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    expert_footer.add_css_class("download-selector-footer");
    let expert_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    expert_spacer.set_hexpand(true);
    expert_footer.append(&expert_spacer);
    let expert_done = gtk::Button::with_label("Done");
    expert_footer.append(&expert_done);
    expert_root.append(&expert_footer);
    expert_dialog.set_child(Some(&expert_root));

    let plan_content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let plan_title = gtk::Label::new(Some("Download plan"));
    plan_title.set_xalign(0.0);
    plan_title.add_css_class("heading");
    body.append(&plan_title);
    body.append(&plan_content);

    let selected_products = products
        .iter()
        .map(|product| product.product_id)
        .collect::<HashSet<_>>();
    let state = Rc::new(RefCell::new(DownloadDialogState {
        selected_products,
        selected_operating_systems: selected_os,
        selected_languages,
        selected_groups: HashSet::new(),
        include_extras,
        include_patches,
        applying: false,
    }));
    let mut rows = Vec::new();
    let mut warnings = HashMap::new();
    let mut product_content = HashMap::new();
    let mut category_expanders = HashMap::new();
    let mut plan_boxes = HashMap::new();
    let mut product_toggles = HashMap::new();
    let mut dlc_toggles = Vec::new();
    let mut category_controls = Vec::new();
    for product in &products {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        card.add_css_class("download-selector-card");
        let product_heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        product_heading.append(&card_picture(product.artwork.as_ref(), 96, 54));
        let product_toggle = gtk::CheckButton::with_label(&product.title);
        product_toggle.set_active(
            state
                .borrow()
                .selected_products
                .contains(&product.product_id),
        );
        product_toggle.set_sensitive(!product.is_primary);
        product_toggle.add_css_class("section-title");
        product_toggles.insert(product.product_id, product_toggle.clone());
        product_heading.append(&product_toggle);
        card.append(&product_heading);
        if !product.is_primary {
            dlc_toggles.push((product.product_id, product_toggle));
        }
        let warning = gtk::Label::new(None);
        warning.set_xalign(0.0);
        warning.set_wrap(true);
        warning.add_css_class("warning");
        warning.set_visible(false);
        card.append(&warning);
        warnings.insert(product.product_id, warning);
        let plan_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        plan_boxes.insert(product.product_id, plan_box.clone());
        let expert_card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        expert_card.add_css_class("download-selector-card");
        let expert_title = gtk::Label::new(Some(&product.title));
        expert_title.set_xalign(0.0);
        expert_title.add_css_class("section-title");
        expert_card.append(&expert_title);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        for (kind, heading) in [
            (ArtifactKind::Installer, "Offline installers"),
            (ArtifactKind::Patch, "Patches"),
            (ArtifactKind::Extra, "Extras"),
        ] {
            let groups = product
                .groups
                .iter()
                .filter(|group| group.kind == kind)
                .cloned()
                .map(|group| {
                    let state = DialogArtifactState::Available;
                    (group, state)
                })
                .collect::<Vec<_>>();
            if groups.is_empty() {
                continue;
            }
            let downloaded = groups
                .iter()
                .filter(|(_, state)| *state == DialogArtifactState::Downloaded)
                .count();
            let preferred_downloaded = if kind == ArtifactKind::Installer {
                let selection = state.borrow();
                groups
                    .iter()
                    .filter(|(_, status)| *status == DialogArtifactState::Downloaded)
                    .filter(|(group, _)| {
                        download_selection::matches_preferences(
                            group,
                            &selection.selected_operating_systems,
                            &selection.selected_languages,
                        )
                    })
                    .count()
            } else {
                0
            };
            let section = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let heading_text = if preferred_downloaded > 0 {
                format!("{heading}  ·  {preferred_downloaded} preferred downloaded")
            } else if downloaded > 0 {
                format!("{heading}  ·  {downloaded} downloaded")
            } else {
                heading.to_owned()
            };
            let heading_label = gtk::Label::new(Some(&heading_text));
            heading_label.set_xalign(0.0);
            heading_label.set_hexpand(true);
            heading_label.add_css_class("file-name");
            section.append(&heading_label);
            let controls: &[(&str, &str)] = match kind {
                ArtifactKind::Patch => &[
                    ("All compatible", "compatible"),
                    ("All", "all"),
                    ("Clear", "clear"),
                ],
                ArtifactKind::Extra => &[("All", "all"), ("Clear", "clear")],
                ArtifactKind::Installer => &[],
            };
            for (label, mode) in controls {
                let button = gtk::Button::with_label(label);
                button.add_css_class("flat");
                button.add_css_class("compact-selector-action");
                section.append(&button);
                category_controls.push((button, product.product_id, kind, *mode));
            }
            let section_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (group, artifact_state) in groups {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("download-selector-row");
                let check = gtk::CheckButton::new();
                check.set_sensitive(matches!(
                    artifact_state,
                    DialogArtifactState::Available
                        | DialogArtifactState::Resumable
                        | DialogArtifactState::Downloaded
                ));
                row.append(&check);
                if group.operating_system.is_some() || group.language.is_some() {
                    row.append(&artifact_identity_badge(&group.artifacts[0]));
                }
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
                labels.set_hexpand(true);
                let name = gtk::Label::new(Some(&group.name));
                name.set_xalign(0.0);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.add_css_class("file-name");
                labels.append(&name);
                let mut parts = [
                    group.operating_system.as_deref(),
                    group.language.as_deref(),
                    group.version.as_deref(),
                ]
                .into_iter()
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>();
                if group.artifacts.len() > 1 {
                    parts.push(format!("{} parts", group.artifacts.len()));
                }
                let status = match artifact_state {
                    DialogArtifactState::Downloaded => Some("Downloaded"),
                    DialogArtifactState::Busy => Some("Queued or downloading"),
                    DialogArtifactState::Resumable => Some("Resume or retry"),
                    DialogArtifactState::Available => None,
                };
                if let Some(status) = status {
                    parts.push(status.into());
                }
                let metadata = gtk::Label::new(Some(&parts.join("  ·  ")));
                metadata.set_xalign(0.0);
                metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);
                metadata.add_css_class("dim-label");
                labels.append(&metadata);
                row.append(&labels);
                if let Some(size) = group.total_size {
                    let size = gtk::Label::new(Some(&human_size(size)));
                    size.add_css_class("dim-label");
                    row.append(&size);
                }
                section_rows.append(&row);
                rows.push(DownloadDialogRow {
                    group,
                    check,
                    state: artifact_state,
                });
            }
            let expander = gtk::Expander::new(None);
            expander.set_label_widget(Some(&section));
            expander.set_child(Some(&section_rows));
            expander.set_expanded(false);
            expander.add_css_class("download-selector-expander");
            category_expanders.insert((product.product_id, kind.as_str()), expander.clone());
            content.append(&expander);
        }
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(product.is_primary)
            .child(&plan_box)
            .build();
        card.append(&revealer);
        product_content.insert(product.product_id, revealer);
        plan_content.append(&card);
        expert_card.append(&content);
        expert_body.append(&expert_card);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    root.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("download-selector-footer");
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_hexpand(true);
    summary.set_wrap(true);
    footer.append(&summary);
    let install_after = gtk::CheckButton::with_label("Install after downloading");
    install_after.set_active(true);
    install_after.set_tooltip_text(Some(
        "Install the selected base game using saved folder, language and compatibility defaults. Extras, patches and DLC-only selections download without installing. Missing prerequisites remain visible in Downloads; nothing is downloaded automatically.",
    ));
    footer.append(&install_after);
    let cancel = gtk::Button::with_label("Cancel");
    footer.append(&cancel);
    let confirm = gtk::Button::with_label("Add to Download Queue");
    confirm.add_css_class("suggested-action");
    footer.append(&confirm);
    root.append(&footer);
    dialog.set_child(Some(&root));
    let widgets = Rc::new(DownloadDialogWidgets {
        rows,
        products,
        warnings,
        product_content,
        category_expanders,
        plan_boxes,
        product_toggles,
        language_summary,
        summary,
        confirm: confirm.clone(),
        authenticated: true,
        online,
        artifact_states: RefCell::new(HashMap::new()),
        libraries_available: RefCell::new(Vec::new()),
    });

    connect_download_selector_controls(
        &state,
        &widgets,
        os_buttons,
        language_buttons,
        dlc_toggles,
        category_controls,
        (include_extras_toggle, include_patches_toggle),
    );
    for row in &widgets.rows {
        let job_id = row.group.job_id.clone();
        let product_id = row.group.product_id;
        let state = state.clone();
        let widgets = widgets.clone();
        row.check.connect_toggled(move |check| {
            if state.borrow().applying {
                return;
            }
            if check.is_active() {
                let mut selection = state.borrow_mut();
                selection.selected_groups.insert(job_id.clone());
                selection.selected_products.insert(product_id);
            } else {
                state.borrow_mut().selected_groups.remove(&job_id);
            }
            refresh_download_selector(&state, &widgets, false);
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let expert_dialog = expert_dialog.clone();
        view_file_details.connect_clicked(move |_| expert_dialog.present());
    }
    {
        let expert_dialog = expert_dialog.clone();
        expert_done.connect_clicked(move |_| expert_dialog.close());
    }
    {
        let dialog = dialog.clone();
        let state = state.clone();
        let widgets = widgets.clone();
        let status = w.status.clone();
        let model = model.clone();
        let product_id = detail.parent_id.unwrap_or(detail.product_id);
        let slug = detail
            .parent_slug
            .clone()
            .unwrap_or_else(|| detail.slug.clone());
        let title = detail
            .parent_title
            .clone()
            .unwrap_or_else(|| detail.title.clone());
        let epoch = model.borrow().account_epoch;
        let queue_pending = Rc::new(std::cell::Cell::new(false));
        let window = w.window.clone();
        confirm.connect_clicked(move |_| {
            if queue_pending.get() {
                return;
            }
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                dialog.close();
                return;
            }
            let selected = selected_download_groups(&state.borrow(), &widgets)
                .into_iter()
                .map(|(product, group)| (product.clone(), group.clone()))
                .collect::<Vec<_>>();
            let install_selected = install_after.is_active()
                && selected.iter().any(|(product, group)| {
                    product.product_id == product_id && group.kind == ArtifactKind::Installer
                });
            let mut kinds = selected
                .iter()
                .map(|(_, group)| crate::storage::artifact_library_kind(&group.artifacts[0]))
                .collect::<Vec<_>>();
            if install_selected {
                kinds.push(LibraryKind::GameFiles);
            }
            let widgets = widgets.clone();
            let model = model.clone();
            let dialog = dialog.clone();
            let status = status.clone();
            let queue_pending = queue_pending.clone();
            let slug = slug.clone();
            let title = title.clone();
            let token = token.clone();
            choose_download_libraries(&window, kinds, move |libraries| {
                if model.borrow().account_epoch != epoch
                    || model.borrow().logout_pending
                    || !dialog.is_visible()
                {
                    return;
                }
                let install = install_selected.then(|| download::AutoInstallRequest {
                    product_id,
                    slug: slug.clone(),
                    title: title.clone(),
                    config: model.borrow().config.clone(),
                    library_id: libraries
                        .iter()
                        .find(|(kind, _)| *kind == LibraryKind::GameFiles)
                        .expect("installation target selected")
                        .1
                        .id
                        .clone(),
                });
                widgets.confirm.set_sensitive(false);
                queue_pending.set(true);
                widgets.summary.set_label("Adding downloads…");
                let token = token.clone();
                let session = online::account_session();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let requests = selected
                        .into_iter()
                        .map(|(product, group)| {
                            let refs = group.artifacts.iter().collect::<Vec<_>>();
                            let library = &libraries
                                .iter()
                                .find(|(kind, _)| {
                                    *kind
                                        == crate::storage::artifact_library_kind(
                                            &group.artifacts[0],
                                        )
                                })
                                .expect("typed destination selected")
                                .1;
                            let destination = download::destination(
                                &library.path,
                                product.parent_slug.as_deref().unwrap_or(&product.slug),
                                product.parent_slug.as_ref().map(|_| product.slug.as_str()),
                                &refs,
                            );
                            let paths = StateStore::open()?.current_managed_paths(&refs)?;
                            anyhow::ensure!(paths.iter().filter(|path| path.parent() == Some(destination.as_path()) && path.is_file()).count() < refs.len(),
                                "{} already has downloaded files in this library. Choose another library or deselect it.", group.name);
                            let (events, _receiver) = mpsc::channel();
                            Ok(download::DownloadRequest {
                                artifacts: group.artifacts,
                                title: product.title,
                                access_token: token.clone(),
                                destination,
                                library_id: library.id.clone(),
                                events,
                            })
                        })
                        .collect::<anyhow::Result<Vec<_>>>();
                    let result = if online::account_session() == session {
                        requests.and_then(|requests| download::enqueue_with_install(requests, install, session))
                    } else {
                        Err(anyhow::anyhow!(
                            "The account changed. Reopen the download chooser."
                        ))
                    };
                    let _ = sender.send(result);
                });
                let model = model.clone();
                let dialog = dialog.clone();
                let widgets = widgets.clone();
                let status = status.clone();
                let queue_pending = queue_pending.clone();
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    if model.borrow().account_epoch != epoch {
                        dialog.close();
                        return glib::ControlFlow::Break;
                    }
                    match receiver.try_recv() {
                        Ok(Ok(added)) => {
                            status.set_label(&format!("Added {added} downloads to the queue"));
                            dialog.close();
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            queue_pending.set(false);
                            widgets
                                .summary
                                .set_label(&format!("Could not queue downloads: {error}"));
                            widgets.confirm.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            queue_pending.set(false);
                            widgets
                                .summary
                                .set_label("Queue preparation stopped. Try again.");
                            widgets.confirm.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                    }
                });
            });
        });
    }
    poll_download_selector(dialog, model, state, widgets);
}

fn poll_download_selector(
    dialog: &gtk::Window,
    model: &Rc<RefCell<AppModel>>,
    state: Rc<RefCell<DownloadDialogState>>,
    widgets: Rc<DownloadDialogWidgets>,
) {
    let epoch = model.borrow().account_epoch;
    let session = (online::account_session(), auth::session());
    refresh_download_selector(&state, &widgets, true);
    let dialog = dialog.downgrade();
    let model = Rc::downgrade(model);
    let groups = widgets
        .rows
        .iter()
        .map(|row| row.group.clone())
        .collect::<Vec<_>>();
    let (sender, receiver) = mpsc::channel();
    let mut running = false;
    let mut initialized = false;
    glib::timeout_add_local(Duration::from_millis(500), move || {
        let (Some(dialog), Some(model)) = (dialog.upgrade(), model.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        let current = || {
            let model = model.borrow();
            dialog.is_visible()
                && model.account_epoch == epoch
                && !model.logout_pending
                && session == (online::account_session(), auth::session())
        };
        if !current() {
            return glib::ControlFlow::Break;
        }
        if let Ok((current, available)) = receiver.try_recv() {
            running = false;
            let changed = *widgets.artifact_states.borrow() != current
                || *widgets.libraries_available.borrow() != available;
            *widgets.artifact_states.borrow_mut() = current;
            *widgets.libraries_available.borrow_mut() = available;
            if changed || !initialized {
                refresh_download_selector(&state, &widgets, !initialized);
                initialized = true;
            }
        }
        // Refresh emits GTK signals; an observer may retire this chooser.
        if !current() {
            return glib::ControlFlow::Break;
        }
        if !running {
            let Ok(activity) = crate::profile_reset::begin_activity("checking download choices")
            else {
                return glib::ControlFlow::Continue;
            };
            let sender = sender.clone();
            let groups = groups.clone();
            let available = model
                .borrow()
                .library_statuses
                .iter()
                .filter(|status| {
                    matches!(
                        status.compatibility,
                        crate::storage::LibraryCompatibility::Compatible
                    )
                })
                .map(|status| status.kind)
                .collect::<Vec<_>>();
            #[cfg(test)]
            let probe = archive_poll_tests::POLL_PROBES.with_borrow_mut(|probes| {
                probes.as_mut().map(|probes| {
                    probes
                        .pop_front()
                        .expect("missing inert archive poll probe")
                })
            });
            running = true;
            std::thread::spawn(move || {
                let _activity = activity;
                #[cfg(test)]
                if let Some(probe) = &probe {
                    archive_poll_tests::hold_poll(&probe.before_read);
                }
                if session != (online::account_session(), auth::session()) {
                    return;
                }
                let paths = managed_artifact_paths();
                let states = dialog_artifact_states(&groups, &paths, || {
                    anyhow::ensure!(
                        session == (online::account_session(), auth::session()),
                        "Account changed during download inspection"
                    );
                    StateStore::open()?.download_jobs()
                });
                #[cfg(test)]
                if let Some(probe) = &probe {
                    archive_poll_tests::hold_poll(&probe.before_publish);
                }
                if session != (online::account_session(), auth::session()) {
                    return;
                }
                let _ = sender.send((states, available));
            });
        }
        glib::ControlFlow::Continue
    });
}

fn cached_artifact_state(
    group: &ArtifactGroup,
    states: &HashMap<String, DialogArtifactState>,
) -> DialogArtifactState {
    states
        .get(&group.job_id)
        .copied()
        .unwrap_or(DialogArtifactState::Available)
}

type DialogCategoryControl = (gtk::Button, i64, ArtifactKind, &'static str);

pub(super) fn connect_download_selector_controls(
    state: &Rc<RefCell<DownloadDialogState>>,
    widgets: &Rc<DownloadDialogWidgets>,
    os_buttons: Vec<(&'static str, gtk::ToggleButton)>,
    language_buttons: Vec<(String, gtk::CheckButton)>,
    dlc_toggles: Vec<(i64, gtk::CheckButton)>,
    category_controls: Vec<DialogCategoryControl>,
    optional_toggles: (gtk::Switch, gtk::Switch),
) {
    for (os, button) in os_buttons {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state
                    .borrow_mut()
                    .selected_operating_systems
                    .insert(os.into());
            } else {
                state.borrow_mut().selected_operating_systems.remove(os);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    let (include_extras, include_patches) = optional_toggles;
    for (language, button) in language_buttons {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state
                    .borrow_mut()
                    .selected_languages
                    .insert(language.clone());
            } else {
                state.borrow_mut().selected_languages.remove(&language);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    for (product_id, button) in dlc_toggles {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state.borrow_mut().selected_products.insert(product_id);
            } else {
                state.borrow_mut().selected_products.remove(&product_id);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    for (button, product_id, kind, mode) in category_controls {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_clicked(move |_| {
            let mut selection = state.borrow_mut();
            for row in widgets
                .rows
                .iter()
                .filter(|row| row.group.product_id == product_id && row.group.kind == kind)
                .filter(|row| {
                    matches!(
                        row.state,
                        DialogArtifactState::Available | DialogArtifactState::Resumable
                    )
                })
            {
                let selected = match mode {
                    "all" => true,
                    "compatible" => download_selection::matches_preferences(
                        &row.group,
                        &selection.selected_operating_systems,
                        &selection.selected_languages,
                    ),
                    _ => false,
                };
                if selected {
                    selection.selected_groups.insert(row.group.job_id.clone());
                } else {
                    selection.selected_groups.remove(&row.group.job_id);
                }
            }
            drop(selection);
            refresh_download_selector(&state, &widgets, false);
        });
    }
    {
        let state = state.clone();
        let widgets = widgets.clone();
        include_extras.connect_active_notify(move |button| {
            if state.borrow().applying {
                return;
            }
            state.borrow_mut().include_extras = button.is_active();
            refresh_download_selector(&state, &widgets, true);
        });
    }
    {
        let state = state.clone();
        let widgets = widgets.clone();
        include_patches.connect_active_notify(move |button| {
            if state.borrow().applying {
                return;
            }
            state.borrow_mut().include_patches = button.is_active();
            refresh_download_selector(&state, &widgets, true);
        });
    }
}

pub(super) fn dialog_artifact_state(
    group: &ArtifactGroup,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
) -> DialogArtifactState {
    dialog_artifact_state_with(group, managed_paths, || {
        matching_download_job(&group.artifacts.iter().collect::<Vec<_>>())
    })
}

fn dialog_artifact_states(
    groups: &[ArtifactGroup],
    managed_paths: &HashSet<ManagedArtifactIdentity>,
    load_jobs: impl FnOnce() -> anyhow::Result<Vec<DownloadJobRecord>>,
) -> HashMap<String, DialogArtifactState> {
    let jobs = std::cell::LazyCell::new(|| {
        let mut latest = HashMap::<String, DownloadJobRecord>::new();
        for job in load_jobs().unwrap_or_default() {
            if job.artifacts.is_empty() {
                continue;
            }
            let identity = download::job_id(&job.artifacts.iter().collect::<Vec<_>>());
            // max_by_key in matching_download_job keeps the last database row on ties.
            if latest
                .get(&identity)
                .is_none_or(|previous| job.updated_at >= previous.updated_at)
            {
                latest.insert(identity, job);
            }
        }
        latest
    });
    groups
        .iter()
        .map(|group| {
            let state = dialog_artifact_state_with(group, managed_paths, || {
                jobs.get(&group.job_id).cloned()
            });
            (group.job_id.clone(), state)
        })
        .collect()
}

fn dialog_artifact_state_with(
    group: &ArtifactGroup,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
    matching_job: impl FnOnce() -> Option<DownloadJobRecord>,
) -> DialogArtifactState {
    if group.artifacts.iter().all(|artifact| {
        let identity = artifact
            .provider_file_id
            .as_ref()
            .unwrap_or(&artifact.download_path);
        managed_paths.contains(&(group.product_id, identity.clone(), artifact.version.clone()))
    }) {
        return DialogArtifactState::Downloaded;
    }
    let Some(job) = matching_job() else {
        return DialogArtifactState::Available;
    };
    if download_job_is_complete(&job) {
        DialogArtifactState::Downloaded
    } else if download::is_active(&job.job_id) || job.state == "queued" {
        DialogArtifactState::Busy
    } else {
        DialogArtifactState::Resumable
    }
}

pub(super) fn refresh_download_selector(
    state: &Rc<RefCell<DownloadDialogState>>,
    widgets: &Rc<DownloadDialogWidgets>,
    apply_defaults: bool,
) {
    let artifact_states = widgets.artifact_states.borrow();
    {
        let mut selection = state.borrow_mut();
        selection.applying = true;
        for row in &widgets.rows {
            let current = cached_artifact_state(&row.group, &artifact_states);
            row.check.set_sensitive(matches!(
                current,
                DialogArtifactState::Available
                    | DialogArtifactState::Resumable
                    | DialogArtifactState::Downloaded
            ));
            if matches!(current, DialogArtifactState::Busy) {
                selection.selected_groups.remove(&row.group.job_id);
            }
        }
        if apply_defaults {
            let products = selection.selected_products.clone();
            let operating_systems = selection.selected_operating_systems.clone();
            let languages = selection.selected_languages.clone();
            let include_extras = selection.include_extras;
            let include_patches = selection.include_patches;
            for row in &widgets.rows {
                let selectable = matches!(
                    cached_artifact_state(&row.group, &artifact_states),
                    DialogArtifactState::Available | DialogArtifactState::Resumable
                );
                let selected = selectable
                    && products.contains(&row.group.product_id)
                    && match row.group.kind {
                        ArtifactKind::Extra => include_extras,
                        ArtifactKind::Installer => {
                            download_selection::matches_preferences(
                                &row.group,
                                &operating_systems,
                                &languages,
                            ) && preferred_installer_group(row, &widgets.rows)
                        }
                        ArtifactKind::Patch => {
                            include_patches
                                && download_selection::matches_preferences(
                                    &row.group,
                                    &operating_systems,
                                    &languages,
                                )
                        }
                    };
                if selected {
                    selection.selected_groups.insert(row.group.job_id.clone());
                } else {
                    selection.selected_groups.remove(&row.group.job_id);
                }
            }
        }
    }
    let (selected_products, selected_operating_systems, selected_languages, selected_groups) = {
        let selection = state.borrow();
        (
            selection.selected_products.clone(),
            selection.selected_operating_systems.clone(),
            selection.selected_languages.clone(),
            selection.selected_groups.clone(),
        )
    };
    for (product_id, toggle) in &widgets.product_toggles {
        toggle.set_active(selected_products.contains(product_id));
    }
    let language_summary = if selected_languages.is_empty() {
        "Any language".to_string()
    } else if selected_languages.len() == 1 {
        selected_languages
            .first()
            .cloned()
            .unwrap_or_else(|| "Not specified".to_string())
    } else {
        format!("{} selected", selected_languages.len())
    };
    widgets.language_summary.set_label(&language_summary);
    for row in &widgets.rows {
        row.check
            .set_active(selected_groups.contains(&row.group.job_id));
    }
    for product in &widgets.products {
        let selected = selected_products.contains(&product.product_id);
        if let Some(content) = widgets.product_content.get(&product.product_id) {
            content.set_reveal_child(selected);
        }
        if let Some(plan_box) = widgets.plan_boxes.get(&product.product_id) {
            rebuild_compact_product_plan(
                product,
                plan_box,
                &widgets.rows,
                &selected_groups,
                &selected_operating_systems,
                &selected_languages,
                &artifact_states,
            );
        }
    }
    let mut warning_count = 0;
    let mut valid = true;
    for product in &widgets.products {
        let selected = selected_products.contains(&product.product_id);
        let installers = product
            .groups
            .iter()
            .filter(|group| group.kind == ArtifactKind::Installer)
            .collect::<Vec<_>>();
        let selected_installers = installers
            .iter()
            .filter(|group| selected_groups.contains(&group.job_id))
            .count();
        let available_preferred_os = selected_operating_systems
            .iter()
            .filter(|os| {
                installers.iter().any(|group| {
                    group
                        .operating_system
                        .as_deref()
                        .is_some_and(|available| same_os(available, os))
                })
            })
            .collect::<Vec<_>>();
        let preferred_language_available = selected_languages.is_empty()
            || installers.iter().any(|group| {
                group.operating_system.as_deref().is_some_and(|os| {
                    available_preferred_os
                        .iter()
                        .any(|preferred| same_os(os, preferred))
                }) && group.language.as_deref().is_none_or(|language| {
                    selected_languages
                        .iter()
                        .any(|preferred| language.eq_ignore_ascii_case(preferred))
                })
            });
        let needs_os_fallback =
            selected && !installers.is_empty() && available_preferred_os.is_empty();
        let needs_language_fallback = selected
            && !installers.is_empty()
            && !available_preferred_os.is_empty()
            && !preferred_language_available;
        if let Some(label) = widgets.warnings.get(&product.product_id) {
            if needs_os_fallback {
                warning_count += 1;
                label.set_label(&format!(
                    "No installer is available for your preferred operating systems ({}). Choose an available operating system below.",
                    selected_operating_systems.iter().map(|os| display_os(os)).collect::<Vec<_>>().join(", ")
                ));
                label.set_visible(true);
            } else if needs_language_fallback {
                warning_count += 1;
                label.set_label(&format!(
                    "Your preferred languages ({}) are not available for {}. Choose an available language below.",
                    selected_languages.iter().cloned().collect::<Vec<_>>().join(", "),
                    available_preferred_os.iter().map(|os| display_os(os)).collect::<Vec<_>>().join(", ")
                ));
                label.set_visible(true);
            } else {
                label.set_visible(false);
            }
        }
        if let Some(expander) = widgets
            .category_expanders
            .get(&(product.product_id, ArtifactKind::Installer.as_str()))
            && (needs_os_fallback || needs_language_fallback)
        {
            expander.set_expanded(true);
        }
        let downloaded_installer = widgets.rows.iter().any(|row| {
            row.group.product_id == product.product_id
                && row.group.kind == ArtifactKind::Installer
                && cached_artifact_state(&row.group, &artifact_states)
                    == DialogArtifactState::Downloaded
        });
        if selected && !installers.is_empty() && selected_installers == 0 && !downloaded_installer {
            valid = false;
        }
    }
    let selected_rows = widgets
        .rows
        .iter()
        .filter(|row| selected_groups.contains(&row.group.job_id))
        .collect::<Vec<_>>();
    let size = selected_rows
        .iter()
        .filter_map(|row| row.group.total_size)
        .sum::<u64>();
    let unknown = selected_rows
        .iter()
        .any(|row| row.group.total_size.is_none());
    widgets.summary.set_label(&format!(
        "{} download{}  ·  {}{}  ·  {} warning{}",
        selected_rows.len(),
        if selected_rows.len() == 1 { "" } else { "s" },
        if unknown { "at least " } else { "" },
        human_size(size),
        warning_count,
        if warning_count == 1 { "" } else { "s" },
    ));
    widgets.confirm.set_sensitive(
        !selected_rows.is_empty()
            && valid
            && widgets.authenticated
            && widgets.online
            && selected_rows.iter().all(|row| {
                widgets.libraries_available.borrow().contains(
                    &crate::storage::artifact_library_kind(&row.group.artifacts[0]),
                )
            }),
    );
    widgets
        .confirm
        .set_tooltip_text((!widgets.online).then_some("Connect to GOG to add downloads"));
    let missing = selected_rows
        .iter()
        .map(|row| crate::storage::artifact_library_kind(&row.group.artifacts[0]))
        .filter(|kind| !widgets.libraries_available.borrow().contains(kind))
        .map(LibraryKind::label)
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        widgets.summary.set_label(&format!(
            "Configure a usable {} library in Storage settings before downloading.",
            missing.into_iter().collect::<Vec<_>>().join(" and ")
        ));
    }
    state.borrow_mut().applying = false;
}

pub(super) fn rebuild_compact_product_plan(
    product: &DownloadDialogProduct,
    container: &gtk::Box,
    rows: &[DownloadDialogRow],
    selected_groups: &HashSet<String>,
    operating_systems: &BTreeSet<String>,
    languages: &BTreeSet<String>,
    artifact_states: &HashMap<String, DialogArtifactState>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let product_rows = rows
        .iter()
        .filter(|row| row.group.product_id == product.product_id)
        .collect::<Vec<_>>();
    let downloaded_preferred = product_rows
        .iter()
        .filter(|row| row.group.kind == ArtifactKind::Installer)
        .filter(|row| {
            cached_artifact_state(&row.group, artifact_states) == DialogArtifactState::Downloaded
        })
        .filter(|row| {
            download_selection::matches_preferences(&row.group, operating_systems, languages)
        })
        .count();
    if downloaded_preferred > 0 {
        let downloaded = gtk::Label::new(Some(&format!(
            "✓ {downloaded_preferred} preferred OS/language combination{} already downloaded",
            if downloaded_preferred == 1 { "" } else { "s" },
        )));
        downloaded.set_xalign(0.0);
        downloaded.add_css_class("success");
        container.append(&downloaded);
    }
    for row in product_rows
        .iter()
        .filter(|row| row.group.kind == ArtifactKind::Installer)
        .filter(|row| selected_groups.contains(&row.group.job_id))
        .filter(|row| {
            cached_artifact_state(&row.group, artifact_states) != DialogArtifactState::Downloaded
        })
    {
        let mut details = format!(
            "Version {}",
            row.group.version.as_deref().unwrap_or("not listed")
        );
        let part_count = row
            .group
            .artifacts
            .iter()
            .filter_map(|artifact| artifact.part_count)
            .max()
            .unwrap_or(row.group.artifacts.len() as u32);
        if part_count > 1 {
            details.push_str(&format!(" · {part_count} parts"));
        }
        let plan = compact_plan_row(
            &format!(
                "{} · {}",
                row.group
                    .operating_system
                    .as_deref()
                    .map(display_os)
                    .unwrap_or("Any OS"),
                row.group.language.as_deref().unwrap_or("Any language"),
            ),
            row.group.total_size,
            &details,
        );
        container.append(&plan);
    }
    for (kind, title) in [
        (ArtifactKind::Extra, "Extras"),
        (ArtifactKind::Patch, "Compatible patches"),
    ] {
        let selected = product_rows
            .iter()
            .filter(|row| row.group.kind == kind)
            .filter(|row| selected_groups.contains(&row.group.job_id))
            .filter(|row| {
                cached_artifact_state(&row.group, artifact_states)
                    != DialogArtifactState::Downloaded
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        let size = selected
            .iter()
            .map(|row| row.group.total_size)
            .collect::<Option<Vec<_>>>()
            .map(|sizes| sizes.into_iter().sum());
        container.append(&compact_optional_plan(title, &selected, size));
    }
    if container.first_child().is_none() {
        let empty = gtk::Label::new(Some("No new downloads selected for this product"));
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        container.append(&empty);
    }
}

pub(super) fn compact_optional_plan(
    title: &str,
    selected: &[&&DownloadDialogRow],
    size: Option<u64>,
) -> gtk::Expander {
    let expander = gtk::Expander::new(None);
    expander.add_css_class("compact-optional-plan");
    let summary = compact_plan_row(
        &format!(
            "{title} · {} item{}",
            selected.len(),
            if selected.len() == 1 { "" } else { "s" }
        ),
        size,
        "Click to show included files",
    );
    expander.set_label_widget(Some(&summary));
    let items = gtk::Box::new(gtk::Orientation::Vertical, 3);
    items.add_css_class("compact-optional-items");
    for row in selected {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(&row.group.name));
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_hexpand(true);
        item.append(&name);
        if let Some(size) = row.group.total_size {
            let size = gtk::Label::new(Some(&human_size(size)));
            size.add_css_class("dim-label");
            item.append(&size);
        }
        items.append(&item);
    }
    expander.set_child(Some(&items));
    expander
}

pub(super) fn compact_plan_row(title: &str, size: Option<u64>, subtitle: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("compact-download-plan-row");
    let selected = gtk::Image::from_icon_name("object-select-symbolic");
    selected.add_css_class("accent");
    row.append(&selected);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("file-name");
    labels.append(&title);
    let subtitle = gtk::Label::new(Some(subtitle));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    labels.append(&subtitle);
    row.append(&labels);
    if let Some(size) = size {
        let size = gtk::Label::new(Some(&human_size(size)));
        size.add_css_class("dim-label");
        row.append(&size);
    }
    row
}

pub(super) fn preferred_installer_group(
    candidate: &DownloadDialogRow,
    rows: &[DownloadDialogRow],
) -> bool {
    preferred_artifact_group(
        &candidate.group,
        rows.iter()
            .filter(|row| row.group.kind == ArtifactKind::Installer)
            .map(|row| &row.group),
    )
}

pub(super) fn preferred_artifact_group<'a>(
    candidate: &ArtifactGroup,
    groups: impl Iterator<Item = &'a ArtifactGroup>,
) -> bool {
    let candidate_key = (
        candidate.release_sort_key(),
        candidate.version.as_deref().unwrap_or_default(),
    );
    groups
        .filter(|group| group.product_id == candidate.product_id)
        .filter(|group| {
            group.operating_system == candidate.operating_system
                && group.language == candidate.language
        })
        .all(|group| {
            (
                group.release_sort_key(),
                group.version.as_deref().unwrap_or_default(),
            ) <= candidate_key
        })
}

pub(super) fn selected_download_groups<'a>(
    state: &DownloadDialogState,
    widgets: &'a DownloadDialogWidgets,
) -> Vec<(&'a DownloadDialogProduct, &'a ArtifactGroup)> {
    let mut selected = Vec::new();
    for product in &widgets.products {
        if !state.selected_products.contains(&product.product_id) {
            continue;
        }
        for kind in [
            ArtifactKind::Installer,
            ArtifactKind::Patch,
            ArtifactKind::Extra,
        ] {
            selected.extend(
                widgets
                    .rows
                    .iter()
                    .filter(|row| {
                        row.group.product_id == product.product_id && row.group.kind == kind
                    })
                    .filter(|row| state.selected_groups.contains(&row.group.job_id))
                    .map(|row| (product, &row.group)),
            );
        }
    }
    selected
}

pub(super) fn same_os(left: &str, right: &str) -> bool {
    normalize_os(left) == normalize_os(right)
}

pub(super) fn normalize_os(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "mac" | "osx" | "macos" => "macos".into(),
        "win" | "windows" => "windows".into(),
        "linux" => "linux".into(),
        value => value.into(),
    }
}

pub(super) fn display_os(value: &str) -> &str {
    match value.to_ascii_lowercase().as_str() {
        "windows" | "win" => "Windows",
        "linux" => "Linux",
        "mac" | "osx" | "macos" => "macOS",
        _ => value,
    }
}

#[derive(Clone)]
pub(super) struct DetailFileManagement {
    pub(super) menu: gtk::MenuButton,
    pub(super) status: gtk::Label,
    pub(super) progress: gtk::ProgressBar,
}

#[cfg(test)]
mod archive_poll_tests {
    use super::*;
    use std::path::PathBuf;

    type PollBarrier = (mpsc::Sender<()>, mpsc::Receiver<()>);

    pub(super) struct PollProbe {
        pub(super) before_read: Option<PollBarrier>,
        pub(super) before_publish: Option<PollBarrier>,
    }

    thread_local! {
        pub(super) static POLL_PROBES: RefCell<Option<VecDeque<PollProbe>>> = const { RefCell::new(None) };
    }

    pub(super) fn hold_poll(barrier: &Option<PollBarrier>) {
        if let Some((entered, release)) = barrier {
            entered.send(()).unwrap();
            release
                .recv_timeout(Duration::from_secs(30))
                .expect("private poll barrier was not released");
        }
    }

    #[test]
    #[ignore = "requires fresh private p403 HOME/all XDG/TMP and GTK; empty synthetic database only"]
    fn archive_poll_admits_profile_work_and_rejects_retired_results() {
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
                    .starts_with("/tmp/ludomere-p403-"),
                "{key}"
            );
        }
        let database = crate::identity::database();
        assert!(!database.exists());
        assert!(!Config::path().exists());
        POLL_PROBES.with_borrow_mut(|probes| *probes = Some(VecDeque::new()));
        gtk::init().unwrap();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn tick_twice() {
            let complete = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(1100), {
                let complete = complete.clone();
                move || complete.set(true)
            });
            wait(|| complete.get());
        }
        fn wait_for_entry(receiver: &mpsc::Receiver<()>) {
            let entered = std::cell::Cell::new(false);
            wait(|| {
                if receiver.try_recv().is_ok() {
                    entered.set(true);
                }
                entered.get()
            });
        }
        let barrier = || {
            let (entered, observed) = mpsc::channel();
            let (release, proceed) = mpsc::channel();
            ((entered, proceed), observed, release)
        };
        let fixture = || {
            let window = gtk::Window::new();
            window.set_default_size(600, 400);
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let group = group(9403001, true);
            let check = gtk::CheckButton::with_label("Synthetic archive");
            body.append(&check);
            let widgets = Rc::new(DownloadDialogWidgets {
                rows: vec![DownloadDialogRow {
                    group: group.clone(),
                    check,
                    state: DialogArtifactState::Available,
                }],
                products: vec![DownloadDialogProduct {
                    product_id: group.product_id,
                    slug: "synthetic".into(),
                    parent_slug: None,
                    title: "Synthetic".into(),
                    artwork: None,
                    groups: vec![group.clone()],
                    is_primary: true,
                }],
                warnings: HashMap::new(),
                product_content: HashMap::new(),
                category_expanders: HashMap::new(),
                plan_boxes: HashMap::new(),
                product_toggles: HashMap::new(),
                language_summary: gtk::Label::new(None),
                summary: gtk::Label::new(None),
                confirm: gtk::Button::with_label("Add to Download Queue"),
                authenticated: false,
                online: false,
                artifact_states: RefCell::new(HashMap::new()),
                libraries_available: RefCell::new(Vec::new()),
            });
            body.append(&widgets.language_summary);
            body.append(&widgets.summary);
            body.append(&widgets.confirm);
            let state = Rc::new(RefCell::new(DownloadDialogState {
                selected_products: HashSet::from([group.product_id]),
                selected_operating_systems: BTreeSet::from(["windows".into()]),
                selected_languages: BTreeSet::from(["English".into()]),
                selected_groups: HashSet::from([group.job_id]),
                include_extras: false,
                include_patches: false,
                applying: false,
            }));
            let model = Rc::new(RefCell::new(AppModel {
                network_available: false,
                library_statuses: vec![crate::storage::LibraryStatus {
                    kind: LibraryKind::OfflineInstallers,
                    library_id: "inert-poll-library".into(),
                    path: crate::identity::data_root().join("never-created-library"),
                    compatibility: crate::storage::LibraryCompatibility::Compatible,
                    game_issues: Vec::new(),
                }],
                ..AppModel::default()
            }));
            window.set_child(Some(&body));
            window.present();
            wait(|| widgets.confirm.is_mapped());
            (window, model, state, widgets)
        };

        // No readiness/token requirement: each new origin remains usable when signed out.
        auth::invalidate_session();
        for change in ["auth", "account"] {
            let (window, model, state, widgets) = fixture();
            let (held, entered, release) = barrier();
            POLL_PROBES.with_borrow_mut(|probes| {
                probes.as_mut().unwrap().push_back(PollProbe {
                    before_read: Some(held),
                    before_publish: None,
                })
            });
            poll_download_selector(&window, &model, state.clone(), widgets.clone());
            wait_for_entry(&entered);
            assert!(
                crate::profile_reset::reserve()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("checking download choices")
            );
            if change == "auth" {
                auth::invalidate_session();
            } else {
                online::invalidate_library_session();
            }
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            tick_twice();
            assert!(
                !database.exists(),
                "retired raw origin must stop before opening the database"
            );
            assert!(widgets.artifact_states.borrow().is_empty());
            assert_eq!(state.borrow().selected_groups.len(), 1);
            assert!(!widgets.confirm.is_sensitive());
            window.destroy();
        }

        // Refusal consumes no probe; admission registers before the held worker can enter DB.
        let (window, model, state, widgets) = fixture();
        let frozen = crate::profile_reset::reserve().unwrap();
        let (held, entered, release) = barrier();
        let (published, ready, publish) = barrier();
        POLL_PROBES.with_borrow_mut(|probes| {
            probes.as_mut().unwrap().push_back(PollProbe {
                before_read: Some(held),
                before_publish: Some(published),
            })
        });
        poll_download_selector(&window, &model, state.clone(), widgets.clone());
        tick_twice();
        assert_eq!(
            POLL_PROBES.with_borrow(|probes| probes.as_ref().unwrap().len()),
            1
        );
        assert!(entered.try_recv().is_err());
        assert!(!database.exists());
        drop(frozen);
        wait_for_entry(&entered);
        assert!(crate::profile_reset::reserve().is_err());
        tick_twice(); // A real GTK timer advances while the worker is held across two poll ticks.
        assert!(POLL_PROBES.with_borrow(|probes| probes.as_ref().unwrap().is_empty()));
        assert!(!database.exists());
        release.send(()).unwrap();
        wait_for_entry(&ready);
        assert!(database.is_file());
        assert!(crate::profile_reset::reserve().is_err());
        assert!(widgets.artifact_states.borrow().is_empty());
        let selected = state.borrow().selected_groups.clone();
        // Mutably borrowing the model from a refresh signal must work, and retirement
        // there must prevent the next worker dispatch in this same timer callback.
        widgets.summary.set_label("Waiting for current sample");
        widgets.summary.connect_notify_local(Some("label"), {
            let model = Rc::downgrade(&model);
            move |_, _| model.upgrade().unwrap().borrow_mut().account_epoch += 1
        });
        publish.send(()).unwrap();
        wait(|| model.borrow().account_epoch == 1);
        wait(|| crate::profile_reset::reserve().is_ok());
        tick_twice();
        assert!(
            widgets
                .artifact_states
                .borrow()
                .values()
                .all(|value| *value == DialogArtifactState::Available)
        );
        assert_eq!(widgets.artifact_states.borrow().len(), 1);
        assert_eq!(state.borrow().selected_groups, selected);
        assert!(widgets.summary.label().starts_with("1 download"));
        assert!(!widgets.confirm.is_sensitive());
        assert!(model.borrow().account_token.is_none());
        assert!(!auth::session_is_current(auth::session()));
        window.destroy();

        // Results already read must not mutate a retired chooser. Owner loss is real Rc
        // collection, distinct from hiding/closing a still-retained window.
        for change in [
            "epoch", "logout", "close", "destroy", "owner", "auth", "account",
        ] {
            let (window, model, state, widgets) = fixture();
            let weak_model = Rc::downgrade(&model);
            let weak_window = window.downgrade();
            let mut window = Some(window);
            let mut model = Some(model);
            let (held, entered, release) = barrier();
            POLL_PROBES.with_borrow_mut(|probes| {
                probes.as_mut().unwrap().push_back(PollProbe {
                    before_read: None,
                    before_publish: Some(held),
                })
            });
            poll_download_selector(
                window.as_ref().unwrap(),
                model.as_ref().unwrap(),
                state.clone(),
                widgets.clone(),
            );
            wait_for_entry(&entered);
            let selected = state.borrow().selected_groups.clone();
            let summary = widgets.summary.label();
            let sensitive = widgets.confirm.is_sensitive();
            match change {
                "epoch" => model.as_ref().unwrap().borrow_mut().account_epoch += 1,
                "logout" => model.as_ref().unwrap().borrow_mut().logout_pending = true,
                "close" => window.as_ref().unwrap().close(),
                "destroy" => window.take().unwrap().destroy(),
                "owner" => drop(model.take()),
                "auth" => auth::invalidate_session(),
                "account" => online::invalidate_library_session(),
                _ => unreachable!(),
            }
            if change == "owner" {
                assert!(weak_model.upgrade().is_none());
            }
            if change == "destroy" {
                wait(|| weak_window.upgrade().is_none());
            }
            tick_twice();
            assert!(crate::profile_reset::reserve().is_err());
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            tick_twice();
            assert!(widgets.artifact_states.borrow().is_empty());
            assert_eq!(state.borrow().selected_groups, selected);
            assert_eq!(widgets.summary.label(), summary);
            assert_eq!(widgets.confirm.is_sensitive(), sensitive);
            if let Some(window) = window {
                window.destroy();
            }
        }
        assert!(
            POLL_PROBES
                .with_borrow_mut(Option::take)
                .unwrap()
                .is_empty()
        );
        assert!(crate::profile_reset::reserve().is_ok());
        assert!(
            StateStore::open()
                .unwrap()
                .download_jobs()
                .unwrap()
                .is_empty()
        );
        assert!(!Config::path().exists());
    }

    fn group(product_id: i64, provider: bool) -> ArtifactGroup {
        let artifacts = (1..=2)
            .map(|part| {
                serde_json::from_value::<RemoteArtifact>(serde_json::json!({
                    "product_id": product_id, "kind": "installer", "name": "Fixture",
                    "operating_system": "windows", "language": "English", "version": "1",
                    "part_number": part, "part_count": 2, "size_bytes": 10,
                    "provider_group_id": provider.then_some("group"),
                    "provider_category": provider.then_some("installer"),
                    "provider_file_id": provider.then(|| part.to_string()),
                    "download_path": format!("/synthetic/{product_id}/{part}")
                }))
                .unwrap()
            })
            .collect::<Vec<_>>();
        download_selection::group_artifacts(&artifacts)
            .pop()
            .unwrap()
    }

    fn job(
        group: &ArtifactGroup,
        id: &str,
        state: DownloadState,
        updated_at: i64,
    ) -> DownloadJobRecord {
        DownloadJobRecord {
            job_id: id.into(),
            product_id: group.product_id,
            title: "Fixture".into(),
            artifacts: group.artifacts.clone(),
            state,
            destination: PathBuf::from("/unused").join(id),
            bytes_downloaded: 0,
            total_bytes: Some(20),
            completed_files: Vec::new(),
            error: None,
            status_message: None,
            queue_position: None,
            retry_started_at: None,
            next_retry_at: None,
            created_at: 0,
            updated_at,
            completed_at: None,
        }
    }

    #[test]
    fn batch_job_matching_preserves_artifact_identity_and_last_row_timestamp_ties() {
        let official = group(11, true);
        let legacy = group(12, false);
        let mut revision = official.artifacts.clone();
        revision[0].download_path.push_str("-new-revision");
        revision[0].size_bytes = Some(11);
        let revision = download_selection::group_artifacts(&revision)
            .pop()
            .unwrap();
        let missing = group(13, true);
        let groups = [official, legacy, revision, missing];
        let mut legacy_job = job(
            &groups[1],
            "legacy-destination-id",
            DownloadState::Paused,
            30,
        );
        // The existing matcher trusts the artifacts' identity rather than these stored columns.
        legacy_job.product_id = -1;
        let mut empty = job(&groups[0], "empty-artifacts", DownloadState::Queued, 999);
        empty.artifacts.clear();
        let mut jobs = vec![
            job(&groups[0], "old-destination", DownloadState::Queued, 10),
            job(&groups[0], "new-destination", DownloadState::Failed, 20),
            job(
                &groups[0],
                "tied-last-destination",
                DownloadState::Queued,
                20,
            ),
            legacy_job,
            job(&groups[2], "revised-destination", DownloadState::Failed, 40),
            empty,
        ];
        for reverse in [false, true] {
            if reverse {
                jobs.reverse();
            }
            let calls = std::cell::Cell::new(0);
            let states = dialog_artifact_states(&groups, &HashSet::new(), || {
                calls.set(calls.get() + 1);
                Ok(jobs.clone())
            });
            assert_eq!(calls.get(), 1);
            for group in &groups {
                let expected = jobs
                    .iter()
                    .filter(|job| {
                        !job.artifacts.is_empty()
                            && download::job_id(&job.artifacts.iter().collect::<Vec<_>>())
                                == group.job_id
                    })
                    .max_by_key(|job| job.updated_at)
                    .map_or(DialogArtifactState::Available, |job| {
                        if job.state == DownloadState::Queued {
                            DialogArtifactState::Busy
                        } else {
                            DialogArtifactState::Resumable
                        }
                    });
                assert!(states[&group.job_id] == expected);
            }
            assert!(
                states[&groups[0].job_id]
                    == if reverse {
                        DialogArtifactState::Resumable
                    } else {
                        DialogArtifactState::Busy
                    }
            );
            assert!(states[&groups[1].job_id] == DialogArtifactState::Resumable);
            assert!(states[&groups[2].job_id] == DialogArtifactState::Resumable);
            assert!(states[&groups[3].job_id] == DialogArtifactState::Available);
        }
    }

    #[test]
    fn batch_job_loader_runs_once_and_managed_groups_keep_independent_fallbacks() {
        let groups = (1..=100)
            .map(|id| group(id, id % 2 == 0))
            .collect::<Vec<_>>();
        let calls = std::cell::Cell::new(0);
        let states = dialog_artifact_states(&groups, &HashSet::new(), || {
            calls.set(calls.get() + 1);
            Ok(vec![job(&groups[0], "queued", DownloadState::Queued, 1)])
        });
        assert_eq!(calls.get(), 1);
        assert!(states[&groups[0].job_id] == DialogArtifactState::Busy);
        assert_eq!(
            states
                .values()
                .filter(|state| **state == DialogArtifactState::Available)
                .count(),
            99
        );
        let mut managed = groups
            .iter()
            .flat_map(|group| {
                group.artifacts.iter().map(|artifact| {
                    (
                        group.product_id,
                        artifact
                            .provider_file_id
                            .clone()
                            .unwrap_or_else(|| artifact.download_path.clone()),
                        artifact.version.clone(),
                    )
                })
            })
            .collect::<HashSet<_>>();
        let states = dialog_artifact_states(&groups, &managed, || {
            panic!("fully managed groups must not read jobs")
        });
        assert!(
            states
                .values()
                .all(|state| *state == DialogArtifactState::Downloaded)
        );
        let artifact = &groups[1].artifacts[0];
        managed.remove(&(
            groups[1].product_id,
            artifact.provider_file_id.clone().unwrap(),
            artifact.version.clone(),
        ));
        // Wrong-version and download-path entries cannot replace a provider-file identity.
        managed.insert((
            groups[1].product_id,
            artifact.provider_file_id.clone().unwrap(),
            Some("old".into()),
        ));
        managed.insert((
            groups[1].product_id,
            artifact.download_path.clone(),
            artifact.version.clone(),
        ));
        let states = dialog_artifact_states(&groups, &managed, || {
            anyhow::bail!("inert jobs read failure")
        });
        assert!(states[&groups[1].job_id] == DialogArtifactState::Available);
        assert_eq!(
            states
                .values()
                .filter(|state| **state == DialogArtifactState::Downloaded)
                .count(),
            99
        );
        // Empty managed input is the unchanged managed-read failure fallback; valid jobs still work.
        let states = dialog_artifact_states(&groups, &HashSet::new(), || {
            Ok(vec![job(&groups[1], "paused", DownloadState::Paused, 1)])
        });
        assert!(states[&groups[1].job_id] == DialogArtifactState::Resumable);
    }

    #[test]
    fn batch_completed_jobs_require_existing_files_and_managed_matches_take_precedence() {
        let root = tempfile::tempdir().unwrap();
        let groups = [group(14, true)];
        let path = root.path().join("completed.bin");
        std::fs::write(&path, b"inert downloaded artifact").unwrap();
        let mut completed = job(&groups[0], "complete", DownloadState::Complete, 1);
        for files in [
            vec![path.clone()],
            vec![path.clone(), root.path().join("missing")],
            vec![],
        ] {
            completed.completed_files = files;
            let states =
                dialog_artifact_states(&groups, &HashSet::new(), || Ok(vec![completed.clone()]));
            assert!(
                states[&groups[0].job_id]
                    == if completed.completed_files == [path.clone()] {
                        DialogArtifactState::Downloaded
                    } else {
                        DialogArtifactState::Resumable
                    }
            );
        }
        let managed = groups[0]
            .artifacts
            .iter()
            .map(|artifact| {
                (
                    14,
                    artifact.provider_file_id.clone().unwrap(),
                    artifact.version.clone(),
                )
            })
            .collect();
        assert!(
            dialog_artifact_states(&groups, &managed, || Ok(vec![job(
                &groups[0],
                "new-failure",
                DownloadState::Failed,
                99
            )]))[&groups[0].job_id]
                == DialogArtifactState::Downloaded
        );
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn batch_poll_results_update_existing_rows_without_resetting_selection_or_focus() {
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
                    .starts_with("/tmp/ludomere-p338-")
            );
        }
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.ArchivePollTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let focus_target = gtk::Entry::new();
        body.append(&focus_target);
        let groups = [group(21, true), group(22, false)];
        let rows = groups
            .iter()
            .map(|group| {
                let check = gtk::CheckButton::with_label(&group.name);
                body.append(&check);
                DownloadDialogRow {
                    group: group.clone(),
                    check,
                    state: DialogArtifactState::Available,
                }
            })
            .collect::<Vec<_>>();
        let plan_boxes = groups
            .iter()
            .map(|group| {
                let plan = gtk::Box::new(gtk::Orientation::Vertical, 0);
                body.append(&plan);
                (group.product_id, plan)
            })
            .collect();
        let widgets = Rc::new(DownloadDialogWidgets {
            rows,
            products: groups
                .iter()
                .map(|group| DownloadDialogProduct {
                    product_id: group.product_id,
                    slug: "fixture".into(),
                    parent_slug: None,
                    title: "Fixture".into(),
                    artwork: None,
                    groups: vec![group.clone()],
                    is_primary: true,
                })
                .collect(),
            warnings: HashMap::new(),
            product_content: HashMap::new(),
            category_expanders: HashMap::new(),
            plan_boxes,
            product_toggles: HashMap::new(),
            language_summary: gtk::Label::new(None),
            summary: gtk::Label::new(None),
            confirm: gtk::Button::with_label("Add to Download Queue"),
            authenticated: true,
            online: true,
            artifact_states: RefCell::new(HashMap::new()),
            libraries_available: RefCell::new(vec![LibraryKind::OfflineInstallers]),
        });
        body.append(&widgets.summary);
        body.append(&widgets.confirm);
        let state = Rc::new(RefCell::new(DownloadDialogState {
            selected_products: HashSet::from([21, 22]),
            selected_operating_systems: BTreeSet::from(["windows".into()]),
            selected_languages: BTreeSet::from(["English".into()]),
            selected_groups: HashSet::new(),
            include_extras: false,
            include_patches: false,
            applying: false,
        }));
        window.set_content(Some(&body));
        window.present();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !focus_target.is_mapped() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(focus_target.is_mapped() && focus_target.grab_focus());
        let focus = gtk::prelude::GtkWindowExt::focus(&window);
        let checks = widgets
            .rows
            .iter()
            .map(|row| row.check.clone())
            .collect::<Vec<_>>();
        *widgets.artifact_states.borrow_mut() =
            dialog_artifact_states(&groups, &HashSet::new(), || Ok(Vec::new()));
        refresh_download_selector(&state, &widgets, true);
        assert!(
            checks
                .iter()
                .all(|check| check.is_active() && check.is_sensitive())
        );
        assert!(widgets.summary.text().starts_with("2 downloads"));
        assert!(widgets.confirm.is_sensitive());
        *widgets.artifact_states.borrow_mut() =
            dialog_artifact_states(&groups, &HashSet::new(), || {
                Ok(vec![
                    job(&groups[0], "queued", DownloadState::Queued, 1),
                    job(&groups[1], "failed", DownloadState::Failed, 1),
                ])
            });
        refresh_download_selector(&state, &widgets, false);
        assert!(!checks[0].is_active() && !checks[0].is_sensitive());
        assert!(checks[1].is_active() && checks[1].is_sensitive());
        assert_eq!(
            state.borrow().selected_groups,
            HashSet::from([groups[1].job_id.clone()])
        );
        assert!(widgets.summary.text().starts_with("1 download"));
        assert!(!widgets.confirm.is_sensitive());
        let managed = groups[0]
            .artifacts
            .iter()
            .map(|artifact| {
                (
                    21,
                    artifact.provider_file_id.clone().unwrap(),
                    artifact.version.clone(),
                )
            })
            .collect();
        *widgets.artifact_states.borrow_mut() = dialog_artifact_states(&groups, &managed, || {
            Ok(vec![job(&groups[1], "paused", DownloadState::Paused, 2)])
        });
        refresh_download_selector(&state, &widgets, false);
        assert!(!checks[0].is_active() && checks[0].is_sensitive());
        assert!(checks[1].is_active() && checks[1].is_sensitive());
        assert!(widgets.confirm.is_sensitive());
        assert!(
            widgets.plan_boxes[&21]
                .first_child()
                .and_downcast::<gtk::Label>()
                .unwrap()
                .text()
                .contains("already downloaded")
        );
        for (row, check) in widgets.rows.iter().zip(checks) {
            assert_eq!(row.check, check);
            assert_eq!(check.parent().as_ref(), Some(body.upcast_ref()));
        }
        assert_eq!(window.content().as_ref(), Some(body.upcast_ref()));
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
        assert!(window.visible_dialog().is_none());
        window.destroy();
    }
}

#[cfg(test)]
mod installer_version_tests {
    use super::*;

    #[test]
    #[ignore = "private p370a HOME/all XDG/TMP, GTK and D-Bus; consent layout only, no admission"]
    fn component_consent_aligns_text_and_keeps_actions_visible() {
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
                    .starts_with("/tmp/ludomere-p370a-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        #[track_caller]
        fn inside(widget: &gtk::Widget, container: &gtk::Widget) {
            assert!(widget.is_mapped());
            let bounds = widget.compute_bounds(container).unwrap();
            assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
            assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0, "{bounds:?}");
            assert!(
                bounds.x() + bounds.width() <= container.width() as f32 + 1.0,
                "{bounds:?}"
            );
            assert!(
                bounds.y() + bounds.height() <= container.height() as f32 + 1.0,
                "{bounds:?}"
            );
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.ComponentConsentTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(900)
            .default_height(400)
            .build();
        let original = gtk::Label::new(Some("Original page"));
        window.set_content(Some(&original));
        window.present();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let directory = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("inert-game");
        let installed: crate::domain::InstalledGame = serde_json::from_value(serde_json::json!({
            "product_id": 9370001, "library_id": "synthetic", "installation_directory": directory,
            "installer_files": [], "installer_complete": true, "installer_operating_system": "windows",
            "launch_arguments": [], "state": "installed", "playtime_seconds": 0, "created_at": 0, "updated_at": 0
        })).unwrap();
        let authentication = crate::gog::depot_service::DepotSession::new(
            auth::Token {
                access_token: "inert-never-used".into(),
                refresh_token: String::new(),
                user_id: "synthetic".into(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
            },
            (online::account_session(), auth::session()),
        )
        .unwrap();
        for long in [false, true] {
            let plan = crate::gog::dependencies::Plan {
                version: 1,
                catalog_build: "synthetic".into(),
                entries: (0..if long { 24 } else { 3 })
                    .map(|index| crate::gog::dependencies::Dependency {
                        id: format!("fixture-{index}"),
                        name: if long {
                            format!(
                                "{} FinalComponent{index}",
                                "LongUnbrokenComponentName".repeat(12)
                            )
                        } else {
                            format!("Fixture component {index}")
                        },
                        manifest_id: String::new(),
                        manifest_bytes: vec![],
                        method: crate::gog::dependencies::Method::GameFiles,
                    })
                    .collect(),
            };
            let expected = crate::installation::dependency_setup::describe(&plan);
            assert!(
                !plan.entries.is_empty(),
                "never enter automatic no-review admission"
            );
            let operation_id = format!("p370a-inert-{long}");
            let request = crate::installation::DepotOperationRequest {
                account_session: online::account_session(),
                recovery_generation: crate::installation::recovery::generation(
                    installed.product_id,
                ),
                operation_id: operation_id.clone(),
                product_id: installed.product_id,
                build_id: "synthetic".into(),
                branch: None,
                kind: crate::domain::DepotOperationKind::Repair,
                sources: vec![],
                current_sources: vec![],
                current_manifest_json: None,
                library_id: "synthetic".into(),
                dependencies: vec![],
                dependency_plan: Some(plan),
                entitlement_dlc: vec![],
                library_root: directory.parent().unwrap().to_owned(),
                slug: "inert-game".into(),
                destination: directory.clone(),
                staging_path: directory.join("unused-staging"),
                target_marker: crate::installation::installation_marker_from_game(
                    &installed,
                    vec![],
                ),
                access_token: "inert-never-used".into(),
            };
            let dialog = adw::Dialog::new();
            let preparing = gtk::Label::new(Some("Checking required components…"));
            preparing.set_selectable(true);
            dialog.set_child(Some(&preparing));
            dialog.present(Some(&window));
            wait(|| preparing.is_mapped());
            drop(preparing);
            confirm_depot_plan(&window, &model, request, authentication.clone(), &dialog);
            let root = dialog.child().unwrap();
            let header = find_named_descendant(&root, "component-consent-header").unwrap();
            let introduction = find_named_descendant(&root, "component-consent-introduction")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let description = find_named_descendant(&root, "component-consent-description")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let scroll = find_named_descendant(&root, "component-consent-description-scroll")
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let status = find_named_descendant(&root, "component-consent-status")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let status_scroll = find_named_descendant(&root, "component-consent-status-scroll")
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let buttons = ["cancel", "offline", "confirm"].map(|name| {
                find_named_descendant(&root, &format!("component-consent-{name}"))
                    .and_downcast::<gtk::Button>()
                    .unwrap()
            });
            wait(|| {
                description.is_mapped()
                    && description.width() > 0
                    && buttons[2].height() > 0
                    && root.height() > 0
            });
            assert_eq!(
                (dialog.content_width(), dialog.content_height()),
                (600, 440)
            );
            assert_eq!(dialog.title(), "Required game components");
            assert_eq!(dialog.focus().as_ref(), Some(buttons[0].upcast_ref()));
            for (direction, target) in [
                (gtk::DirectionType::TabForward, 1),
                (gtk::DirectionType::TabForward, 2),
                (gtk::DirectionType::TabBackward, 1),
                (gtk::DirectionType::TabBackward, 0),
            ] {
                assert!(window.child_focus(direction));
                assert_eq!(dialog.focus().as_ref(), Some(buttons[target].upcast_ref()));
            }
            assert!(root.height() <= 400 && root.width() <= 600);
            assert_eq!(description.text(), expected);
            assert!(description.wraps() && description.is_selectable());
            assert_eq!(description.ellipsize(), gtk::pango::EllipsizeMode::None);
            assert_eq!(buttons[0].label().as_deref(), Some("Cancel"));
            assert_eq!(buttons[1].label().as_deref(), Some("Offline installers…"));
            assert_eq!(
                buttons[2].label().as_deref(),
                Some("Install required components")
            );
            for button in &buttons {
                assert!(button.is_sensitive());
                inside(button.upcast_ref(), &root);
            }
            // Libadwaita gives headerbar a -1px margin on each side for adjoining borders.
            let header_origin = header
                .compute_point(&root, &gtk::graphene::Point::new(0.0, 0.0))
                .unwrap();
            assert!(header.is_mapped() && header.height() > 0);
            assert_eq!(header.parent().as_ref(), Some(&root));
            assert_eq!((header.margin_start(), header.margin_end()), (0, 0));
            assert!((-1.0..=0.0).contains(&header_origin.x()));
            let right = header_origin.x() + header.width() as f32;
            assert!((root.width() as f32..=root.width() as f32 + 1.0).contains(&right));
            assert!(header_origin.y() >= 0.0);
            assert!(header_origin.y() + header.height() as f32 <= root.height() as f32);
            let header_bounds = header.compute_bounds(&root).unwrap();
            // Pango origins detect centered text even when label bounds appear aligned.
            let intro_bounds = introduction.compute_bounds(&root).unwrap();
            let description_bounds = description.compute_bounds(&root).unwrap();
            let (intro_x, intro_y) = introduction.layout_offsets();
            let (text_x, text_y) = description.layout_offsets();
            let intro_extent = introduction.layout().pixel_extents().1;
            let text_extent = description.layout().pixel_extents().1;
            assert!(
                (intro_bounds.x() + intro_x as f32 + intro_extent.x() as f32
                    - description_bounds.x()
                    - text_x as f32
                    - text_extent.x() as f32)
                    .abs()
                    <= 1.0
            );
            let gap = description_bounds.y() + text_y as f32 + text_extent.y() as f32
                - (intro_bounds.y()
                    + intro_y as f32
                    + intro_extent.y() as f32
                    + intro_extent.height() as f32);
            assert!(
                (8.0..=24.0).contains(&gap),
                "paragraph-to-component text gap {gap}px"
            );
            if long {
                wait(|| scroll.vadjustment().upper() > scroll.vadjustment().page_size());
                scroll.vadjustment().set_value(scroll.vadjustment().upper());
                wait(|| {
                    description
                        .compute_bounds(&scroll)
                        .is_some_and(|bounds| bounds.y() < 0.0)
                });
                let bounds = description.compute_bounds(&scroll).unwrap();
                assert!(bounds.y() + bounds.height() <= scroll.height() as f32 + 1.0);
                assert!(description.text().contains("FinalComponent23"));
            } else {
                inside(description.upcast_ref(), scroll.upcast_ref());
            }
            for button in &buttons {
                inside(button.upcast_ref(), &root);
            }
            assert_eq!(header.compute_bounds(&root).unwrap(), header_bounds);
            // An admission error must coexist with the component list and action row.
            let error = format!(
                "{}Final admission error",
                "Synthetic admission failure detail. ".repeat(90)
            );
            status.set_label(&error);
            wait(|| {
                status_scroll.vadjustment().upper() > status_scroll.vadjustment().page_size()
                    && status_scroll.height() > 0
            });
            assert!(root.height() <= 400);
            assert!(scroll.height() >= 32);
            for button in &buttons {
                inside(button.upcast_ref(), &root);
            }
            assert_eq!(status.text(), error);
            status_scroll
                .vadjustment()
                .set_value(status_scroll.vadjustment().upper());
            wait(|| {
                status.compute_bounds(&status_scroll).is_some_and(|bounds| {
                    bounds.y() < 0.0
                        && bounds.y() + bounds.height() <= status_scroll.height() as f32 + 1.0
                })
            });
            let bounds = status.compute_bounds(&status_scroll).unwrap();
            assert!(bounds.y() + bounds.height() <= status_scroll.height() as f32 + 1.0);
            assert!(dialog.can_close());
            assert!(crate::installation::depot_operation_snapshot(&operation_id).is_none());
            assert_eq!(window.content().as_ref(), Some(original.upcast_ref()));
            // Only Cancel is exercised, never confirmation, navigation or setup.
            buttons[0].emit_clicked();
            wait(|| window.visible_dialog().is_none());
        }
        assert!(!crate::identity::database().exists());
        window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG/TMP under /tmp/ludomere-p355-"]
    fn cached_offline_preparation_preserves_preferences_before_refused_enqueue() {
        use crate::compatibility::{
            CompatibilityBackendKind, GameCompatibilityPreferences, UmuProfile, UmuProfileSource,
        };
        use crate::domain::GamePreferences;
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
                    .starts_with("/tmp/ludomere-p355-"),
                "{key}"
            );
        }
        let root = tempfile::tempdir().unwrap();
        let library = GameLibrary {
            id: "selected".into(),
            name: "Selected".into(),
            path: root.path().join("games"),
            default: true,
        };
        let store = StateStore::open().unwrap();
        let session = (online::account_session(), auth::session());
        let retained = GameCompatibilityPreferences {
            backend: CompatibilityBackendKind::Umu,
            prefix_slug: "previous-location".into(),
            profile: UmuProfile {
                game_id: "umu-active".into(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            },
            pending_profile: Some(UmuProfile {
                game_id: "umu-pending".into(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            }),
        };
        for (index, (os, saved)) in [
            ("linux", 2),
            ("windows", 2),
            ("linux", 1),
            ("windows", 1),
            ("linux", 0),
            ("windows", 0),
        ]
        .into_iter()
        .enumerate()
        {
            let id = index as i64 + 1;
            if saved != 0 {
                store
                    .upsert_game_preferences(&GamePreferences {
                        product_id: id,
                        executable_path: Some("old.exe".into()),
                        launch_arguments: if saved == 2 {
                            vec!["--retained".into()]
                        } else {
                            vec![]
                        },
                        compatibility: (saved == 2).then_some(retained.clone()),
                        ..Default::default()
                    })
                    .unwrap();
            }
            store.preserve_product_activity(id, Some(100), 234).unwrap();
            let candidate = crate::installation::InstallerCandidate {
                product_id: id,
                revision_id: Some(81),
                version: Some("chosen-version".into()),
                operating_system: Some(os.into()),
                language: Some("Polish".into()),
                paths: vec![root.path().join(if os == "linux" {
                    "chosen.sh"
                } else {
                    "chosen.exe"
                })],
                launcher: None,
                method: if os == "linux" {
                    crate::installation::InstallationMethod::NativeLinux
                } else {
                    crate::installation::InstallationMethod::WindowsCompatibility
                },
                total_size: 4,
                currently_offered: true,
                complete: true,
            };
            // Use the exact constructor and worker called by the cached chooser.
            let plan = offline_installation_plan(id, &library, "chosen-folder", &candidate);
            let mut expected = plan.clone();
            expected.launch_arguments = if saved == 2 {
                vec!["--retained".into()]
            } else {
                vec![]
            };
            expected.compatibility = (os == "windows" && saved == 2).then_some(retained.clone());
            let (captured, capture) = mpsc::channel();
            let (release, waiting) = mpsc::channel();
            let result = prepare_cached_offline_installation(
                plan,
                true,
                session,
                move |plan| -> anyhow::Result<()> {
                    captured.send(plan)?;
                    waiting.recv()?;
                    anyhow::bail!("synthetic enqueue refusal")
                },
            )
            .unwrap();
            assert_eq!(
                capture.recv_timeout(Duration::from_secs(5)).unwrap(),
                expected
            );
            assert!(
                crate::profile_reset::reserve()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("preparing offline installation")
            );
            release.send(()).unwrap();
            assert!(
                result
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("synthetic enqueue refusal")
            );
            assert!(crate::profile_reset::reserve().is_ok());
            let after = store.game_preferences(id).unwrap().unwrap();
            assert_eq!(after.launch_arguments, expected.launch_arguments);
            assert_eq!(
                after.compatibility,
                (saved == 2).then_some(retained.clone())
            );
            assert_eq!(
                after.executable_path, None,
                "old executable must not select the new source launcher"
            );
            assert_eq!(store.product_activity(id).unwrap(), (Some(100), 234));
            assert!(
                !library.path.exists(),
                "preparation never creates payloads or prefixes"
            );

            // Existing repair/DLC plans retain their explicit current choices.
            expected.launch_arguments = vec!["--current-plan".into()];
            if let Some(profile) = &mut expected.compatibility {
                profile.profile.game_id = "umu-current-plan".into();
            }
            let result =
                prepare_cached_offline_installation(expected.clone(), false, session, Ok).unwrap();
            assert_eq!(
                result
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap(),
                expected
            );
        }
        let candidate = crate::installation::InstallerCandidate {
            product_id: 99,
            revision_id: None,
            version: None,
            operating_system: Some("linux".into()),
            language: None,
            paths: vec![],
            launcher: None,
            method: crate::installation::InstallationMethod::NativeLinux,
            total_size: 0,
            currently_offered: false,
            complete: true,
        };
        let plan = offline_installation_plan(99, &library, "new-native", &candidate);
        let frozen = crate::profile_reset::reserve().unwrap();
        assert!(
            prepare_cached_offline_installation(
                plan.clone(),
                true,
                session,
                |_| -> anyhow::Result<()> { panic!("reset-frozen preparation enqueued") }
            )
            .is_err()
        );
        drop(frozen);
        assert!(
            prepare_cached_offline_installation(
                plan.clone(),
                true,
                (session.0, session.1.wrapping_add(1)),
                |_| -> anyhow::Result<()> { panic!("stale preparation enqueued") }
            )
            .is_err()
        );
        assert!(store.game_preferences(99).unwrap().is_none());
        rusqlite::Connection::open(crate::identity::database())
            .unwrap()
            .execute("DROP TABLE game_preferences", [])
            .unwrap();
        let result =
            prepare_cached_offline_installation(plan, true, session, |_| -> anyhow::Result<()> {
                panic!("failed preference read enqueued")
            })
            .unwrap();
        assert!(
            result
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .is_err()
        );
        assert!(crate::profile_reset::reserve().is_ok());
        assert!(!library.path.exists());
    }

    #[test]
    fn library_free_space_checks_never_create_missing_directories() {
        let root = tempfile::tempdir().unwrap();
        assert!(install_library_free_space(root.path()).is_ok());
        for path in [
            root.path().join("missing"),
            root.path().join("missing/nested"),
        ] {
            assert_eq!(
                install_library_free_space(&path).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
            assert!(!path.exists());
        }
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);

        let file = root.path().join("file");
        std::fs::write(&file, b"inert neighboring file").unwrap();
        assert_eq!(
            install_library_free_space(&file).unwrap_err().kind(),
            std::io::ErrorKind::NotADirectory
        );
        let link = root.path().join("link");
        std::os::unix::fs::symlink(root.path(), &link).unwrap();
        assert_eq!(
            install_library_free_space(&link).unwrap_err().kind(),
            std::io::ErrorKind::NotADirectory
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"inert neighboring file");
        assert!(!root.path().join("missing").exists());
    }

    #[test]
    fn setup_failure_summary_preserves_context_and_redacts_diagnostics() {
        assert_eq!(
            setup_failure_summary(
                "Fixture Game\n\nRequired dependency DirectX failed (exit status: 1). Log: /synthetic/install.log\nRepeated diagnostic output"
            ),
            "Fixture Game Required dependency DirectX failed (exit status: 1)"
        );
        assert_eq!(
            setup_failure_summary("Fixture: setup failed:\nCannot write prefix registry"),
            "Fixture: setup failed: Cannot write prefix registry"
        );
        let summary = setup_failure_summary(
            "Request failed access_token=synthetic-secret https://example.invalid/signed?token=secret",
        );
        assert!(!summary.contains("synthetic-secret"));
        assert!(!summary.contains("https://"));
        assert!(summary.contains("[credential redacted]"));
        assert!(summary.contains("[URL redacted]"));
        let summary = setup_failure_summary(&"é".repeat(400));
        assert_eq!(summary.chars().count(), 321);
        assert!(summary.ends_with('…'));
        assert_eq!(
            setup_failure_summary("\n  "),
            "Setup could not finish. Review Details before retrying."
        );
    }

    fn setup_counter_snapshot(state: &str) -> crate::installation::DepotOperationSnapshot {
        crate::installation::DepotOperationSnapshot {
            operation_id: "synthetic-setup-counters".into(),
            product_id: 1,
            state: state.into(),
            bytes_completed: 0,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: 0,
            download_total_bytes: None,
            error: None,
            setup: None,
        }
    }

    #[test]
    fn setup_counters_preserve_actual_values_without_inferred_work() {
        let mut snapshot = setup_counter_snapshot("complete");
        snapshot.download_total_bytes = Some(0);
        snapshot.total_write_bytes = 200;
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: complete\nNo Depot file download required.\nPayload data written this run: 0 B\nFull write estimate: 200 B\nExisting files can be reused without rewriting them."
        );
        for (state, downloaded, written) in [("materializing", 25, 50), ("complete", 100, 200)] {
            snapshot.state = state.into();
            snapshot.bytes_downloaded = downloaded;
            snapshot.download_total_bytes = Some(100);
            snapshot.bytes_written = written;
            assert_eq!(
                depot_setup_details(&snapshot),
                format!(
                    "Stage: {state}\nDepot files downloaded: {downloaded} B / 100 B\nPayload data written this run: {written} B\nFull write estimate: 200 B\nExisting files can be reused without rewriting them."
                )
            );
        }
        snapshot.download_total_bytes = Some(0);
        snapshot.total_write_bytes = 0;
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: complete\nDepot files downloaded: 100 B\nPayload data written this run: 200 B"
        );
        snapshot = setup_counter_snapshot("dependencies");
        snapshot.bytes_completed = 100;
        snapshot.total_bytes = 200;
        // Dependency acquisition also reports verified cached bytes in this field.
        snapshot.bytes_downloaded = 100;
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: dependencies\nComponent data processed: 100 B / 200 B"
        );
        snapshot.total_bytes = 0;
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: dependencies\nComponent data processed: 100 B"
        );
        // Terminal snapshots can retain that count without retaining its origin.
        for state in ["failed", "interrupted", "paused", "cancelled", "abandoned"] {
            snapshot.state = state.into();
            assert_eq!(
                depot_setup_details(&snapshot),
                format!("Stage: {state}\nData processed: 100 B")
            );
        }
        snapshot = setup_counter_snapshot("verifying_existing");
        snapshot.bytes_completed = 50;
        snapshot.total_bytes = 200;
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: verifying existing\nProcessed: 50 B / 200 B"
        );
        snapshot = setup_counter_snapshot("downloading");
        assert_eq!(depot_setup_details(&snapshot), "Stage: downloading");
        snapshot.download_total_bytes = Some(100);
        assert_eq!(
            depot_setup_details(&snapshot),
            "Stage: downloading\nDepot files downloaded: 0 B / 100 B"
        );
    }

    #[test]
    #[ignore = "requires private HOME/all XDG, D-Bus and GTK; synthetic setup results only"]
    fn setup_outcome_and_close_stay_visible_while_diagnostics_scroll() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p332-")
        );
        adw::init().unwrap();
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.SetupFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(800)
            .default_height(600)
            .build();
        let original_page = gtk::Label::new(Some("Original page"));
        window.set_content(Some(&original_page));
        window.present();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let error = format!(
            "Fixture Game\n\nRequired dependency DirectX failed (exit status: 1). Log: /synthetic/install.log\n{}\naccess_token=synthetic-secret https://example.invalid/signed?token=secret\nFinal diagnostic line",
            "Repeated diagnostic detail with a very long path-like-token/".repeat(150)
        );
        for (result, stage) in [
            (Err(error.clone()), "failed"),
            (Err(error.clone()), "dependencies"),
            (Ok(()), "complete"),
        ] {
            let dialog = adw::Dialog::builder().content_width(600).build();
            let failed = result.is_err();
            let mut snapshot = setup_counter_snapshot(stage);
            if failed {
                snapshot.bytes_downloaded = 100;
                if stage == "dependencies" {
                    snapshot.bytes_completed = 100;
                    snapshot.total_bytes = 200;
                }
            } else {
                snapshot.download_total_bytes = Some(0);
                snapshot.total_write_bytes = 558_300_000;
            }
            let counters = depot_setup_details(&snapshot);
            monitor_setup(
                &dialog,
                &model,
                SetupOperation::Fixture(result, counters.clone()),
            );
            // Presentation here is the fixture's direct action, never a worker result.
            dialog.present(Some(&window));
            let root = dialog.child().unwrap();
            let status = find_named_descendant(&root, "setup-status")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let failure = find_named_descendant(&root, "setup-failure-summary")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let scroll = find_named_descendant(&root, "setup-details-scroll")
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let close = find_named_descendant(&root, "setup-close")
                .and_downcast::<gtk::Button>()
                .unwrap();
            wait(|| close.is_mapped() && close.label().as_deref() == Some("Close"));
            // Collapsed expanders retain their child outside the traversable widget tree.
            let details = scroll
                .child()
                .and_downcast::<gtk::Viewport>()
                .unwrap()
                .child()
                .and_downcast::<gtk::Expander>()
                .unwrap()
                .child()
                .and_downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(dialog.content_height(), 400);
            assert_eq!(
                dialog.title(),
                if failed {
                    "Setup failed"
                } else {
                    "Setup complete"
                }
            );
            assert_eq!(failure.is_visible(), failed);
            if failed {
                wait(|| scroll.vadjustment().upper() > scroll.vadjustment().page_size());
            }
            assert!(details.text().contains(&counters));
            assert!(details.is_selectable());
            assert_eq!(details.ellipsize(), gtk::pango::EllipsizeMode::None);
            let close_bounds = close.compute_bounds(&root).unwrap();
            assert!(close_bounds.y() >= 0.0);
            assert!(close_bounds.y() + close_bounds.height() <= root.height() as f32);
            assert!(root.height() <= 400);
            assert!(close.is_sensitive());
            if failed {
                assert!(
                    failure
                        .text()
                        .contains("Required dependency DirectX failed")
                );
                assert!(!failure.text().contains("Repeated diagnostic"));
                assert!(!failure.layout().is_ellipsized());
                assert_eq!(
                    details.text(),
                    notifications::failure_message("", &format!(
                        "Result: {error}\n\nSynthetic setup attempt\n{counters}\n\nRecent stages:\nWaiting to start setup…"
                    )).trim_start()
                );
                let cause_bounds = failure.compute_bounds(&root).unwrap();
                let status_bounds = status.compute_bounds(&root).unwrap();
                assert!(cause_bounds.height() > 0.0);
                assert!(cause_bounds.y() >= 0.0);
                assert!(
                    cause_bounds.y() + cause_bounds.height()
                        <= scroll.compute_bounds(&root).unwrap().y()
                );
                scroll.vadjustment().set_value(scroll.vadjustment().upper());
                wait(|| scroll.vadjustment().value() > 0.0);
                assert_eq!(failure.compute_bounds(&root).unwrap(), cause_bounds);
                assert_eq!(status.compute_bounds(&root).unwrap(), status_bounds);
                assert_eq!(close.compute_bounds(&root).unwrap(), close_bounds);
            } else {
                assert_eq!(status.text(), "Game setup completed.");
                assert!(details.text().starts_with("Result: Game setup completed."));
            }
            assert_eq!(window.content().as_ref(), Some(original_page.upcast_ref()));
            close.emit_clicked();
            wait(|| window.visible_dialog().is_none());
        }
        window.destroy();
    }

    #[test]
    fn depot_action_captures_current_token_and_rejects_unavailable_sessions() {
        let model = Rc::new(RefCell::new(AppModel::default()));
        assert!(current_depot_session(&model).is_err());
        model.borrow_mut().account_token = Some(auth::Token {
            access_token: "first-access".into(),
            refresh_token: "refresh".into(),
            user_id: "user".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
        });
        let first = current_depot_session(&model).unwrap();
        model
            .borrow_mut()
            .account_token
            .as_mut()
            .unwrap()
            .access_token = "renewed-access".into();
        assert_eq!(
            current_depot_session(&model).unwrap().token.access_token,
            "renewed-access"
        );
        assert_eq!(first.token.access_token, "first-access");
        model.borrow_mut().logout_pending = true;
        assert!(current_depot_session(&model).is_err());
        model.borrow_mut().logout_pending = false;
        model
            .borrow_mut()
            .account_token
            .as_mut()
            .unwrap()
            .expires_at = chrono::Utc::now().timestamp();
        assert!(current_depot_session(&model).is_err());
        model.borrow_mut().account_token = None;
        assert!(current_depot_session(&model).is_err());
    }

    #[test]
    #[ignore = "private HOME/all XDG/TMP, GTK and D-Bus; actual delayed inspection with synthetic empty libraries"]
    fn existing_inspection_tracks_activity_and_rejects_stale_requests() {
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
                    .starts_with("/tmp/ludomere-p368-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn delay() -> mpsc::Sender<()> {
            let (sender, receiver) = mpsc::channel();
            TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow_mut().push_back(receiver));
            sender
        }
        #[track_caller]
        fn assert_registered() {
            let error = crate::profile_reset::reserve().err().unwrap().to_string();
            assert!(error.contains("inspecting installed game"), "{error}");
            assert!(TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow().is_empty()));
        }
        fn assert_responsive() {
            let heartbeat = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local_once(Duration::from_millis(100), {
                let heartbeat = heartbeat.clone();
                move || heartbeat.set(true)
            });
            wait(|| heartbeat.get());
        }
        let database = crate::identity::database();
        assert!(database.starts_with(std::env::var("XDG_DATA_HOME").unwrap()));
        assert!(
            !database.exists(),
            "fixture needs a fresh synthetic profile"
        );
        assert!(TEST_DEPOT_INSPECTION_RESULT.with(|result| result.borrow().is_none()));
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.InspectionLifecycleTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(640)
            .default_height(480)
            .build();
        let original_page = gtk::Label::new(Some("Original page"));
        window.set_content(Some(&original_page));
        window.present();
        wait(|| window.is_mapped());
        let model = Rc::new(RefCell::new(AppModel {
            config: Config {
                game_libraries: vec![],
                offline_libraries: vec![],
                extras_libraries: vec![],
                ..Config::default()
            },
            network_available: false,
            account_token: None,
            ..AppModel::default()
        }));
        let detail = DetailPageModel::game(
            Game {
                product_id: 9368001,
                slug: "synthetic-inspection".into(),
                title: "Synthetic inspection".into(),
                ..Game::default()
            },
            false,
        );
        let start = || {
            start_existing_depot_operation_dialog(
                &window,
                &model,
                &detail,
                crate::domain::DepotOperationKind::Repair,
                None,
            );
        };
        let message = |dialog: &adw::Dialog| {
            find_named_descendant(dialog.upcast_ref(), "repair-inspection-message")
                .and_downcast::<gtk::Label>()
                .unwrap()
        };

        // Local inspection needs no authenticated/online account; only generation equality.
        auth::invalidate_session();
        assert!(!auth::session_is_current(auth::session()));
        for invalidate_auth in [true, false] {
            let epoch = model.borrow().account_epoch;
            let release = delay();
            start();
            let dialog = window.visible_dialog().unwrap();
            assert_registered();
            assert_responsive();
            assert_eq!(message(&dialog).text(), "Checking the installed game…");
            assert!(!database.exists());
            if invalidate_auth {
                auth::invalidate_session();
            } else {
                online::invalidate_library_session();
            }
            assert_eq!(model.borrow().account_epoch, epoch);
            wait(|| window.visible_dialog().is_none());
            assert_registered();
            release.send(()).unwrap();
            wait(|| crate::profile_reset::reserve().is_ok());
            assert_responsive();
            assert!(!database.exists(), "stale work must stop before DB access");
            assert!(window.visible_dialog().is_none());
            assert!(
                find_named_descendant(dialog.upcast_ref(), "repair-inspection-browse").is_none()
            );
            assert_eq!(window.content().as_ref(), Some(original_page.upcast_ref()));
        }

        let unused = delay();
        model.borrow_mut().logout_pending = true;
        start();
        assert!(window.visible_dialog().is_none());
        assert_eq!(
            TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow().len()),
            1
        );
        assert!(crate::profile_reset::reserve().is_ok());
        TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow_mut().clear());
        drop(unused);
        model.borrow_mut().logout_pending = false;

        let frozen = crate::profile_reset::reserve().unwrap();
        let release = delay();
        start();
        let refused = window.visible_dialog().unwrap();
        wait(|| message(&refused).text().contains("Profile reset"));
        assert_eq!(
            TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow().len()),
            1
        );
        assert!(!database.exists());
        let spinner = find_named_descendant(refused.upcast_ref(), "repair-inspection-spinner")
            .and_downcast::<gtk::Spinner>()
            .unwrap();
        assert!(!spinner.is_spinning() && !spinner.get_visible());
        let retry = find_named_descendant(refused.upcast_ref(), "repair-inspection-retry")
            .and_downcast::<gtk::Button>()
            .unwrap();
        assert!(retry.is_sensitive());
        drop(frozen);
        retry.emit_clicked();
        let current = window.visible_dialog().unwrap();
        assert!(current != refused);
        assert_registered();
        assert_responsive();
        assert!(!database.exists());
        release.send(()).unwrap();
        wait(|| {
            message(&current)
                .text()
                .contains("This repair tool checks Galaxy Depot")
        });
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(database.is_file());
        assert!(!model.borrow().network_available && model.borrow().account_token.is_none());
        assert!(!auth::session_is_current(auth::session()));
        assert!(
            find_named_descendant(current.upcast_ref(), "repair-review-reinstallation").is_some()
        );
        assert!(find_named_descendant(current.upcast_ref(), "repair-inspection-reset").is_some());
        current.close();
        wait(|| window.visible_dialog().is_none());

        // This is an actual DB-open/initialization failure, not the P367 injected outcome.
        let invalid_database = b"deliberately invalid synthetic SQLite database";
        std::fs::write(&database, invalid_database).unwrap();
        let release = delay();
        start();
        let failed = window.visible_dialog().unwrap();
        assert_registered();
        release.send(()).unwrap();
        wait(|| {
            message(&failed)
                .text()
                .starts_with("Could not inspect the installation:")
        });
        wait(|| crate::profile_reset::reserve().is_ok());
        assert!(find_named_descendant(failed.upcast_ref(), "repair-inspection-retry").is_some());
        assert_eq!(
            std::fs::read(&database).unwrap().as_slice(),
            invalid_database
        );
        failed.close();
        wait(|| window.visible_dialog().is_none());

        let release = delay();
        start();
        let closed = window.visible_dialog().unwrap();
        assert_registered();
        closed.close();
        wait(|| window.visible_dialog().is_none());
        assert_responsive();
        assert_registered();
        release.send(()).unwrap();
        wait(|| crate::profile_reset::reserve().is_ok());
        assert_responsive();
        assert!(window.visible_dialog().is_none());
        assert!(find_named_descendant(closed.upcast_ref(), "repair-inspection-retry").is_none());
        assert_eq!(window.content().as_ref(), Some(original_page.upcast_ref()));
        assert!(TEST_DEPOT_INSPECTION_RESULT.with(|result| result.borrow().is_none()));
        assert!(TEST_DEPOT_INSPECTION_DELAYS.with(|delays| delays.borrow().is_empty()));
        window.destroy();
    }

    #[test]
    #[ignore = "private HOME/all XDG/TMP, GTK and D-Bus; inert inspection/prepared choices only"]
    fn repair_inspection_and_reused_chooser_keep_choices_and_controls_visible() {
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
                    .starts_with("/tmp/ludomere-p367-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut found = vec![];
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                found.extend(descendants(&widget));
                found.push(widget);
            }
            found
        }
        #[track_caller]
        fn inside(widget: &gtk::Widget, container: &gtk::Widget) {
            assert!(widget.is_mapped());
            let bounds = widget.compute_bounds(container).unwrap();
            assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
            assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0, "{bounds:?}");
            assert!(
                bounds.x() + bounds.width() <= container.width() as f32 + 1.0,
                "{bounds:?}"
            );
            assert!(
                bounds.y() + bounds.height() <= container.height() as f32 + 1.0,
                "{bounds:?}"
            );
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.RepairLayoutTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(900)
            .default_height(400)
            .build();
        let original_page = gtk::Label::new(Some("Original page"));
        window.set_content(Some(&original_page));
        window.present();
        wait(|| window.is_mapped());
        let config = Config::default();
        let detail = DetailPageModel::game(
            Game {
                product_id: 9367001,
                slug: "fixture-native".into(),
                title: "Fixture Native".into(),
                ..Game::default()
            },
            false,
        );
        let model = Rc::new(RefCell::new(AppModel {
            config: config.clone(),
            ..AppModel::default()
        }));
        let long_error = format!(
            "{}Final inspection cause",
            "LongDiagnosticToken/".repeat(500)
        );
        for (kind, directory, failed) in [
            (crate::domain::DepotOperationKind::Repair, None, false),
            (crate::domain::DepotOperationKind::Update, None, false),
            (
                crate::domain::DepotOperationKind::Repair,
                Some(
                    std::path::PathBuf::from(std::env::var("HOME").unwrap())
                        .join("synthetic-directory"),
                ),
                false,
            ),
            (crate::domain::DepotOperationKind::Repair, None, true),
        ] {
            TEST_DEPOT_INSPECTION_RESULT.with(|result| {
                *result.borrow_mut() = Some(if failed {
                    Err(anyhow::anyhow!(long_error.clone()))
                } else {
                    Ok(None)
                })
            });
            start_existing_depot_operation_dialog(
                &window,
                &model,
                &detail,
                kind,
                directory.clone(),
            );
            let dialog = window.visible_dialog().unwrap();
            let root = dialog.child().unwrap();
            let message = find_named_descendant(&root, "repair-inspection-message")
                .and_downcast::<gtk::Label>()
                .unwrap();
            let spinner = find_named_descendant(&root, "repair-inspection-spinner")
                .and_downcast::<gtk::Spinner>()
                .unwrap();
            let scroll = find_named_descendant(&root, "repair-inspection-scroll")
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let header = find_named_descendant(&root, "repair-inspection-header").unwrap();
            wait(|| !spinner.get_visible() && root.height() >= 330 && scroll.height() > 0);
            assert!(!spinner.is_spinning());
            assert_eq!(
                dialog.title(),
                if kind == crate::domain::DepotOperationKind::Update {
                    "Update game"
                } else {
                    "Repair game"
                }
            );
            assert!(message.wraps() && message.is_selectable());
            assert!(
                root.height() <= 400,
                "fallback expanded to {}px",
                root.height()
            );
            assert!(window.height() <= 400);
            let close = descendants(&header)
                .into_iter()
                .find(|widget| widget.is::<gtk::Button>() && widget.has_css_class("close"))
                .unwrap();
            wait(|| close.is_mapped() && close.width() > 0);
            inside(&close, &root);
            let close_bounds = close.compute_bounds(&root).unwrap();
            let browse = find_named_descendant(&root, "repair-inspection-browse").unwrap();
            let reset = find_named_descendant(&root, "repair-inspection-reset").unwrap();
            if failed {
                assert_eq!(
                    message.text(),
                    format!("Could not inspect the installation: {long_error}")
                );
                wait(|| scroll.vadjustment().upper() > scroll.vadjustment().page_size());
                assert!(find_named_descendant(&root, "repair-inspection-retry").is_some());
                scroll.vadjustment().set_value(scroll.vadjustment().upper());
                wait(|| {
                    reset
                        .compute_bounds(&scroll)
                        .is_some_and(|b| b.y() + b.height() <= scroll.height() as f32 + 1.0)
                });
                assert!(scroll.vadjustment().value() > 0.0);
                inside(
                    &find_named_descendant(&root, "repair-inspection-retry").unwrap(),
                    scroll.upcast_ref(),
                );
            } else {
                assert!(message.text().contains(if directory.is_some() {
                    "Other installed copies are not changed"
                } else {
                    "Nothing has been changed"
                }));
                assert!(!message.text().contains("No recognized"));
                let reinstall =
                    find_named_descendant(&root, "repair-review-reinstallation").unwrap();
                assert_eq!(reinstall.get_visible(), directory.is_none());
                if directory.is_none() {
                    inside(&reinstall, scroll.upcast_ref());
                }
                inside(message.upcast_ref(), scroll.upcast_ref());
            }
            for button in [&browse, &reset] {
                inside(button, scroll.upcast_ref());
                let bounds = button.compute_bounds(&scroll).unwrap();
                assert!(bounds.x() >= 18.0);
                assert!(bounds.x() + bounds.width() <= scroll.width() as f32 - 18.0);
            }
            assert_eq!(close.compute_bounds(&root).unwrap(), close_bounds);
            assert_eq!(window.content().as_ref(), Some(original_page.upcast_ref()));
            dialog.close();
            wait(|| window.visible_dialog().is_none());
        }

        // Reuse an already-mapped inspection-sized dialog, then compare the fresh entry.
        window.set_default_size(900, 760);
        wait(|| window.height() >= 700);
        for reused in [true, false] {
            let dialog = adw::Dialog::builder()
                .title("Update game")
                .content_width(if reused { 420 } else { 680 })
                .content_height(if reused { 180 } else { 620 })
                .build();
            let compact = gtk::Label::new(Some("Inspection complete"));
            dialog.set_child(Some(&compact));
            dialog.present(Some(&window));
            wait(|| compact.is_mapped() && compact.height() > 0);
            if reused {
                assert!(compact.height() <= 180);
            }
            populate_install_dialog(
                &dialog,
                &window,
                &model,
                &detail,
                true,
                InstallPreparation {
                    local_only: true,
                    config: config.clone(),
                    existing_installation: None,
                    installed_dlc_ids: HashSet::new(),
                    candidates: crate::installation::InstallerCandidates {
                        usable: vec![crate::installation::InstallerCandidate {
                            product_id: detail.product_id,
                            revision_id: None,
                            version: Some("1.2 fixture".into()),
                            operating_system: Some("linux".into()),
                            language: Some("English".into()),
                            paths: vec![],
                            launcher: None,
                            method: crate::installation::InstallationMethod::NativeLinux,
                            total_size: 4,
                            currently_offered: false,
                            complete: true,
                        }],
                        incomplete: vec![],
                        preferred: Some(0),
                    },
                    dlc_candidates: HashMap::new(),
                    galaxy_preflight: Ok(()),
                    galaxy_selection: default_galaxy_selection(&detail, &config, None),
                    library_statuses: vec![],
                    remote_installers: vec![],
                    offline_error: None,
                },
            );
            let root = dialog.child().unwrap();
            let menu = find_named_descendant(&root, "install-source-menu")
                .and_downcast::<gtk::MenuButton>()
                .unwrap();
            let scroll = find_named_descendant(&root, "install-choices-scroll")
                .and_downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let source = descendants(menu.child().unwrap().upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.text() == "linux · English · 1.2 fixture")
                .unwrap();
            let cancel = find_named_descendant(&root, "install-cancel")
                .and_downcast::<gtk::Button>()
                .unwrap();
            let install = find_named_descendant(&root, "install-confirm").unwrap();
            wait(|| menu.is_mapped() && menu.height() >= 52 && scroll.height() >= 250);
            assert_eq!(
                (dialog.content_width(), dialog.content_height()),
                (680, 620)
            );
            assert_eq!(dialog.title(), "Repair game");
            inside(menu.upcast_ref(), scroll.upcast_ref());
            inside(source.upcast_ref(), scroll.upcast_ref());
            let mut ancestor = source.parent();
            while let Some(widget) = ancestor {
                if widget.is::<gtk::ScrolledWindow>() {
                    inside(source.upcast_ref(), &widget);
                }
                if widget == root {
                    break;
                }
                ancestor = widget.parent();
            }
            for button in [cancel.upcast_ref::<gtk::Widget>(), &install] {
                inside(button, &root);
            }
            // At a smaller parent, the choices remain scrollable and the footer stays reachable.
            window.set_default_size(900, 400);
            wait(|| {
                window.height() <= 400
                    && root.height() <= 400
                    && descendants(scroll.upcast_ref())
                        .into_iter()
                        .chain(std::iter::once(scroll.clone().upcast()))
                        .filter_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
                        .any(|scroll| {
                            scroll.vadjustment().page_size() > 0.0
                                && scroll.vadjustment().upper() > scroll.vadjustment().page_size()
                        })
            });
            for button in [cancel.upcast_ref::<gtk::Widget>(), &install] {
                inside(button, &root);
            }
            let body_scroll = descendants(scroll.upcast_ref())
                .into_iter()
                .chain(std::iter::once(scroll.clone().upcast()))
                .filter_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
                .find(|scroll| scroll.vadjustment().upper() > scroll.vadjustment().page_size())
                .unwrap();
            body_scroll
                .vadjustment()
                .set_value(body_scroll.vadjustment().upper());
            wait(|| body_scroll.vadjustment().value() > 0.0);
            for button in [cancel.upcast_ref::<gtk::Widget>(), &install] {
                inside(button, &root);
            }
            assert_eq!(window.content().as_ref(), Some(original_page.upcast_ref()));
            cancel.emit_clicked();
            wait(|| window.visible_dialog().is_none());
            window.set_default_size(900, 760);
            wait(|| window.height() >= 700);
        }
        assert!(!crate::identity::database().exists());
        window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn install_destinations_and_galaxy_feedback_follow_current_choices() {
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
                    .starts_with("/tmp/ludomere-p346-")
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let root = tempfile::tempdir().unwrap();
        let libraries = [
            root.path().join("nested/first-games"),
            root.path()
                .join("nested")
                .join("configured-game-files-with-a-long-path-".repeat(4)),
            root.path().join("unavailable"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            if index < 2 {
                std::fs::create_dir_all(&path).unwrap();
            }
            GameLibrary {
                id: index.to_string(),
                name: format!("Library {index}"),
                path,
                default: index == 0,
            }
        })
        .collect::<Vec<_>>();
        let archive = GameLibrary {
            id: "archive".into(),
            name: "Archive".into(),
            path: root.path().join("offline"),
            default: true,
        };
        std::fs::create_dir(&archive.path).unwrap();
        let config = Config {
            game_libraries: libraries.clone(),
            offline_libraries: vec![archive.clone()],
            extras_libraries: vec![],
            installation_source_order: vec![
                crate::config::PreferredInstallationSource::LinuxOffline,
                crate::config::PreferredInstallationSource::WindowsOffline,
                crate::config::PreferredInstallationSource::WindowsGalaxy,
            ],
            ..Config::default()
        };
        let detail =
            DetailPageModel::game(
                Game {
                    product_id: 9346001,
                    slug: "fixture-game".into(),
                    title: "Fixture".into(),
                    galaxy_builds: vec![serde_json::from_value(serde_json::json!({
                "build_id":"fixture", "product_id":9346001, "operating_system":"windows",
                "tags":[], "public":true, "generation":2,
                "repository_url":"https://invalid.test/unused", "currently_returned":true,
                "first_seen_at":0, "last_seen_at":0
            })).unwrap()],
                    ..Game::default()
                },
                false,
            );
        let candidates = ["linux", "windows"]
            .into_iter()
            .map(|os| crate::installation::InstallerCandidate {
                product_id: detail.product_id,
                revision_id: None,
                version: Some("1".into()),
                operating_system: Some(os.into()),
                language: Some("English".into()),
                paths: vec![archive.path.join(format!("fixture-{os}"))],
                launcher: None,
                method: if os == "linux" {
                    crate::installation::InstallationMethod::NativeLinux
                } else {
                    crate::installation::InstallationMethod::WindowsCompatibility
                },
                total_size: 4,
                currently_offered: false,
                complete: true,
            })
            .collect::<Vec<_>>();
        let remote =
            download_selection::group_artifacts(&[serde_json::from_value::<RemoteArtifact>(
                serde_json::json!({
                    "product_id":detail.product_id, "kind":"installer", "name":"Fixture",
                    "operating_system":"windows", "language":"English", "version":"1",
                    "part_number":1, "part_count":1, "provider_group_id":"english",
                    "provider_file_id":"part", "download_path":"/synthetic/unused"
                }),
            )
            .unwrap()]);
        let preparation = |existing_installation, galaxy_preflight| InstallPreparation {
            local_only: false,
            config: config.clone(),
            existing_installation,
            installed_dlc_ids: HashSet::new(),
            candidates: crate::installation::InstallerCandidates {
                usable: candidates.clone(),
                incomplete: vec![],
                preferred: Some(0),
            },
            dlc_candidates: HashMap::new(),
            galaxy_preflight,
            galaxy_selection: default_galaxy_selection(&detail, &config, None),
            library_statuses: libraries
                .iter()
                .map(|library| crate::storage::LibraryStatus {
                    kind: LibraryKind::GameFiles,
                    library_id: library.id.clone(),
                    path: library.path.clone(),
                    compatibility: if library.id == "2" {
                        crate::storage::LibraryCompatibility::Unavailable(
                            "Synthetic unavailable library".into(),
                        )
                    } else {
                        crate::storage::LibraryCompatibility::Compatible
                    },
                    game_issues: vec![],
                })
                .chain(std::iter::once(crate::storage::LibraryStatus {
                    kind: LibraryKind::OfflineInstallers,
                    library_id: archive.id.clone(),
                    path: archive.path.clone(),
                    compatibility: crate::storage::LibraryCompatibility::Compatible,
                    game_issues: vec![],
                }))
                .collect(),
            remote_installers: remote.clone(),
            offline_error: None,
        };
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.ChooserFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(900)
            .default_height(760)
            .build();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.set_content(Some(&page));
        window.present();
        // No token or model game: cached source choices cannot start metadata requests.
        let model = Rc::new(RefCell::new(AppModel {
            config: config.clone(),
            ..AppModel::default()
        }));
        let dialog = adw::Dialog::builder()
            .content_width(680)
            .content_height(620)
            .build();
        populate_install_dialog(
            &dialog,
            &window,
            &model,
            &detail,
            false,
            preparation(None, Ok(())),
        );
        dialog.present(Some(&window));
        wait(|| dialog.is_mapped());
        let destination = find_named_descendant(dialog.upcast_ref(), "install-destination")
            .and_downcast::<adw::ActionRow>()
            .unwrap();
        let choices = find_named_descendant(dialog.upcast_ref(), "install-game-libraries")
            .and_downcast::<gtk::ListBox>()
            .unwrap();
        let menu = find_named_descendant(dialog.upcast_ref(), "install-source-menu")
            .and_downcast::<gtk::MenuButton>()
            .unwrap();
        let install = find_named_descendant(dialog.upcast_ref(), "install-confirm")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let feedback = find_named_descendant(dialog.upcast_ref(), "install-galaxy-feedback")
            .and_downcast::<gtk::Box>()
            .unwrap();
        let preflight = find_named_descendant(dialog.upcast_ref(), "install-galaxy-preflight")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let galaxy_status = find_named_descendant(dialog.upcast_ref(), "install-galaxy-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let status = find_named_descendant(dialog.upcast_ref(), "install-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let scroll = find_named_descendant(dialog.upcast_ref(), "install-status-scroll")
            .and_downcast::<gtk::ScrolledWindow>()
            .unwrap();
        wait(|| destination.is_mapped() && destination.width() > 0);
        assert!(destination.is_mapped());
        assert_eq!(
            destination.subtitle().unwrap(),
            libraries[0].path.join(&detail.slug).display().to_string()
        );
        for (index, library) in libraries.iter().enumerate() {
            let content = choices.row_at_index(index as i32).unwrap().child().unwrap();
            let label = content
                .first_child()
                .unwrap()
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(label.text(), library.path.display().to_string());
            assert!(label.wraps());
        }
        assert!(!choices.row_at_index(2).unwrap().is_sensitive());
        assert!(!feedback.is_mapped() && !scroll.is_mapped());
        assert!(install.is_sensitive());
        choices.select_row(choices.row_at_index(1).as_ref());
        assert_eq!(
            destination.subtitle().unwrap(),
            libraries[1].path.join(&detail.slug).display().to_string()
        );
        let choose = |index| {
            let button = find_named_descendant(
                menu.popover().unwrap().upcast_ref(),
                &format!("install-source-{index}"),
            )
            .and_downcast::<gtk::Button>()
            .unwrap();
            assert!(button.is_sensitive());
            button.emit_clicked();
        };
        // Local Linux, local Windows, remote Windows, then Depot, in configured order.
        for index in 0..4 {
            choose(index);
            assert_eq!(
                destination.subtitle().unwrap(),
                libraries[1].path.join(&detail.slug).display().to_string()
            );
            assert_eq!(install.is_sensitive(), index != 2); // remote needs a signed-in account
        }
        preflight.set_label("Synthetic Galaxy metadata failure");
        preflight.set_visible(true);
        galaxy_status.set_label("Synthetic Galaxy preparation failure");
        wait(|| feedback.is_mapped() && galaxy_status.is_mapped() && scroll.is_mapped());
        choose(0);
        wait(|| !feedback.is_mapped() && !galaxy_status.is_mapped() && !scroll.is_mapped());
        assert!(menu.grab_focus());
        let focus = gtk::prelude::GtkWindowExt::focus(&window);
        let generation = model.borrow().detail_generation;
        // The production notify wiring must keep a late Galaxy update hidden offline.
        galaxy_status.set_label("Late synthetic Galaxy preparation failure");
        preflight.set_label("Late synthetic Galaxy metadata failure");
        assert!(!feedback.is_visible() && !galaxy_status.is_visible() && !scroll.is_visible());
        assert!(install.is_sensitive());
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
        assert_eq!(model.borrow().detail_generation, generation);
        status.set_label("Synthetic offline failure");
        wait(|| status.is_mapped() && scroll.is_mapped());
        assert!(!galaxy_status.is_mapped());
        status.set_label("");
        choose(3);
        wait(|| galaxy_status.is_mapped() && feedback.is_mapped());
        assert_eq!(
            galaxy_status.text(),
            "Late synthetic Galaxy preparation failure"
        );
        assert_eq!(preflight.text(), "Late synthetic Galaxy metadata failure");
        let content = dialog.child().unwrap();
        wait(|| destination.width() > 0);
        assert!(
            content.width() <= 680,
            "long paths must not widen the chooser"
        );
        assert!(destination.width() <= content.width());
        assert_eq!(window.content().as_ref(), Some(page.upcast_ref()));
        dialog.close();
        wait(|| window.visible_dialog().is_none());

        let installed = crate::domain::InstalledGame {
            product_id: detail.product_id,
            library_id: libraries[1].id.clone(),
            installed_version: Some("1".into()),
            installation_directory: libraries[1].path.join("actual-existing-folder"),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: candidates[0].paths.clone(),
            installer_complete: true,
            installer_operating_system: Some("linux".into()),
            installer_language: Some("English".into()),
            compatibility: None,
            primary_executable: None,
            launch_arguments: vec![],
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: None,
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 0,
            updated_at: 0,
        };
        let dialog = adw::Dialog::builder()
            .content_width(680)
            .content_height(620)
            .build();
        populate_install_dialog(
            &dialog,
            &window,
            &model,
            &detail,
            true,
            preparation(
                Some(installed.clone()),
                Err("Synthetic unavailable Galaxy metadata".into()),
            ),
        );
        dialog.present(Some(&window));
        wait(|| dialog.is_mapped());
        let destination = find_named_descendant(dialog.upcast_ref(), "install-destination")
            .and_downcast::<adw::ActionRow>()
            .unwrap();
        wait(|| destination.is_mapped() && destination.width() > 0);
        let choices = find_named_descendant(dialog.upcast_ref(), "install-game-libraries")
            .and_downcast::<gtk::ListBox>()
            .unwrap();
        assert_eq!(
            destination.subtitle().unwrap(),
            installed.installation_directory.display().to_string()
        );
        choices.select_row(choices.row_at_index(0).as_ref());
        assert_eq!(choices.selected_row(), choices.row_at_index(1));
        assert_eq!(
            destination.subtitle().unwrap(),
            installed.installation_directory.display().to_string()
        );
        assert!(
            !find_named_descendant(dialog.upcast_ref(), "install-galaxy-feedback")
                .unwrap()
                .is_mapped()
        );
        assert!(
            find_named_descendant(dialog.upcast_ref(), "install-confirm")
                .unwrap()
                .is_sensitive()
        );
        dialog.close();
        wait(|| window.visible_dialog().is_none());
        window.destroy();
    }

    #[test]
    #[ignore = "private HOME/all XDG, D-Bus and GTK; captures queues without downloads or helpers"]
    fn unified_sources_queue_parts_with_initial_and_changed_libraries() {
        let _capture = DownloadQueueCapture::start();
        adw::init().unwrap();
        fn wait(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let root = tempfile::tempdir().unwrap();
        let libraries = (0..3)
            .map(|index| {
                let path = root.path().join(format!("library-{index}"));
                std::fs::create_dir(&path).unwrap();
                GameLibrary {
                    id: index.to_string(),
                    name: format!("Library {index}"),
                    path,
                    default: index != 1,
                }
            })
            .collect::<Vec<_>>();
        let mut config = Config {
            game_libraries: vec![libraries[0].clone()],
            offline_libraries: libraries[1..].to_vec(),
            extras_libraries: vec![],
            installer_language: Some("English".into()),
            installation_source_order: vec![
                crate::config::PreferredInstallationSource::WindowsOffline,
                crate::config::PreferredInstallationSource::WindowsGalaxy,
                crate::config::PreferredInstallationSource::LinuxOffline,
            ],
            ..Config::default()
        };
        config.save().unwrap();
        let build = serde_json::from_value(serde_json::json!({"build_id":"fixture", "product_id":9296001,"operating_system":"windows","tags":[],"public":true,"generation":2,"repository_url":"https://invalid.test/unused","currently_returned":true,"first_seen_at":0,"last_seen_at":0})).unwrap();
        let detail = DetailPageModel::game(
            Game {
                product_id: 9296001,
                slug: "fixture".into(),
                title: "Fixture".into(),
                galaxy_builds: vec![build],
                ..Game::default()
            },
            false,
        );
        let mut artifacts = (1..=2).map(|part| serde_json::from_value::<RemoteArtifact>(serde_json::json!({"product_id":detail.product_id,"kind":"installer","name":"Fixture","operating_system":"windows","language":"English","version":"1","part_number":part,"part_count":2,"provider_group_id":"english","provider_file_id":part.to_string(),"download_path":format!("/synthetic/{part}")})).unwrap()).collect::<Vec<_>>();
        let mut french = artifacts[0].clone();
        french.language = Some("French".into());
        french.provider_group_id = Some("french".into());
        french.part_count = Some(1);
        artifacts.push(french);
        let remote = download_selection::group_artifacts(&artifacts);
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.UnifiedInstallTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.present();
        let model = Rc::new(RefCell::new(AppModel {
            config: config.clone(),
            account_token: Some(auth::Token {
                access_token: "synthetic".into(),
                refresh_token: String::new(),
                user_id: "fixture".into(),
                expires_at: i64::MAX,
            }),
            ..AppModel::default()
        }));
        for case in 0..4 {
            if case == 3 {
                config.offline_libraries = libraries[1..].to_vec();
                config.save().unwrap();
                model.borrow_mut().config = config.clone();
            }
            if case == 2 {
                config.offline_libraries.clear();
                config.save().unwrap();
                model.borrow_mut().config = config.clone();
            }
            let dialog = adw::Dialog::new();
            let preparation = InstallPreparation {
                local_only: case == 3,
                config: config.clone(),
                existing_installation: None,
                installed_dlc_ids: HashSet::new(),
                candidates: crate::installation::InstallerCandidates {
                    usable: vec![crate::installation::InstallerCandidate {
                        product_id: detail.product_id,
                        revision_id: None,
                        version: Some("local".into()),
                        operating_system: Some("linux".into()),
                        language: Some("English".into()),
                        paths: vec![libraries[1].path.join("fixture.sh")],
                        launcher: None,
                        method: crate::installation::InstallationMethod::NativeLinux,
                        total_size: 4,
                        currently_offered: false,
                        complete: true,
                    }],
                    incomplete: vec![],
                    preferred: Some(0),
                },
                dlc_candidates: HashMap::new(),
                galaxy_preflight: Ok(()),
                galaxy_selection: default_galaxy_selection(&detail, &config, None),
                library_statuses: crate::storage::inspect_libraries(&config).unwrap(),
                remote_installers: remote.clone(),
                offline_error: None,
            };
            populate_install_dialog(&dialog, &window, &model, &detail, false, preparation);
            dialog.present(Some(&window));
            wait(|| dialog.is_mapped());
            let menu = find_named_descendant(dialog.upcast_ref(), "install-source-menu")
                .and_downcast::<gtk::MenuButton>()
                .unwrap();
            let archive = find_named_descendant(dialog.upcast_ref(), "install-archive-library")
                .and_downcast::<gtk::DropDown>()
                .unwrap();
            let install = find_named_descendant(dialog.upcast_ref(), "install-confirm")
                .and_downcast::<gtk::Button>()
                .unwrap();
            let choice = |index| {
                find_named_descendant(
                    menu.popover().unwrap().upcast_ref(),
                    &format!("install-source-{index}"),
                )
                .and_downcast::<gtk::Button>()
                .unwrap()
            };
            if case == 3 {
                assert!(choice(0).child().is_some());
                assert!(
                    find_named_descendant(menu.popover().unwrap().upcast_ref(), "install-source-1")
                        .is_none(),
                    "local-only entry must exclude Depot and remote downloads"
                );
                assert!(!archive.is_mapped());
                assert!(install.is_sensitive());
                dialog.close();
                wait(|| window.visible_dialog().is_none());
                continue;
            }
            assert!(
                choice(2).child().is_some() && choice(3).child().is_some(),
                "Depot and downloaded Linux source remain available"
            );
            if case == 2 {
                assert!(!install.is_sensitive());
                choice(2).emit_clicked();
                assert!(install.is_sensitive());
                dialog.close();
                wait(|| window.visible_dialog().is_none());
                continue;
            }
            assert_eq!(archive.selected(), 1, "default applies without reselecting");
            if case == 1 {
                archive.set_selected(0);
                choice(1).emit_clicked();
            }
            install.emit_clicked();
            assert!(!install.is_sensitive());
            wait(|| TEST_DOWNLOAD_QUEUE.lock().unwrap().as_ref().unwrap().len() == case + 1);
            wait(|| window.visible_dialog().is_none());
            let captured = TEST_DOWNLOAD_QUEUE.lock().unwrap();
            let (requests, intent) = &captured.as_ref().unwrap()[case];
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].library_id,
                libraries[if case == 0 { 2 } else { 1 }].id
            );
            assert_eq!(requests[0].artifacts.len(), if case == 0 { 2 } else { 1 });
            assert_eq!(
                requests[0].artifacts[0].language.as_deref(),
                Some(if case == 0 { "English" } else { "French" })
            );
            if case == 0 {
                assert_eq!(
                    requests[0]
                        .artifacts
                        .iter()
                        .map(|artifact| artifact.part_number.unwrap())
                        .collect::<Vec<_>>(),
                    vec![1, 2]
                );
            }
            assert_eq!(intent.as_ref().unwrap().library_id, libraries[0].id);
            assert!(
                !intent
                    .as_ref()
                    .unwrap()
                    .config
                    .interactive_installer_prompts
            );
            assert!(!requests[0].destination.starts_with(&libraries[0].path));
        }
        window.close();
    }

    #[test]
    fn initial_depot_selection_uses_saved_game_language_for_base_and_dlc() {
        use crate::domain::{Dlc, Game, GamePreferences, ProductLocalization};
        let directory = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::open_at(&directory.path().join("state.db")).unwrap();
        let mut detail = super::DetailPageModel::game(
            Game {
                product_id: 100,
                dlcs: vec![Dlc {
                    product_id: 200,
                    owned: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            false,
        );
        detail.metadata.localizations = [("en-US", "English"), ("fr", "French"), ("de", "German")]
            .into_iter()
            .map(|(code, name)| ProductLocalization {
                language_code: code.into(),
                name: name.into(),
                text: true,
                audio: false,
            })
            .collect();
        let config = super::Config {
            installer_language: Some("German".into()),
            ..Default::default()
        };
        store
            .set_game_update_preferences(100, None, None, None, Some("FRENCH"))
            .unwrap();
        let selection = super::default_galaxy_selection(
            &detail,
            &config,
            store.game_preferences(100).unwrap().as_ref(),
        );
        assert_eq!(selection.language, "fr");
        let repository = crate::gog::repository::parse(br#"{
          "version":2,"baseProductId":"100","buildId":"fixture","platform":"windows","installDirectory":"Game",
          "products":[{"productId":"100"},{"productId":"200"}],
          "depots":[
            {"manifest":"neutral","productId":"100","languages":[],"size":1},
            {"manifest":"base-fr","productId":"100","languages":["fr"],"size":1},
            {"manifest":"base-de","productId":"100","languages":["de"],"size":1},
            {"manifest":"dlc-fr","productId":"200","languages":["fr"],"size":1},
            {"manifest":"dlc-de","productId":"200","languages":["de"],"size":1}
          ]}"#).unwrap();
        let selected =
            crate::gog::depot_acquisition::select_depots(&repository, &selection).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|depot| depot.manifest_id.as_str())
                .collect::<Vec<_>>(),
            ["neutral", "base-fr", "dlc-fr"]
        );
        // A later saved choice must replace the dialog's old selection at preparation.
        store
            .set_game_update_preferences(100, None, None, None, Some("de"))
            .unwrap();
        assert_eq!(
            super::default_galaxy_selection(
                &detail,
                &config,
                store.game_preferences(100).unwrap().as_ref()
            )
            .language,
            "de"
        );
        assert_eq!(
            super::default_galaxy_selection(&detail, &config, None).language,
            "de"
        );
        assert_eq!(
            super::default_galaxy_selection(&detail, &super::Config::default(), None).language,
            "en-US"
        );
        assert_eq!(
            super::default_galaxy_selection(
                &detail,
                &super::Config {
                    installer_language: Some("Unavailable".into()),
                    ..Default::default()
                },
                None
            )
            .language,
            "en-US"
        );
        assert_eq!(
            super::default_galaxy_selection(
                &detail,
                &config,
                Some(&GamePreferences {
                    galaxy_language: Some("pt-BR".into()),
                    ..Default::default()
                })
            )
            .language,
            "pt-BR"
        );
        detail.metadata.localizations.clear();
        assert_eq!(
            super::default_galaxy_selection(&detail, &super::Config::default(), None).language,
            "en"
        );
    }

    #[test]
    fn dlc_must_match_the_selected_base_version_exactly() {
        assert!(versions_match(Some("1.3.0.6"), Some("1.3.0.6")));
        assert!(!versions_match(Some("1.3.0.5"), Some("1.3.0.6")));
        assert!(!versions_match(None, Some("1.3.0.6")));
    }

    #[test]
    fn repair_summary_distinguishes_selected_and_unavailable_dlc() {
        assert_eq!(
            dlc_summary_text(3, 1, true),
            "DLC · 3 selected · 1 unavailable"
        );
    }
}
