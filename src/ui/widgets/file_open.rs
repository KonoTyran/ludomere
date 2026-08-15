use adw::prelude::*;
use gtk::gio;
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
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(Some(parent), gio::Cancellable::NONE, move |result| {
        if let Err(error) = result {
            tracing::warn!(%error, "could not open {description}");
        }
    });
}
