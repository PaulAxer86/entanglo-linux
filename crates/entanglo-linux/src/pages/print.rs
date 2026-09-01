//! Printer Bridge — send a print job to a peer's CUPS queue. Phase 3,
//! see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Print — Printer Bridge via CUPS (Phase 3)")
        .build()
        .into()
}
