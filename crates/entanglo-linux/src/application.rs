use adw::prelude::*;
use adw::Application;
use entanglo_core::net::CoordinatorEvent;

use crate::app_state::Backend;

pub fn build(
    app_id: &str,
    backend: Backend,
    events_rx: async_channel::Receiver<CoordinatorEvent>,
) -> Application {
    let app = Application::builder().application_id(app_id).build();
    app.connect_activate(move |app| {
        let window = crate::window::build(app, backend.clone(), events_rx.clone());
        window.present();
    });
    app
}
