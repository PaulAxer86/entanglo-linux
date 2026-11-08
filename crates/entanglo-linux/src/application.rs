use adw::prelude::*;
use adw::Application;
use entanglo_core::net::CoordinatorEvent;

use crate::app_state::Backend;

pub fn configure(
    app: &Application,
    backend: Backend,
    events_rx: async_channel::Receiver<CoordinatorEvent>,
) {
    let coordinator = backend.coordinator.clone();
    let discovery = backend.discovery.clone();
    app.connect_shutdown(move |_| {
        coordinator.emergency_stop();
        if let Some(discovery) = &discovery { discovery.shutdown(); }
        tracing::info!("application shutdown: input sharing stopped");
    });
    let quit = gtk::gio::SimpleAction::new("quit", None);
    let app_weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = app_weak.upgrade() { app.quit(); }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Primary>q"]);
    app.connect_activate(move |app| {
        gtk::Window::set_default_icon_name("com.paoloasara.Entanglo");
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let window = crate::window::build(app, backend.clone(), events_rx.clone());
        let start_hidden = crate::startup::take_start_hidden();
        if !start_hidden {
            window.present();
        }
        crate::tray::start(app, &window, &backend.handle, start_hidden);
    });
}
