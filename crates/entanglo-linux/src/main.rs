mod application;
mod pages;
mod window;

use adw::prelude::*;

const APP_ID: &str = "com.paoloasara.Entanglo";

fn main() -> glib::ExitCode {
    entanglo_core::logging::init();
    let app = application::build(APP_ID);
    app.run()
}
