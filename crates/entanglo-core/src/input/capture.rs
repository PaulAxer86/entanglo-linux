//! Global keyboard + mouse capture via raw `/dev/input/eventX`
//! (evdev), bypassing X11/Wayland entirely — see `STACK.md` for why.
//!
//! **Skeleton status**: this compiles against the `evdev` crate's
//! general shape but has not been built against the exact pinned
//! version yet (no Rust toolchain on the scaffolding machine — see
//! `docs/DEV.md`). Expect to adjust method names during the first
//! `cargo check` on the real Debian dev box; the architecture
//! (enumerate devices with keyboard/pointer capability bits, merge
//! into one event stream, translate via `keymap`) is the part meant
//! to survive that pass.

use crate::protocol::keymap;
use crate::protocol::payloads::input_event::{InputEventKind, InputEventMessage};

use super::modifiers::ModifierState;

/// Emitted for every physical event this side captures, already
/// translated into wire terms. The caller (net layer) wraps this in
/// an `EntangloMessage` and sends it, but only while this device is
/// in the "controller" role for some peer — see
/// `net::coordinator::ConnectionCoordinator`.
pub struct InputCaptureService {
    modifiers: ModifierState,
}

impl InputCaptureService {
    pub fn new() -> Self {
        Self {
            modifiers: ModifierState::default(),
        }
    }

    /// Enumerate `/dev/input/event*`, keep only devices that report
    /// `EV_KEY` (keyboards, mouse buttons) or `EV_REL` (mouse motion,
    /// scroll wheels) capabilities, per the "skip anything that isn't
    /// a keyboard or pointer" rule in `SKELETON.md`.
    pub fn enumerate_devices() -> std::io::Result<Vec<evdev::Device>> {
        let mut devices = Vec::new();
        for (path, device) in evdev::enumerate() {
            let supports_input = device.supported_events().contains(evdev::EventType::KEY)
                || device
                    .supported_events()
                    .contains(evdev::EventType::RELATIVE);
            if supports_input {
                tracing::debug!(?path, name = ?device.name(), "capturing input device");
                devices.push(device);
            }
        }
        Ok(devices)
    }

    /// Translate one raw evdev `InputEvent` into zero or one wire
    /// `InputEventMessage`s. Modifier key events update internal
    /// state (via `ModifierState::observe`) *and* still produce a
    /// regular `keyDown`/`keyUp` frame — modifiers are ordinary keys
    /// on the wire, per `PROTOCOL.md` §5.5.
    pub fn translate(&mut self, event: evdev::InputEvent) -> Option<InputEventMessage> {
        match event.kind() {
            evdev::InputEventKind::Key(key) => {
                let pressed = event.value() != 0;
                let linux_code = key.code();
                self.modifiers.observe(linux_code, pressed);
                let mac_code = keymap::linux_to_mac(linux_code)?;
                Some(InputEventMessage {
                    kind: if pressed {
                        InputEventKind::KeyDown
                    } else {
                        InputEventKind::KeyUp
                    },
                    x: None,
                    y: None,
                    delta_x: None,
                    delta_y: None,
                    button: None,
                    key_code: Some(mac_code),
                    media_key: None,
                    modifier_flags: self.modifiers.to_bitmask(),
                    pressed: Some(pressed),
                    click_state: None,
                })
            }
            evdev::InputEventKind::RelAxis(axis) => {
                let delta = event.value() as f64;
                let (delta_x, delta_y) = match axis {
                    evdev::RelativeAxisType::REL_X => (Some(delta), None),
                    evdev::RelativeAxisType::REL_Y => (None, Some(delta)),
                    evdev::RelativeAxisType::REL_WHEEL => {
                        return Some(InputEventMessage {
                            kind: InputEventKind::Scroll,
                            x: None,
                            y: None,
                            delta_x: None,
                            delta_y: Some(delta * 10.0), // ticks -> ~pixels, PROTOCOL.md §5.5
                            button: None,
                            key_code: None,
                            media_key: None,
                            modifier_flags: self.modifiers.to_bitmask(),
                            pressed: None,
                            click_state: None,
                        });
                    }
                    _ => return None,
                };
                Some(InputEventMessage {
                    kind: InputEventKind::MouseMove,
                    x: None,
                    y: None,
                    delta_x,
                    delta_y,
                    button: None,
                    key_code: None,
                    media_key: None,
                    modifier_flags: self.modifiers.to_bitmask(),
                    pressed: None,
                    click_state: None,
                })
            }
            _ => None,
        }
    }
}

impl Default for InputCaptureService {
    fn default() -> Self {
        Self::new()
    }
}
