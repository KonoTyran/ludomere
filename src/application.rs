use adw::prelude::*;

pub use crate::identity::APP_ID;

pub fn build() -> adw::Application {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        adw::init().expect("failed to initialize libadwaita");
        crate::ui::install_css();
    });
    app.connect_activate(crate::ui::build_window);
    app.connect_shutdown(|_| {
        crate::ui::shutdown_tray();
        if !crate::profile_reset::keeps_running_games() {
            crate::installation::stop_all_games();
        }
        crate::installation::shutdown();
        crate::download::shutdown();
        if let Err(error) = crate::auth::wait_for_sign_out() {
            tracing::warn!(%error, "sign-out cleanup did not finish before closing");
        }
    });
    app
}
