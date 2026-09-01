use adw::prelude::*;
use adw::Application;

pub fn build(app_id: &str) -> Application {
    let app = Application::builder().application_id(app_id).build();
    app.connect_activate(|app| {
        let window = crate::window::build(app);
        window.present();
    });
    app
}
