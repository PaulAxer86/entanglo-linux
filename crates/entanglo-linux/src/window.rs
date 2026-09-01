//! `AdwNavigationSplitView` app shell — sidebar + content, the GTK4
//! analogue of the macOS `NavigationSplitView` Entanglo uses today.
//! See `STACK.md` for the fidelity rationale.

use adw::prelude::*;
use adw::{Application, ApplicationWindow, NavigationPage, NavigationSplitView};
use gtk::{Label, ListBox, ListBoxRow};

pub fn build(app: &Application) -> ApplicationWindow {
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

    // Content starts on the Dashboard page (index 0); selecting a
    // sidebar row swaps both the content's child widget and its
    // title to match, via `pages::build_by_id`.
    let content = NavigationPage::builder()
        .title(crate::pages::ALL_PAGES[0].title)
        .child(&crate::pages::dashboard::build())
        .build();

    let content_for_selection = content.clone();
    sidebar_list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Some(page) = crate::pages::ALL_PAGES.get(row.index() as usize) else {
            return;
        };
        content_for_selection.set_title(page.title);
        content_for_selection.set_child(Some(&crate::pages::build_by_id(page.id)));
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
