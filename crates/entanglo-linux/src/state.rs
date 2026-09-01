//! GTK-thread-only shared state. Lives entirely on the main context —
//! nothing here is `Send`, and nothing needs to be: `app_state.rs`'s
//! background Tokio thread only ever reaches this through the
//! `async-channel` of `CoordinatorEvent`s, consumed inside
//! `glib::spawn_future_local` (see `window.rs`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use entanglo_core::net::{ConnId, CoordinatorEvent};
use entanglo_core::protocol::payloads::PairRequestPayload;

use crate::app_state::Backend;

pub struct DeviceRow {
    pub device_id: Option<String>,
    pub name: String,
    pub trusted: bool,
}

pub struct PendingPairing {
    pub request: PairRequestPayload,
    pub respond: tokio::sync::oneshot::Sender<bool>,
}

/// Owns the long-lived widgets that need to be updated in place as
/// `CoordinatorEvent`s arrive, plus the data driving them. Pages
/// embed `devices_list`/`pairing_list` directly (see
/// `pages::devices`/`pages::pairing`) rather than each page rebuilding
/// its own list on every sidebar click — that would lose state (and
/// miss updates) while the user is looking at a different page.
pub struct AppShared {
    pub backend: Backend,
    pub devices_list: gtk::ListBox,
    pub pairing_list: gtk::ListBox,
    devices: RefCell<HashMap<ConnId, DeviceRow>>,
    pending_pairing: RefCell<HashMap<ConnId, PendingPairing>>,
}

impl AppShared {
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            devices_list: gtk::ListBox::new(),
            pairing_list: gtk::ListBox::new(),
            devices: RefCell::new(HashMap::new()),
            pending_pairing: RefCell::new(HashMap::new()),
        }
    }
}

pub fn handle_event(shared: &Rc<AppShared>, event: CoordinatorEvent) {
    match event {
        CoordinatorEvent::PeerConnected { conn_id, direction } => {
            shared.devices.borrow_mut().insert(
                conn_id,
                DeviceRow {
                    device_id: None,
                    name: format!("Connecting… ({direction:?})"),
                    trusted: false,
                },
            );
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerHello { conn_id, hello } => {
            if let Some(row) = shared.devices.borrow_mut().get_mut(&conn_id) {
                row.name = hello.device_name;
            }
            redraw_devices(shared);
        }
        CoordinatorEvent::PairingRequested {
            conn_id,
            request,
            respond,
        } => {
            shared
                .pending_pairing
                .borrow_mut()
                .insert(conn_id, PendingPairing { request, respond });
            redraw_pairing(shared);
        }
        CoordinatorEvent::Trusted { conn_id, device_id } => {
            if let Some(row) = shared.devices.borrow_mut().get_mut(&conn_id) {
                row.device_id = Some(device_id);
                row.trusted = true;
            }
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            redraw_devices(shared);
            redraw_pairing(shared);
        }
        CoordinatorEvent::PairingRejected { conn_id } => {
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            redraw_pairing(shared);
        }
        CoordinatorEvent::PeerDisconnected { conn_id, .. } => {
            shared.devices.borrow_mut().remove(&conn_id);
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            redraw_devices(shared);
            redraw_pairing(shared);
        }
        // Heartbeat/InputEvent/ReleaseControl don't have a Phase 1 UI
        // surface yet (Network page RTT tile and Input Sharing status
        // are both still placeholders — see ROADMAP.md). InputEvent
        // in particular is already injected on the backend thread
        // before it ever reaches this channel (see
        // `Coordinator::spawn_peer`'s relay loop), so there's nothing
        // for the UI to do with it today besides eventually showing
        // it happened.
        CoordinatorEvent::Heartbeat { .. }
        | CoordinatorEvent::InputEvent { .. }
        | CoordinatorEvent::ReleaseControl { .. } => {}
    }
}

fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn redraw_devices(shared: &Rc<AppShared>) {
    clear(&shared.devices_list);
    let devices = shared.devices.borrow();
    let mut entries: Vec<(&ConnId, &DeviceRow)> = devices.iter().collect();
    entries.sort_by_key(|(conn_id, _)| **conn_id);

    for (&conn_id, row) in entries {
        let label_text = match (&row.device_id, row.trusted) {
            (Some(id), true) => format!("{}  ·  {id}  ·  trusted", row.name),
            (Some(id), false) => format!("{}  ·  {id}  ·  pairing…", row.name),
            (None, _) => format!("{}  ·  connecting…", row.name),
        };

        let entry = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        entry.append(&gtk::Label::new(Some(&label_text)));

        if row.trusted {
            // Manual fallback for "who am I controlling" until real
            // edge-detection lands — see the ⬜ item in ROADMAP.md
            // Phase 1. Calls `Coordinator::set_active_receiver` on
            // the backend thread via its `Handle`, since this button
            // click runs on the GTK thread.
            let control_button = gtk::Button::with_label("Control this device");
            let handle = shared.backend.handle.clone();
            let coordinator = Arc::clone(&shared.backend.coordinator);
            control_button.connect_clicked(move |_| {
                let coordinator = Arc::clone(&coordinator);
                handle.spawn(async move {
                    coordinator.set_active_receiver(Some(conn_id)).await;
                });
            });
            entry.append(&control_button);
        }

        shared.devices_list.append(&entry);
    }
}

fn redraw_pairing(shared: &Rc<AppShared>) {
    clear(&shared.pairing_list);
    let pending = shared.pending_pairing.borrow();
    let conn_ids: Vec<ConnId> = pending.keys().copied().collect();
    drop(pending);

    for conn_id in conn_ids {
        let requester_name = shared
            .pending_pairing
            .borrow()
            .get(&conn_id)
            .map(|p| p.request.requester_device_name.clone())
            .unwrap_or_default();

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Label::new(Some(&format!(
            "{requester_name} wants to pair"
        ))));
        let accept = gtk::Button::with_label("Accept");
        let reject = gtk::Button::with_label("Reject");
        row.append(&accept);
        row.append(&reject);
        shared.pairing_list.append(&row);

        let shared_accept = Rc::clone(shared);
        accept.connect_clicked(move |_| {
            respond_to_pairing(&shared_accept, conn_id, true);
        });
        let shared_reject = Rc::clone(shared);
        reject.connect_clicked(move |_| {
            respond_to_pairing(&shared_reject, conn_id, false);
        });
    }
}

fn respond_to_pairing(shared: &Rc<AppShared>, conn_id: ConnId, accept: bool) {
    if let Some(pending) = shared.pending_pairing.borrow_mut().remove(&conn_id) {
        let _ = pending.respond.send(accept);
    }
    redraw_pairing(shared);
}
