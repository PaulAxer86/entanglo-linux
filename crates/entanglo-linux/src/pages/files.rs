//! Sending files to trusted devices, and the history of files sent and received.
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Label, Orientation, Widget};

use super::layout;
use crate::edge::EdgeTarget;
use crate::state::AppShared;

const TARGET_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// The device picker and the devices its rows stand for.
#[derive(Clone)]
struct SendTarget {
    shared: Rc<AppShared>,
    dropdown: gtk::DropDown,
    targets: Rc<RefCell<Vec<EdgeTarget>>>,
}

impl SendTarget {
    fn send(&self, paths: Vec<PathBuf>) {
        let selected = self.dropdown.selected() as usize;
        if let Some(target) = self.targets.borrow().get(selected) {
            self.shared.send_files(&target.device_id, paths);
        }
    }

    /// Rebuilds the picker only when devices change, keeping the selected one.
    fn refresh(&self, zone: &gtk::Box, choose: &gtk::Button) {
        let current = self.shared.file_targets();
        if *self.targets.borrow() != current {
            let previous = self.targets.borrow().get(self.dropdown.selected() as usize).map(|target| target.device_id.clone());
            let names: Vec<&str> = current.iter().map(|target| target.name.as_str()).collect();
            self.dropdown.set_model(Some(&gtk::StringList::new(&names)));
            let selected = previous
                .and_then(|id| current.iter().position(|target| target.device_id == id))
                .unwrap_or(0);
            self.dropdown.set_selected(selected as u32);
            *self.targets.borrow_mut() = current;
        }
        let available = !self.targets.borrow().is_empty();
        self.dropdown.set_sensitive(available);
        zone.set_sensitive(available);
        choose.set_sensitive(available);
    }
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column = gtk::Box::new(Orientation::Vertical, 12);
    column.append(&send_card(shared));

    let folder = entanglo_core::features::file_transfer::FileTransferService::download_dir();
    let open = gtk::Button::with_label("Open download folder");
    open.set_halign(Align::Start);
    open.connect_clicked(move |_| {
        if std::fs::create_dir_all(&folder).is_ok() {
            let _ = std::process::Command::new("xdg-open").arg(&folder).spawn();
        }
    });
    column.append(&open);
    shared.files_list.set_show_separators(true);
    shared.files_list.set_selection_mode(gtk::SelectionMode::None);
    shared.files_list.set_valign(Align::Start);
    crate::state::initialize_files(shared);
    let scroll = gtk::ScrolledWindow::builder().child(&shared.files_list).vexpand(true).build();
    column.append(&scroll);
    layout::page(
        "Files",
        "Send files to trusted devices. Received files are saved in Downloads/Entanglo; existing files are kept.",
        &column,
    )
}

fn send_card(shared: &Rc<AppShared>) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 10);
    card.add_css_class("card");
    card.set_margin_top(4);
    card.set_margin_start(4);
    card.set_margin_end(4);

    let recipient = gtk::Box::new(Orientation::Horizontal, 12);
    recipient.set_margin_top(14);
    recipient.set_margin_start(16);
    recipient.set_margin_end(16);
    let heading = Label::new(Some("Send a file to"));
    heading.add_css_class("heading");
    heading.set_hexpand(true);
    heading.set_halign(Align::Start);
    recipient.append(&heading);
    let dropdown = gtk::DropDown::from_strings(&[]);
    dropdown.set_size_request(240, -1);
    recipient.append(&dropdown);
    card.append(&recipient);

    let zone = gtk::Box::new(Orientation::Vertical, 6);
    zone.add_css_class("view");
    zone.set_margin_start(16);
    zone.set_margin_end(16);
    let zone_label = Label::new(Some("Drop files here"));
    zone_label.add_css_class("dim-label");
    zone_label.set_margin_top(26);
    zone_label.set_margin_bottom(26);
    zone.append(&zone_label);
    card.append(&zone);

    let choose = gtk::Button::with_label("Choose files…");
    choose.set_halign(Align::End);
    choose.set_margin_bottom(14);
    choose.set_margin_end(16);
    card.append(&choose);

    let target = SendTarget {
        shared: Rc::clone(shared),
        dropdown,
        targets: Rc::default(),
    };
    target.refresh(&zone, &choose);

    let drop_target = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY);
    let target_drop = target.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gtk::gdk::FileList>() else { return false };
        let paths: Vec<PathBuf> = files.files().iter().filter_map(|file| file.path()).collect();
        let accepted = !paths.is_empty();
        if accepted {
            target_drop.send(paths);
        }
        accepted
    });
    zone.add_controller(drop_target);

    let target_choose = target.clone();
    // A native dialog disappears unless something keeps a reference to it.
    let open_dialog: Rc<RefCell<Option<gtk::FileChooserNative>>> = Rc::default();
    choose.connect_clicked(move |button| {
        let parent = button.root().and_downcast::<gtk::Window>();
        let dialog = gtk::FileChooserNative::new(
            Some("Choose files to send"),
            parent.as_ref(),
            gtk::FileChooserAction::Open,
            Some("Send"),
            Some("Cancel"),
        );
        dialog.set_select_multiple(true);
        let target = target_choose.clone();
        let held = Rc::clone(&open_dialog);
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                let files = dialog.files();
                let paths: Vec<PathBuf> = (0..files.n_items())
                    .filter_map(|index| files.item(index).and_downcast::<gtk::gio::File>())
                    .filter_map(|file| file.path())
                    .collect();
                if !paths.is_empty() {
                    target.send(paths);
                }
            }
            held.borrow_mut().take();
        });
        dialog.show();
        *open_dialog.borrow_mut() = Some(dialog);
    });

    let zone_poll = zone.clone();
    let choose_poll = choose.clone();
    glib::source::timeout_add_local(TARGET_REFRESH_INTERVAL, move || {
        target.refresh(&zone_poll, &choose_poll);
        glib::ControlFlow::Continue
    });
    card
}
