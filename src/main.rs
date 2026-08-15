use adw::prelude::*;
use ludomere::{application, gog};

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ludomere=info".into()),
        )
        .init();

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
    app.run()
}
