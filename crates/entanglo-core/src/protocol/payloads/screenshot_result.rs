use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.10. Requester MUST enforce its own deadline
/// (Mac uses 8 s) — critical on Linux since the capture path waits on
/// an `xdg-desktop-portal` consent dialog that may never be answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResultPayload {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
