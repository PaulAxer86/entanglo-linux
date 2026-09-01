//! Structured logging, filtered by category (Network / Pairing /
//! Input / Files / Error) for the Logs page — mirrors
//! `entanglo-macos`'s `AppLogService`. Built on `tracing`.
//!
//! **Keystroke content is never logged** — `InputEventMessage` only
//! ever carries key codes and modifier bitmasks (see `PROTOCOL.md`
//! §5.5), so there is no Unicode text path into the logs to begin
//! with; this module must not add one.
//!
//! `init()` wires `tracing` to stderr (filtered by `$RUST_LOG`,
//! defaulting to `info`) *and* an in-memory ring buffer
//! (`LogBuffer`), returned so a UI can display recent log lines
//! without polling `journalctl`. Implemented by reusing
//! `tracing_subscriber::fmt`'s own formatter with a custom
//! `MakeWriter` that appends into the buffer, rather than hand-rolling
//! a `tracing_subscriber::Layer` that visits fields itself.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

const MAX_LOG_LINES: usize = 500;

/// Cheap to clone (an `Arc` inside) — hand a clone to the UI at
/// startup and keep logging from anywhere via the ordinary `tracing`
/// macros; nothing needs to route messages through this type by hand.
#[derive(Clone)]
pub struct LogBuffer(Arc<Mutex<VecDeque<String>>>);

impl LogBuffer {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))))
    }

    /// Oldest first. A UI polling this should just re-render the
    /// whole list — `MAX_LOG_LINES` keeps that cheap.
    pub fn recent(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("log buffer mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }

    fn push_line(&self, line: String) {
        let mut buf = self.0.lock().expect("log buffer mutex poisoned");
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(line);
    }
}

struct RingBufferWriter(LogBuffer);

impl Write for RingBufferWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Ok(text) = std::str::from_utf8(bytes) {
            for line in text.lines().filter(|l| !l.is_empty()) {
                self.0.push_line(line.to_string());
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
struct RingBufferMakeWriter(LogBuffer);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RingBufferMakeWriter {
    type Writer = RingBufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RingBufferWriter(self.0.clone())
    }
}

/// Sets up global logging and returns the buffer a UI can poll. Call
/// once, at the very start of `main()`.
pub fn init() -> LogBuffer {
    use tracing_subscriber::prelude::*;

    let buffer = LogBuffer::new();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer();
    let buffer_layer = tracing_subscriber::fmt::layer()
        .with_writer(RingBufferMakeWriter(buffer.clone()))
        .with_ansi(false);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(buffer_layer)
        .try_init();

    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_returns_pushed_lines_oldest_first() {
        let buffer = LogBuffer::new();
        buffer.push_line("first".to_string());
        buffer.push_line("second".to_string());
        assert_eq!(
            buffer.recent(),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn caps_at_max_lines_dropping_the_oldest() {
        let buffer = LogBuffer::new();
        for i in 0..MAX_LOG_LINES + 10 {
            buffer.push_line(format!("line {i}"));
        }
        let recent = buffer.recent();
        assert_eq!(recent.len(), MAX_LOG_LINES);
        assert_eq!(recent.first().unwrap(), "line 10"); // first 10 dropped
        assert_eq!(
            recent.last().unwrap(),
            &format!("line {}", MAX_LOG_LINES + 9)
        );
    }

    #[test]
    fn writer_splits_multi_line_writes_and_skips_blank_lines() {
        let buffer = LogBuffer::new();
        let mut writer = RingBufferWriter(buffer.clone());
        writer.write_all(b"one\n\ntwo\n").unwrap();
        assert_eq!(buffer.recent(), vec!["one".to_string(), "two".to_string()]);
    }
}
