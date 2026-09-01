//! One module per sidebar section, matching the Mac Dashboard's page
//! set (`entanglo-macos/Entanglo/Views/`) and the Windows `Pages/`
//! folder in `entanglo-windows/SKELETON.md`. Each `build()` returns a
//! placeholder `gtk::Widget` today — replace with the real page UI as
//! Phase 1/2 work lands, per `ROADMAP.md`.

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

/// Dispatch by `PageInfo::id` to the matching page's placeholder
/// widget. `window.rs` calls this when a sidebar row is selected.
pub fn build_by_id(id: &str) -> gtk::Widget {
    match id {
        "dashboard" => dashboard::build(),
        "devices" => devices::build(),
        "pairing" => pairing::build(),
        "input-sharing" => input_sharing::build(),
        "files" => files::build(),
        "print" => print::build(),
        "network" => network::build(),
        "news-updates" => news_updates::build(),
        "settings" => settings::build(),
        "logs" => logs::build(),
        _ => unreachable!("unknown page id {id:?} — add it to ALL_PAGES"),
    }
}
