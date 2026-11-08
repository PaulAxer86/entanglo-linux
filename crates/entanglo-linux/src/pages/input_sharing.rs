//! Live receiver status and Emergency Stop, laid out in the same sections as
//! the Mac's Input Sharing page. Only states this receiver really has are
//! shown; controller and edge-switching settings stay out until local input
//! capture is a supported mode.
//!
//! The global Emergency Stop hotkey can change state while this page is open,
//! and devices come and go, so a short poll keeps every row honest.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{Label, Orientation, Widget};

use super::layout;
use crate::state::AppShared;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

struct StatusWidgets {
    receiver: Label,
    controlled_by: Label,
    heartbeat: Label,
    stop_state: Label,
    stop_button: gtk::Button,
    shortcuts: adw::PreferencesGroup,
    devices: gtk::ListBox,
    drawn_devices: RefCell<Option<Vec<(String, bool)>>>,
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column = gtk::Box::new(Orientation::Vertical, 24);

    let sharing = section("Sharing");
    let (row, receiver) = value_row("Receive remote keyboard & mouse");
    sharing.add(&row);
    column.append(&sharing);

    let status = section("Status");
    let (row, controlled_by) = value_row("Controlled by");
    status.add(&row);
    let (row, heartbeat) = value_row("Heartbeat");
    status.add(&row);
    let (stop_row, stop_state) = value_row("Emergency Stop");
    let stop_button = gtk::Button::new();
    stop_button.set_valign(gtk::Align::Center);
    stop_row.add_suffix(&stop_button);
    status.add(&stop_row);
    column.append(&status);

    let connected = section("Connected devices");
    let devices = gtk::ListBox::new();
    devices.add_css_class("boxed-list");
    devices.set_selection_mode(gtk::SelectionMode::None);
    connected.add(&devices);
    column.append(&connected);

    let shortcuts = section("Shortcuts");
    let (row, shortcut) = value_row("Emergency stop");
    shortcut.set_text("Ctrl+Shift+Escape");
    shortcuts.add(&row);
    column.append(&shortcuts);

    let safety = section("Safety");
    let (row, _) = value_row(
        "Input injection is blocked unless the sender is trusted and has its Controller role \
         enabled, this device can write to /dev/uinput, and the emergency stop is not active.",
    );
    safety.add(&row);
    column.append(&safety);

    let widgets = Rc::new(StatusWidgets {
        receiver,
        controlled_by,
        heartbeat,
        stop_state,
        stop_button: stop_button.clone(),
        shortcuts,
        devices,
        drawn_devices: RefCell::new(None),
    });
    refresh(shared, &widgets);

    let shared_click = Rc::clone(shared);
    let widgets_click = Rc::clone(&widgets);
    stop_button.connect_clicked(move |_| {
        let coordinator = &shared_click.backend.coordinator;
        if coordinator.is_emergency_stopped() {
            coordinator.resume();
        } else {
            coordinator.emergency_stop();
        }
        refresh(&shared_click, &widgets_click);
    });

    let shared_poll = Rc::clone(shared);
    glib::source::timeout_add_local(POLL_INTERVAL, move || {
        refresh(&shared_poll, &widgets);
        glib::ControlFlow::Continue
    });

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&column)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    layout::page(
        "Input Sharing",
        "See who can control this device and stop remote input immediately.",
        &scrolled,
    )
}

fn section(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

/// Device names and "&" must not be parsed as Pango markup.
fn plain_row(title: &str) -> adw::ActionRow {
    adw::ActionRow::builder().title(title).use_markup(false).build()
}

fn value_row(title: &str) -> (adw::ActionRow, Label) {
    let row = plain_row(title);
    let value = Label::new(None);
    value.add_css_class("dim-label");
    row.add_suffix(&value);
    (row, value)
}

fn refresh(shared: &Rc<AppShared>, widgets: &StatusWidgets) {
    let coordinator = &shared.backend.coordinator;
    let connected = shared.connected_devices();

    widgets.receiver.set_text(if coordinator.receiver_enabled() {
        "Enabled"
    } else {
        "Unavailable: check 'input' group membership"
    });
    let controller = coordinator
        .receiving_controller_sync()
        .and_then(|conn_id| shared.device_name(conn_id));
    widgets.controlled_by.set_text(controller.as_deref().unwrap_or("None"));
    // A session closes as soon as its heartbeat times out, so a connected
    // trusted device always has a live heartbeat.
    widgets.heartbeat.set_text(heartbeat_label(&connected));

    let stopped = coordinator.is_emergency_stopped();
    widgets.stop_state.set_text(if stopped { "Stopped" } else { "Armed" });
    widgets.stop_button.set_label(if stopped { "Resume" } else { "Stop" });
    if stopped {
        widgets.stop_button.remove_css_class("destructive-action");
        widgets.stop_button.add_css_class("suggested-action");
    } else {
        widgets.stop_button.remove_css_class("suggested-action");
        widgets.stop_button.add_css_class("destructive-action");
    }

    // The hotkey is read by local input capture, which receiver mode leaves off.
    let (controller_enabled, _) = coordinator.controller_status();
    widgets.shortcuts.set_visible(controller_enabled);

    let mut drawn = widgets.drawn_devices.borrow_mut();
    if drawn.as_ref() != Some(&connected) {
        redraw_devices(&widgets.devices, &connected);
        *drawn = Some(connected);
    }
}

fn heartbeat_label(connected: &[(String, bool)]) -> &'static str {
    if connected.iter().any(|(_, trusted)| *trusted) { "Alive" } else { "No connection" }
}

fn redraw_devices(list: &gtk::ListBox, connected: &[(String, bool)]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if connected.is_empty() {
        list.append(&plain_row("No devices connected"));
    }
    for (name, trusted) in connected {
        let (row, value) = value_row(name);
        value.set_text(if *trusted { "Connected" } else { "Not trusted" });
        list.append(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_needs_a_trusted_connection() {
        assert_eq!(heartbeat_label(&[]), "No connection");
        assert_eq!(heartbeat_label(&[("PC".into(), false)]), "No connection");
        assert_eq!(heartbeat_label(&[("PC".into(), false), ("Mac".into(), true)]), "Alive");
    }
}
