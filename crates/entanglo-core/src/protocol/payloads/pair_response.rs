use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponsePayload {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}
