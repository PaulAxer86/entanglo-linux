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
    (22, 6),  // 6 -> KEY_6
    (23, 8),  // 5 -> KEY_5 (Mac's odd ordering, verified against HIToolbox)
    (25, 10), // 9 -> KEY_9
    (26, 7),  // 7 -> KEY_7
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
];

/// Mouse `button` field (0/1/2, see `PROTOCOL.md` §5.5) to Linux
/// `BTN_*` evdev code.
pub const MOUSE_BUTTON_TO_LINUX: &[(u8, u16)] = &[
    (0, 0x110), // primary -> BTN_LEFT
    (1, 0x111), // secondary -> BTN_RIGHT
    (2, 0x112), // middle -> BTN_MIDDLE
];

pub fn mac_to_linux(mac_code: u16) -> Option<u16> {
    MAC_TO_LINUX
        .iter()
        .find(|(mac, _)| *mac == mac_code)
        .map(|(_, linux)| *linux)
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
    fn roundtrips_return_key() {
        assert_eq!(mac_to_linux(36), Some(28));
        assert_eq!(linux_to_mac(28), Some(36));
    }
}
