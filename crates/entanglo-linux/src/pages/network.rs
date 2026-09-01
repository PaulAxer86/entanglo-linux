//! Network Quality dashboard tile, backed by
//! `entanglo_core::features::network_quality::NetworkQualityMonitor`
//! — fed from real `heartbeat` RTTs via `state::handle_event`, not a
//! placeholder. One row per peer that has exchanged at least one
//! heartbeat since this app started.

use std::rc::Rc;

use entanglo_core::features::network_quality::SuggestedMode;
use gtk::prelude::*;
use gtk::{ListBox, ScrolledWindow, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let list = ListBox::new();
    refresh(shared, &list);

    let shared_poll = Rc::clone(shared);
    let list_poll = list.clone();
    glib::source::timeout_add_local(std::time::Duration::from_millis(1000), move || {
        refresh(&shared_poll, &list_poll);
        glib::ControlFlow::Continue
    });

    ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build()
        .into()
}

fn refresh(shared: &Rc<AppShared>, list: &ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let stats = shared.network_stats_snapshot();
    if stats.is_empty() {
        list.append(&gtk::Label::new(Some(
            "No heartbeats yet — connect to a peer first.",
        )));
        return;
    }

    for stat in stats {
        let rtt_text = stat
            .average_rtt_ms
            .map(|ms| format!("{ms:.1} ms avg RTT"))
            .unwrap_or_else(|| "RTT not yet available".to_string());
        let mode_text = match stat.suggested_mode {
            SuggestedMode::EthernetPreferred => "Ethernet Preferred",
            SuggestedMode::WifiOk => "Wi-Fi OK",
            SuggestedMode::Unstable => "Unstable",
            SuggestedMode::Offline => "Offline",
        };
        list.append(&gtk::Label::new(Some(&format!(
            "{}  ·  {rtt_text}  ·  {mode_text}",
            stat.device_name
        ))));
    }
}
