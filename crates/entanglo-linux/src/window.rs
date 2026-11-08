//! `AdwNavigationSplitView` app shell — sidebar + content, the GTK4
//! analogue of the macOS `NavigationSplitView` Entanglo uses today.
//! See `STACK.md` for the fidelity rationale.

use std::rc::Rc;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, NavigationPage, NavigationSplitView};
use entanglo_core::net::CoordinatorEvent;
use gtk::{Align, Label, ListBox, ListBoxRow, Orientation};

use crate::app_state::Backend;
use crate::state::AppShared;

pub fn build(
    app: &Application,
    backend: Backend,
    events_rx: async_channel::Receiver<CoordinatorEvent>,
) -> ApplicationWindow {
    let shared = Rc::new(AppShared::new(backend));
    let pages = crate::pages::build_all(&shared);
    crate::edge::start(&shared);
    crate::clipboard::start(&shared);

    let sidebar_list = ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
    for page in crate::pages::ALL_PAGES {
        let row = ListBoxRow::new();
        let label = Label::new(Some(page.title));
        label.set_halign(Align::Start);
        label.set_margin_top(10);
        label.set_margin_bottom(10);
        label.set_margin_start(14);
        label.set_margin_end(14);
        row.set_child(Some(&label));
        sidebar_list.append(&row);
    }

    let sidebar_box = gtk::Box::new(Orientation::Vertical, 12);
    sidebar_box.set_width_request(220);
    let app_title = Label::new(Some("Entanglo"));
    app_title.set_halign(Align::Start);
    app_title.set_margin_top(22);
    app_title.set_margin_start(18);
    app_title.set_margin_end(18);
    app_title.add_css_class("title-2");
    let brand = gtk::Box::new(Orientation::Horizontal, 8);
    let rabbit = gtk::Image::from_icon_name("com.paoloasara.Entanglo");
    rabbit.set_pixel_size(32); rabbit.set_margin_start(18); rabbit.set_margin_top(18);
    brand.append(&rabbit);
    brand.append(&app_title);
    sidebar_box.append(&brand);
    sidebar_box.append(&sidebar_list);
    let version_label = Label::new(Some(concat!("Version ", env!("CARGO_PKG_VERSION"))));
    version_label.set_halign(Align::Start);
    version_label.set_valign(Align::End);
    version_label.set_vexpand(true);
    version_label.set_margin_start(18);
    version_label.set_margin_bottom(14);
    version_label.add_css_class("caption");
    version_label.add_css_class("dim-label");
    sidebar_box.append(&version_label);

    let sidebar = NavigationPage::builder()
        .title("Entanglo")
        .child(&sidebar_box)
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
    // it, the live-bound Devices widgets) in place. See
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

    let header = adw::HeaderBar::builder()
        .decoration_layout(":minimize,maximize,close")
        .show_start_title_buttons(false)
        .show_end_title_buttons(true)
        .build();
    let logo = gtk::Image::from_icon_name("com.paoloasara.Entanglo");
    logo.set_pixel_size(24);
    header.pack_start(&logo);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&split_view));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Entanglo")
        .icon_name("com.paoloasara.Entanglo")
        .default_width(1120)
        .default_height(860)
        .content(&toolbar)
        .build();
    let app_weak = app.downgrade();
    window.connect_close_request(move |_| {
        if let Some(app) = app_weak.upgrade() {
            app.quit();
        }
        glib::Propagation::Proceed
    });
    window
}
