//! Discovered + trusted peer list, mirroring `entanglo-macos`'s
//! `DevicesView`. Phase 1, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Devices — discovered + trusted peers (Phase 1)")
        .build()
        .into()
}
