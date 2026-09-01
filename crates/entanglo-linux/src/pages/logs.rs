//! Log panel filtered by Network / Pairing / Input / Files / Error,
//! backed by `entanglo_core::logging`. Keystroke content is never
//! logged — see the note in that module. Phase 1, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Logs — filtered log panel (Phase 1)")
        .build()
        .into()
}
