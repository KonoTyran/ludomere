use adw::prelude::*;
use gtk::{gio, glib};
use std::path::Path;

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
                launcher.launch(Some(&parent), gio::Cancellable::NONE, move |result| {
                    if let Err(error) = result {
                        tracing::warn!(%error, "could not open {description}");
                    }
                });
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                let dialog = adw::AlertDialog::builder()
                    .heading("Cannot open folder")
                    .body(error.to_string())
                    .build();
                dialog.add_response("close", "Close");
                if parent.is_visible() {
                    dialog.present(Some(&parent));
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
}
