//! Dial reconciler + live session-per-peer map. Mirrors
//! `entanglo-macos`'s `ConnectionCoordinator` and
//! `entanglo-windows`'s `ConnectionCoordinator.cs`: a device may hold
//! simultaneous outbound (self-initiated) and inbound (peer-initiated)
//! connections to the same peer. Whichever direction is currently
//! sending `inputEvent`s is "the controller" for that link.
//!
//! The actual protocol engine (hello, pairing, heartbeat, event
//! forwarding) lives in `session.rs` and is complete + tested — see
//! `session::run_session`. This module is still a thin skeleton: it
//! needs to actually spawn a `run_session` task per accepted/dialed
//! `NetworkTransport`, merge each peer's outbound/inbound
//! `SessionEvent` streams into one view for the UI, and reconcile
//! "we have two connections to the same trusted peer" into a single
//! logical link. That's app-shell wiring (needs a running GTK event
//! loop / display to exercise end to end) rather than protocol logic,
//! so it's deferred past this pass — see `docs/DEV.md`.

use std::collections::HashMap;
use tokio::sync::Mutex;

use super::transport::NetworkTransport;

pub struct ConnectionCoordinator {
    peers: Mutex<HashMap<String, NetworkTransport>>,
}

impl ConnectionCoordinator {
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn insert(&self, device_id: String, transport: NetworkTransport) {
        self.peers.lock().await.insert(device_id, transport);
    }

    pub async fn remove(&self, device_id: &str) {
        self.peers.lock().await.remove(device_id);
    }
}

impl Default for ConnectionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
