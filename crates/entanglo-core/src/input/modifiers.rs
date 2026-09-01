//! Bitmask (macOS `CGEventFlags`, wire format) <-> individual evdev
//! `KEY_*` down/up state. See `PROTOCOL.md` §5.5 for why this
//! translation exists: evdev has no bitmask concept, only discrete
//! key events.

use crate::protocol::payloads::input_event::modifier_flags;

/// Evdev codes this side currently believes are held down, tracked
/// independently for capture (what *we* are pressing) and injection
/// (what we have synthesized as held on the virtual uinput device).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierState {
    pub caps_lock: bool,
    pub left_shift: bool,
    pub right_shift: bool,
    pub left_control: bool,
    pub right_control: bool,
    pub left_alt: bool,
    pub right_alt: bool,
    pub left_meta: bool,
    pub right_meta: bool,
}

impl ModifierState {
    /// evdev `KEY_*` codes this state tracks, paired with the setter
    /// to flip on a key-down/key-up event. Used by both capture (to
    /// update state as physical keys move) and injection (to know
    /// which synthetic key to press/release to reach a target mask).
    const TRACKED_KEYS: &'static [(u16, fn(&mut Self, bool))] = &[
        (58, |s, v| s.caps_lock = v),     // KEY_CAPSLOCK
        (42, |s, v| s.left_shift = v),    // KEY_LEFTSHIFT
        (54, |s, v| s.right_shift = v),   // KEY_RIGHTSHIFT
        (29, |s, v| s.left_control = v),  // KEY_LEFTCTRL
        (97, |s, v| s.right_control = v), // KEY_RIGHTCTRL
        (56, |s, v| s.left_alt = v),      // KEY_LEFTALT
        (100, |s, v| s.right_alt = v),    // KEY_RIGHTALT
        (125, |s, v| s.left_meta = v),    // KEY_LEFTMETA
        (126, |s, v| s.right_meta = v),   // KEY_RIGHTMETA
    ];

    /// Update tracked state from an observed evdev key event. Returns
    /// `true` if `linux_key_code` was a tracked modifier (caller can
    /// use this to decide whether the key event also needs
    /// forwarding as a regular `keyDown`/`keyUp` `inputEvent` — it
    /// does, modifiers are ordinary keys on the wire too, this just
    /// additionally maintains the bitmask view).
    pub fn observe(&mut self, linux_key_code: u16, pressed: bool) -> bool {
        for (code, setter) in Self::TRACKED_KEYS {
            if *code == linux_key_code {
                setter(self, pressed);
                return true;
            }
        }
        false
    }

    /// Compute the wire `modifierFlags` bitmask from current state.
    pub fn to_bitmask(self) -> u64 {
        let mut flags = 0u64;
        if self.caps_lock {
            flags |= modifier_flags::CAPS_LOCK;
        }
        if self.left_shift || self.right_shift {
            flags |= modifier_flags::SHIFT;
        }
        if self.left_control || self.right_control {
            flags |= modifier_flags::CONTROL;
        }
        if self.left_alt || self.right_alt {
            flags |= modifier_flags::OPTION_ALT;
        }
        if self.left_meta || self.right_meta {
            flags |= modifier_flags::COMMAND_META;
        }
        flags
    }

    /// Given a target wire bitmask and the current injected state,
    /// return the list of (evdev `KEY_*` code, pressed) synthetic
    /// events needed to make the virtual device's modifier state
    /// match, preferring the left-hand variant of each modifier when
    /// one must be pressed and neither side is currently down.
    pub fn diff_to_reach(&mut self, target: u64) -> Vec<(u16, bool)> {
        let mut events = Vec::new();
        let mut want = |bit: u64, held: bool, code: u16, held_field: &mut bool| {
            let want_down = target & bit != 0;
            if want_down != held {
                events.push((code, want_down));
                *held_field = want_down;
            }
        };
        let shift_held = self.left_shift || self.right_shift;
        want(modifier_flags::SHIFT, shift_held, 42, &mut self.left_shift);
        let control_held = self.left_control || self.right_control;
        want(
            modifier_flags::CONTROL,
            control_held,
            29,
            &mut self.left_control,
        );
        let alt_held = self.left_alt || self.right_alt;
        want(modifier_flags::OPTION_ALT, alt_held, 56, &mut self.left_alt);
        let meta_held = self.left_meta || self.right_meta;
        want(
            modifier_flags::COMMAND_META,
            meta_held,
            125,
            &mut self.left_meta,
        );
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_press_sets_bitmask() {
        let mut state = ModifierState::default();
        assert!(state.observe(42, true)); // KEY_LEFTSHIFT down
        assert_eq!(state.to_bitmask(), modifier_flags::SHIFT);
    }

    #[test]
    fn diff_to_reach_synthesizes_missing_shift() {
        let mut state = ModifierState::default();
        let events = state.diff_to_reach(modifier_flags::SHIFT);
        assert_eq!(events, vec![(42, true)]);
        assert!(state.left_shift);
    }
}
