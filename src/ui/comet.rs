use super::*;
use crate::compatibility::{acquisition::DownloadProgress, comet, components};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct ComponentStatus {
    updates: components::ComponentUpdates,
    comet_version: Result<String, String>,
    peer_version: Result<Option<String>, String>,
    candidate: Result<Option<comet::CometUpdate>, String>,
}

static STATUS: OnceLock<Mutex<Option<Arc<ComponentStatus>>>> = OnceLock::new();
static CHECKING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

pub(super) fn start_component_check() {
    if !STARTED.swap(true, Ordering::Relaxed) {
        check_updates();
    }
}

fn check_updates() {
    if CHECKING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let cancel = AtomicBool::new(false);
        let status = ComponentStatus {
            updates: components::check_component_updates(&cancel),
            comet_version: comet::installed_version().map_err(|error| error.to_string()),
            peer_version: components::installed_peer_version().map_err(|error| error.to_string()),
            candidate: comet::update_candidate(&cancel).map_err(|error| error.to_string()),
        };
        *STATUS.get_or_init(Default::default).lock().unwrap() = Some(Arc::new(status));
        CHECKING.store(false, Ordering::Release);
    });
}

fn version_message(available: &str, installed: Option<&str>) -> String {
    match installed {
        None => format!("Available: {available}; no local version has been reported"),
        Some(installed) if components::newer_version(available, installed).unwrap_or(false) => {
            format!("Newer version available: {available} (local: {installed})")
        }
        Some(installed) if components::newer_version(installed, available).unwrap_or(false) => {
            format!("Local version {installed} is newer than available version {available}")
        }
        Some(installed) => format!("Up to date: {installed}"),
    }
}

pub(super) fn comet_page(window: &adw::ApplicationWindow) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("GOG online services");
    let group = adw::PreferencesGroup::new();
    group.set_title("Comet and GOG peer libraries");
    group.set_description(Some("Comet provides GOG authentication, achievements and statistics for supported games. Unmodified Comet automatically downloads and updates its GOG peer libraries when used, according to its own update checks. Ludomere's startup and manual checks read version information only. Updating Comet itself requires your confirmation."));
    let status = gtk::Label::new(Some("Checking component versions…"));
    status.set_wrap(true);
    status.set_xalign(0.0);
    group.add(&status);
    let check = gtk::Button::with_label("Check for updates");
    let update = gtk::Button::with_label("Update Comet…");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&check);
    actions.append(&update);
    group.add(&actions);
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_visible(false);
    group.add(&progress);
    let message = gtk::Label::new(None);
    message.set_wrap(true);
    message.set_xalign(0.0);
    group.add(&message);
    let cancel = gtk::Button::with_label("Cancel download");
    cancel.set_sensitive(false);
    group.add(&cancel);
    page.add(&group);
    let running: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));
    cancel.connect_clicked({
        let running = running.clone();
        move |button| {
            if let Some(cancel) = running.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
            button.set_sensitive(false);
        }
    });
    page.connect_unrealize({
        let running = running.clone();
        move |_| {
            if let Some(cancel) = running.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    });
    check.connect_clicked(|_| check_updates());
    update.connect_clicked({
        let window = window.clone(); let running = running.clone();
        let progress = progress.clone(); let message = message.clone(); let cancel_button = cancel.clone();
        move |_| {
            if running.borrow().is_some() { return; }
            let snapshot = STATUS.get_or_init(Default::default).lock().unwrap().clone();
            let Some(candidate) = snapshot.and_then(|snapshot| snapshot.candidate.as_ref().ok().and_then(Clone::clone)) else { return; };
            let dialog = adw::AlertDialog::builder().heading("Update Comet?")
                .body(format!("Download official Comet {} ({}) into Ludomere's application data? Your existing helper remains available if the download fails. Comet will continue to manage its peer-library downloads and updates automatically when used.", candidate.version, human_size(candidate.download_bytes)))
                .build();
            dialog.add_responses(&[("cancel", "Not now"), ("download", "Download")]);
            dialog.set_close_response("cancel");
            let running = running.clone(); let progress = progress.clone();
            let message = message.clone(); let cancel_button = cancel_button.clone();
            dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "download" || running.borrow().is_some() { return; }
                let cancelled = Arc::new(AtomicBool::new(false));
                *running.borrow_mut() = Some(cancelled.clone());
                cancel_button.set_sensitive(true); progress.set_visible(true); progress.set_fraction(0.0);
                message.set_label("Preparing Comet update…");
                let (updates, updates_rx) = mpsc::sync_channel::<DownloadProgress>(32);
                let (done, done_rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = comet::install_update(&candidate, &cancelled, |value| { updates.try_send(value).ok(); })
                        .map_err(|error| error.to_string());
                    done.send(result).ok();
                });
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    for update in updates_rx.try_iter().take(32) {
                        progress.set_text(Some(&format!("{} — {}", update.phase, human_size(update.completed))));
                        if let Some(total) = update.total.filter(|total| *total > 0) {
                            progress.set_fraction((update.completed as f64 / total as f64).min(1.0));
                        } else { progress.pulse(); }
                    }
                    let result = match done_rx.try_recv() {
                        Ok(result) => result,
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => Err("Comet update stopped unexpectedly".into()),
                    };
                    running.borrow_mut().take(); cancel_button.set_sensitive(false); progress.set_visible(false);
                    match result {
                        Ok(()) => { message.set_label("Comet updated. Games will use it on their next launch."); check_updates(); }
                        Err(error) => message.set_label(&error),
                    }
                    glib::ControlFlow::Break
                });
            });
        }
    });
    let weak = page.downgrade();
    glib::timeout_add_local(Duration::from_millis(250), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let busy = running.borrow().is_some();
        let checking = CHECKING.load(Ordering::Acquire);
        check.set_sensitive(!busy && !checking);
        let snapshot = STATUS.get_or_init(Default::default).lock().unwrap().clone();
        if let Some(snapshot) = snapshot {
            let comet_status = match &snapshot.updates.comet {
                Ok(release) => version_message(
                    &release.version,
                    snapshot.comet_version.as_ref().ok().map(String::as_str),
                ),
                Err(error) => format!("Version check failed: {error}"),
            };
            let local_comet_error = snapshot
                .comet_version
                .as_ref()
                .err()
                .map(|error| format!("\nComet installation: {error}"))
                .unwrap_or_default();
            let candidate = match &snapshot.candidate {
                Ok(Some(candidate)) => format!(
                    "Official Comet {} update is ready to download",
                    candidate.version
                ),
                Ok(None) => "No newer official Comet update is available".into(),
                Err(error) => format!("Comet update check: {error}"),
            };
            let peers = match &snapshot.updates.peers {
                Ok(release) => version_message(
                    &release.version,
                    snapshot
                        .peer_version
                        .as_ref()
                        .ok()
                        .and_then(|version| version.as_deref()),
                ),
                Err(error) => format!("Version check failed: {error}"),
            };
            let local_peer_error = snapshot
                .peer_version
                .as_ref()
                .err()
                .map(|error| format!("\nCached peer metadata: {error}"))
                .unwrap_or_default();
            status.set_label(&format!("Comet: {comet_status}{local_comet_error}\n{candidate}\nGOG peer libraries (Comet-reported): {peers}{local_peer_error}{}",
                if checking { "\nChecking for updates…" } else { "" }));
            update.set_sensitive(
                !busy && !checking && snapshot.candidate.as_ref().is_ok_and(Option::is_some),
            );
        } else {
            update.set_sensitive(false);
        }
        glib::ControlFlow::Continue
    });
    page
}

#[cfg(test)]
mod tests {
    use super::version_message;

    #[test]
    fn version_status_distinguishes_missing_newer_current_and_local_newer() {
        assert!(version_message("1.2.33.1", None).contains("no local version"));
        assert!(version_message("1.2.33.1", Some("1.2.32.1")).contains("Newer version available"));
        assert_eq!(
            version_message("v0.3.2", Some("0.3.2.0")),
            "Up to date: 0.3.2.0"
        );
        assert!(version_message("0.3.2", Some("0.3.3")).contains("Local version 0.3.3 is newer"));
    }
}
