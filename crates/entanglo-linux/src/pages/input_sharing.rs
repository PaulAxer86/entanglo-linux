//! Live controller/receiver status + Emergency Stop, mirroring the
//! Mac's triple-Escape + explicit button (`ROADMAP.md` Phase 1).
//! Edge-switching itself is still ⬜ (see `ROADMAP.md` Risks) — the
//! Devices page's manual "Control this device" button is the interim
//! stand-in for choosing a target.
//!
//! Emergency Stop can also be toggled by the global Ctrl+Shift+Escape
//! hotkey (`net::coordinator::is_emergency_stop_hotkey`), which works
//! even when this window doesn't have focus — so the button's label
//! can't just track its own clicks; a short poll keeps it honest.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Label, Orientation, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let container = gtk::Box::new(Orientation::Vertical, 8);
    container.set_margin_top(16);
    container.set_margin_bottom(16);
    container.set_margin_start(16);
    container.set_margin_end(16);

    let (controller_enabled, device_count) = shared.backend.coordinator.controller_status();
    let controller_label = Label::new(Some(&if controller_enabled {
        format!("Controller: enabled ({device_count} local input device(s))")
    } else {
        "Controller: unavailable — check 'input' group membership".to_string()
    }));
    controller_label.set_halign(gtk::Align::Start);
    container.append(&controller_label);

    let receiver_label = Label::new(Some(&if shared.backend.coordinator.receiver_enabled() {
        "Receiver: enabled (/dev/uinput open)".to_string()
    } else {
        "Receiver: unavailable — check 'input' group membership".to_string()
    }));
    receiver_label.set_halign(gtk::Align::Start);
    container.append(&receiver_label);

    let status_label = Label::new(None);
    status_label.set_halign(gtk::Align::Start);
    container.append(&status_label);

    let stop_button = gtk::Button::new();
    container.append(&stop_button);

    refresh(shared, &status_label, &stop_button);

    let shared_click = Rc::clone(shared);
    let status_label_click = status_label.clone();
    let stop_button_click = stop_button.clone();
    stop_button.connect_clicked(move |_| {
        let coordinator = &shared_click.backend.coordinator;
        if coordinator.is_emergency_stopped() {
            coordinator.resume();
        } else {
            coordinator.emergency_stop();
        }
        refresh(&shared_click, &status_label_click, &stop_button_click);
    });

    // Picks up the global hotkey toggling emergency-stop while this
    // page happens to be open — see the module doc comment.
    let shared_poll = Rc::clone(shared);
    let status_label_poll = status_label.clone();
    let stop_button_poll = stop_button.clone();
    glib::source::timeout_add_local(std::time::Duration::from_millis(500), move || {
        refresh(&shared_poll, &status_label_poll, &stop_button_poll);
        glib::ControlFlow::Continue
    });

    container.into()
}

fn refresh(shared: &Rc<AppShared>, status_label: &Label, stop_button: &gtk::Button) {
    if shared.backend.coordinator.is_emergency_stopped() {
        status_label.set_text("Status: STOPPED — no input is being sent or received");
        stop_button.set_label("Resume");
        stop_button.add_css_class("suggested-action");
    } else {
        status_label.set_text("Status: active");
        stop_button.set_label("Emergency Stop");
        stop_button.remove_css_class("suggested-action");
        stop_button.add_css_class("destructive-action");
    }
}
