//! Discovered + trusted peer list, mirroring `entanglo-macos`'s
//! `DevicesView`. Phase 1, see `ROADMAP.md`. Live-updated from
//! `CoordinatorEvent`s via `state::handle_event` — see `state.rs`.

use std::rc::Rc;

use gtk::{ScrolledWindow, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    ScrolledWindow::builder()
        .child(&shared.devices_list)
        .vexpand(true)
        .build()
        .into()
}
