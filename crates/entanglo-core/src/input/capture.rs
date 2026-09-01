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
    /// a keyboard or pointer" rule in `SKELETON.md`. Explicitly skips
    /// our own `/dev/uinput` virtual device (see
    /// `input::inject::VIRTUAL_DEVICE_NAME`) — without this, a device
    /// that's both controller and receiver would capture its own
    /// injected events back as "local input", feeding them straight
    /// into an infinite loop (and, once Checkpoint D's `releaseControl`
    /// lands, immediately releasing control the instant it was
    /// granted).
    pub fn enumerate_devices() -> std::io::Result<Vec<evdev::Device>> {
        let mut devices = Vec::new();
        for (path, device) in evdev::enumerate() {
            if device.name() == Some(crate::input::inject::VIRTUAL_DEVICE_NAME) {
                continue;
            }
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

                // Mouse buttons (BTN_LEFT/RIGHT/MIDDLE) arrive as
                // ordinary EV_KEY events, same as keyboard keys, but
                // the wire protocol models them as a distinct
                // `mouseDown`/`mouseUp` kind with a `button` index
                // (PROTOCOL.md §5.5) — check that table first.
                if let Some((wire_button, _)) = keymap::MOUSE_BUTTON_TO_LINUX
                    .iter()
                    .find(|(_, btn)| *btn == linux_code)
                {
                    return Some(InputEventMessage {
                        kind: if pressed {
                            InputEventKind::MouseDown
                        } else {
                            InputEventKind::MouseUp
                        },
                        x: None,
                        y: None,
                        delta_x: None,
                        delta_y: None,
                        button: Some(*wire_button),
                        key_code: None,
                        media_key: None,
                        modifier_flags: self.modifiers.to_bitmask(),
                        pressed: Some(pressed),
                        click_state: Some(1),
                    });
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::{EventType, InputEvent};

    /// Regression test: mouse button presses were silently dropped
    /// before this fix — `linux_to_mac` has no entry for `BTN_LEFT`
    /// (that table is keyboard keys only), so `translate` returned
    /// `None` for every click. Found while wiring Checkpoint D.
    #[test]
    fn left_click_translates_to_mouse_down_not_a_dropped_keycode() {
        let mut capture = InputCaptureService::new();
        let event = InputEvent::new(EventType::KEY, 0x110 /* BTN_LEFT */, 1);
        let msg = capture
            .translate(event)
            .expect("BTN_LEFT must translate to a wire message");
        assert_eq!(msg.kind, InputEventKind::MouseDown);
        assert_eq!(msg.button, Some(0));
        assert_eq!(msg.pressed, Some(true));
        assert_eq!(msg.key_code, None);
    }

    #[test]
    fn right_click_release_translates_to_mouse_up() {
        let mut capture = InputCaptureService::new();
        let event = InputEvent::new(EventType::KEY, 0x111 /* BTN_RIGHT */, 0);
        let msg = capture.translate(event).expect("BTN_RIGHT must translate");
        assert_eq!(msg.kind, InputEventKind::MouseUp);
        assert_eq!(msg.button, Some(1));
        assert_eq!(msg.pressed, Some(false));
    }

    #[test]
    fn ordinary_key_still_translates_as_a_keyboard_event() {
        let mut capture = InputCaptureService::new();
        let event = InputEvent::new(EventType::KEY, 30 /* KEY_A */, 1);
        let msg = capture.translate(event).expect("KEY_A must translate");
        assert_eq!(msg.kind, InputEventKind::KeyDown);
        assert_eq!(msg.key_code, Some(0)); // macOS virtual key code for 'A'
        assert_eq!(msg.button, None);
    }
}
