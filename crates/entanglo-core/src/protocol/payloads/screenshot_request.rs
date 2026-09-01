use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRequestPayload {
    pub request_id: String,
}
