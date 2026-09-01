//! Network Quality dashboard tile, backed by
//! `entanglo_core::features::network_quality`. Phase 2, see
//! `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Network — RTT, loss estimate, suggested mode (Phase 2)")
        .build()
        .into()
}
