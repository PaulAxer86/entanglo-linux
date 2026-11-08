//! Shared visual structure for every main page.

use adw::prelude::*;
use gtk::{Align, Label, Orientation, Widget};

pub fn page(title: &str, subtitle: &str, content: &impl IsA<Widget>) -> Widget {
    let column = gtk::Box::new(Orientation::Vertical, 20);
    column.set_margin_top(28);
    column.set_margin_bottom(28);
    column.set_margin_start(28);
    column.set_margin_end(28);
    column.set_vexpand(true);

    let heading = gtk::Box::new(Orientation::Vertical, 6);
    let title_label = Label::new(Some(title));
    title_label.set_halign(Align::Start);
    title_label.add_css_class("title-1");
    heading.append(&title_label);

    let subtitle_label = Label::new(Some(subtitle));
    subtitle_label.set_halign(Align::Start);
    subtitle_label.set_wrap(true);
    subtitle_label.set_xalign(0.0);
    subtitle_label.add_css_class("dim-label");
    heading.append(&subtitle_label);
    column.append(&heading);

    content.set_hexpand(true);
    content.set_vexpand(true);
    column.append(content);

    adw::Clamp::builder()
        .maximum_size(980)
        .tightening_threshold(680)
        .child(&column)
        .build()
        .into()
}

pub fn placeholder(title: &str, subtitle: &str, message: &str) -> Widget {
    let card = gtk::Box::new(Orientation::Vertical, 8);
    card.add_css_class("card");
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(4);
    card.set_margin_end(4);

    let label = Label::new(Some(message));
    label.set_halign(Align::Start);
    label.set_wrap(true);
    label.set_margin_top(18);
    label.set_margin_bottom(18);
    label.set_margin_start(18);
    label.set_margin_end(18);
    card.append(&label);

    page(title, subtitle, &card)
}
