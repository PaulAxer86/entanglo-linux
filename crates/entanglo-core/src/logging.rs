//! Structured logging, filtered by category (Network / Pairing /
//! Input / Files / Error) for the Logs page — mirrors
//! `entanglo-macos`'s `AppLogService`. Built on `tracing`.
//!
//! **Keystroke content is never logged** — `InputEventMessage` only
//! ever carries key codes and modifier bitmasks (see `PROTOCOL.md`
//! §5.5), so there is no Unicode text path into the logs to begin
//! with; this module must not add one.
//!
//! `init()` wires `tracing` to stderr, filtered by `$RUST_LOG`
//! (defaulting to `info`). The in-memory ring buffer a real Logs page
//! would read from — a `tracing_subscriber::Layer` that also
//! retains recent events for the UI — is Phase 1 UI work not done
//! yet; this gets useful `journalctl`/terminal output today, which
//! matters more while there's no Logs page to look at instead.

pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
