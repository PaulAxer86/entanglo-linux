//! Approve/reject incoming `pairRequest`, mirroring
//! `entanglo-macos`'s pairing UI. Phase 1, see `ROADMAP.md` and
//! `PROTOCOL.md` §5.2–§5.3. Live-updated from `CoordinatorEvent`s via
//! `state::handle_event` — see `state.rs` for the Accept/Reject
//! button wiring.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{ScrolledWindow, Widget};

use super::layout;
use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    shared.pairing_list.add_css_class("boxed-list");
    shared
        .pairing_list
        .set_selection_mode(gtk::SelectionMode::None);
    let scrolled = ScrolledWindow::builder()
        .child(&shared.pairing_list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    layout::page(
        "Pairing",
        "Accept or reject new pairing requests.",
        &scrolled,
    )
}
