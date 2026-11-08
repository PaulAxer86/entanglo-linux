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

use entanglo_core::features::network_quality::{NetworkQualityMonitor, SuggestedMode};
use entanglo_core::net::{trust_store, ConnId, CoordinatorEvent, Direction};
use entanglo_core::protocol::payloads::PairRequestPayload;

use crate::app_state::Backend;
use crate::clipboard::ClipboardBridge;
use crate::edge::{EdgeAssignments, EdgeTarget, ScreenEdge};
use crate::sound::{Cue, SoundPlayer};

const EDGE_SETTING: &str = "edge_assignments";

pub struct DeviceRow {
    pub direction: Direction,
    pub device_id: Option<String>,
    pub name: String,
    pub trusted: bool,
    pub roles: Vec<String>,
    pub platform: Option<String>,
    pub local_addr: Option<std::net::SocketAddr>,
    pub peer_addr: Option<std::net::SocketAddr>,
}

pub struct DashboardPeer {
    pub name: String,
    pub trusted: bool,
    pub interface: Option<String>,
    pub peer_address: Option<String>,
    pub rtt_ms: Option<f64>,
    pub online: bool,
}

pub struct PendingPairing {
    pub request: PairRequestPayload,
    pub respond: tokio::sync::oneshot::Sender<bool>,
}

struct NetworkStat {
    device_name: String,
    monitor: NetworkQualityMonitor,
    /// Locally-incrementing counter fed to `NetworkQualityMonitor`,
    /// which wants a sequence number to detect gaps — but
    /// `CoordinatorEvent::Heartbeat` only carries the computed RTT,
    /// not the peer's own sequence (see `net::session::SessionEvent`).
    /// Every arriving Heartbeat is recorded, so this never actually
    /// has a gap; the RTT averaging / suggested-mode logic that
    /// matters for this page doesn't depend on gap detection anyway.
    local_sequence: u64,
}

/// Snapshot for the Network page to render — deliberately not a
/// reference into `AppShared`'s internals, so the page can poll this
/// without holding a borrow across its own widget-building code.
pub struct NetworkStatSnapshot {
    pub device_name: String,
    pub average_rtt_ms: Option<f64>,
    pub suggested_mode: SuggestedMode,
}

/// Owns the long-lived widgets that need to be updated in place as
/// `CoordinatorEvent`s arrive, plus the data driving them. Pages
/// embed `devices_list` directly (see
/// `pages::devices`) rather than each page rebuilding
/// its own list on every sidebar click — that would lose state (and
/// miss updates) while the user is looking at a different page.
pub struct AppShared {
    pub backend: Backend,
    pub clipboard: ClipboardBridge,
    sounds: SoundPlayer,
    pub devices_list: gtk::ListBox,
    pub files_list: gtk::ListBox,
    transfers: RefCell<Vec<(ConnId, entanglo_core::features::file_transfer::FileProgress)>>,
    devices: RefCell<HashMap<ConnId, DeviceRow>>,
    discovered: RefCell<HashMap<String, entanglo_core::net::DiscoveredPeer>>,
    pending_pairing: RefCell<HashMap<ConnId, PendingPairing>>,
    network_stats: RefCell<HashMap<ConnId, NetworkStat>>,
    /// Which device each screen edge hands control to — set on the Input
    /// Sharing page, read by `edge::start`'s poll loop. See `edge.rs`.
    edge_assignments: RefCell<EdgeAssignments>,
}

impl AppShared {
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            clipboard: ClipboardBridge::new(),
            sounds: SoundPlayer::default(),
            devices_list: gtk::ListBox::new(),
            files_list: gtk::ListBox::new(),
            transfers: RefCell::new(saved_transfers()),
            devices: RefCell::new(HashMap::new()),
            discovered: RefCell::new(HashMap::new()),
            pending_pairing: RefCell::new(HashMap::new()),
            network_stats: RefCell::new(HashMap::new()),
            edge_assignments: RefCell::new(crate::edge::parse_assignments(
                &trust_store::read_setting(EDGE_SETTING).unwrap_or_default(),
            )),
        }
    }

    pub fn dashboard_peers(&self) -> Vec<DashboardPeer> {
        let devices = self.devices.borrow();
        device_groups(&devices).into_iter().filter_map(|group| {
            let id = group.iter().find(|id| devices[id].trusted
                && self.network_stats.borrow().get(id).is_some_and(|stat| stat.monitor.suggested_mode() != SuggestedMode::Offline))
                .or_else(|| group.iter().find(|id| devices[id].trusted)).unwrap_or(&group[0]);
            let row = &devices[id];
            row.device_id.as_ref()?;
            let stat = self.network_stats.borrow().get(id).map(|stat| (
                stat.monitor.average_rtt_ms(), stat.monitor.suggested_mode(),
            ));
            let interface = row.local_addr.and_then(|address| {
                self.backend.interfaces.iter().find(|(ip, _)| *ip == address.ip()).map(|(_, name)| name.clone())
            });
            Some(DashboardPeer {
                name: row.name.clone(), trusted: group.iter().any(|id| devices[id].trusted),
                interface, peer_address: row.peer_addr.map(|address| address.ip().to_string()),
                rtt_ms: stat.and_then(|s| s.0), online: stat.is_some_and(|s| s.1 != SuggestedMode::Offline),
            })
        }).collect()
    }

    /// One `(name, trusted)` entry per identified device.
    pub fn connected_devices(&self) -> Vec<(String, bool)> {
        let devices = self.devices.borrow();
        device_groups(&devices).into_iter().filter(|group| devices[&group[0]].device_id.is_some()).map(|group| {
            (devices[&group[0]].name.clone(), group.iter().any(|id| devices[id].trusted))
        }).collect()
    }

    pub fn device_name(&self, conn_id: ConnId) -> Option<String> {
        self.devices.borrow().get(&conn_id).map(|row| row.name.clone())
    }

    pub fn recent_transfers(&self) -> Vec<entanglo_core::features::file_transfer::FileProgress> {
        self.transfers.borrow().iter().take(3).map(|(_, progress)| progress.clone()).collect()
    }

    /// Looks up which trusted peer (if any) is assigned to `edge` —
    /// `edge::start`'s poll loop calls this on every crossing.
    pub fn conn_id_for_edge(&self, edge: ScreenEdge) -> Option<ConnId> {
        let assignments = self.edge_assignments.borrow();
        let target = assignments.get(&edge)?;
        self.devices.borrow().iter()
            .filter(|(_, row)| row.trusted && row.device_id.as_deref() == Some(target.device_id.as_str()))
            .map(|(conn_id, _)| *conn_id)
            .min()
    }

    pub fn edge_target(&self, edge: ScreenEdge) -> Option<EdgeTarget> {
        self.edge_assignments.borrow().get(&edge).cloned()
    }

    pub fn set_edge_target(&self, edge: ScreenEdge, target: Option<EdgeTarget>) {
        let mut assignments = self.edge_assignments.borrow_mut();
        match target {
            Some(target) => { assignments.insert(edge, target); }
            None => { assignments.remove(&edge); }
        }
        if let Err(error) = trust_store::write_setting(EDGE_SETTING, &crate::edge::serialize_assignments(&assignments)) {
            tracing::warn!(%error, "could not save edge assignments");
        }
    }

    /// Trusted, connected devices that accept remote input.
    pub fn control_targets(&self) -> Vec<EdgeTarget> {
        let devices = self.devices.borrow();
        device_groups(&devices).into_iter().filter_map(|group| {
            let id = group.iter().find(|id| devices[id].trusted)?;
            group.iter().any(|id| devices[id].roles.iter().any(|role| role == "receiver")).then(|| EdgeTarget {
                device_id: devices[id].device_id.clone().unwrap_or_default(),
                name: devices[id].name.clone(),
            })
        }).collect()
    }

    /// For the Network page's poll-based redraw — see
    /// `NetworkStatSnapshot`.
    pub fn network_stats_snapshot(&self) -> Vec<NetworkStatSnapshot> {
        let mut snapshot: Vec<NetworkStatSnapshot> = self
            .network_stats
            .borrow()
            .values()
            .map(|stat| NetworkStatSnapshot {
                device_name: stat.device_name.clone(),
                average_rtt_ms: stat.monitor.average_rtt_ms(),
                suggested_mode: stat.monitor.suggested_mode(),
            })
            .collect();
        snapshot.sort_by(|a, b| a.device_name.cmp(&b.device_name));
        snapshot
    }
}

pub fn handle_event(shared: &Rc<AppShared>, event: CoordinatorEvent) {
    match event {
        CoordinatorEvent::PeerDiscovered(peer) => {
            let mut discovered=shared.discovered.borrow_mut();
            if let Some(previous)=discovered.get_mut(&peer.device_name) {
                let mut addresses=previous.addresses.clone();addresses.extend(peer.addresses.iter().copied());addresses.sort();addresses.dedup();
                *previous=peer;previous.addresses=addresses;
            } else {discovered.insert(peer.device_name.clone(),peer);}
            drop(discovered);
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerDiscoveryRemoved(name) => {
            shared.discovered.borrow_mut().remove(&name);
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerConnected { conn_id, direction, local_addr, peer_addr } => {
            shared.devices.borrow_mut().insert(
                conn_id,
                DeviceRow {
                    direction,
                    device_id: None,
                    name: format!("Connecting… ({direction:?})"),
                    trusted: false,
                    roles: Vec::new(),
                    platform: None,
                    local_addr,
                    peer_addr,
                },
            );
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerIdentified { conn_id, device_id } => {
            // Fires before any trust decision — see the doc comment
            // on `CoordinatorEvent::PeerIdentified`. This is what
            // makes the "Trust" button below possible for peers
            // (like the real `entanglo-macos` v0.1.58) that never
            // send us a `pairRequest`.
            if let Some(row) = shared.devices.borrow_mut().get_mut(&conn_id) {
                row.device_id = Some(device_id);
            }
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerHello { conn_id, hello } => {
            if let Some(row) = shared.devices.borrow_mut().get_mut(&conn_id) {
                row.name = hello.device_name;
                row.roles = hello.roles;
                row.platform = hello.platform;
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
            redraw_devices(shared);
        }
        CoordinatorEvent::Trusted { conn_id, device_id } => {
            if let Some(row) = shared.devices.borrow_mut().get_mut(&conn_id) {
                row.device_id = Some(device_id);
                row.trusted = true;
            }
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            redraw_devices(shared);
            shared.sounds.play(Cue::Connected);
        }
        CoordinatorEvent::PairingRejected { conn_id } => {
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            redraw_devices(shared);
        }
        CoordinatorEvent::PeerDisconnected { conn_id, .. } => {
            let was_trusted = shared.devices.borrow_mut().remove(&conn_id).is_some_and(|row| row.trusted);
            shared.pending_pairing.borrow_mut().remove(&conn_id);
            shared.network_stats.borrow_mut().remove(&conn_id);
            redraw_devices(shared);
            let mut transfer_interrupted = false;
            for (owner, progress) in shared.transfers.borrow_mut().iter_mut() {
                if *owner == conn_id && !progress.complete && progress.error.is_none() {
                    progress.error = Some("Connection closed before transfer completed".into());
                    progress.updated_at = std::time::SystemTime::now();
                    transfer_interrupted = true;
                }
            }
            redraw_files(shared);
            if transfer_interrupted {
                shared.sounds.play(Cue::Error);
            } else if was_trusted {
                shared.sounds.play(Cue::Disconnected);
            }
        }
        CoordinatorEvent::Heartbeat { conn_id, rtt_ms } => {
            let device_name = shared
                .devices
                .borrow()
                .get(&conn_id)
                .map(|row| row.name.clone())
                .unwrap_or_else(|| format!("peer {conn_id}"));
            let mut stats = shared.network_stats.borrow_mut();
            let stat = stats.entry(conn_id).or_insert_with(|| NetworkStat {
                device_name: device_name.clone(),
                monitor: NetworkQualityMonitor::new(),
                local_sequence: 0,
            });
            stat.device_name = device_name;
            stat.monitor.record_heartbeat(stat.local_sequence, rtt_ms);
            stat.local_sequence += 1;
        }
        CoordinatorEvent::FileTransfer { conn_id, mut progress } => {
            if progress.error.is_some() {
                shared.sounds.play(Cue::Error);
            } else if progress.complete {
                shared.sounds.play(Cue::FileReceived);
            }
            let mut transfers = shared.transfers.borrow_mut();
            if let Some((_, previous)) = transfers.iter_mut().find(|(owner, item)| *owner == conn_id && item.transfer_id == progress.transfer_id) {
                if progress.name.is_empty() {
                    progress.name = previous.name.clone();
                    progress.received = previous.received;
                    progress.total = previous.total;
                }
                *previous = progress;
            } else {
                transfers.insert(0, (conn_id, progress));
                if transfers.len() > 100 {
                    if let Some(index) = transfers.iter().rposition(|(_, p)| p.complete || p.error.is_some()) { transfers.remove(index); }
                }
            }
            drop(transfers);
            redraw_files(shared);
        }
        CoordinatorEvent::ClipboardText { text, .. } => {
            shared.clipboard.apply_remote(&text);
        }
        // InputEvent/ReleaseControl don't have a Phase 1 UI surface
        // yet. InputEvent in particular is already injected on the
        // backend thread before it ever reaches this channel (see
        // `Coordinator::spawn_peer`'s relay loop), so there's nothing
        // for the UI to do with it today besides eventually showing
        // it happened.
        CoordinatorEvent::InputEvent { .. } | CoordinatorEvent::ReleaseControl { .. } => {}
    }
}

fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn device_groups(devices: &HashMap<ConnId, DeviceRow>) -> Vec<Vec<ConnId>> {
    let mut by_device: HashMap<String, Vec<ConnId>> = HashMap::new();
    for (&conn_id, row) in devices {
        let key = row.device_id.as_ref().map(|id| format!("device:{id}"))
            .unwrap_or_else(|| format!("pending:{conn_id}"));
        by_device.entry(key).or_default().push(conn_id);
    }
    let mut groups: Vec<_> = by_device.into_values().collect();
    for group in &mut groups { group.sort_unstable(); }
    groups.sort_by_key(|group| group[0]);
    groups
}

fn capability_icon(icon: &str, label: &str, available: bool) -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let image = gtk::Image::from_icon_name(icon);
    item.append(&image);
    item.set_tooltip_text(Some(label));
    if !available { item.set_opacity(0.3); }
    item
}

fn redraw_devices(shared: &Rc<AppShared>) {
    clear(&shared.devices_list);
    let devices = shared.devices.borrow();
    for group in device_groups(&devices) {
        // Prefer a trusted connection. Device identity, not its name, defines a row.
        let conn_id = *group.iter().find(|id| devices[id].trusted)
            .unwrap_or(&group[0]);
        let row = &devices[&conn_id];
        let trusted = group.iter().any(|id| devices[id].trusted);
        let peer_controller = group.iter().any(|id| devices[id].roles.iter().any(|role| role == "controller"));
        let peer_receiver = group.iter().any(|id| devices[id].roles.iter().any(|role| role == "receiver"));
        let stopped = shared.backend.coordinator.is_emergency_stopped();
        let can_receive = trusted && peer_controller && shared.backend.coordinator.receiver_enabled() && !stopped;
        let (controller, count) = shared.backend.coordinator.controller_status();
        let can_control = trusted && peer_receiver && controller && count > 0 && !stopped;
        let entry = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        entry.set_margin_top(10);
        entry.set_margin_bottom(10);
        entry.set_margin_start(12);
        entry.set_margin_end(12);

        let identity = gtk::Box::new(gtk::Orientation::Vertical, 3);
        identity.set_hexpand(true);
        identity.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&row.name));
        title.set_xalign(0.0);
        title.add_css_class("heading");
        identity.append(&title);
        let detail = gtk::Label::new(Some(&format!("{} · {}",
            row.platform.as_deref().unwrap_or("Device"),
            if trusted { "Trusted" } else { "Not trusted" })));
        detail.set_xalign(0.0);
        detail.add_css_class("dim-label");
        identity.append(&detail);
        let connections = group.iter().map(|id| match devices[id].direction {
            Direction::Inbound => "Inbound: connection opened by this device",
            Direction::Outbound => "Outbound: connection opened by this iMac",
        }).collect::<Vec<_>>().join("\n");
        identity.set_tooltip_text(Some(&format!("{}\n{connections}", row.device_id.as_deref().unwrap_or("Identifying device"))));
        entry.append(&identity);

        let capabilities = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        capabilities.set_valign(gtk::Align::Center);
        capabilities.append(&capability_icon("security-high-symbolic",
            if trusted { "Device trusted on this iMac" } else { "Permission required: click Trust" }, trusted));
        let receive_hint = if can_receive {
            "This device can control the mouse and keyboard on this iMac"
        } else if !trusted {
            "Mouse and keyboard blocked: device not trusted"
        } else if stopped {
            "Mouse and keyboard blocked: emergency stop is active"
        } else if !shared.backend.coordinator.receiver_enabled() {
            "Input reception unavailable: check permissions in Input Sharing"
        } else {
            "The device has not enabled its Controller role"
        };
        capabilities.append(&capability_icon("input-mouse-symbolic", receive_hint, can_receive));
        capabilities.append(&capability_icon("input-keyboard-symbolic", receive_hint, can_receive));
        capabilities.append(&capability_icon("text-x-generic-symbolic",
            if trusted { "Receive files from this device into Downloads/Entanglo" }
            else { "File reception blocked: device not trusted" }, trusted));
        capabilities.append(&capability_icon("network-wired-symbolic", &format!("Device connected\n{connections}"), true));
        entry.append(&capabilities);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_valign(gtk::Align::Center);

        let pending_request = group.iter().copied()
            .find(|id| shared.pending_pairing.borrow().contains_key(id));
        if let Some(pending_id) = pending_request {
            // The session waits for this answer, so a device with an open
            // pairRequest is answered here rather than through manual trust.
            let accept = gtk::Button::with_label("Accept");
            let requester = shared.pending_pairing.borrow().get(&pending_id)
                .map(|pending| pending.request.requester_device_name.clone()).unwrap_or_default();
            accept.set_tooltip_text(Some(&format!("{requester} wants to pair with this device")));
            let shared_accept = Rc::clone(shared);
            accept.connect_clicked(move |_| respond_to_pairing(&shared_accept, pending_id, true));
            actions.append(&accept);
            let reject = gtk::Button::with_label("Reject");
            let shared_reject = Rc::clone(shared);
            reject.connect_clicked(move |_| respond_to_pairing(&shared_reject, pending_id, false));
            actions.append(&reject);
        } else if row.device_id.is_some() && !trusted {
            // Manual trust, matching how the real entanglo-macos
            // v0.1.58 actually works: reading its source shows it
            // never sends or handles pairRequest/pairResponse at all
            // (PairingState.swift: "the wire-side PIN ceremony lands
            // in 0.2") — each side just decides who to trust from its
            // own local Devices list.
            let trust_button = gtk::Button::with_label("Trust");
            let handle = shared.backend.handle.clone();
            let coordinator = Arc::clone(&shared.backend.coordinator);
            trust_button.connect_clicked(move |_| {
                let coordinator = Arc::clone(&coordinator);
                handle.spawn(async move {
                    coordinator.trust_manually(conn_id).await;
                });
            });
            actions.append(&trust_button);
        }

        if can_control {
            // Immediate manual fallback for "who am I controlling" —
            // calls `Coordinator::set_active_receiver` on the backend
            // thread via its `Handle`, since this button click runs
            // on the GTK thread.
            let control_button = gtk::Button::from_icon_name("input-mouse-symbolic");
            control_button.set_tooltip_text(Some("Control the mouse and keyboard on this device"));
            let handle = shared.backend.handle.clone();
            let coordinator = Arc::clone(&shared.backend.coordinator);
            control_button.connect_clicked(move |_| {
                let coordinator = Arc::clone(&coordinator);
                handle.spawn(async move {
                    coordinator.set_active_receiver(Some(conn_id)).await;
                });
            });
            actions.append(&control_button);

        }

        if actions.first_child().is_some() {
            entry.append(&actions);
        }

        shared.devices_list.append(&entry);
    }
    let discovered = shared.discovered.borrow();
    for peer in discovered.values() {
        let connected = devices.values().any(|row| row.peer_addr.is_some_and(|address| {
            peer.addresses.contains(&address.ip().to_canonical())
        }));
        if connected { continue; }
        // Resolve metadata is not authorization; trust is granted only after
        // the peer's real protocol identity has been exchanged.
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(12);row.set_margin_bottom(12);row.set_margin_start(12);row.set_margin_end(12);
        let name = peer.device_name.strip_suffix("._entanglo._tcp.local.").unwrap_or(&peer.device_name);
        let label = gtk::Label::new(Some(name));label.set_hexpand(true);label.set_xalign(0.0);label.add_css_class("heading");row.append(&label);
        let status = gtk::Label::new(Some("Discovered · waiting for connection"));status.add_css_class("dim-label");row.append(&status);
        let connect = gtk::Button::with_label("Connect");
        connect.set_tooltip_text(Some("Exchange identities with this device; saved trust will be reused"));
        let coordinator = Arc::clone(&shared.backend.coordinator);
        let handle = shared.backend.handle.clone();
        let address = peer.endpoint();
        connect.set_sensitive(address.is_some());
        connect.connect_clicked(move |_| {
            if let Some(address) = address {
                let coordinator = Arc::clone(&coordinator);
                handle.spawn(async move {
                    if let Err(error) = coordinator.dial(address).await { tracing::warn!(%error, "manual connection failed"); }
                });
            }
        });
        row.append(&connect);
        row.set_tooltip_text(Some(&format!("Bonjour detected this device. Waiting for a protocol connection; no new device identity has been created.\n{}:{}",peer.host,peer.port)));
        shared.devices_list.append(&row);
    }

}

fn redraw_files(shared: &Rc<AppShared>) {
    clear(&shared.files_list);
    let transfers = shared.transfers.borrow();
    if transfers.is_empty() {
        let empty = gtk::Label::new(Some("No incoming files yet. Send a file from a trusted Entanglo device."));
        empty.set_wrap(true);
        empty.set_margin_top(18);
        empty.set_margin_bottom(18);
        shared.files_list.append(&empty);
    }
    for (conn_id, progress) in transfers.iter() {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.set_margin_top(12); row.set_margin_bottom(12);
        row.set_margin_start(12); row.set_margin_end(12);
        let title = gtk::Label::new(Some(&progress.name));
        title.set_xalign(0.0); title.add_css_class("heading"); row.append(&title);
        let peer = shared.devices.borrow().get(conn_id).map(|p| p.name.clone()).unwrap_or_else(|| "Entanglo device".into());
        let status = transfer_summary(progress, &peer, &format_timestamp(progress.updated_at));
        let detail = gtk::Label::new(Some(&status)); detail.set_xalign(0.0);
        detail.set_wrap(true); detail.set_selectable(true); detail.add_css_class("dim-label"); row.append(&detail);
        if !progress.complete && progress.error.is_none() {
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(if progress.total == 0 { 0.0 } else { progress.received as f64 / progress.total as f64 });
            row.append(&bar);
        }
        row.set_hexpand(true);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let direction = gtk::Image::from_icon_name("go-down-symbolic");
        direction.set_tooltip_text(Some("Received"));
        direction.set_valign(gtk::Align::Start);
        direction.set_margin_top(14);
        direction.set_margin_start(12);
        line.append(&direction);
        line.append(&row);
        shared.files_list.append(&line);
    }
}

fn format_timestamp(time: std::time::SystemTime) -> String {
    time.duration_since(std::time::UNIX_EPOCH).ok()
        .and_then(|elapsed| glib::DateTime::from_unix_local(elapsed.as_secs() as i64).ok())
        .and_then(|local| local.format("%-d %b %Y, %H:%M").ok())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn transfer_summary(progress: &entanglo_core::features::file_transfer::FileProgress, peer: &str, when: &str) -> String {
    let size = glib::format_size(progress.total);
    if let Some(error) = &progress.error {
        // A rejected chunk with no earlier offer has no known size.
        if progress.total == 0 { format!("Failed {when} · From {peer} · {error}") }
        else { format!("Failed {when} · {size} · From {peer} · {error}") }
    } else if let Some(path) = &progress.destination {
        format!("Received {when} · {size} · From {peer} · Saved to {}", path.display())
    } else {
        format!("Receiving · {} of {size} · From {peer}", glib::format_size(progress.received))
    }
}

#[cfg(test)]
mod transfer_summary_tests {
    use super::*;
    use entanglo_core::features::file_transfer::FileProgress;

    fn progress(total: u64) -> FileProgress {
        FileProgress { transfer_id: "id".into(), name: "a.png".into(), received: total, total,
            destination: None, error: None, complete: false, updated_at: std::time::UNIX_EPOCH }
    }

    #[test]
    fn received_file_shows_time_size_and_peer() {
        let mut item = progress(5_000_000);
        item.destination = Some("/tmp/a.png".into());
        item.complete = true;
        let text = transfer_summary(&item, "iMac", "8 Nov 2026, 13:00");
        assert!(text.starts_with("Received 8 Nov 2026, 13:00 · "));
        assert!(text.contains(glib::format_size(5_000_000).as_str()));
        assert!(text.contains("From iMac"));
    }

    #[test]
    fn failure_without_known_size_omits_the_size() {
        let mut item = progress(0);
        item.error = Some("Rejected".into());
        assert_eq!(transfer_summary(&item, "PC", "now"), "Failed now · From PC · Rejected");
    }
}

pub fn initialize_files(shared: &Rc<AppShared>) { redraw_files(shared); }

fn respond_to_pairing(shared: &Rc<AppShared>, conn_id: ConnId, accept: bool) {
    if let Some(pending) = shared.pending_pairing.borrow_mut().remove(&conn_id) {
        let _ = pending.respond.send(accept);
    }
    redraw_devices(shared);
}

#[cfg(test)]
mod device_group_tests {
    use super::*;

    fn row(id: Option<&str>, direction: Direction) -> DeviceRow {
        DeviceRow {
            device_id: id.map(str::to_owned), name: "iMac".into(), trusted: true,
            direction, roles: vec!["controller".into()], platform: Some("macOS".into()), local_addr: None, peer_addr: None,
        }
    }

    #[test]
    fn inbound_and_outbound_for_one_device_share_a_row() {
        let devices = HashMap::from([
            (2, row(Some("mac-one"), Direction::Outbound)),
            (3, row(Some("mac-one"), Direction::Inbound)),
        ]);
        assert_eq!(device_groups(&devices), vec![vec![2, 3]]);
    }

    #[test]
    fn same_names_and_unidentified_connections_are_not_merged() {
        let devices = HashMap::from([
            (1, row(Some("mac-one"), Direction::Inbound)),
            (2, row(Some("mac-two"), Direction::Inbound)),
            (3, row(None, Direction::Inbound)),
            (4, row(None, Direction::Outbound)),
        ]);
        assert_eq!(device_groups(&devices), vec![vec![1], vec![2], vec![3], vec![4]]);
    }
}

fn saved_transfers() -> Vec<(ConnId, entanglo_core::features::file_transfer::FileProgress)> {
    let folder = entanglo_core::features::file_transfer::FileTransferService::download_dir();
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new(); };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).filter_map(|entry| {
        if !entry.file_type().ok()?.is_file() || entry.file_name().to_string_lossy().starts_with('.') { return None; }
        let metadata = entry.metadata().ok()?;
        Some((metadata.modified().ok()?, entry.path(), metadata.len()))
    }).collect();
    entries.sort_by_key(|item| std::cmp::Reverse(item.0));
    entries.into_iter().take(20).map(|(modified, path, size)| {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        (ConnId::MAX, entanglo_core::features::file_transfer::FileProgress {
            transfer_id: format!("saved:{name}"), name, received: size, total: size,
            destination: Some(path), error: None, complete: true, updated_at: modified,
        })
    }).collect()
}
