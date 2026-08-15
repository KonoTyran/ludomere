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
        crate::installation::stop_all_games();
        crate::installation::shutdown();
        crate::download::shutdown();
    });
    app
}
