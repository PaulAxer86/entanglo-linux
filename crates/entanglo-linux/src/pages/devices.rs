//! Discovered + trusted peer list, mirroring `entanglo-macos`'s
//! `DevicesView`. Phase 1, see `ROADMAP.md`. Live-updated from
//! `CoordinatorEvent`s via `state::handle_event` — see `state.rs`.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{ScrolledWindow, Widget};

use super::layout;
use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    shared.devices_list.set_show_separators(true);
    shared.devices_list.set_valign(gtk::Align::Start);
    shared
        .devices_list
        .set_selection_mode(gtk::SelectionMode::None);
    let scrolled = ScrolledWindow::builder()
        .child(&shared.devices_list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    layout::page(
        "Devices",
        "One row per device. Hover over the trust, mouse, keyboard and connection icons for details. Dimmed icons indicate unavailable features.",
        &scrolled,
    )
}
