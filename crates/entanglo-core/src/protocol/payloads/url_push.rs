use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.9. Receiver MUST refuse non-`http(s)`/`file`
/// schemes unless explicitly enabled, and MUST launch via
/// `std::process::Command::new("xdg-open")` — never a shell string —
/// to avoid injection through a crafted URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlPushPayload {
    pub url: String,
}
