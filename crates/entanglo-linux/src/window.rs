//! `AdwNavigationSplitView` app shell — sidebar + content, the GTK4
//! analogue of the macOS `NavigationSplitView` Entanglo uses today.
//! See `STACK.md` for the fidelity rationale.

use std::rc::Rc;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, NavigationPage, NavigationSplitView};
use entanglo_core::net::CoordinatorEvent;
use gtk::{Label, ListBox, ListBoxRow};

use crate::app_state::Backend;
use crate::state::AppShared;

pub fn build(
    app: &Application,
    backend: Backend,
    events_rx: async_channel::Receiver<CoordinatorEvent>,
) -> ApplicationWindow {
    let shared = Rc::new(AppShared::new(backend));
    let pages = crate::pages::build_all(&shared);

    let sidebar_list = ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    for page in crate::pages::ALL_PAGES {
        let row = ListBoxRow::new();
        row.set_child(Some(&Label::new(Some(page.title))));
        sidebar_list.append(&row);
    }

    let sidebar = NavigationPage::builder()
        .title("Entanglo")
        .child(&sidebar_list)
        .build();

    let content = NavigationPage::builder()
        .title(crate::pages::ALL_PAGES[0].title)
        .child(pages.get("dashboard").expect("dashboard page always built"))
        .build();

    let content_for_selection = content.clone();
    sidebar_list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Some(page) = crate::pages::ALL_PAGES.get(row.index() as usize) else {
            return;
        };
        content_for_selection.set_title(page.title);
        if let Some(widget) = pages.get(page.id) {
            content_for_selection.set_child(Some(widget));
        }
    });

    // Drains the backend's merged CoordinatorEvent stream for as
    // long as the main context runs, updating `shared` (and, through
    // it, the live-bound Devices/Pairing widgets) in place. See
    // `state::handle_event`.
    let shared_for_events = Rc::clone(&shared);
    glib::spawn_future_local(async move {
        while let Ok(event) = events_rx.recv().await {
            crate::state::handle_event(&shared_for_events, event);
        }
    });

    let split_view = NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        .build();

    ApplicationWindow::builder()
        .application(app)
        .title("Entanglo")
        .default_width(760)
        .default_height(520)
        .content(&split_view)
        .build()
}
