//! Parses `latest-linux.json` and surfaces the Install button, backed
//! by `entanglo_core::update`. Phase 2, see `ROADMAP.md`.

use gtk::{Label, Widget};

pub fn build() -> Widget {
    Label::builder()
        .label("News & Updates — release notes, auto-update (Phase 2)")
        .build()
        .into()
}
