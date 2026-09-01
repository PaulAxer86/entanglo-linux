//! Local device identity (name, role, interfaces), mirroring
//! `entanglo-macos`'s `DashboardView`. Phase 1, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Dashboard — device identity, role, interfaces (Phase 1)")
        .build()
        .into()
}
