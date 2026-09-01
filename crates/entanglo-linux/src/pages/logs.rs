//! Log panel backed by `entanglo_core::logging::LogBuffer` — a real
//! in-memory ring buffer of recent `tracing` events, not a
//! placeholder. Keystroke content is never logged (see that module's
//! doc comment), so there's nothing sensitive to filter out here.
//!
//! Category filtering (Network / Pairing / Input / Files / Error,
//! matching `entanglo-macos`'s Logs view) is future work — today it's
//! everything at `$RUST_LOG`'s level, unfiltered.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{ScrolledWindow, TextView, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let text_view = TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .build();
    text_view.set_left_margin(8);
    text_view.set_top_margin(8);

    let scrolled = ScrolledWindow::builder()
        .child(&text_view)
        .vexpand(true)
        .build();

    refresh(shared, &text_view, &scrolled);

    let shared_poll = Rc::clone(shared);
    let text_view_poll = text_view.clone();
    let scrolled_poll = scrolled.clone();
    glib::source::timeout_add_local(std::time::Duration::from_millis(1000), move || {
        refresh(&shared_poll, &text_view_poll, &scrolled_poll);
        glib::ControlFlow::Continue
    });

    scrolled.into()
}

fn refresh(shared: &Rc<AppShared>, text_view: &TextView, scrolled: &ScrolledWindow) {
    let lines = shared.backend.log_buffer.recent();
    let buffer = text_view.buffer();
    let was_at_bottom = {
        let adj = scrolled.vadjustment();
        adj.value() + adj.page_size() >= adj.upper() - 1.0
    };

    buffer.set_text(&lines.join("\n"));

    if was_at_bottom {
        let mut end = buffer.end_iter();
        text_view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }
}
