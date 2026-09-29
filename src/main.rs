use adw::prelude::*;
use ludomere::{application, gog, profile_reset};

fn main() -> gtk::glib::ExitCode {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_some_and(|argument| argument == profile_reset::CLEANUP_ARGUMENT)
    {
        return match profile_reset::complete(&arguments) {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => recovery_window(format!("The profile reset could not finish: {error:#}")),
        };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ludomere=info".into()),
        )
        .init();

    if let Err(error) = profile_reset::initialize() {
        return recovery_window(format!("{error:#}"));
    }
    if std::env::args().any(|argument| argument == "--audit-gog-sources") {
        return match gog::audit::run() {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("GOG source audit failed: {error:#}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    let app = application::build();
    let result = app.run();
    match profile_reset::finish_application() {
        Ok(()) => result,
        Err(error) => recovery_window(format!("{error:#}")),
    }
}

fn recovery_window(message: String) -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("io.github.legendarylinux.ludomere.ProfileReset")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Ludomere — profile reset incomplete")
            .default_width(560)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let label = gtk::Label::new(Some(&format!(
            "{message}\n\nNormal library controls remain unavailable until this reset is resolved. Retry the reset, or close Ludomere and correct the reported problem. Installed payloads are preserved."
        )));
        label.set_wrap(true);
        label.set_selectable(true);
        content.append(&label);
        let retry = gtk::Button::with_label("Retry reset");
        let close = gtk::Button::with_label("Close");
        content.append(&retry);
        content.append(&close);
        let app_close = app.clone();
        close.connect_clicked(move |_| app_close.quit());
        let app_retry = app.clone();
        retry.connect_clicked(move |button| {
            button.set_sensitive(false);
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(profile_reset::retry_pending().map_err(|error| format!("{error:#}")));
            });
            let button = button.clone();
            let label = label.clone();
            let app = app_retry.clone();
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match receiver.try_recv() {
                    Ok(Ok(())) => app.quit(),
                    Ok(Err(error)) => {
                        label.set_text(&format!("Reset remains incomplete: {error}"));
                        button.set_sensitive(true);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return gtk::glib::ControlFlow::Continue,
                    Err(_) => {
                        label.set_text("Reset preparation stopped. Close Ludomere and retry.");
                        button.set_sensitive(true);
                    }
                }
                gtk::glib::ControlFlow::Break
            });
        });
        window.set_content(Some(&content));
        window.present();
    });
    app.run_with_args::<&str>(&[]);
    if let Err(error) = profile_reset::finish_application() {
        eprintln!("Profile reset remains incomplete: {error:#}");
    }
    gtk::glib::ExitCode::FAILURE
}
