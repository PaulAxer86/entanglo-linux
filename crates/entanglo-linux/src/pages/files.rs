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
const DROP_HINT: &str = "or choose them from this computer";
const NO_DEVICE_HINT: &str = "Connect a trusted device to send files";
const STYLE: &str = "
.send-card { padding: 14px 16px; }
.send-zone {
    padding: 14px 16px;
    border: 1px dashed alpha(currentColor, 0.28);
    border-radius: 12px;
    background-color: alpha(@accent_color, 0.05);
    transition: background-color 120ms ease-out, border-color 120ms ease-out;
}
.send-zone:drop(active) {
    border: 1px solid @accent_color;
    background-color: alpha(@accent_color, 0.16);
}
.file-badge { min-width: 26px; min-height: 26px; border-radius: 13px; font-weight: 700; }
.file-badge.large { min-width: 38px; min-height: 38px; border-radius: 19px; font-size: 1.25em; }
.file-badge.received { color: @success_color; background-color: alpha(@success_color, 0.15); }
.file-badge.sent { color: @accent_color; background-color: alpha(@accent_color, 0.15); }
.file-badge.failed { color: @error_color; background-color: alpha(@error_color, 0.15); }
.file-name { font-weight: 600; }
.file-status { font-size: 0.82em; font-weight: 600; }
.file-status.received { color: @success_color; }
.file-status.sent { color: @accent_color; }
.file-status.failed { color: @error_color; }
.file-meta { font-size: 0.85em; }
";

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
    fn refresh(&self, zone: &gtk::Box, hint: &Label) {
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
        hint.set_text(if available { DROP_HINT } else { NO_DEVICE_HINT });
    }
}

fn install_style() {
    let Some(display) = gtk::gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(STYLE);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    install_style();
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
    let card = gtk::Box::new(Orientation::Vertical, 12);
    card.add_css_class("card");
    card.add_css_class("send-card");
    card.set_margin_top(4);
    card.set_margin_start(4);
    card.set_margin_end(4);

    let recipient = gtk::Box::new(Orientation::Horizontal, 12);
    let heading = Label::new(Some("Send a file to"));
    heading.add_css_class("heading");
    heading.set_hexpand(true);
    heading.set_halign(Align::Start);
    recipient.append(&heading);
    let dropdown = gtk::DropDown::from_strings(&[]);
    dropdown.set_size_request(240, -1);
    recipient.append(&dropdown);
    card.append(&recipient);

    let zone = gtk::Box::new(Orientation::Horizontal, 14);
    zone.add_css_class("send-zone");
    let badge = Label::new(Some("\u{2191}"));
    badge.add_css_class("file-badge");
    badge.add_css_class("large");
    badge.add_css_class("sent");
    badge.set_valign(Align::Center);
    zone.append(&badge);

    let zone_text = gtk::Box::new(Orientation::Vertical, 2);
    zone_text.set_hexpand(true);
    zone_text.set_valign(Align::Center);
    let zone_title = Label::new(Some("Drop files here"));
    zone_title.add_css_class("heading");
    zone_title.set_halign(Align::Start);
    zone_text.append(&zone_title);
    let hint = Label::new(Some(DROP_HINT));
    hint.add_css_class("caption");
    hint.add_css_class("dim-label");
    hint.set_halign(Align::Start);
    zone_text.append(&hint);
    zone.append(&zone_text);

    let choose = gtk::Button::with_label("Choose files\u{2026}");
    choose.add_css_class("suggested-action");
    choose.set_valign(Align::Center);
    zone.append(&choose);
    card.append(&zone);

    let target = SendTarget {
        shared: Rc::clone(shared),
        dropdown,
        targets: Rc::default(),
    };
    target.refresh(&zone, &hint);

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
    choose.connect_clicked(move |button| choose_files(button, target_choose.clone()));

    let zone_poll = zone.clone();
    glib::source::timeout_add_local(TARGET_REFRESH_INTERVAL, move || {
        target.refresh(&zone_poll, &hint);
        glib::ControlFlow::Continue
    });
    card
}

/// Uses GTK's own dialog: the desktop portal refuses requests from a process
/// that gained the `input` group through `sg`, and then shows nothing.
fn choose_files(button: &gtk::Button, target: SendTarget) {
    let parent = button.root().and_downcast::<gtk::Window>();
    let dialog = gtk::FileChooserDialog::new(
        Some("Choose files to send"),
        parent.as_ref(),
        gtk::FileChooserAction::Open,
        &[("Cancel", gtk::ResponseType::Cancel), ("Send", gtk::ResponseType::Accept)],
    );
    dialog.set_modal(true);
    dialog.set_select_multiple(true);
    dialog.set_default_size(860, 560);
    dialog.set_default_response(gtk::ResponseType::Accept);
    // GTK otherwise opens on its recent files, which are empty on many desktops.
    let _ = dialog.set_current_folder(Some(&gtk::gio::File::for_path(glib::home_dir())));
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
        dialog.destroy();
    });
    dialog.present();
}
