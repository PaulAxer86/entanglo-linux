//! Live input status, Emergency Stop and controller settings, laid out in the
//! same sections as the Mac's Input Sharing page. Controller-only sections
//! appear once this instance is actually capturing local input.
//!
//! The global Emergency Stop hotkey can change state while this page is open,
//! and devices come and go, so a short poll keeps every row honest.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{Label, Orientation, Widget};

use super::layout;
use crate::edge::{EdgeTarget, ScreenEdge};
use crate::startup;
use crate::state::AppShared;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

type TargetChoices = Vec<Option<EdgeTarget>>;

struct EdgeRow {
    edge: ScreenEdge,
    row: adw::ComboRow,
    choices: Rc<RefCell<TargetChoices>>,
    updating: Rc<Cell<bool>>,
}

struct StatusWidgets {
    share: adw::SwitchRow,
    restart_row: adw::ActionRow,
    receiver: Label,
    active_target_row: adw::ActionRow,
    active_target: Label,
    controlled_by: Label,
    heartbeat: Label,
    stop_state: Label,
    stop_button: gtk::Button,
    edge_switching: adw::PreferencesGroup,
    edge_rows: Vec<EdgeRow>,
    shortcuts: adw::PreferencesGroup,
    devices: gtk::ListBox,
    drawn_devices: RefCell<Option<Vec<(String, bool)>>>,
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column = gtk::Box::new(Orientation::Vertical, 24);

    let sharing = section("Sharing");
    let share = adw::SwitchRow::builder()
        .title("Share my keyboard & mouse")
        .subtitle("Control other trusted devices from this computer")
        .use_markup(false)
        .active(startup::controller_mode_requested())
        .build();
    sharing.add(&share);
    let restart_row = plain_row("Restart Entanglo to apply this change");
    let restart_button = gtk::Button::with_label("Restart");
    restart_button.set_valign(gtk::Align::Center);
    restart_row.add_suffix(&restart_button);
    sharing.add(&restart_row);
    let (row, receiver) = value_row("Receive remote keyboard & mouse");
    sharing.add(&row);
    column.append(&sharing);

    let status = section("Status");
    let (active_target_row, active_target) = value_row("Active target");
    status.add(&active_target_row);
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

    let edge_switching = adw::PreferencesGroup::builder()
        .title("Edge Switching")
        .description("Push the cursor against an edge to move to that device. Requires an X11 session.")
        .build();
    let edge_rows: Vec<EdgeRow> = ScreenEdge::ALL.into_iter().map(|edge| {
        let row = adw::ComboRow::builder()
            .title(format!("{} → device", edge.label()))
            .use_markup(false)
            .build();
        edge_switching.add(&row);
        let choices: Rc<RefCell<TargetChoices>> = Rc::default();
        let updating = Rc::new(Cell::new(false));
        let shared_edge = Rc::clone(shared);
        let choices_changed = Rc::clone(&choices);
        let updating_changed = Rc::clone(&updating);
        row.connect_selected_notify(move |row| {
            if updating_changed.get() {
                return;
            }
            let target = choices_changed.borrow().get(row.selected() as usize).cloned().flatten();
            shared_edge.set_edge_target(edge, target);
        });
        EdgeRow { edge, row, choices, updating }
    }).collect();
    column.append(&edge_switching);

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
        share: share.clone(),
        restart_row,
        receiver,
        active_target_row,
        active_target,
        controlled_by,
        heartbeat,
        stop_state,
        stop_button: stop_button.clone(),
        edge_switching,
        edge_rows,
        shortcuts,
        devices,
        drawn_devices: RefCell::new(None),
    });
    refresh(shared, &widgets);

    let shared_share = Rc::clone(shared);
    let widgets_share = Rc::clone(&widgets);
    share.connect_active_notify(move |share| {
        if let Err(error) = startup::save_controller_mode(share.is_active()) {
            tracing::warn!(%error, "could not save the sharing preference");
        }
        refresh(&shared_share, &widgets_share);
    });

    restart_button.connect_clicked(|_| restart_application());

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
        "Choose how this device shares input and stop remote input immediately.",
        &scrolled,
    )
}

fn restart_application() {
    let restarted = std::env::current_exe().and_then(|executable| startup::spawn_replacement(&executable));
    match restarted {
        Ok(()) => {
            if let Some(application) = gtk::gio::Application::default() {
                application.quit();
            }
        }
        Err(error) => tracing::warn!(%error, "could not restart Entanglo"),
    }
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
    let capturing = startup::controller_mode_running();

    widgets.restart_row.set_visible(widgets.share.is_active() != capturing);
    widgets.receiver.set_text(if coordinator.receiver_enabled() {
        "Enabled"
    } else {
        "Unavailable: check 'input' group membership"
    });
    widgets.active_target_row.set_visible(capturing);
    let target = coordinator.active_receiver_sync().and_then(|conn_id| shared.device_name(conn_id));
    widgets.active_target.set_text(target.as_deref().unwrap_or("Local"));
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

    widgets.edge_switching.set_visible(capturing);
    if capturing {
        let targets = shared.control_targets();
        for edge_row in &widgets.edge_rows {
            redraw_edge_row(edge_row, &targets, shared.edge_target(edge_row.edge));
        }
    }

    let mut drawn = widgets.drawn_devices.borrow_mut();
    if drawn.as_ref() != Some(&connected) {
        redraw_devices(&widgets.devices, &connected);
        *drawn = Some(connected);
    }
}

fn redraw_edge_row(edge_row: &EdgeRow, targets: &[EdgeTarget], assigned: Option<EdgeTarget>) {
    let choices = target_choices(targets, assigned.as_ref());
    if *edge_row.choices.borrow() == choices {
        return;
    }
    let names: Vec<&str> = choices.iter()
        .map(|choice| choice.as_ref().map_or("None", |target| target.name.as_str()))
        .collect();
    let selected = choices.iter().position(|choice| {
        choice.as_ref().map(|target| &target.device_id) == assigned.as_ref().map(|target| &target.device_id)
    }).unwrap_or(0);
    edge_row.updating.set(true);
    edge_row.row.set_model(Some(&gtk::StringList::new(&names)));
    edge_row.row.set_selected(selected as u32);
    edge_row.updating.set(false);
    *edge_row.choices.borrow_mut() = choices;
}

/// "None", every available device, and the saved device even while offline.
fn target_choices(targets: &[EdgeTarget], assigned: Option<&EdgeTarget>) -> TargetChoices {
    let mut choices: TargetChoices = vec![None];
    choices.extend(targets.iter().cloned().map(Some));
    if let Some(assigned) = assigned {
        if !targets.iter().any(|target| target.device_id == assigned.device_id) {
            choices.push(Some(assigned.clone()));
        }
    }
    choices
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

    fn target(id: &str) -> EdgeTarget {
        EdgeTarget { device_id: id.into(), name: id.to_uppercase() }
    }

    #[test]
    fn heartbeat_needs_a_trusted_connection() {
        assert_eq!(heartbeat_label(&[]), "No connection");
        assert_eq!(heartbeat_label(&[("PC".into(), false)]), "No connection");
        assert_eq!(heartbeat_label(&[("PC".into(), false), ("Mac".into(), true)]), "Alive");
    }

    #[test]
    fn saved_device_stays_selectable_while_offline() {
        assert_eq!(target_choices(&[], Some(&target("mac"))), vec![None, Some(target("mac"))]);
    }

    #[test]
    fn online_saved_device_is_listed_once() {
        assert_eq!(target_choices(&[target("mac")], Some(&target("mac"))), vec![None, Some(target("mac"))]);
    }
}
