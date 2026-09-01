//! Cursor-at-screen-edge detection, the real edge-switching this
//! project has been substituting with a manual "Control this device"
//! button — see `ROADMAP.md`'s ⬜ item. X11-only: the Wayland pointer
//! position problem `ROADMAP.md`'s Risks section flags as the hard
//! case is still unsolved; on Wayland `EdgeWatcher::connect` just
//! fails and edge-switching is unavailable, same as today, while the
//! manual button keeps working everywhere.
//!
//! Polls rather than subscribes to motion events — `XQueryPointer` is
//! a cheap local round trip, and polling means no X11 event mask /
//! grab is needed on any window, so this can't interfere with normal
//! use of the desktop.
//!
//! **Known rough edge, not yet fixed**: this does not grab or warp
//! the local pointer. Pushing the cursor to the assigned edge starts
//! forwarding input to that peer (`Coordinator::set_active_receiver`),
//! but the local cursor itself keeps moving/clamping normally — it
//! doesn't "hand off" visually the way a polished KVM switch would.
//! Getting control back today relies on the peer touching *their*
//! input (`releaseControl`, Checkpoint D) or the local
//! Ctrl+Shift+Escape Emergency Stop hotkey — both already work.

use std::rc::Rc;
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

use entanglo_core::net::ConnId;

use crate::state::AppShared;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenEdge {
    Left,
    Right,
}

impl ScreenEdge {
    pub const ALL: [ScreenEdge; 2] = [ScreenEdge::Left, ScreenEdge::Right];

    pub fn label(self) -> &'static str {
        match self {
            ScreenEdge::Left => "Left edge",
            ScreenEdge::Right => "Right edge",
        }
    }
}

pub struct EdgeWatcher {
    conn: RustConnection,
    root: u32,
    screen_width: i16,
    /// Debounces so a cursor held at the edge only triggers once per
    /// crossing, not once per poll — cleared as soon as the cursor
    /// leaves the edge zone (`x11rb`'s coordinates clamp at 0 /
    /// `screen_width - 1`, so "held at the edge" reads as the same
    /// value on every poll until the user pulls back).
    was_at_edge: Option<ScreenEdge>,
}

impl EdgeWatcher {
    /// `None` if there's no X11 display to connect to (a Wayland-only
    /// session, or `$DISPLAY` unset) — not an error the caller needs
    /// to act on, just "edge-switching isn't available here."
    pub fn connect() -> Option<Self> {
        let (conn, screen_num) = match x11rb::rust_connection::RustConnection::connect(None) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::info!(error = %e, "no X11 connection, edge-switching unavailable (Wayland session?)");
                return None;
            }
        };
        let screen = &conn.setup().roots[screen_num];
        Some(Self {
            root: screen.root,
            screen_width: screen.width_in_pixels as i16,
            conn,
            was_at_edge: None,
        })
    }

    /// Returns `Some(edge)` the moment the cursor arrives at an edge
    /// (not on every subsequent poll while it stays there).
    pub fn poll(&mut self) -> Option<ScreenEdge> {
        let pointer = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let at_edge = edge_for_x(pointer.root_x, self.screen_width);
        let crossed = if at_edge.is_some() && at_edge != self.was_at_edge {
            at_edge
        } else {
            None
        };
        self.was_at_edge = at_edge;
        crossed
    }
}

fn edge_for_x(x: i16, screen_width: i16) -> Option<ScreenEdge> {
    if x <= 0 {
        Some(ScreenEdge::Left)
    } else if x >= screen_width - 1 {
        Some(ScreenEdge::Right)
    } else {
        None
    }
}

/// Starts the poll loop (50 ms, ~20 Hz — plenty responsive for a
/// mouse push, cheap enough to leave running for the app's lifetime)
/// on the GTK main context, so it can freely touch `shared`'s
/// `Rc<RefCell<..>>` state with no cross-thread synchronization. A
/// missing X11 connection (`EdgeWatcher::connect` returning `None`)
/// just skips starting the loop — logged once, not a hard error.
pub fn start(shared: &Rc<AppShared>) {
    let Some(mut watcher) = EdgeWatcher::connect() else {
        return;
    };
    let shared = Rc::clone(shared);
    glib::source::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Some(edge) = watcher.poll() {
            if let Some(conn_id) = shared.conn_id_for_edge(edge) {
                activate(&shared, conn_id);
            }
        }
        glib::ControlFlow::Continue
    });
}

fn activate(shared: &Rc<AppShared>, conn_id: ConnId) {
    let coordinator = Arc::clone(&shared.backend.coordinator);
    shared.backend.handle.spawn(async move {
        coordinator.set_active_receiver(Some(conn_id)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_edge_is_x_at_or_below_zero() {
        assert_eq!(edge_for_x(0, 1920), Some(ScreenEdge::Left));
        assert_eq!(edge_for_x(-1, 1920), Some(ScreenEdge::Left));
        assert_eq!(edge_for_x(1, 1920), None);
    }

    #[test]
    fn right_edge_is_x_at_or_above_screen_width_minus_one() {
        assert_eq!(edge_for_x(1919, 1920), Some(ScreenEdge::Right));
        assert_eq!(edge_for_x(1920, 1920), Some(ScreenEdge::Right));
        assert_eq!(edge_for_x(1918, 1920), None);
    }

    #[test]
    fn middle_of_screen_is_no_edge() {
        assert_eq!(edge_for_x(960, 1920), None);
    }
}
