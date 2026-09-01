//! Dial reconciler + live transport-per-peer map. Mirrors
//! `entanglo-macos`'s `ConnectionCoordinator` and
//! `entanglo-windows`'s `ConnectionCoordinator.cs`: a device may hold
//! simultaneous outbound (self-initiated) and inbound (peer-initiated)
//! connections to the same peer. Whichever direction is currently
//! sending `inputEvent`s is "the controller" for that link.
//!
//! This is a skeleton — the real implementation needs a `HashMap`
//! keyed by trusted device ID, safety-gate checks (trusted +
//! heartbeat-alive + emergency-stop-off, matching the gates in
//! `entanglo-macos/docs/ARCHITECTURE.md`) before dispatching any
//! `inputEvent` to `crate::input::inject`, and reconnect-on-drop
//! logic. Fill in as Phase 1 work per `ROADMAP.md`.

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
