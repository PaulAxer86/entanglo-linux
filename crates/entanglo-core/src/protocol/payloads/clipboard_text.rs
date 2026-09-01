use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.7. UTF-8, sender-side truncation (Mac caps
/// at ~1 MiB — mirror that cap here too).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardTextPayload {
    pub text: String,
}

pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
