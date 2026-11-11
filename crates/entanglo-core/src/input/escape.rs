//! Key combinations that hand a captured mouse and keyboard back to this
//! computer. The same detector runs on two independent key sources: raw
//! evdev capture and the X11 keyboard grab. The X11 path keeps working even
//! when evdev capture has lost a device, for example after a Bluetooth
//! keyboard reconnects under a new `/dev/input/event*` node.

use std::time::{Duration, Instant};

/// Three Escape presses within this window, matching the Mac app.
pub const TRIPLE_ESCAPE_WINDOW: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeKey {
    Escape,
    LeftShift,
    RightShift,
    Control,
    Other,
}

impl EscapeKey {
    /// Wire (macOS virtual) key codes, as produced by evdev capture.
    pub fn from_mac_key_code(code: u16) -> Self {
        match code {
            53 => Self::Escape,
            56 => Self::LeftShift,
            60 => Self::RightShift,
            59 | 62 => Self::Control,
            _ => Self::Other,
        }
    }

    /// X11 key codes under the evdev/libinput driver (Linux code + 8).
    pub fn from_x11_keycode(code: u8) -> Self {
        match code {
            9 => Self::Escape,
            50 => Self::LeftShift,
            62 => Self::RightShift,
            37 | 105 => Self::Control,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeCombo {
    /// Left Shift + Escape.
    LeftShiftEscape,
    /// Ctrl + Shift + Escape, which also toggles Emergency Stop when no
    /// peer is being controlled.
    ControlShiftEscape,
    /// Escape pressed three times in quick succession.
    TripleEscape,
}

#[derive(Debug, Default)]
pub struct EscapeDetector {
    left_shift: bool,
    right_shift: bool,
    control: bool,
    escapes: Vec<Instant>,
}

impl EscapeDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed every key press and release. Auto-repeat must be filtered out
    /// by the caller so a held Escape is not counted three times.
    pub fn observe(&mut self, key: EscapeKey, pressed: bool, now: Instant) -> Option<EscapeCombo> {
        match key {
            EscapeKey::LeftShift => self.left_shift = pressed,
            EscapeKey::RightShift => self.right_shift = pressed,
            EscapeKey::Control => self.control = pressed,
            EscapeKey::Other => {
                if pressed {
                    self.escapes.clear();
                }
            }
            EscapeKey::Escape if pressed => {
                if self.control && (self.left_shift || self.right_shift) {
                    self.escapes.clear();
                    return Some(EscapeCombo::ControlShiftEscape);
                }
                if self.left_shift {
                    self.escapes.clear();
                    return Some(EscapeCombo::LeftShiftEscape);
                }
                self.escapes.retain(|at| now.duration_since(*at) <= TRIPLE_ESCAPE_WINDOW);
                self.escapes.push(now);
                if self.escapes.len() >= 3 {
                    self.escapes.clear();
                    return Some(EscapeCombo::TripleEscape);
                }
            }
            EscapeKey::Escape => {}
        }
        None
    }

    /// Modifier state from another source can go stale while keys are
    /// grabbed elsewhere; start each capture session from a clean slate.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(detector: &mut EscapeDetector, key: EscapeKey, at: Instant) -> Option<EscapeCombo> {
        let combo = detector.observe(key, true, at);
        detector.observe(key, false, at);
        combo
    }

    #[test]
    fn left_shift_escape_returns_control() {
        let mut detector = EscapeDetector::new();
        let now = Instant::now();
        detector.observe(EscapeKey::LeftShift, true, now);
        assert_eq!(press(&mut detector, EscapeKey::Escape, now), Some(EscapeCombo::LeftShiftEscape));
    }

    #[test]
    fn right_shift_escape_alone_is_forwarded() {
        let mut detector = EscapeDetector::new();
        let now = Instant::now();
        detector.observe(EscapeKey::RightShift, true, now);
        assert_eq!(press(&mut detector, EscapeKey::Escape, now), None);
    }

    #[test]
    fn control_shift_escape_is_recognised_with_either_shift() {
        for shift in [EscapeKey::LeftShift, EscapeKey::RightShift] {
            let mut detector = EscapeDetector::new();
            let now = Instant::now();
            detector.observe(EscapeKey::Control, true, now);
            detector.observe(shift, true, now);
            assert_eq!(press(&mut detector, EscapeKey::Escape, now), Some(EscapeCombo::ControlShiftEscape));
        }
    }

    #[test]
    fn triple_escape_within_the_window() {
        let mut detector = EscapeDetector::new();
        let start = Instant::now();
        assert_eq!(press(&mut detector, EscapeKey::Escape, start), None);
        assert_eq!(press(&mut detector, EscapeKey::Escape, start + Duration::from_millis(300)), None);
        assert_eq!(
            press(&mut detector, EscapeKey::Escape, start + Duration::from_millis(600)),
            Some(EscapeCombo::TripleEscape)
        );
        assert_eq!(press(&mut detector, EscapeKey::Escape, start + Duration::from_millis(700)), None,
            "the count restarts after a combo");
    }

    #[test]
    fn slow_or_interrupted_escapes_do_not_count() {
        let mut detector = EscapeDetector::new();
        let start = Instant::now();
        press(&mut detector, EscapeKey::Escape, start);
        press(&mut detector, EscapeKey::Escape, start + Duration::from_millis(200));
        assert_eq!(press(&mut detector, EscapeKey::Escape, start + Duration::from_millis(1500)), None);

        let mut detector = EscapeDetector::new();
        press(&mut detector, EscapeKey::Escape, start);
        press(&mut detector, EscapeKey::Escape, start);
        press(&mut detector, EscapeKey::Other, start);
        assert_eq!(press(&mut detector, EscapeKey::Escape, start), None);
    }

    #[test]
    fn key_code_tables() {
        assert_eq!(EscapeKey::from_mac_key_code(53), EscapeKey::Escape);
        assert_eq!(EscapeKey::from_mac_key_code(56), EscapeKey::LeftShift);
        assert_eq!(EscapeKey::from_mac_key_code(60), EscapeKey::RightShift);
        assert_eq!(EscapeKey::from_x11_keycode(9), EscapeKey::Escape);
        assert_eq!(EscapeKey::from_x11_keycode(50), EscapeKey::LeftShift);
        assert_eq!(EscapeKey::from_x11_keycode(105), EscapeKey::Control);
    }
}
