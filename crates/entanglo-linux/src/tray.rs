//! System tray icon with Show and Quit actions, using the desktop's
//! StatusNotifier service. Desktops without one simply get no icon, and
//! closing the window then quits as usual.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray, TrayMethods};

const ICON_NAME: &str = "com.paoloasara.Entanglo";
/// At login the tray host can appear shortly after this app starts.
const REGISTER_ATTEMPTS: u32 = 15;
const REGISTER_RETRY_DELAY: Duration = Duration::from_secs(2);

static TRAY_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether a hidden window can still be reached and the app quit from the tray.
pub fn is_active() -> bool {
    TRAY_ACTIVE.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
enum TrayAction {
    Show,
    Quit,
}

struct EntangloTray {
    actions: async_channel::Sender<TrayAction>,
}

impl EntangloTray {
    fn send(&self, action: TrayAction) {
        let _ = self.actions.try_send(action);
    }
}

impl Tray for EntangloTray {
    fn id(&self) -> String {
        ICON_NAME.into()
    }

    fn title(&self) -> String {
        "Entanglo".into()
    }

    fn icon_name(&self) -> String {
        ICON_NAME.into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Entanglo".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::Show)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit Entanglo".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Registers the tray icon and routes its actions to the application. A window
/// that started hidden is shown if no tray ever becomes available.
pub fn start(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    handle: &tokio::runtime::Handle,
    started_hidden: bool,
) {
    let (action_tx, action_rx) = async_channel::unbounded();
    let (failed_tx, failed_rx) = async_channel::bounded::<()>(1);

    handle.spawn(async move {
        for attempt in 1..=REGISTER_ATTEMPTS {
            match (EntangloTray { actions: action_tx.clone() }).spawn().await {
                Ok(tray_handle) => {
                    TRAY_ACTIVE.store(true, Ordering::Relaxed);
                    tracing::info!("system tray icon registered");
                    // Holding the handle for the app's lifetime keeps the icon.
                    std::future::pending::<()>().await;
                    drop(tray_handle);
                }
                Err(error) => {
                    tracing::debug!(%error, attempt, "system tray not available yet");
                    tokio::time::sleep(REGISTER_RETRY_DELAY).await;
                }
            }
        }
        tracing::info!("no system tray available");
        let _ = failed_tx.send(()).await;
    });

    if started_hidden {
        let window = window.clone();
        glib::spawn_future_local(async move {
            if failed_rx.recv().await.is_ok() {
                window.present();
            }
        });
    }

    let window = window.clone();
    let app = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(action) = action_rx.recv().await {
            match action {
                TrayAction::Show => window.present(),
                TrayAction::Quit => app.quit(),
            }
        }
    });
}
