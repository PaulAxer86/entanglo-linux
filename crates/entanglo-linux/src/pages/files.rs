//! Incoming file history and destination folder.
use std::rc::Rc;
use gtk::prelude::*;
use gtk::Widget;
use crate::state::AppShared;
use super::layout;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let folder = entanglo_core::features::file_transfer::FileTransferService::download_dir();
    let open = gtk::Button::with_label("Open download folder");
    open.set_halign(gtk::Align::Start);
    open.connect_clicked(move |_| {
        if std::fs::create_dir_all(&folder).is_ok() {
            let _ = std::process::Command::new("xdg-open").arg(&folder).spawn();
        }
    });
    column.append(&open);
    shared.files_list.set_show_separators(true);
    shared.files_list.set_selection_mode(gtk::SelectionMode::None);
    shared.files_list.set_valign(gtk::Align::Start);
    crate::state::initialize_files(shared);
    let scroll = gtk::ScrolledWindow::builder().child(&shared.files_list).vexpand(true).build();
    column.append(&scroll);
    layout::page("Files", "Receive files from trusted Entanglo devices. Files are saved in Downloads/Entanglo; existing files are kept.", &column)
}
