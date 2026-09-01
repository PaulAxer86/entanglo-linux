//! App settings + permission status (`input` group membership,
//! `/dev/uinput` access — see `ROADMAP.md` Phase 1 udev rule note).

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("Settings — preferences, permission status (Phase 1)")
        .build()
        .into()
}
