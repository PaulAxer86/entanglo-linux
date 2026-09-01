//! One module per sidebar section, matching the Mac Dashboard's page
//! set (`entanglo-macos/Entanglo/Views/`) and the Windows `Pages/`
//! folder in `entanglo-windows/SKELETON.md`. Dashboard/Devices/
//! Pairing/Input Sharing/Settings/Network/Logs are wired to live state via
//! `state::AppShared` and `entanglo_core::net::Coordinator`; the rest
//! are still placeholder `gtk::Widget`s — replace as Phase 1/2 work
//! lands, per `ROADMAP.md`.

use std::collections::HashMap;
use std::rc::Rc;

pub mod dashboard;
pub mod devices;
pub mod files;
pub mod input_sharing;
pub mod logs;
pub mod network;
pub mod news_updates;
pub mod pairing;
pub mod print;
pub mod settings;

use crate::state::AppShared;

pub struct PageInfo {
    pub id: &'static str,
    pub title: &'static str,
}

pub const ALL_PAGES: &[PageInfo] = &[
    PageInfo {
        id: "dashboard",
        title: "Dashboard",
    },
    PageInfo {
        id: "devices",
        title: "Devices",
    },
    PageInfo {
        id: "pairing",
        title: "Pairing",
    },
    PageInfo {
        id: "input-sharing",
        title: "Input Sharing",
    },
    PageInfo {
        id: "files",
        title: "Files",
    },
    PageInfo {
        id: "print",
        title: "Print",
    },
    PageInfo {
        id: "network",
        title: "Network",
    },
    PageInfo {
        id: "news-updates",
        title: "News & Updates",
    },
    PageInfo {
        id: "settings",
        title: "Settings",
    },
    PageInfo {
        id: "logs",
        title: "Logs",
    },
];

/// Builds every page's widget once, up front. `window.rs` keeps this
/// map alive for the app's lifetime and just swaps which widget is
/// shown on sidebar selection — building fresh widgets per click
/// (the earlier design) would lose Devices/Pairing state, and miss
/// live updates, whenever the user wasn't looking at that page.
pub fn build_all(shared: &Rc<AppShared>) -> HashMap<&'static str, gtk::Widget> {
    HashMap::from([
        ("dashboard", dashboard::build(shared)),
        ("devices", devices::build(shared)),
        ("pairing", pairing::build(shared)),
        ("input-sharing", input_sharing::build(shared)),
        ("files", files::build()),
        ("print", print::build()),
        ("network", network::build(shared)),
        ("news-updates", news_updates::build()),
        ("settings", settings::build(shared)),
        ("logs", logs::build(shared)),
    ])
}
