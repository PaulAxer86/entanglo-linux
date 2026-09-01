//! Approve/reject incoming `pairRequest`, mirroring
//! `entanglo-macos`'s pairing UI. Phase 1, see `ROADMAP.md` and
//! `PROTOCOL.md` §5.2–§5.3.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Pairing — approve/reject incoming peers (Phase 1)")
        .build()
        .into()
}
