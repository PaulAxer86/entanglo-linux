//! Live connection overview following the macOS Dashboard layout.
use std::rc::Rc;
use gtk::prelude::*;
use gtk::{Label, Orientation, Widget};
use crate::state::AppShared;
use super::layout;

fn card(title: &str, subtitle: &str) -> gtk::Box {
    let outer = gtk::Box::new(Orientation::Vertical, 0);
    outer.add_css_class("card");
    outer.set_hexpand(true);
    let inner = gtk::Box::new(Orientation::Vertical, 9);
    inner.set_margin_top(16); inner.set_margin_bottom(16);
    inner.set_margin_start(16); inner.set_margin_end(16);
    let heading = Label::new(Some(title));
    heading.set_xalign(0.0); heading.add_css_class("heading"); inner.append(&heading);
    if !subtitle.is_empty() {
        let note = Label::new(Some(subtitle));
        note.set_xalign(0.0); note.set_wrap(true); note.add_css_class("dim-label"); inner.append(&note);
    }
    outer.append(&inner);
    outer
}
fn body(card: &gtk::Box) -> gtk::Box { card.first_child().unwrap().downcast().unwrap() }
fn value(container: &gtk::Box, key: &str) -> Label {
    let row = gtk::Box::new(Orientation::Horizontal, 12);
    let label = Label::new(Some(key)); label.set_xalign(0.0); label.add_css_class("dim-label");
    label.set_hexpand(true); row.append(&label);
    let result = Label::new(None); result.set_xalign(1.0); result.set_selectable(true);
    result.set_max_width_chars(30); result.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    row.append(&result); container.append(&row); result
}
fn replace_rows(container: &gtk::Box) { while let Some(child) = container.first_child() { container.remove(&child); } }
fn interface_label(name: Option<&str>) -> String {
    match name {
        Some(name) if name.starts_with("wl") => format!("Wi-Fi · {name}"),
        Some(name) if name.starts_with("en") || name.starts_with("eth") => format!("Ethernet · {name}"),
        Some(name) => name.to_string(),
        None => "Waiting for connection".into(),
    }
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column = gtk::Box::new(Orientation::Vertical, 14);
    let hero = gtk::Box::new(Orientation::Horizontal, 14); hero.add_css_class("card");
    let hero_inner = gtk::Box::new(Orientation::Horizontal, 14);
    hero_inner.set_margin_top(16); hero_inner.set_margin_bottom(16);
    hero_inner.set_margin_start(16); hero_inner.set_margin_end(16); hero_inner.set_hexpand(true);
    let status_icon = gtk::Image::from_icon_name("emblem-ok-symbolic"); status_icon.set_pixel_size(30);
    hero_inner.append(&status_icon);
    let status_text = gtk::Box::new(Orientation::Vertical, 4); status_text.set_hexpand(true);
    let status = Label::new(None); status.set_xalign(0.0); status.add_css_class("title-3");
    let status_detail = Label::new(None); status_detail.set_xalign(0.0); status_detail.set_wrap(true); status_detail.add_css_class("dim-label");
    status_text.append(&status); status_text.append(&status_detail); hero_inner.append(&status_text);
    let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic"); pause.set_valign(gtk::Align::Center);
    let state = Rc::clone(shared);
    pause.connect_clicked(move |_| {
        if state.backend.coordinator.is_emergency_stopped() { state.backend.coordinator.resume(); }
        else { state.backend.coordinator.emergency_stop(); }
    });
    hero_inner.append(&pause); hero.append(&hero_inner); column.append(&hero);

    let grid = gtk::Grid::builder().column_spacing(14).row_spacing(14).column_homogeneous(true).build();
    let local = card("This Device", "Local identity and sharing permissions");
    let local_body = body(&local);
    value(&local_body, "Name").set_text(&shared.backend.local_hello.device_name);
    let id = value(&local_body, "Device ID");
    id.set_text(&shared.backend.local_device_id); id.set_tooltip_text(Some(&shared.backend.local_device_id));
    let roles = value(&local_body, "Role");
    roles.set_text(if std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_some() { "Receiver" } else { "Controller + receiver" });
    let input = value(&local_body, "Mouse & keyboard");
    let files = value(&local_body, "File reception");
    let clipboard = value(&local_body, "Clipboard sync");
    grid.attach(&local, 0, 0, 1, 1);

    let network = card("Network Quality", "Active connection to your Entanglo peer");
    let network_body = body(&network);
    let iface = value(&network_body, "Interface");
    let address = value(&network_body, "Peer address");
    let latency = value(&network_body, "Latency");
    let heartbeat = value(&network_body, "Heartbeat");
    let quality = value(&network_body, "Quality");
    grid.attach(&network, 1, 0, 1, 1);

    let pairing = card("Pairing", "Connected devices, grouped by identity");
    let peer_rows = gtk::Box::new(Orientation::Vertical, 8); body(&pairing).append(&peer_rows);
    grid.attach(&pairing, 0, 1, 1, 1);
    let recent = card("Recent Transfers", "Last 3 incoming files");
    let transfer_rows = gtk::Box::new(Orientation::Vertical, 8); body(&recent).append(&transfer_rows);
    grid.attach(&recent, 1, 1, 1, 1); column.append(&grid);

    let quick = card("Quick Actions", "Actions available on this computer");
    let actions = gtk::Box::new(Orientation::Horizontal, 10);
    let downloads = gtk::Button::with_label("Open received files");
    downloads.set_tooltip_text(Some("Open the folder containing files received from Entanglo devices"));
    downloads.connect_clicked(|_| {
        let folder = entanglo_core::features::file_transfer::FileTransferService::download_dir();
        if std::fs::create_dir_all(&folder).is_ok() { let _ = std::process::Command::new("xdg-open").arg(folder).spawn(); }
    });
    actions.append(&downloads);
    let reconnect = gtk::Button::with_label("Reconnect");
    let state = Rc::clone(shared);
    reconnect.connect_clicked(move |_| {
        if let Some(service) = state.backend.discovery.clone() {
            state.backend.handle.spawn_blocking(move || {
                if let Err(error) = service.refresh() { tracing::warn!(%error, "manual reconnect failed"); }
            });
        }
    });
    actions.append(&reconnect);
    let clipboard_toggle = gtk::CheckButton::with_label("Sync clipboard");
    let state = Rc::clone(shared);
    clipboard_toggle.connect_toggled(move |toggle| { state.clipboard.set_enabled(toggle.is_active()); });
    actions.append(&clipboard_toggle); body(&quick).append(&actions); column.append(&quick);
    let footer = Label::new(Some(&format!("Entanglo {} · Linux / {}", shared.backend.local_hello.app_version,
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown session".into()))));
    footer.set_xalign(1.0); footer.add_css_class("dim-label"); column.append(&footer);

    let state = Rc::clone(shared);
    let refresh = move || {
        let peers = state.dashboard_peers();
        let trusted: Vec<_> = peers.iter().filter(|peer| peer.trusted && peer.online).collect();
        let stopped = state.backend.coordinator.is_emergency_stopped();
        let receiver = state.backend.coordinator.receiver_enabled();
        let receiving = state.backend.coordinator.receiving_controller_sync().is_some();
        let controller = state.backend.coordinator.active_receiver_sync().is_some();
        let (title, detail, icon) = if stopped {
            ("Input sharing paused".to_string(), "File reception remains available to trusted devices.".to_string(), "media-playback-pause-symbolic")
        } else if !receiver {
            ("Input permission required".into(), "Open Settings to check access to mouse and keyboard devices.".into(), "dialog-warning-symbolic")
        } else if peers.is_empty() {
            ("Ready to connect".into(), "Open Entanglo on your other device on the same network.".into(), "network-offline-symbolic")
        } else if !peers.iter().any(|peer| peer.trusted) {
            ("Trust required".into(), "Authorize the device in Devices before sharing input or files.".into(), "security-low-symbolic")
        } else if trusted.is_empty() {
            ("Connecting…".into(), "Waiting for a live heartbeat from the trusted device.".into(), "emblem-ok-symbolic")
        } else {
            let names = trusted.iter().map(|peer| peer.name.as_str()).collect::<Vec<_>>().join(", ");
            let title = if controller { "Controlling remote device" } else if receiving { "Receiving remote input" } else { "Connected and ready" };
            (title.into(), format!("Connected to {names} · mouse, keyboard and file reception ready"), "emblem-ok-symbolic")
        };
        status.set_text(&title); status_detail.set_text(&detail); status_icon.set_icon_name(Some(icon));
        pause.set_icon_name(if stopped { "media-playback-start-symbolic" } else { "media-playback-pause-symbolic" });
        pause.set_tooltip_text(Some(if stopped { "Resume mouse and keyboard sharing" } else { "Pause mouse and keyboard sharing" }));
        input.set_text(if !receiver { "Permission required" } else if stopped { "Paused" } else { "Ready" });
        files.set_text(if peers.iter().any(|p| p.trusted) { "Ready · trusted devices" } else { "Waiting for trusted device" });
        clipboard.set_text(if state.clipboard.enabled() { "On · plain text" } else { "Off" });
        clipboard_toggle.set_active(state.clipboard.enabled());
        let active = trusted.first().copied().or_else(|| peers.first());
        iface.set_text(&interface_label(active.and_then(|p| p.interface.as_deref())));
        address.set_text(active.and_then(|p| p.peer_address.as_deref()).unwrap_or("—"));
        address.set_tooltip_text(active.and_then(|p| p.peer_address.as_deref()));
        latency.set_text(&active.and_then(|p| p.rtt_ms).map(|ms| format!("{ms:.1} ms TCP RTT")).unwrap_or_else(|| "Measuring…".into()));
        heartbeat.set_text(if active.is_some_and(|p| p.online) { "Healthy" } else { "Waiting" });
        let network_quality = if !active.is_some_and(|p| p.online) { "Not linked" }
            else if active.and_then(|p| p.rtt_ms).is_some_and(|ms| ms >= 120.0) { "Unstable" }
            else if active.and_then(|p| p.interface.as_deref()).is_some_and(|name| name.starts_with("en") || name.starts_with("eth")) { "Ethernet ready" }
            else if active.and_then(|p| p.interface.as_deref()).is_some_and(|name| name.starts_with("wl")) { "Wi-Fi OK" }
            else { "Connected" };
        quality.set_text(network_quality);
        replace_rows(&peer_rows);
        if peers.is_empty() {
            let empty = Label::new(Some("No devices connected")); empty.set_xalign(0.0); empty.add_css_class("dim-label"); peer_rows.append(&empty);
        }
        for peer in &peers {
            let row = gtk::Box::new(Orientation::Horizontal, 10);
            let name = Label::new(Some(&peer.name)); name.set_hexpand(true); name.set_xalign(0.0); row.append(&name);
            let image = gtk::Image::from_icon_name(if peer.trusted && peer.online { "emblem-ok-symbolic" } else { "dialog-warning-symbolic" });
            image.set_tooltip_text(Some(if !peer.trusted { "Not trusted — authorize in Devices" } else if !peer.online { "Trusted — waiting for heartbeat" } else { "Trusted and connected" })); row.append(&image); peer_rows.append(&row);
        }
        replace_rows(&transfer_rows);
        let transfers = state.recent_transfers();
        if transfers.is_empty() { let empty = Label::new(Some("No incoming files yet")); empty.set_xalign(0.0); empty.add_css_class("dim-label"); transfer_rows.append(&empty); }
        for transfer in transfers {
            let row = gtk::Box::new(Orientation::Vertical, 3);
            let name = Label::new(Some(&transfer.name)); name.set_xalign(0.0); name.set_ellipsize(gtk::pango::EllipsizeMode::Middle); name.set_max_width_chars(34);
            name.set_tooltip_text(Some(&transfer.name)); row.append(&name);
            let text = if let Some(error) = &transfer.error { format!("Failed · {error}") }
                else if transfer.complete { format!("Received · {:.1} KB", transfer.received as f64 / 1024.0) }
                else { format!("Receiving · {} / {} bytes", transfer.received, transfer.total) };
            let detail = Label::new(Some(&text)); detail.set_xalign(0.0); detail.add_css_class("dim-label"); row.append(&detail);
            transfer_rows.append(&row);
        }
    };
    refresh();
    let weak = column.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        refresh(); glib::ControlFlow::Continue
    });
    let scroll = gtk::ScrolledWindow::builder().child(&column).hscrollbar_policy(gtk::PolicyType::Never).build();
    layout::page("Dashboard", "Your Entanglo connection, sharing permissions and recent activity.", &scroll)
}
