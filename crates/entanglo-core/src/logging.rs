//! Structured logging, filtered by category (Network / Pairing /
//! Input / Files / Error) for the Logs page — mirrors
//! `entanglo-macos`'s `AppLogService`. Built on `tracing`; a
//! ring-buffer `tracing_subscriber::Layer` feeds the UI's Logs view.
//!
//! **Keystroke content is never logged** — `InputEventMessage` only
//! ever carries key codes and modifier bitmasks (see `PROTOCOL.md`
//! §5.5), so there is no Unicode text path into the logs to begin
//! with; this module must not add one.

pub fn init() {
    tracing_subscriber_placeholder();
}

// Placeholder until `tracing-subscriber` is added as a dependency
// alongside the real ring-buffer Logs-view layer (Phase 1 UI work).
fn tracing_subscriber_placeholder() {}
