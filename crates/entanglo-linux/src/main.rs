mod app_state;
mod application;
mod clipboard;
mod edge;
mod pages;
mod sound;
mod state;
mod startup;
mod window;

use adw::prelude::*;

const APP_ID: &str = "com.paoloasara.Entanglo";

fn main() -> glib::ExitCode {
    startup::prepare();
    let app = adw::Application::builder().application_id(APP_ID).build();
    if let Err(error) = app.register(None::<&gtk::gio::Cancellable>) {
        eprintln!("Could not register Entanglo: {error}");
        return glib::ExitCode::FAILURE;
    }
    if app.is_remote() {
        app.activate();
        return glib::ExitCode::SUCCESS;
    }
    startup::prepare_logs();
    let log_buffer = entanglo_core::logging::init();
    let (backend, events_rx) = app_state::start(log_buffer);
    application::configure(&app, backend, events_rx);
    app.run()
}
