//! Printer Bridge — send a print job to a peer's CUPS queue. Phase 3,
//! see `ROADMAP.md`.

use gtk::Widget;

use super::layout;

pub fn build() -> Widget {
    layout::placeholder(
        "Print",
        "Use a printer configured on another device.",
        "The CUPS printer bridge is planned for Phase 3.",
    )
}
