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

pub struct InputInjectionService {
    device: VirtualDevice,
    modifiers: ModifierState,
}

#[derive(Debug, thiserror::Error)]
pub enum InjectionError {
    #[error("uinput io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unmapped mac key code {0}")]
    UnmappedKeyCode(u16),
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
            .name("Entanglo Virtual Input")
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;

        Ok(Self {
            device,
            modifiers: ModifierState::default(),
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
                if let (Some(dx), Some(dy)) = (event.delta_x, event.delta_y) {
                    self.device.emit(&[
                        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx as i32),
                        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy as i32),
                    ])?;
                }
                // Absolute x/y (screen points) requires an EV_ABS
                // device instead of EV_REL — deferred; Phase 1 ships
                // relative-delta forwarding only, matching how the
                // Mac controller already emits deltas for edge-drag.
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
