use adw::prelude::*;
use gtk::{gio, glib};
use std::path::Path;

#[cfg(test)]
thread_local! {
    pub(crate) static DIRECTORY_LAUNCHES: std::cell::RefCell<Option<Vec<std::path::PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// Activate a directory already validated by the caller's worker, without another async hop.
pub(crate) fn launch_validated_directory(
    path: &Path,
    parent: &impl IsA<gtk::Window>,
    description: &'static str,
    is_current: impl Fn() -> bool + 'static,
) {
    if !parent.as_ref().is_visible() || !is_current() {
        return;
    }
    #[cfg(test)]
    if DIRECTORY_LAUNCHES.with_borrow_mut(|capture| {
        if let Some(paths) = capture {
            paths.push(path.to_path_buf());
            true
        } else {
            false
        }
    }) {
        return;
    }
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    let weak = parent.as_ref().downgrade();
    launcher.launch(Some(parent), gio::Cancellable::NONE, move |result| {
        if let Some(parent) = weak.upgrade()
            && parent.is_visible()
            && is_current()
        {
            report_launch_result(&parent, description, result);
        }
    });
}

pub(crate) fn report_launch_result(
    parent: &impl IsA<gtk::Window>,
    description: &str,
    result: Result<(), glib::Error>,
) {
    if let Err(error) = result {
        if error.matches(gtk::DialogError::Dismissed)
            || error.matches(gtk::DialogError::Cancelled)
            || error.matches(gio::IOErrorEnum::Cancelled)
        {
            return;
        }
        show_open_error(
            parent.as_ref(),
            &format!("Could not open {description}: {error}"),
        );
    }
}

fn show_open_error(parent: &gtk::Window, message: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading("Could not open the requested item")
        .body(message)
        .build();
    dialog.add_response("close", "Close");
    if parent.is_visible() {
        dialog.present(Some(parent));
    }
}

/// Opens a directory through the desktop's file-manager activation path.
///
/// Supplying the originating window lets GTK attach a Wayland/X11 activation
/// token to the request. Compositors may still refuse to raise an existing
/// file-manager window, but this is the portable foreground request.
pub(crate) fn open_directory(
    path: &Path,
    parent: &impl IsA<gtk::Window>,
    description: &'static str,
) {
    if !parent.as_ref().is_visible() {
        return;
    }
    let session = (crate::online::account_session(), crate::auth::session());
    let is_current = move || session == (crate::online::account_session(), crate::auth::session());
    let path = path.to_path_buf();
    let parent = parent.as_ref().downgrade();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if !is_current() {
            return;
        }
        #[cfg(test)]
        let completion = tests::COMPLETIONS.lock().unwrap().as_mut().map(|pending| {
            pending
                .pop_front()
                .expect("missing inert folder completion")
        });
        #[cfg(test)]
        if let Some(completion) = completion {
            assert_eq!(path, completion.path);
            completion.entered.send(()).unwrap();
            completion
                .release
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            if let Some(result) = completion.result {
                let _ = sender.send(result);
            }
            drop(sender);
            completion.finished.send(()).unwrap();
            return;
        }
        let result = crate::storage::read_config().and_then(|config| {
            if let Some(kind) = crate::config::LibraryKind::ALL.into_iter().find(|kind| {
                config
                    .libraries(*kind)
                    .iter()
                    .any(|library| path.starts_with(&library.path))
            }) {
                crate::storage::validate_path(&config, kind, &path)?;
            }
            Ok(path)
        });
        let _ = sender.send(result);
    });
    #[cfg(test)]
    let poll_lifetime = {
        tests::POLLERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tests::PollLifetime
    };
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        #[cfg(test)]
        let _ = &poll_lifetime;
        let Some(parent) = parent.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !parent.is_visible() || !is_current() {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                launch_validated_directory(&path, &parent, description, is_current);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                show_open_error(&parent, &error.to_string());
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                show_open_error(
                    &parent,
                    "Folder inspection stopped unexpectedly. Try opening the folder again.",
                );
                glib::ControlFlow::Break
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        collections::VecDeque,
        path::PathBuf,
        rc::Rc,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    pub(super) struct Completion {
        pub(super) path: PathBuf,
        pub(super) result: Option<anyhow::Result<PathBuf>>,
        pub(super) entered: mpsc::Sender<()>,
        pub(super) release: mpsc::Receiver<()>,
        pub(super) finished: mpsc::Sender<()>,
    }

    pub(super) static COMPLETIONS: Mutex<Option<VecDeque<Completion>>> = Mutex::new(None);
    pub(super) static POLLERS: AtomicUsize = AtomicUsize::new(0);
    pub(super) struct PollLifetime;

    impl Drop for PollLifetime {
        fn drop(&mut self) {
            POLLERS.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[track_caller]
    fn wait_until(check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !check() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(check());
    }

    fn completion(
        path: &Path,
        result: Option<anyhow::Result<PathBuf>>,
    ) -> (mpsc::Receiver<()>, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (entered, entry) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        COMPLETIONS
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push_back(Completion {
                path: path.to_owned(),
                result,
                entered,
                release: proceed,
                finished,
            });
        (entry, release, done)
    }

    #[test]
    #[ignore = "requires private p396 HOME/all XDG/TMP and GTK; inspection and file-manager activation are fail-closed inert"]
    fn directory_open_rejects_retired_origins_and_keeps_current_feedback() {
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
                    .starts_with("/tmp/ludomere-p396-")
            );
        }
        assert!(!crate::identity::database().exists());
        assert!(!crate::config::Config::path().exists());
        *COMPLETIONS.lock().unwrap() = Some(VecDeque::new());
        DIRECTORY_LAUNCHES.with_borrow_mut(|launches| *launches = Some(Vec::new()));
        let launches =
            || DIRECTORY_LAUNCHES.with_borrow(|launches| launches.as_ref().unwrap().len());
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.FolderOriginTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let path = std::env::temp_dir().join("inert-folder");
        let hidden = adw::ApplicationWindow::new(&app);
        open_directory(&path, &hidden, "folder");
        assert_eq!(POLLERS.load(Ordering::SeqCst), 0);
        assert!(COMPLETIONS.lock().unwrap().as_ref().unwrap().is_empty());
        hidden.destroy();

        // Raw current generations must still allow local folder opening while signed out.
        crate::auth::invalidate_session();
        assert!(!crate::auth::session_is_current(crate::auth::session()));
        let heartbeat = Rc::new(Cell::new(0));
        let clock = glib::timeout_add_local(Duration::from_millis(10), {
            let heartbeat = heartbeat.clone();
            move || {
                heartbeat.set(heartbeat.get() + 1);
                glib::ControlFlow::Continue
            }
        });
        for case in [
            "current",
            "error",
            "disconnect",
            "hidden",
            "closed",
            "auth",
            "account",
            "stale-error",
        ] {
            let window = adw::ApplicationWindow::new(&app);
            window.set_content(Some(&gtk::Label::new(Some("Synthetic folder request"))));
            window.present();
            wait_until(|| window.is_mapped());
            let before = launches();
            let outcome = match case {
                "error" | "stale-error" => {
                    Some(Err(anyhow::anyhow!("inert directory validation error")))
                }
                "disconnect" => None,
                _ => Some(Ok(path.clone())),
            };
            let (entered, release, done) = completion(&path, outcome);
            open_directory(&path, &window, "folder");
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            let ticks = heartbeat.get();
            wait_until(|| heartbeat.get() >= ticks + 8);
            assert_eq!(POLLERS.load(Ordering::SeqCst), 1);
            assert_eq!(launches(), before);
            assert!(window.visible_dialog().is_none());
            match case {
                "hidden" => window.set_visible(false),
                "closed" => window.close(),
                "auth" | "stale-error" => crate::auth::invalidate_session(),
                "account" => crate::online::invalidate_library_session(),
                _ => {}
            }
            let stale = matches!(
                case,
                "hidden" | "closed" | "auth" | "account" | "stale-error"
            );
            if stale {
                wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
            }
            release.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(5)).unwrap();
            wait_until(|| POLLERS.load(Ordering::SeqCst) == 0);
            assert_eq!(launches(), before + usize::from(case == "current"));
            if matches!(case, "error" | "disconnect") {
                let dialog = window
                    .visible_dialog()
                    .and_downcast::<adw::AlertDialog>()
                    .unwrap();
                assert!(dialog.body().contains(if case == "error" {
                    "inert directory validation error"
                } else {
                    "Folder inspection stopped unexpectedly"
                }));
                dialog.close();
                wait_until(|| window.visible_dialog().is_none());
            } else {
                assert!(window.visible_dialog().is_none());
            }
            // A closed but still strongly referenced GTK window must not qualify by weak upgrade.
            if matches!(case, "hidden" | "closed") {
                assert!(!window.is_visible());
            }
            window.destroy();
        }

        let window = adw::ApplicationWindow::new(&app);
        window.present();
        wait_until(|| window.is_mapped());
        let (entered, release, done) = completion(&path, Some(Ok(path.clone())));
        open_directory(&path, &window, "folder");
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let weak = window.downgrade();
        window.destroy();
        drop(window);
        wait_until(|| weak.upgrade().is_none() && POLLERS.load(Ordering::SeqCst) == 0);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(launches(), 1);
        clock.remove();
        assert!(COMPLETIONS.lock().unwrap().take().unwrap().is_empty());
        assert_eq!(
            DIRECTORY_LAUNCHES.with_borrow_mut(Option::take).unwrap(),
            [path]
        );
        assert!(!crate::identity::database().exists());
        assert!(!crate::config::Config::path().exists());
    }
}
