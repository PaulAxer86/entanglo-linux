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
    /// Main-display coordinate space used for absolute pointer events.
    /// Optional so pre-negotiation peers remain wire-compatible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
}

impl HelloPayload {
    pub fn screen_size(&self) -> Option<(f64, f64)> {
        match (self.screen_width, self.screen_height) {
            (Some(width), Some(height)) if width > 1 && height > 1 => {
                Some((f64::from(width), f64::from(height)))
            }
            _ => None,
        }
    }
}
