use adw::prelude::*;
use ludomere::{application, gog};

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ludomere=info".into()),
        )
        .init();

    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "--audit-gog-sources")
    {
        return match gog::audit::run() {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("GOG source audit failed: {error:#}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    if arguments
        .iter()
        .any(|argument| argument == "--audit-gog-capabilities")
    {
        return match gog::capability_audit::run() {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("GOG capability audit failed: {error:#}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    if arguments
        .iter()
        .any(|argument| argument == "--validate-gog-write")
    {
        return match gog::capability_audit::validate_write(&arguments) {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("GOG write validation failed: {error:#}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    let app = application::build();
    app.run()
}
