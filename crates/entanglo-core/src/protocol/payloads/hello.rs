use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.1. Sent immediately after TCP connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloPayload {
    pub device_name: String,
    pub device_model: String,
    pub app_version: String,
    /// One or more of `"controller"` / `"receiver"`.
    pub roles: Vec<String>,
    /// Coarse OS identifier. This client MUST send `"Linux"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}
