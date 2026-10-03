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
    let path = path.to_path_buf();
    let parent = parent.as_ref().downgrade();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
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
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let Some(parent) = parent.upgrade() else {
            return glib::ControlFlow::Break;
        };
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                launcher.launch(
                    Some(&parent.clone()),
                    gio::Cancellable::NONE,
                    move |result| {
                        report_launch_result(&parent, description, result);
                    },
                );
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
