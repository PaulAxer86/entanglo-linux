//! GTK clipboard bridge for the platform-neutral `clipboardText`
//! protocol path. Mirrors the Swift app: opt-in, off by default,
//! polls every 500 ms, plain text only, and suppresses the write-back
//! echo after applying text received from a peer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

use crate::state::AppShared;

pub struct ClipboardBridge {
    enabled: Cell<bool>,
    initialized: Cell<bool>,
    last_text: RefCell<Option<String>>,
    poll_in_flight: Cell<bool>,
    generation: Cell<u64>,
}

impl ClipboardBridge {
    pub fn new() -> Self {
        Self {
            enabled: Cell::new(false),
            initialized: Cell::new(false),
            last_text: RefCell::new(None),
            poll_in_flight: Cell::new(false),
            generation: Cell::new(0),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.replace(enabled) == enabled {
            return;
        }
        // Match the Mac's startPolling(): the first observation after
        // enabling establishes a baseline and is never broadcast.
        self.initialized.set(false);
        self.last_text.replace(None);
        self.bump_generation();
        tracing::info!(enabled, "clipboard sync setting changed");
    }

    /// Records a local clipboard observation. `None` means the
    /// clipboard currently contains no text. Returns text only for a
    /// genuine change after the initial baseline has been established.
    fn observe_local(&self, text: Option<String>) -> Option<String> {
        if !self.enabled() {
            return None;
        }
        if !self.initialized.replace(true) {
            self.last_text.replace(text);
            return None;
        }
        if *self.last_text.borrow() == text {
            return None;
        }
        self.last_text.replace(text.clone());
        text
    }

    /// Applies trusted remote text to the desktop clipboard and marks
    /// it as already seen before writing, preventing the next poll from
    /// bouncing it straight back to every peer.
    pub fn apply_remote(&self, text: &str) {
        if !self.enabled() {
            return;
        }
        let Some(display) = gtk::gdk::Display::default() else {
            tracing::warn!("cannot apply remote clipboard text: no display");
            return;
        };
        self.initialized.set(true);
        self.last_text.replace(Some(text.to_string()));
        self.bump_generation();
        display.clipboard().set_text(text);
        tracing::info!(
            chars = text.chars().count(),
            "clipboard updated from trusted peer"
        );
    }

    fn bump_generation(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }
}

impl Default for ClipboardBridge {
    fn default() -> Self {
        Self::new()
    }
}

pub fn start(shared: &Rc<AppShared>) {
    let shared = Rc::clone(shared);
    glib::source::timeout_add_local(Duration::from_millis(500), move || {
        if !shared.clipboard.enabled() || shared.clipboard.poll_in_flight.replace(true) {
            return glib::ControlFlow::Continue;
        }

        let Some(display) = gtk::gdk::Display::default() else {
            shared.clipboard.poll_in_flight.set(false);
            return glib::ControlFlow::Continue;
        };
        let clipboard = display.clipboard();
        let generation = shared.clipboard.generation.get();
        let shared_poll = Rc::clone(&shared);
        glib::spawn_future_local(async move {
            match clipboard.read_text_future().await {
                Ok(text) if shared_poll.clipboard.generation.get() == generation => {
                    let text = text.map(|value| value.to_string());
                    if let Some(changed) = shared_poll.clipboard.observe_local(text) {
                        tracing::info!(
                            chars = changed.chars().count(),
                            "clipboard changed locally; broadcasting to trusted peers"
                        );
                        let coordinator = shared_poll.backend.coordinator.clone();
                        let handle = shared_poll.backend.handle.clone();
                        handle.spawn(async move {
                            coordinator.broadcast_clipboard(&changed).await;
                        });
                    }
                }
                Ok(_) => {
                    // A remote write or setting change happened while
                    // this async read was pending. Discard the stale
                    // snapshot; the next 500 ms poll reads fresh state.
                }
                Err(error) => tracing::debug!(%error, "clipboard text read failed"),
            }
            shared_poll.clipboard.poll_in_flight.set(false);
        });

        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_observation_after_enabling_is_only_a_baseline() {
        let bridge = ClipboardBridge::new();
        bridge.set_enabled(true);
        assert_eq!(bridge.observe_local(Some("already there".into())), None);
        assert_eq!(
            bridge.observe_local(Some("new text".into())),
            Some("new text".into())
        );
    }

    #[test]
    fn unchanged_text_is_not_rebroadcast() {
        let bridge = ClipboardBridge::new();
        bridge.set_enabled(true);
        bridge.observe_local(Some("same".into()));
        assert_eq!(bridge.observe_local(Some("same".into())), None);
    }

    #[test]
    fn disabled_bridge_never_emits_text() {
        let bridge = ClipboardBridge::new();
        assert_eq!(bridge.observe_local(Some("private".into())), None);
    }
}
