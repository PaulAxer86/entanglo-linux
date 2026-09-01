//! File drop UI + Recent Transfers, backed by
//! `entanglo_core::features::file_transfer`. Phase 2, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Files — send/receive + recent transfers (Phase 2)")
        .build()
        .into()
}
