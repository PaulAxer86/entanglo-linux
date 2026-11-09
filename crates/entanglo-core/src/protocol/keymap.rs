//! macOS HIToolbox virtual key code ↔ Linux evdev `KEY_*` code.
//! See `PROTOCOL.md` §7. Codes are physical-position, layout-independent
//! on both ends, so a single table covers every US ANSI layout.
//!
//! Extend this table as gaps are found during testing — it currently
//! covers the common alphanumeric, punctuation, modifier, arrow, and
//! function-key rows. Source of truth for the Mac side is
//! `<HIToolbox/Events.h>`; for Linux it's
//! `linux/input-event-codes.h`.

/// (macOS virtual key code, Linux `KEY_*` code)
pub const MAC_TO_LINUX: &[(u16, u16)] = &[
    // Letters
    (0, 30),  // A -> KEY_A
    (1, 31),  // S -> KEY_S
    (2, 32),  // D -> KEY_D
    (3, 33),  // F -> KEY_F
    (4, 35),  // H -> KEY_H
    (5, 34),  // G -> KEY_G
    (6, 44),  // Z -> KEY_Z
    (7, 45),  // X -> KEY_X
    (8, 46),  // C -> KEY_C
    (9, 47),  // V -> KEY_V
    (11, 48), // B -> KEY_B
    (12, 16), // Q -> KEY_Q
    (13, 17), // W -> KEY_W
    (14, 18), // E -> KEY_E
    (15, 19), // R -> KEY_R
    (16, 21), // Y -> KEY_Y
    (17, 20), // T -> KEY_T
    (31, 24), // O -> KEY_O
    (32, 22), // U -> KEY_U
    (34, 23), // I -> KEY_I
    (35, 25), // P -> KEY_P
    (37, 38), // L -> KEY_L
    (38, 36), // J -> KEY_J
    (40, 37), // K -> KEY_K
    (45, 49), // N -> KEY_N
    (46, 50), // M -> KEY_M
    // Digits (top row)
    (18, 2),  // 1 -> KEY_1
    (19, 3),  // 2 -> KEY_2
    (20, 4),  // 3 -> KEY_3
    (21, 5),  // 4 -> KEY_4
    (22, 7),  // 6 -> KEY_6
    (23, 6),  // 5 -> KEY_5 (Mac's codes for 5 and 6 are out of order)
    (25, 10), // 9 -> KEY_9
    (26, 8),  // 7 -> KEY_7
    (28, 9),  // 8 -> KEY_8
    (29, 11), // 0 -> KEY_0
    // Punctuation
    (24, 13), // = -> KEY_EQUAL
    (27, 12), // - -> KEY_MINUS
    (30, 27), // ] -> KEY_RIGHTBRACE
    (33, 26), // [ -> KEY_LEFTBRACE
    (39, 40), // ' -> KEY_APOSTROPHE
    (41, 39), // ; -> KEY_SEMICOLON
    (42, 43), // \ -> KEY_BACKSLASH
    (43, 51), // , -> KEY_COMMA
    (44, 53), // / -> KEY_SLASH
    (47, 52), // . -> KEY_DOT
    (50, 41), // ` -> KEY_GRAVE
    // Whitespace / editing
    (36, 28),   // Return -> KEY_ENTER
    (48, 15),   // Tab -> KEY_TAB
    (49, 57),   // Space -> KEY_SPACE
    (51, 14),   // Delete (backspace) -> KEY_BACKSPACE
    (53, 1),    // Escape -> KEY_ESC
    (117, 111), // Forward Delete -> KEY_DELETE
    (115, 102), // Home -> KEY_HOME
    (119, 107), // End -> KEY_END
    (116, 104), // Page Up -> KEY_PAGEUP
    (121, 109), // Page Down -> KEY_PAGEDOWN
    // Modifiers
    (54, 126), // Command (right) -> KEY_RIGHTMETA
    (55, 125), // Command (left) -> KEY_LEFTMETA
    (56, 42),  // Shift (left) -> KEY_LEFTSHIFT
    (57, 58),  // Caps Lock -> KEY_CAPSLOCK
    (58, 56),  // Option (left) -> KEY_LEFTALT
    (59, 29),  // Control (left) -> KEY_LEFTCTRL
    (60, 54),  // Shift (right) -> KEY_RIGHTSHIFT
    (61, 100), // Option (right) -> KEY_RIGHTALT
    (62, 97),  // Control (right) -> KEY_RIGHTCTRL
    (63, 464), // Function (Fn) -> KEY_FN (best-effort; not all kernels expose this)
    // Arrows
    (123, 105), // Left -> KEY_LEFT
    (124, 106), // Right -> KEY_RIGHT
    (125, 108), // Down -> KEY_DOWN
    (126, 103), // Up -> KEY_UP
    // Function row
    (122, 59), // F1 -> KEY_F1
    (120, 60), // F2 -> KEY_F2
    (99, 61),  // F3 -> KEY_F3
    (118, 62), // F4 -> KEY_F4
    (96, 63),  // F5 -> KEY_F5
    (97, 64),  // F6 -> KEY_F6
    (98, 65),  // F7 -> KEY_F7
    (100, 66), // F8 -> KEY_F8
    (101, 67), // F9 -> KEY_F9
    (109, 68), // F10 -> KEY_F10
    (103, 87), // F11 -> KEY_F11
    (111, 88), // F12 -> KEY_F12
    // Numeric keypad
    (82, 82), // Keypad 0 -> KEY_KP0
    (83, 79), // Keypad 1 -> KEY_KP1
    (84, 80), // Keypad 2 -> KEY_KP2
    (85, 81), // Keypad 3 -> KEY_KP3
    (86, 75), // Keypad 4 -> KEY_KP4
    (87, 76), // Keypad 5 -> KEY_KP5
    (88, 77), // Keypad 6 -> KEY_KP6
    (89, 71), // Keypad 7 -> KEY_KP7
    (91, 72), // Keypad 8 -> KEY_KP8
    (92, 73), // Keypad 9 -> KEY_KP9
    (65, 83), // Keypad . -> KEY_KPDOT
    (67, 55), // Keypad * -> KEY_KPASTERISK
    (69, 78), // Keypad + -> KEY_KPPLUS
    (75, 98), // Keypad / -> KEY_KPSLASH
    (76, 96), // Keypad Enter -> KEY_KPENTER
    (78, 74), // Keypad - -> KEY_KPMINUS
    (81, 117), // Keypad = -> KEY_KPEQUAL
];

/// Linux keypad keys whose meaning depends on local state, paired with the key
/// that always does the same thing. Digits and dot act as navigation unless
/// NumLock is on, and Mac keyboard layouts turn keypad Enter into AltGr.
const KEYPAD_TO_MAIN_KEY: &[(u16, u16)] = &[
    (96, 28), // KEY_KPENTER -> KEY_ENTER
    (82, 11), // KEY_KP0 -> KEY_0
    (79, 2),  // KEY_KP1 -> KEY_1
    (80, 3),  // KEY_KP2 -> KEY_2
    (81, 4),  // KEY_KP3 -> KEY_3
    (75, 5),  // KEY_KP4 -> KEY_4
    (76, 6),  // KEY_KP5 -> KEY_5
    (77, 7),  // KEY_KP6 -> KEY_6
    (71, 8),  // KEY_KP7 -> KEY_7
    (72, 9),  // KEY_KP8 -> KEY_8
    (73, 10), // KEY_KP9 -> KEY_9
    (83, 52), // KEY_KPDOT -> KEY_DOT
];

/// Mouse `button` field (0/1/2, see `PROTOCOL.md` §5.5) to Linux
/// `BTN_*` evdev code.
pub const MOUSE_BUTTON_TO_LINUX: &[(u8, u16)] = &[
    (0, 0x110), // primary -> BTN_LEFT
    (1, 0x111), // secondary -> BTN_RIGHT
    (2, 0x112), // middle -> BTN_MIDDLE
];

/// Existing protocol media names mapped to Linux input-event-codes.h.
pub const MEDIA_TO_LINUX: &[(crate::protocol::payloads::input_event::MediaKey, u16)] = &[
    (crate::protocol::payloads::input_event::MediaKey::Mute, 113),
    (crate::protocol::payloads::input_event::MediaKey::VolumeDown, 114),
    (crate::protocol::payloads::input_event::MediaKey::VolumeUp, 115),
    (crate::protocol::payloads::input_event::MediaKey::Eject, 161),
    (crate::protocol::payloads::input_event::MediaKey::Next, 163),
    (crate::protocol::payloads::input_event::MediaKey::PlayPause, 164),
    (crate::protocol::payloads::input_event::MediaKey::Previous, 165),
    (crate::protocol::payloads::input_event::MediaKey::Rewind, 168),
    (crate::protocol::payloads::input_event::MediaKey::FastForward, 208),
    (crate::protocol::payloads::input_event::MediaKey::BrightnessDown, 224),
    (crate::protocol::payloads::input_event::MediaKey::BrightnessUp, 225),
];

pub fn mac_to_linux(mac_code: u16) -> Option<u16> {
    MAC_TO_LINUX
        .iter()
        .find(|(mac, _)| *mac == mac_code)
        .map(|(_, linux)| *linux)
}

/// The key to press on Linux for a received Mac key, chosen so a keypad key
/// does what its label says whatever this computer's NumLock or layout.
pub fn mac_to_linux_for_injection(mac_code: u16) -> Option<u16> {
    let linux_code = mac_to_linux(mac_code)?;
    let main_key = KEYPAD_TO_MAIN_KEY.iter().find(|(keypad, _)| *keypad == linux_code);
    Some(main_key.map_or(linux_code, |(_, main)| *main))
}

pub fn linux_to_mac(linux_code: u16) -> Option<u16> {
    MAC_TO_LINUX
        .iter()
        .find(|(_, linux)| *linux == linux_code)
        .map(|(mac, _)| *mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicate_mac_codes() {
        let mut codes: Vec<u16> = MAC_TO_LINUX.iter().map(|(mac, _)| *mac).collect();
        let before = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(before, codes.len(), "duplicate macOS key code in table");
    }

    #[test]
    fn table_has_no_duplicate_linux_codes() {
        let mut codes: Vec<u16> = MAC_TO_LINUX.iter().map(|(_, linux)| *linux).collect();
        let before = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(before, codes.len(), "duplicate Linux KEY_* code in table");
    }

    #[test]
    fn digit_row_maps_each_mac_key_to_the_same_digit() {
        // Mac key codes for 1234567890, then Linux KEY_1 to KEY_9 and KEY_0.
        let mac = [18, 19, 20, 21, 23, 22, 26, 28, 25, 29];
        let linux = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        for (mac_code, linux_code) in mac.into_iter().zip(linux) {
            assert_eq!(mac_to_linux(mac_code), Some(linux_code));
            assert_eq!(linux_to_mac(linux_code), Some(mac_code));
        }
    }

    #[test]
    fn keypad_digits_type_digits_regardless_of_numlock() {
        // Mac key codes for keypad 0 to 9.
        let mac = [82, 83, 84, 85, 86, 87, 88, 89, 91, 92];
        let digits = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        for (mac_code, digit) in mac.into_iter().zip(digits) {
            assert_eq!(mac_to_linux_for_injection(mac_code), Some(digit));
        }
        assert_eq!(mac_to_linux_for_injection(65), Some(52), "keypad dot");
    }

    #[test]
    fn keypad_operators_stay_on_the_keypad() {
        for (mac_code, linux_code) in [(67, 55), (69, 78), (75, 98), (78, 74), (81, 117)] {
            assert_eq!(mac_to_linux_for_injection(mac_code), Some(linux_code));
        }
    }

    /// Regression: the X11 `gb(mac)` layout maps keypad Enter to AltGr, so
    /// injecting KEY_KPENTER did nothing visible on an iMac.
    #[test]
    fn keypad_enter_is_injected_as_the_main_enter_key() {
        assert_eq!(mac_to_linux_for_injection(76), Some(28));
    }

    #[test]
    fn linux_keypad_keys_are_sent_as_mac_keypad_keys() {
        assert_eq!(linux_to_mac(75), Some(86), "keypad 4");
        assert_eq!(linux_to_mac(96), Some(76), "keypad enter");
    }

    #[test]
    fn roundtrips_return_key() {
        assert_eq!(mac_to_linux(36), Some(28));
        assert_eq!(linux_to_mac(28), Some(36));
    }
}
