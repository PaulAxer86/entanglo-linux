//! Bootstraps the async backend (Coordinator + listener + mDNS
//! discovery + input capture/injection) on a dedicated OS thread with
//! its own Tokio runtime, and bridges its `CoordinatorEvent` stream
//! to the GTK main thread via `async-channel` — GTK objects are
//! single-threaded, so nothing from `entanglo-core`'s async world
//! touches a widget directly; the GTK side (`window.rs`,
//! `pages/*.rs`) only ever reads `CoordinatorEvent`s off this channel
//! inside a `glib::spawn_future_local` loop running on the main
//! context.

use std::sync::Arc;

use entanglo_core::logging::LogBuffer;
use entanglo_core::net::{
    trust_store, Coordinator, CoordinatorEvent, DiscoveryService, TrustStore,
};
use entanglo_core::protocol::payloads::HelloPayload;
use x11rb::connection::Connection;

/// Handle the GTK side holds. `coordinator` and `handle` are cheap to
/// clone (an `Arc` and a `tokio::runtime::Handle` respectively) so UI
/// callbacks can freely call coordinator methods by scheduling them
/// onto the backend runtime with `handle.spawn(...)` — no `.await`
/// needed on the GTK thread itself.
#[derive(Clone)]
pub struct Backend {
    pub coordinator: Arc<Coordinator>,
    pub handle: tokio::runtime::Handle,
    pub local_device_id: String,
    pub local_hello: HelloPayload,
    pub log_buffer: LogBuffer,
    pub discovery: Option<Arc<DiscoveryService>>,
}

struct Ready {
    discovery: Option<Arc<DiscoveryService>>,
    coordinator: Arc<Coordinator>,
    handle: tokio::runtime::Handle,
    local_device_id: String,
}

/// Starts the backend thread and blocks briefly until the coordinator
/// is ready to use — meant to be called once, early in `main()`,
/// before the GTK main loop starts.
pub fn start(log_buffer: LogBuffer) -> (Backend, async_channel::Receiver<CoordinatorEvent>) {
    let screen_size = local_screen_size();
    let local_hello = HelloPayload {
        device_name: local_device_name(),
        device_model: "Linux".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        roles: if std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_some() {
            vec!["receiver".to_string()]
        } else {
            vec!["controller".to_string(), "receiver".to_string()]
        },
        platform: Some("Linux".to_string()),
        screen_width: screen_size.map(|size| size.0),
        screen_height: screen_size.map(|size| size.1),
    };

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
    let (ui_tx, ui_rx) = async_channel::unbounded();

    let hello_for_thread = local_hello.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("failed to start Tokio runtime");
        let handle = runtime.handle().clone();
        runtime.block_on(async move {
            let local_device_id = trust_store::local_device_id()
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "could not persist a stable device id, using a per-run random one");
                    uuid::Uuid::new_v4().to_string()
                });

            let trust_store = match TrustStore::load().await {
                Ok(store) => store,
                Err(e) => {
                    tracing::error!(error = %e, "failed to load trust store, cannot continue");
                    return;
                }
            };
            let trust_store = Arc::new(tokio::sync::Mutex::new(trust_store));

            let (coordinator, mut events) =
                Coordinator::new(local_device_id.clone(), hello_for_thread.clone(), trust_store);

            // Debian's dual-stack listener also accepts IPv4 peers and
            // permits direct Ethernet peers with only link-local IPv6.
            let listener = match coordinator.listen("[::]:55555").await {
                Ok(port) => Ok(port),
                Err(error) => {
                    tracing::warn!(%error, "IPv6 listener unavailable, falling back to IPv4");
                    coordinator.listen("0.0.0.0:55555").await
                }
            };
            let port = match listener {
                Ok(port) => port,
                Err(e) => {
                    tracing::error!(error = %e, "failed to bind TCP listener, cannot accept peers");
                    return;
                }
            };
            tracing::info!(port, "listening for Entanglo peers");

            if let Err(e) = coordinator.enable_receiver().await {
                tracing::warn!(error = %e, "could not open /dev/uinput — this device cannot act as a receiver until the 'input' group is set up (see ROADMAP.md Phase 1)");
            }
            if std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_none() {
                if let Err(e) = coordinator.enable_controller().await {
                    tracing::warn!(error = %e, "could not open input devices — this device cannot act as a controller (needs 'input' group membership)");
                }
            } else {
                tracing::info!("receiver-only mode: controller capture disabled");
            }
            let discovery = spawn_discovery(
                Arc::clone(&coordinator),
                hello_for_thread.device_name.clone(),
                port,
                local_device_id.clone(),
            );

            let _ = ready_tx.send(Ready {
                discovery,
                coordinator: Arc::clone(&coordinator),
                handle,
                local_device_id,
            });

            while let Some(event) = events.recv().await {
                if ui_tx.send(event).await.is_err() {
                    break; // GTK side went away.
                }
            }
        });
    });

    let ready = ready_rx
        .recv()
        .expect("backend thread exited before it became ready");

    (
        Backend {
            coordinator: ready.coordinator,
            handle: ready.handle,
            local_device_id: ready.local_device_id,
            local_hello,
            log_buffer,
            discovery: ready.discovery,
        },
        ui_rx,
    )
}

fn local_screen_size() -> Option<(u32, u32)> {
    let (conn, screen_num) = x11rb::rust_connection::RustConnection::connect(None).ok()?;
    let screen = conn.setup().roots.get(screen_num)?;
    Some((
        u32::from(screen.width_in_pixels),
        u32::from(screen.height_in_pixels),
    ))
}

fn local_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Debian Desktop".to_string())
}

/// Best-effort mDNS advertise + browse-and-auto-dial. Failure here
/// (no multicast route, no permission to bind UDP 5353, a sandboxed
/// environment) is logged and otherwise ignored — manual-IP pairing
/// is Phase 2 UI work (`entanglo-macos`'s Network page equivalent),
/// so for now a broken discovery path just means "no peers show up
/// automatically," not "the app can't start."
fn spawn_discovery(coordinator: Arc<Coordinator>, device_name: String, port: u16, local_device_id: String) -> Option<Arc<DiscoveryService>> {
    let discovery = match DiscoveryService::new() {
        Ok(d) => Arc::new(d),
        Err(e) => {
            tracing::warn!(error = %e, "mDNS unavailable, discovery disabled");
            return None;
        }
    };
    if let Err(e) = discovery.advertise_identity(&device_name, port, Some(&local_device_id)) {
        tracing::warn!(error = %e, "mDNS advertise failed");
    }
    let receiver_only = std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_some();
    let events = match discovery.browse_events() {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "mDNS browse failed");
            return Some(discovery);
        }
    };
    let candidates = Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, entanglo_core::net::DiscoveredPeer>::new(),
    ));
    {
        let candidates = Arc::clone(&candidates);
        let coordinator = Arc::clone(&coordinator);
        let local_name = format!("{device_name}.{}", entanglo_core::net::SERVICE_TYPE);
        tokio::task::spawn_blocking(move || {
            while let Ok(event) = events.recv() {
                if matches!(&event, entanglo_core::net::DiscoveryEvent::Resolved(peer)
                    if peer.device_name == local_name || peer.device_id.as_ref() == Some(&local_device_id)) { continue; }
                {
                    let mut cache = candidates.lock().unwrap();
                    if let entanglo_core::net::DiscoveryEvent::Resolved(peer) = &event {
                        tracing::debug!(name = %peer.device_name, interface = ?peer.interface, host = %peer.host, "Bonjour path discovered");
                        cache.insert(peer.cache_key(), peer.clone());
                    } else { cache.retain(|_, peer| !event.removes(peer)); }
                }
                coordinator.report_discovery(event);
            }
        });
    }
    let reconnect_discovery = Arc::clone(&discovery);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
        interval.tick().await;
        let mut first_contacts = std::collections::HashSet::new();
        let mut last_refresh = std::time::Instant::now();
        loop {
            // Collect announcements briefly: Ethernet and Wi-Fi results for
            // the same UUID arrive separately and in no guaranteed order.
            interval.tick().await;
            let mut peers: Vec<_> = candidates.lock().unwrap().values().cloned().collect();
            peers.sort_by_key(|peer| (peer.priority(), !peer.endpoint().is_some_and(|address| address.is_ipv4()), peer.cache_key()));
            for peer in peers {
                let result = tokio::time::timeout(std::time::Duration::from_secs(3), coordinator.connect_discovered(&peer)).await;
                if !matches!(result, Ok(Ok(()))) {
                    tracing::debug!(?result, name = %peer.device_name, interface = ?peer.interface, "trusted path connection attempt finished");
                }
                if !receiver_only && first_contacts.insert(peer.device_name.clone()) {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(3), coordinator.connect_first_contact(&peer)).await;
                    if !matches!(result, Ok(Ok(()))) { first_contacts.remove(&peer.device_name); }
                }
            }
            if receiver_only && last_refresh.elapsed() >= std::time::Duration::from_secs(10)
                && coordinator.needs_reconnect().await {
                let service = Arc::clone(&reconnect_discovery);
                if let Err(error) = tokio::task::spawn_blocking(move || service.refresh()).await {
                    tracing::warn!(%error, "Bonjour reconnect refresh failed");
                }
                last_refresh = std::time::Instant::now();
            }
        }
    });
    Some(discovery)
}
