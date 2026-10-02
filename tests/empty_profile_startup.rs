use adw::prelude::*;
use gtk::glib;
use std::{
    cell::Cell,
    fs,
    os::unix::fs::PermissionsExt,
    rc::Rc,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires an isolated display and private D-Bus session; run under Xvfb and dbus-run-session"]
fn empty_profile_reaches_empty_library_page() {
    let profile = tempfile::tempdir().unwrap();
    // This integration executable has one test. Isolate all application state before GTK or
    // application workers start; the caller supplies a disposable display and private D-Bus.
    for (name, directory) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_RUNTIME_DIR", "runtime"),
    ] {
        let path = profile.path().join(directory);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        unsafe { std::env::set_var(name, path) };
    }
    assert!(
        std::process::Command::new("dbus-update-activation-environment")
            .args([
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR"
            ])
            .status()
            .expect("GUI regression requires a private D-Bus session")
            .success()
    );
    adw::init().expect("GUI regression requires a working display");
    let app = ludomere::application::build();
    let reached_empty = Rc::new(Cell::new(false));
    let observed = reached_empty.clone();
    let app_for_poll = app.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if let Some(window) = app_for_poll.active_window() {
            let mut widgets = vec![window.upcast::<gtk::Widget>()];
            while let Some(widget) = widgets.pop() {
                if let Some(stack) = widget.downcast_ref::<gtk::Stack>()
                    && stack.visible_child_name().as_deref() == Some("empty")
                {
                    observed.set(true);
                    app_for_poll.quit();
                    return glib::ControlFlow::Break;
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    widgets.push(widget);
                }
            }
        }
        if Instant::now() >= deadline {
            app_for_poll.quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    let result = app.run_with_args(&["ludomere-empty-profile-test"]);
    assert_eq!(result, glib::ExitCode::SUCCESS);
    assert!(
        reached_empty.get(),
        "fresh profile never reached its empty library page"
    );
}
