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
//! grab is needed on any window just to *watch* the cursor, so this
//! can't interfere with normal use of the desktop by itself. The
//! separate pointer *grab* below (while a peer is actively being
//! controlled) very much does intercept local input by design — see
//! its own doc comment.
//!
//! Same poll loop also owns the local-cursor grab/hide while a peer
//! is being controlled — confirmed live to be needed: pushing the
//! cursor to an assigned edge correctly started forwarding input to
//! the peer (Android), but the local cursor kept visibly moving too,
//! since capture reads raw evdev independently of whatever X11 does
//! with the same physical mouse. `grab_pointer` + an invisible cursor
//! fixes that.

use std::rc::Rc;
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, EventMask, GrabMode};
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
    /// A fully transparent 1×1 cursor, created once and reused for
    /// every grab — X11 cursors are cheap server-side resources, no
    /// need to recreate per grab/ungrab cycle.
    invisible_cursor: u32,
    /// Whether we currently hold the pointer grab — tracked locally
    /// so the poll loop only calls `grab`/`ungrab` on an actual
    /// transition, not every tick.
    grabbed: bool,
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
        let root = screen.root;

        let invisible_cursor = match Self::create_invisible_cursor(&conn, root) {
            Ok(cursor) => cursor,
            Err(e) => {
                tracing::warn!(error = %e, "failed to create an invisible X11 cursor, pointer grab unavailable");
                return None;
            }
        };

        Some(Self {
            root,
            screen_width: screen.width_in_pixels as i16,
            conn,
            was_at_edge: None,
            invisible_cursor,
            grabbed: false,
        })
    }

    /// A 1×1 fully-transparent cursor: a 1-bit depth pixmap used as
    /// both the cursor's source and mask, left all-zero, so nothing
    /// ever gets drawn. Standard X11 technique for "hide the cursor"
    /// (there's no direct `XHideCursor` in core X11).
    fn create_invisible_cursor(
        conn: &RustConnection,
        root: u32,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(1, pixmap, root, 1, 1)?.check()?;
        let cursor = conn.generate_id()?;
        conn.create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?
            .check()?;
        // The cursor keeps its own reference to the pixmap's bitmap
        // data; safe to free our handle to it once the cursor exists.
        conn.free_pixmap(pixmap)?.check()?;
        Ok(cursor)
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

    /// Grabs the pointer with an invisible cursor and no window
    /// confinement — the physical cursor position still moves (we
    /// read it via raw evdev regardless, unaffected by any of this),
    /// but it's no longer *visible*, and local apps stop receiving
    /// its clicks/motion while a peer is being controlled, matching
    /// what a KVM switch is expected to feel like. Idempotent.
    pub fn grab(&mut self) {
        if self.grabbed {
            return;
        }
        let event_mask = EventMask::POINTER_MOTION
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::BUTTON_MOTION;
        let cookie = match self.conn.grab_pointer(
            false,
            self.root,
            event_mask,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE, // no confine_to — see module doc comment
            self.invisible_cursor,
            x11rb::CURRENT_TIME,
        ) {
            Ok(cookie) => cookie,
            Err(e) => {
                tracing::warn!(error = %e, "XGrabPointer request failed to send");
                return;
            }
        };
        match cookie.reply() {
            Ok(_reply) => {
                self.grabbed = true;
                tracing::info!("pointer grabbed (local cursor hidden while controlling a peer)");
            }
            Err(e) => tracing::warn!(error = %e, "XGrabPointer failed"),
        }
    }

    /// Idempotent.
    pub fn ungrab(&mut self) {
        if !self.grabbed {
            return;
        }
        match self.conn.ungrab_pointer(x11rb::CURRENT_TIME) {
            Ok(cookie) => {
                if let Err(e) = cookie.check() {
                    tracing::warn!(error = %e, "XUngrabPointer failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "XUngrabPointer request failed to send"),
        }
        self.grabbed = false;
        tracing::info!("pointer released (local cursor visible again)");
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
        // Grab/ungrab tracks `active_receiver` directly rather than
        // only the edge event above, so it also engages when the
        // Devices page's manual "Control this device" button sets it.
        if shared.backend.coordinator.active_receiver_sync().is_some() {
            watcher.grab();
        } else {
            watcher.ungrab();
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
