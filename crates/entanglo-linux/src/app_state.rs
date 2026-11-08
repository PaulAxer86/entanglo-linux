//! Bootstraps the async backend (Coordinator + listener + mDNS
//! discovery + input capture/injection) on a dedicated OS thread with
//! its own Tokio runtime, and bridges its `CoordinatorEvent` stream
//! to the GTK main thread via `async-channel` — GTK objects are
//! single-threaded, so nothing from `entanglo-core`'s async world
//! touches a widget directly; the GTK side (`window.rs`,
//! `pages/*.rs`) only ever reads `CoordinatorEvent`s off this channel
//! inside a `glib::spawn_future_local` loop running on the main
//! context.

use std::net::SocketAddr;
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
    pub interfaces: Vec<(std::net::IpAddr, String)>,
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
    let interfaces = local_interfaces();
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
            interfaces,
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
    // A receiver advertises itself; the controller opens the connection.
    // This avoids a second Mac-side device keyed by the Linux hello UUID
    // alongside the device keyed by the Bonjour service name.
    if std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_some() {
        tracing::info!("receiver-only discovery: advertising, automatic reconnect enabled");
        let candidates = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String, entanglo_core::net::DiscoveredPeer>::new()));
        if let Ok(events) = discovery.browse_events() {
            let coordinator = Arc::clone(&coordinator);
            let candidates = Arc::clone(&candidates);
            let local_name = format!("{device_name}.{}", entanglo_core::net::SERVICE_TYPE);
            tokio::task::spawn_blocking(move || {
                while let Ok(event) = events.recv() {
                    if matches!(&event, entanglo_core::net::DiscoveryEvent::Resolved(peer) if peer.device_name == local_name || peer.device_id.as_ref() == Some(&local_device_id)) { continue; }
                    if let entanglo_core::net::DiscoveryEvent::Resolved(peer) = &event {
                        tracing::info!(name=%peer.device_name, interface=?peer.interface, "Bonjour peer discovered");
                        let key=format!("{}|{}|{}",peer.device_name,peer.interface.as_deref().unwrap_or(""),peer.host);
                        candidates.lock().unwrap().insert(key,peer.clone());
                        let peer=peer.clone();let coordinator=Arc::clone(&coordinator);
                        tokio::runtime::Handle::current().spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                            match tokio::time::timeout(std::time::Duration::from_secs(3),coordinator.connect_discovered(&peer)).await {
                                Ok(Ok(()))=>{},result=>tracing::info!(?result,"discovered-controller reconnect attempt finished"),
                            }
                        });
                    }
                    if let entanglo_core::net::DiscoveryEvent::Removed(name)=&event { candidates.lock().unwrap().retain(|_,peer| &peer.device_name != name); }
                    coordinator.report_discovery(event);
                }
            });
        }
        let reconnect_discovery = Arc::clone(&discovery);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            interval.tick().await;
            loop {
                interval.tick().await;
                if coordinator.needs_reconnect().await {
                    let peers: Vec<_> = candidates.lock().unwrap().values().cloned().collect();
                    for peer in peers {
                        match tokio::time::timeout(std::time::Duration::from_secs(3),coordinator.connect_discovered(&peer)).await {
                            Ok(Ok(()))=>{},result=>tracing::info!(?result,"cached-controller reconnect attempt finished"),
                        }
                    }
                    let service = Arc::clone(&reconnect_discovery);
                    tracing::info!("no connected controller; refreshing Bonjour announcement");
                    match tokio::task::spawn_blocking(move || service.refresh()).await {
                        Ok(Ok(())) => {},
                        result => tracing::warn!(?result, "Bonjour reconnect refresh failed"),
                    }
                }
            }
        });
        return Some(discovery);
    }
    let peer_rx = match discovery.browse() {
        Ok(rx) => rx,
        Err(e) => {
            tracing::warn!(error = %e, "mDNS browse failed");
            return Some(discovery);
        }
    };

    // The single dial below covers first contact; this restores trusted
    // devices after a dropped link, which neither side would otherwise retry.
    let candidates = Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, entanglo_core::net::DiscoveredPeer>::new(),
    ));
    {
        let candidates = Arc::clone(&candidates);
        let coordinator = Arc::clone(&coordinator);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            interval.tick().await;
            loop {
                interval.tick().await;
                let peers: Vec<_> = candidates.lock().unwrap().values().cloned().collect();
                for peer in peers {
                    let attempt = coordinator.connect_discovered(&peer);
                    if let Ok(Err(error)) = tokio::time::timeout(std::time::Duration::from_secs(3), attempt).await {
                        tracing::debug!(%error, name = %peer.device_name, "trusted device reconnect failed");
                    }
                }
            }
        });
    }

    tokio::spawn(async move {
        let mut peer_rx = peer_rx;
        // mDNS keeps re-announcing every advertised service (itself
        // included — our own advertisement is just another
        // `_entanglo._tcp` service from the browser's point of view),
        // and each re-announcement can carry a different one of the
        // service's several addresses (LAN, VPN, link-local IPv6...).
        // Without this, a single real peer gets dialed over and over
        // on every rediscovery, opening a fresh connection — and, for
        // a peer that isn't trusted yet, sending it a fresh
        // `pairRequest` — every time. One dial attempt per advertised
        // service name for this process's lifetime is enough.
        let mut already_dialed = std::collections::HashSet::new();

        loop {
            let (peer_rx_back, message) = tokio::task::spawn_blocking(move || {
                let message = peer_rx.recv();
                (peer_rx, message)
            })
            .await
            .expect("discovery relay task panicked");
            peer_rx = peer_rx_back;

            let Ok(peer) = message else {
                tracing::debug!("mDNS daemon stopped, discovery ended");
                return;
            };
            candidates.lock().unwrap().insert(
                format!("{}|{}|{}", peer.device_name, peer.interface.as_deref().unwrap_or(""), peer.host),
                peer.clone(),
            );
            if !already_dialed.insert(peer.device_name.clone()) {
                continue; // already dialing or connected to this service
            }
            let Ok(ip) = peer.host.parse::<std::net::IpAddr>() else {
                tracing::debug!(host = %peer.host, "discovered peer host isn't a plain IP, skipping");
                already_dialed.remove(&peer.device_name);
                continue;
            };
            let addr = SocketAddr::new(ip, peer.port);
            tracing::info!(%addr, name = %peer.device_name, "discovered Entanglo peer, dialing");
            if let Err(e) = coordinator.dial(addr).await {
                tracing::warn!(error = %e, %addr, "dial failed");
                // Didn't actually reach the peer — allow a later
                // rediscovery (possibly with a better address, e.g.
                // a scoped IPv6 vs. a bare link-local one) to retry,
                // rather than giving up on this service permanently.
                already_dialed.remove(&peer.device_name);
            }
        }
    });
    Some(discovery)
}

fn local_interfaces() -> Vec<(std::net::IpAddr, String)> {
    let Ok(output) = std::process::Command::new("ip").args(["-o", "address", "show"]).output() else { return Vec::new(); };
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let name = fields.get(1)?.split('@').next()?.to_string();
        let address = fields.get(3)?.split('/').next()?.parse().ok()?;
        Some((address, name))
    }).collect()
}
