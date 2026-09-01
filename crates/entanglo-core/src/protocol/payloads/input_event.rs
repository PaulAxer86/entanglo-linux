use serde::{Deserialize, Serialize};

/// macOS `CGEventFlags` bit values carried on the wire in
/// `modifier_flags`. See `PROTOCOL.md` §5.5.
///
/// evdev has no bitmask equivalent — Linux capture/injection code
/// must translate to/from individual `KEY_LEFTSHIFT`-style key
/// events. See `crate::input::modifiers`.
pub mod modifier_flags {
    pub const CAPS_LOCK: u64 = 1 << 16;
    pub const SHIFT: u64 = 1 << 17;
    pub const CONTROL: u64 = 1 << 18;
    pub const OPTION_ALT: u64 = 1 << 19;
    pub const COMMAND_META: u64 = 1 << 20;
    pub const NUMERIC_PAD: u64 = 1 << 21;
    pub const HELP: u64 = 1 << 22;
    pub const FUNCTION: u64 = 1 << 23;
}

/// See `PROTOCOL.md` §5.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputEventKind {
    MouseMove,
    MouseDown,
    MouseUp,
    Scroll,
    KeyDown,
    KeyUp,
    MediaKey,
}

/// See `PROTOCOL.md` §5.5. `media_key` string enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKey {
    VolumeUp,
    VolumeDown,
    Mute,
    BrightnessUp,
    BrightnessDown,
    PlayPause,
    Next,
    Previous,
    FastForward,
    Rewind,
    Eject,
}

/// Never carries Unicode text — only low-level key codes and
/// modifier bitmasks, so logs/packet dumps never leak what the user
/// typed. See `PROTOCOL.md` §5.5 for the required-fields-by-`kind`
/// table; unused fields MUST be omitted (`skip_serializing_if`),
/// matching the Mac's `Codable` behaviour for optional fields.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputEventMessage {
    pub kind: InputEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_y: Option<f64>,
    /// 0 = primary, 1 = secondary, 2 = middle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<u8>,
    /// macOS HIToolbox virtual key code — translate via
    /// `crate::protocol::keymap`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_key: Option<MediaKey>,
    pub modifier_flags: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<bool>,
    /// 1 = single, 2 = double, 3 = triple. Missing MUST be treated
    /// as 1 by the receiver (older peers omit it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_state: Option<u8>,
}

impl InputEventMessage {
    pub fn click_state_or_default(&self) -> u8 {
        self.click_state.unwrap_or(1)
    }
}
