//! Approve/reject incoming `pairRequest`, mirroring
//! `entanglo-macos`'s pairing UI. Phase 1, see `ROADMAP.md` and
//! `PROTOCOL.md` §5.2–§5.3. Live-updated from `CoordinatorEvent`s via
//! `state::handle_event` — see `state.rs` for the Accept/Reject
//! button wiring.

use std::rc::Rc;

use gtk::{ScrolledWindow, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    ScrolledWindow::builder()
        .child(&shared.pairing_list)
        .vexpand(true)
        .build()
        .into()
}
