//! CLI diagnostic, not part of the app — confirms
//! `Coordinator::trust_manually` actually works against a real
//! `entanglo-macos` peer on the LAN, without needing to click any GTK
//! button or have a display at all. Kept (not deleted after first use)
//! because it's the fastest way to re-check real interop against a
//! live Mac/Android/Windows peer after touching `net::session` or
//! `net::coordinator` — no GTK build, no X11, no clicking required.
//!
//! Discovers peers, dials the first one whose mDNS name contains
//! "mac" (case-insensitive), waits for `PeerIdentified`, calls
//! `trust_manually`, and reports whether `Trusted` and a subsequent
//! `Heartbeat` fire.
//!
//! Run with: cargo run -p entanglo-core --example manual_trust_smoke_test

use std::sync::Arc;
use std::time::Duration;

use entanglo_core::net::{
    trust_store, Coordinator, CoordinatorEvent, DiscoveryService, TrustStore,
};
use entanglo_core::protocol::payloads::HelloPayload;

#[tokio::main]
async fn main() {
    entanglo_core::logging::init();

    let local_device_id =
        trust_store::local_device_id().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let hello = HelloPayload {
        device_name: "Smoke Test".to_string(),
        device_model: "Linux".to_string(),
        app_version: "0.0.0-smoketest".to_string(),
        roles: vec!["controller".to_string(), "receiver".to_string()],
        platform: Some("Linux".to_string()),
    };
    let trust_store = Arc::new(tokio::sync::Mutex::new(
        TrustStore::load().await.expect("load trust store"),
    ));
    let (coordinator, mut events) = Coordinator::new(local_device_id, hello.clone(), trust_store);

    let port = coordinator
        .listen("0.0.0.0:0")
        .await
        .expect("bind listener");
    println!("listening on port {port}");

    let discovery = DiscoveryService::new().expect("mdns daemon");
    discovery
        .advertise(&hello.device_name, port)
        .expect("advertise");
    let peer_rx = discovery.browse().expect("browse");

    let coordinator_for_dial = Arc::clone(&coordinator);
    tokio::task::spawn_blocking(move || {
        let mut dialed = std::collections::HashSet::new();
        while let Ok(peer) = peer_rx.recv() {
            if !peer.device_name.to_lowercase().contains("mac") {
                continue;
            }
            if !dialed.insert(peer.device_name.clone()) {
                continue;
            }
            let Ok(ip) = peer.host.parse::<std::net::IpAddr>() else {
                dialed.remove(&peer.device_name);
                continue;
            };
            let addr = std::net::SocketAddr::new(ip, peer.port);
            println!("dialing {} at {addr}", peer.device_name);
            let coordinator = Arc::clone(&coordinator_for_dial);
            let name = peer.device_name.clone();
            let failed = tokio::runtime::Handle::current().block_on(async move {
                if let Err(e) = coordinator.dial(addr).await {
                    println!("dial failed: {e}");
                    true
                } else {
                    false
                }
            });
            if failed {
                dialed.remove(&name);
            }
        }
    });

    let mut trust_requested = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("TIMED OUT waiting for the Mac peer");
            return;
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(CoordinatorEvent::PeerIdentified { conn_id, device_id })) => {
                println!("PeerIdentified: conn_id={conn_id} device_id={device_id}");
                if !trust_requested {
                    trust_requested = true;
                    println!("calling trust_manually({conn_id})");
                    coordinator.trust_manually(conn_id).await;
                }
            }
            Ok(Some(CoordinatorEvent::Trusted { conn_id, device_id })) => {
                println!("Trusted! conn_id={conn_id} device_id={device_id}");
                break;
            }
            Ok(Some(other)) => println!("event: {other:?}"),
            Ok(None) => {
                println!("event stream closed");
                return;
            }
            Err(_) => unreachable!("checked remaining above"),
        }
    }

    println!("SUCCESS — waiting 5s to observe a heartbeat...");
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(CoordinatorEvent::Heartbeat { rtt_ms, .. }) = events.recv().await {
                println!("heartbeat rtt_ms={rtt_ms:?}");
                break;
            }
        }
    })
    .await;
}
