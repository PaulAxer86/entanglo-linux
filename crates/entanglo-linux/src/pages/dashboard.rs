//! Local device identity (name, role, interfaces), mirroring
//! `entanglo-macos`'s `DashboardView`. Phase 1, see `ROADMAP.md`.
//! Interfaces/network info is still a placeholder — identity is real.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Label, Orientation, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let container = gtk::Box::new(Orientation::Vertical, 6);
    container.set_margin_top(16);
    container.set_margin_bottom(16);
    container.set_margin_start(16);
    container.set_margin_end(16);

    container.append(&Label::new(Some(&format!(
        "Device name: {}",
        shared.backend.local_hello.device_name
    ))));
    container.append(&Label::new(Some(&format!(
        "Device id: {}",
        shared.backend.local_device_id
    ))));
    container.append(&Label::new(Some(&format!(
        "App version: {}",
        shared.backend.local_hello.app_version
    ))));
    container.append(&Label::new(Some(
        "Roles: controller, receiver — Network interfaces (Phase 2)",
    )));

    container.into()
}
