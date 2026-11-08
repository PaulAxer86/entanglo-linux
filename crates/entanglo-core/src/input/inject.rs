//! Virtual keyboard + mouse via `/dev/uinput`, built on the same
//! `evdev` crate as `capture.rs` — its `uinput` module wraps raw
//! `Key`/`RelativeAxisType` codes directly, so no separate `uinput`
//! crate dependency is needed and injection shares exactly the same
//! code-space as capture.
//!
//! Desktop-Debian evolution of
//! `entanglo-android/app/src/main/cpp/uinput.c` — same kernel
//! interface, no root/helper-process indirection needed since
//! desktop Debian lets an `input`-group process open the node
//! directly (see the udev rule in `ROADMAP.md` Phase 1).

use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};

use crate::protocol::keymap;
use crate::protocol::payloads::input_event::{InputEventKind, InputEventMessage};

use super::modifiers::ModifierState;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

struct AbsolutePointer {
    connection: RustConnection,
    root: u32,
    width: u16,
    height: u16,
}

fn map_absolute(x: f64, y: f64, width: u16, height: u16) -> Option<(i16, i16)> {
    if !x.is_finite() || !y.is_finite() || width == 0 || height == 0 {
        return None;
    }
    // macOS 0.1.58 uses a fixed logical remote canvas of 2560 x 1440.
    let x = (x.clamp(0.0, 2559.0) / 2559.0 * f64::from(width - 1)).round();
    let y = (y.clamp(0.0, 1439.0) / 1439.0 * f64::from(height - 1)).round();
    Some((x.min(i16::MAX as f64) as i16, y.min(i16::MAX as f64) as i16))
}


/// Also the filter `input::capture::InputCaptureService::enumerate_devices`
/// uses to make sure our own virtual device never gets captured back
/// as local input.
pub const VIRTUAL_DEVICE_NAME: &str = "Entanglo Virtual Input";

pub struct InputInjectionService {
    device: VirtualDevice,
    modifiers: ModifierState,
    absolute_pointer: Option<AbsolutePointer>,
}

#[derive(Debug, thiserror::Error)]
pub enum InjectionError {
    #[error("uinput io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unmapped mac key code {0}")]
    UnmappedKeyCode(u16),
    #[error("absolute pointer injection unavailable: {0}")]
    AbsolutePointer(String),
}

impl InputInjectionService {
    /// Opens `/dev/uinput` and registers the full keyboard + mouse
    /// button + relative-motion capability set once. Requires the
    /// process to be in the `input` group (see
    /// `packaging/60-entanglo-uinput.rules`) — no root needed on
    /// desktop Debian, unlike the Android root-helper approach this
    /// evolved from.
    pub fn open() -> Result<Self, InjectionError> {
        let mut keys = AttributeSet::<Key>::new();
        for (_, linux_code) in keymap::MAC_TO_LINUX {
            keys.insert(Key::new(*linux_code));
        }
        for (_, btn_code) in keymap::MOUSE_BUTTON_TO_LINUX {
            keys.insert(Key::new(*btn_code));
        }

        let mut axes = AttributeSet::<RelativeAxisType>::new();
        axes.insert(RelativeAxisType::REL_X);
        axes.insert(RelativeAxisType::REL_Y);
        axes.insert(RelativeAxisType::REL_WHEEL);

        let device = VirtualDeviceBuilder::new()?
            .name(VIRTUAL_DEVICE_NAME)
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;

        let absolute_pointer = if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
            None
        } else {
            RustConnection::connect(None).ok().map(|(connection, screen)| {
                let root = &connection.setup().roots[screen];
                let (root, width, height) = (root.root, root.width_in_pixels, root.height_in_pixels);
                AbsolutePointer { connection, root, width, height }
            })
        };
        Ok(Self {
            device,
            modifiers: ModifierState::default(),
            absolute_pointer,
        })
    }

    /// Apply one incoming `InputEventMessage` to the virtual device.
    /// Callers MUST have already passed this through the safety
    /// gates (trusted + receiver role + heartbeat alive + emergency
    /// stop off) described in `entanglo-macos/docs/ARCHITECTURE.md`
    /// §"Safety gates" — this function does not re-check them.
    pub fn inject(&mut self, event: &InputEventMessage) -> Result<(), InjectionError> {
        // Bring the virtual device's modifier state in line with the
        // incoming bitmask before the primary event, per
        // `PROTOCOL.md` §5.5 / `input::modifiers`.
        for (code, pressed) in self.modifiers.diff_to_reach(event.modifier_flags) {
            self.write_key(code, pressed)?;
        }

        match event.kind {
            InputEventKind::MouseMove => {
                if event.delta_x.is_some() || event.delta_y.is_some() {
                    let (dx, dy) = (event.delta_x.unwrap_or(0.0), event.delta_y.unwrap_or(0.0));
                    self.device.emit(&[
                        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx as i32),
                        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy as i32),
                    ])?;
                }
                else if let (Some(x), Some(y)) = (event.x, event.y) {
                    let pointer = self.absolute_pointer.as_ref().ok_or_else(||
                        InjectionError::AbsolutePointer("requires an X11 desktop".into()))?;
                    if let Some((x, y)) = map_absolute(x, y, pointer.width, pointer.height) {
                        pointer.connection.warp_pointer(
                            x11rb::NONE, pointer.root, 0, 0, 0, 0, x, y,
                        ).map_err(|e| InjectionError::AbsolutePointer(e.to_string()))?
                         .check().map_err(|e| InjectionError::AbsolutePointer(e.to_string()))?;
                    }
                }
            }
            InputEventKind::MouseDown | InputEventKind::MouseUp => {
                let pressed = event.kind == InputEventKind::MouseDown;
                let Some(btn_code) = event
                    .button
                    .and_then(|b| keymap::MOUSE_BUTTON_TO_LINUX.iter().find(|(w, _)| *w == b))
                    .map(|(_, code)| *code)
                else {
                    return Ok(());
                };
                self.write_key(btn_code, pressed)?;
            }
            InputEventKind::Scroll => {
                if let Some(dy) = event.delta_y {
                    self.device.emit(&[InputEvent::new(
                        EventType::RELATIVE,
                        RelativeAxisType::REL_WHEEL.0,
                        (dy / 10.0) as i32,
                    )])?;
                }
            }
            InputEventKind::KeyDown | InputEventKind::KeyUp => {
                let Some(mac_code) = event.key_code else {
                    return Ok(());
                };
                let linux_code = keymap::mac_to_linux(mac_code)
                    .ok_or(InjectionError::UnmappedKeyCode(mac_code))?;
                self.write_key(linux_code, event.kind == InputEventKind::KeyDown)?;
            }
            InputEventKind::MediaKey => {
                // Deferred to Phase 2 alongside the rest of the media
                // key surface — see ROADMAP.md.
            }
        }
        Ok(())
    }

    fn write_key(&mut self, linux_code: u16, pressed: bool) -> Result<(), InjectionError> {
        self.modifiers.observe(linux_code, pressed);
        self.device
            .emit(&[InputEvent::new(EventType::KEY, linux_code, pressed as i32)])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::map_absolute;

    #[test]
    fn legacy_mac_canvas_maps_to_linux_screen_edges() {
        assert_eq!(map_absolute(0.0, 0.0, 1920, 1080), Some((0, 0)));
        assert_eq!(map_absolute(2559.0, 1439.0, 1920, 1080), Some((1919, 1079)));
        assert_eq!(map_absolute(1280.0, 720.0, 1920, 1080), Some((960, 540)));
    }

    #[test]
    fn invalid_positions_are_ignored_and_edges_clamped() {
        assert_eq!(map_absolute(f64::NAN, 1.0, 1920, 1080), None);
        assert_eq!(map_absolute(1.0, f64::INFINITY, 1920, 1080), None);
        assert_eq!(map_absolute(1.0, 1.0, 0, 1080), None);
        assert_eq!(map_absolute(-5.0, 9000.0, 1920, 1080), Some((0, 1079)));
    }
}
