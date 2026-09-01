mod app_state;
mod application;
mod pages;
mod state;
mod window;

use adw::prelude::*;

const APP_ID: &str = "com.paoloasara.Entanglo";

fn main() -> glib::ExitCode {
    entanglo_core::logging::init();
    let (backend, events_rx) = app_state::start();
    let app = application::build(APP_ID, backend, events_rx);
    app.run()
}
