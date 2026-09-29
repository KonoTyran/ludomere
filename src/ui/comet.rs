use super::*;
use crate::compatibility::{acquisition::DownloadProgress, comet, components};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct ComponentStatus {
    updates: Option<components::ComponentUpdates>,
    comet_version: Result<String, String>,
    peer_version: Result<Option<String>, String>,
    candidate: Option<Result<Option<comet::CometUpdate>, String>>,
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
        let mut status = ComponentStatus {
            updates: None,
            comet_version: comet::installed_version().map_err(|error| error.to_string()),
            peer_version: components::installed_peer_version().map_err(|error| error.to_string()),
            candidate: None,
        };
        *STATUS.get_or_init(Default::default).lock().unwrap() = Some(Arc::new(ComponentStatus {
            updates: None,
            candidate: None,
            comet_version: status.comet_version.clone(),
            peer_version: status.peer_version.clone(),
        }));
        status.updates = Some(components::check_component_updates(&cancel));
        status.candidate =
            Some(comet::update_candidate(&cancel).map_err(|error| error.to_string()));
        *STATUS.get_or_init(Default::default).lock().unwrap() = Some(Arc::new(status));
        CHECKING.store(false, Ordering::Release);
    });
}

struct ComponentView {
    installation: String,
    updates: String,
    peers: String,
    action: Option<&'static str>,
}

fn component_view(
    local: &Result<String, String>,
    peer: &Result<Option<String>, String>,
    releases: Option<&components::ComponentUpdates>,
    candidate: Option<Result<Option<&str>, &str>>,
) -> ComponentView {
    let installation = match local {
        Ok(version) => format!("Installed and verified — version {version}"),
        Err(error) => format!("Could not verify the local installation: {error}"),
    };
    let (updates, action) = match candidate {
        None => ("Checking official releases…".into(), None),
        Some(Ok(Some(version))) if local.is_ok() => (format!("Version {version} is available."), Some("Update Comet…")),
        Some(Ok(Some(version))) => (format!("Official version {version} is available to install."), Some("Install Comet…")),
        Some(Ok(None)) => match local {
            Ok(installed) => (releases.and_then(|updates| updates.comet.as_ref().ok()).map_or_else(
                || "No newer installable release was found.".into(),
                |release| version_message(&release.version, Some(installed)),
            ), None),
            Err(_) => ("No downloadable release was offered. Reinstall Ludomere, or prepare development helpers for a source build.".into(), None),
        },
        Some(Err(error)) => (format!("Could not check downloadable releases: {error}. Use Check for updates to retry."), None),
    };
    let local_peers = match peer {
        Ok(Some(version)) => format!("Comet last reported version {version}."),
        Ok(None) => "No cached version reported yet. Comet acquires these libraries automatically when a supported game uses it.".into(),
        Err(_) => "Comet's cached version could not be read. Comet manages these libraries automatically.".into(),
    };
    let peer_update = match releases.map(|updates| &updates.peers) {
        None => "Checking the published version…".into(),
        Some(Err(_)) => {
            "Published version check unavailable; use Check for updates to retry.".into()
        }
        Some(Ok(release)) => match peer.as_ref().ok().and_then(|version| version.as_deref()) {
            Some(installed)
                if components::newer_version(&release.version, installed).unwrap_or(false) =>
            {
                format!(
                    "Published version: {}. Comet applies its own update checks when it runs.",
                    release.version
                )
            }
            Some(installed) => version_message(&release.version, Some(installed)),
            None => format!(
                "Published version: {}. No manual installation is needed.",
                release.version
            ),
        },
    };
    ComponentView {
        installation,
        updates,
        peers: format!("{local_peers}\n{peer_update}"),
        action,
    }
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
    group.set_title("Comet");
    group.set_description(Some("Comet provides GOG authentication, achievements and statistics for supported games. Unmodified Comet automatically downloads and updates its GOG peer libraries when used, according to its own update checks. Ludomere's startup and manual checks read version information only. Installing or updating Comet itself requires your confirmation."));
    let installation = adw::ActionRow::builder()
        .title("Local installation")
        .subtitle("Checking the local Comet installation…")
        .build();
    installation.set_subtitle_selectable(true);
    installation.set_use_markup(false);
    let release = adw::ActionRow::builder()
        .title("Official release")
        .subtitle("Checking official releases…")
        .build();
    release.set_subtitle_selectable(true);
    release.set_use_markup(false);
    group.add(&installation);
    group.add(&release);
    let check = gtk::Button::with_label("Check for updates");
    let update = gtk::Button::with_label("Update Comet…");
    update.set_visible(false);
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
    cancel.set_visible(false);
    group.add(&cancel);
    page.add(&group);
    let peer_group = adw::PreferencesGroup::new();
    peer_group.set_title("GOG peer libraries");
    let peers = adw::ActionRow::builder()
        .title("Managed automatically by Comet")
        .subtitle("Reading Comet's cached version…")
        .build();
    peers.set_subtitle_selectable(true);
    peers.set_use_markup(false);
    peer_group.add(&peers);
    page.add(&peer_group);
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
            let Some(snapshot) = snapshot else { return; };
            let Some(Ok(Some(candidate))) = &snapshot.candidate else { return; };
            let candidate = candidate.clone();
            let installing = snapshot.comet_version.is_err();
            let dialog = adw::AlertDialog::builder().heading(if installing { "Install Comet?" } else { "Update Comet?" })
                .body(format!("Download official Comet {} ({}) into Ludomere's application data? {}Comet will manage its peer-library downloads and updates automatically when used.", candidate.version, human_size(candidate.download_bytes), if installing { "" } else { "Your existing helper remains available if the download fails. " }))
                .build();
            dialog.add_responses(&[("cancel", "Not now"), ("download", "Download")]);
            dialog.set_close_response("cancel");
            let running = running.clone(); let progress = progress.clone();
            let message = message.clone(); let cancel_button = cancel_button.clone();
            dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response != "download" || running.borrow().is_some() { return; }
                let cancelled = Arc::new(AtomicBool::new(false));
                *running.borrow_mut() = Some(cancelled.clone());
                cancel_button.set_sensitive(true); cancel_button.set_visible(true); progress.set_visible(true); progress.set_fraction(0.0);
                message.set_label(if installing { "Preparing Comet installation…" } else { "Preparing Comet update…" });
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
                        Err(mpsc::TryRecvError::Disconnected) => Err("Comet download stopped unexpectedly. Try again.".into()),
                    };
                    running.borrow_mut().take(); cancel_button.set_sensitive(false); cancel_button.set_visible(false); progress.set_visible(false);
                    match result {
                        Ok(()) => { message.set_label("Comet download complete."); check_updates(); }
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
            let view = component_view(
                &snapshot.comet_version,
                &snapshot.peer_version,
                snapshot.updates.as_ref(),
                snapshot.candidate.as_ref().map(|candidate| {
                    candidate
                        .as_ref()
                        .map(|candidate| {
                            candidate
                                .as_ref()
                                .map(|candidate| candidate.version.as_str())
                        })
                        .map_err(String::as_str)
                }),
            );
            installation.set_subtitle(&view.installation);
            release.set_subtitle(&if checking && snapshot.candidate.is_some() {
                format!("{}\nChecking for updates…", view.updates)
            } else {
                view.updates
            });
            peers.set_subtitle(&view.peers);
            update.set_visible(view.action.is_some());
            if let Some(label) = view.action {
                update.set_label(label);
            }
            update.set_sensitive(!busy && !checking && view.action.is_some());
        } else {
            update.set_sensitive(false);
        }
        glib::ControlFlow::Continue
    });
    page
}

#[cfg(test)]
mod tests {
    use super::{component_view, components, version_message};

    #[test]
    fn local_installation_remains_clear_while_remote_checks_wait_or_fail() {
        let installed = Ok("0.3.2".into());
        let pending = component_view(&installed, &Ok(None), None, None);
        assert_eq!(
            pending.installation,
            "Installed and verified — version 0.3.2"
        );
        assert!(pending.updates.contains("Checking"));
        assert!(pending.action.is_none());
        let releases = components::ComponentUpdates {
            comet: Err("offline".into()),
            peers: Err("offline".into()),
        };
        let offline = component_view(
            &installed,
            &Ok(Some("1.2.33.1".into())),
            Some(&releases),
            Some(Err("offline")),
        );
        assert_eq!(offline.installation, pending.installation);
        assert!(offline.updates.contains("retry"));
        assert!(
            offline
                .peers
                .contains("Comet last reported version 1.2.33.1")
        );
        assert!(offline.peers.contains("unavailable"));
        assert!(offline.action.is_none());
    }

    #[test]
    fn install_update_and_repair_guidance_follow_local_and_candidate_evidence() {
        let installed = Ok("0.3.2".into());
        let unavailable = Err("Permission denied".into());
        let update = component_view(&installed, &Ok(None), None, Some(Ok(Some("0.4.0"))));
        assert_eq!(update.action, Some("Update Comet…"));
        let install = component_view(&unavailable, &Ok(None), None, Some(Ok(Some("0.4.0"))));
        assert_eq!(install.action, Some("Install Comet…"));
        assert!(install.installation.starts_with("Could not verify"));
        assert!(!install.installation.contains("Not installed"));
        let no_candidate = component_view(&unavailable, &Ok(None), None, Some(Ok(None)));
        assert!(no_candidate.action.is_none());
        assert!(no_candidate.updates.contains("Reinstall Ludomere"));
    }

    #[test]
    fn current_comet_and_automatic_peers_do_not_offer_manual_peer_installation() {
        let releases = components::ComponentUpdates {
            comet: Ok(components::ComponentRelease {
                version: "0.3.2".into(),
            }),
            peers: Ok(components::ComponentRelease {
                version: "1.2.33.1".into(),
            }),
        };
        let current = component_view(
            &Ok("0.3.2".into()),
            &Ok(None),
            Some(&releases),
            Some(Ok(None)),
        );
        assert_eq!(current.updates, "Up to date: 0.3.2");
        assert!(current.action.is_none());
        assert!(current.peers.contains("No cached version reported"));
        assert!(current.peers.contains("No manual installation is needed"));
        let older = component_view(
            &Ok("0.3.2".into()),
            &Ok(Some("1.2.32.0".into())),
            Some(&releases),
            Some(Ok(None)),
        );
        assert!(
            older
                .peers
                .contains("Comet applies its own update checks when it runs")
        );
        assert!(older.action.is_none());
    }

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
