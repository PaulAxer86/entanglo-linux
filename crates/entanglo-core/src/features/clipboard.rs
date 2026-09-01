//! Plain-text clipboard sync between trusted devices, off by default
//! — mirrors `entanglo-macos`'s `ClipboardSyncService`. Phase 2, see
//! `ROADMAP.md`. Planned backing: the `arboard` crate (works on both
//! X11 and Wayland).

use crate::protocol::payloads::clipboard_text::{ClipboardTextPayload, MAX_CLIPBOARD_TEXT_BYTES};

pub struct ClipboardSyncService {
    enabled: bool,
}

impl ClipboardSyncService {
    pub fn new() -> Self {
        Self { enabled: false }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Truncate outgoing text to the shared cap, per `PROTOCOL.md` §5.7.
    /// Truncates on a char boundary so we never emit invalid UTF-8.
    pub fn build_payload(&self, text: &str) -> Option<ClipboardTextPayload> {
        if !self.enabled {
            return None;
        }
        let text = if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            let mut end = MAX_CLIPBOARD_TEXT_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            text
        };
        Some(ClipboardTextPayload {
            text: text.to_string(),
        })
    }
}

impl Default for ClipboardSyncService {
    fn default() -> Self {
        Self::new()
    }
}
