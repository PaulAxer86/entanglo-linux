//! Edge-switching config, emergency stop, current controller/receiver
//! state. Phase 1, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Input Sharing — edge switching, emergency stop (Phase 1)")
        .build()
        .into()
}
