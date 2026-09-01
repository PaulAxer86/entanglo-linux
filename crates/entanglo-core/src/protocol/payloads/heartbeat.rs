use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.4. Sent every 1.0 s while a transport is
/// open. `sent_at_ms`/`echo_sent_at_ms` are opaque monotonic-clock
/// readings — never interpret them, only echo/diff against your own
/// clock (`std::time::Instant`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatPayload {
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo_sent_at_ms: Option<f64>,
}
