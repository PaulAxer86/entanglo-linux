use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.2. `pin_hash` is reserved — send `""` in v1;
/// responders MUST NOT reject an empty value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequestPayload {
    pub requester_device_id: String,
    pub requester_device_name: String,
    pub pin_hash: String,
}
