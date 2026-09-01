use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.8. Sent by the receiver after the last chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAckPayload {
    pub transfer_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}
