use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.6. Sent receiver -> controller when the
/// receiver detects local input; the controller must immediately
/// stop forwarding `inputEvent`s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseControlPayload {
    pub reason: String,
}
