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
//! with the same physical mouse. Successful pointer capture confines the
//! local cursor to an InputOnly window and hides it until control returns.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ConfigureWindowAux, CreateGCAux, CreateWindowAux, EventMask, GrabMode, GrabStatus, Rectangle, WindowClass};
use x11rb::rust_connection::RustConnection;

use entanglo_core::net::{ConnId, RemoteEntryEdge};

use crate::state::AppShared;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl ScreenEdge {
    pub const ALL: [ScreenEdge; 4] = [ScreenEdge::Left, ScreenEdge::Right, ScreenEdge::Top, ScreenEdge::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            ScreenEdge::Left => "Left edge",
            ScreenEdge::Right => "Right edge",
            ScreenEdge::Top => "Top edge",
            ScreenEdge::Bottom => "Bottom edge",
        }
    }

    fn setting_key(self) -> &'static str {
        match self {
            ScreenEdge::Left => "left",
            ScreenEdge::Right => "right",
            ScreenEdge::Top => "top",
            ScreenEdge::Bottom => "bottom",
        }
    }
}

/// A device assigned to a screen edge. Keyed by device identity rather than
/// by connection so the choice survives reconnects and restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeTarget {
    pub device_id: String,
    pub name: String,
}

pub type EdgeAssignments = HashMap<ScreenEdge, EdgeTarget>;

/// One `edge<TAB>device id<TAB>device name` line per assignment.
pub fn parse_assignments(text: &str) -> EdgeAssignments {
    text.lines().filter_map(|line| {
        let mut fields = line.splitn(3, '\t');
        let key = fields.next()?;
        let edge = ScreenEdge::ALL.into_iter().find(|edge| edge.setting_key() == key)?;
        let device_id = fields.next()?.to_owned();
        let name = fields.next()?.to_owned();
        Some((edge, EdgeTarget { device_id, name }))
    }).collect()
}

pub fn serialize_assignments(assignments: &EdgeAssignments) -> String {
    ScreenEdge::ALL.into_iter().filter_map(|edge| {
        let target = assignments.get(&edge)?;
        let name = target.name.replace(['\t', '\n'], " ");
        Some(format!("{}\t{}\t{name}\n", edge.setting_key(), target.device_id))
    }).collect()
}

pub struct EdgeWatcher {
    conn: RustConnection,
    root: u32,
    screen_width: i16,
    screen_height: i16,
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
    confinement_window: u32,
    saved_pointer: Option<(i16, i16)>,
    /// Whether we currently hold the pointer grab — tracked locally
    /// so the poll loop only calls `grab`/`ungrab` on an actual
    /// transition, not every tick.
    grabbed: bool,
    /// Tracked apart from the pointer: another client can briefly hold the
    /// keyboard, in which case the poll loop retries this grab alone.
    keyboard_grabbed: bool,
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

        // InputOnly windows have no pixels. A mapped 1x1 window confines
        // the core pointer while raw evdev motion continues to reach capture.
        let confinement_window = conn.generate_id().ok()?;
        conn.create_window(0, confinement_window, root, (screen.width_in_pixels / 2) as i16,
            (screen.height_in_pixels / 2) as i16, 1, 1, 0,
            WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new().override_redirect(1))
            .ok()?.check().ok()?;

        Some(Self {
            root,
            screen_width: screen.width_in_pixels as i16,
            screen_height: screen.height_in_pixels as i16,
            conn,
            was_at_edge: None,
            invisible_cursor,
            confinement_window,
            saved_pointer: None,
            grabbed: false,
            keyboard_grabbed: false,
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
        // X11 pixmap contents are undefined on creation: clear the mask.
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(0))?.check()?;
        conn.poly_fill_rectangle(pixmap, gc, &[Rectangle { x: 0, y: 0, width: 1, height: 1 }])?.check()?;
        conn.free_gc(gc)?.check()?;
        let cursor = conn.generate_id()?;
        conn.create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?
            .check()?;
        // The cursor keeps its own reference to the pixmap's bitmap
        // data; safe to free our handle to it once the cursor exists.
        conn.free_pixmap(pixmap)?.check()?;
        Ok(cursor)
    }

    fn restore_pointer(&mut self, edge: RemoteEntryEdge, fraction: f64) {
        self.ungrab();
        if let Ok(cookie) = self.conn.get_geometry(self.root) {
            if let Ok(geometry) = cookie.reply() {
                self.screen_width = geometry.width as i16;
                self.screen_height = geometry.height as i16;
            }
        }
        let (x, y) = return_position(edge, fraction, self.screen_width, self.screen_height);
        if let Ok(cookie) = self.conn.warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, x, y) {
            let _ = cookie.check();
        }
        let _ = self.conn.flush();
        self.was_at_edge = None;
    }

    /// Returns `Some(edge)` the moment the cursor arrives at an edge
    /// (not on every subsequent poll while it stays there).
    pub fn poll(&mut self) -> Option<ScreenEdge> {
        let pointer = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let at_edge = edge_at(pointer.root_x, pointer.root_y, self.screen_width, self.screen_height);
        let crossed = if at_edge.is_some() && at_edge != self.was_at_edge {
            at_edge
        } else {
            None
        };
        self.was_at_edge = at_edge;
        crossed
    }

    /// Confine and hide the local pointer before forwarding raw input.
    /// Never report readiness when X11 denied either grab.
    pub fn grab(&mut self) -> bool {
        if !self.grabbed { self.grab_local_pointer(); }
        if self.grabbed && !self.keyboard_grabbed { self.grab_local_keyboard(); }
        self.grabbed && self.keyboard_grabbed
    }

    /// Without this, keys typed for the peer also reach the focused local
    /// window. Capture reads raw evdev, so it is unaffected by the grab.
    fn grab_local_keyboard(&mut self) {
        let reply = self.conn
            .grab_keyboard(false, self.root, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        if reply.is_some_and(|reply| reply.status == GrabStatus::SUCCESS) {
            self.keyboard_grabbed = true;
            tracing::info!("keyboard grabbed while controlling a peer");
        } else {
            tracing::debug!("keyboard grab unavailable; retrying");
        }
    }

    fn grab_local_pointer(&mut self) {
        if let Ok(cookie) = self.conn.get_geometry(self.root) {
            if let Ok(geometry) = cookie.reply() {
                self.screen_width = geometry.width as i16;
                self.screen_height = geometry.height as i16;
                let _ = self.conn.configure_window(self.confinement_window,
                    &ConfigureWindowAux::new().x(i32::from(geometry.width / 2)).y(i32::from(geometry.height / 2)));
            }
        }
        self.saved_pointer = self.conn.query_pointer(self.root).ok()
            .and_then(|cookie| cookie.reply().ok()).map(|pointer| (pointer.root_x, pointer.root_y));
        if self.conn.map_window(self.confinement_window).ok().and_then(|cookie| cookie.check().ok()).is_none() {
            tracing::warn!("could not map pointer confinement window");
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
            self.confinement_window,
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
            Ok(reply) if pointer_grab_succeeded(reply.status) => {
                self.grabbed = true;
                tracing::info!("pointer captured: local cursor hidden and confined while controlling a peer");
            }
            Ok(reply) => {
                let _ = self.conn.unmap_window(self.confinement_window);
                let _ = self.conn.flush();
                tracing::debug!(status = ?reply.status, "X11 denied pointer capture; remote input remains paused");
            }
            Err(e) => {
                let _ = self.conn.unmap_window(self.confinement_window);
                let _ = self.conn.flush();
                tracing::warn!(error = %e, "XGrabPointer failed");
            }
        }
    }

    /// Idempotent.
    pub fn ungrab(&mut self) {
        if self.keyboard_grabbed {
            match self.conn.ungrab_keyboard(x11rb::CURRENT_TIME) {
                Ok(cookie) => {
                    if let Err(e) = cookie.check() {
                        tracing::warn!(error = %e, "XUngrabKeyboard failed");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "XUngrabKeyboard request failed to send"),
            }
            self.keyboard_grabbed = false;
            tracing::info!("keyboard released");
        }
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
        let _ = self.conn.unmap_window(self.confinement_window);
        if let Some((x, y)) = self.saved_pointer.take() {
            let x = x.clamp(2, (self.screen_width - 3).max(2));
            let y = y.clamp(2, (self.screen_height - 3).max(2));
            let _ = self.conn.warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, x, y);
        }
        let _ = self.conn.flush();
        self.was_at_edge = None;
        tracing::info!("pointer released (local cursor visible again)");
    }
}

fn pointer_grab_succeeded(status: GrabStatus) -> bool { status == GrabStatus::SUCCESS }

/// Return just inside the controller's adjoining edge. Preserve the
/// normalized position along that edge across different screen sizes.
fn return_position(remote_entry: RemoteEntryEdge, fraction: f64, width: i16, height: i16) -> (i16, i16) {
    let right = (width - 1).max(0);
    let bottom = (height - 1).max(0);
    let inset_x = (right / 2).min(2);
    let inset_y = (bottom / 2).min(2);
    let x = (fraction.clamp(0.0, 1.0) * f64::from(right)).round() as i16;
    let y = (fraction.clamp(0.0, 1.0) * f64::from(bottom)).round() as i16;
    match remote_entry {
        RemoteEntryEdge::Left => (right - inset_x, y.clamp(inset_y, bottom - inset_y)),
        RemoteEntryEdge::Right => (inset_x, y.clamp(inset_y, bottom - inset_y)),
        RemoteEntryEdge::Top => (x.clamp(inset_x, right - inset_x), bottom - inset_y),
        RemoteEntryEdge::Bottom => (x.clamp(inset_x, right - inset_x), inset_y),
    }
}

/// Corners count as the side edge.
fn edge_at(x: i16, y: i16, screen_width: i16, screen_height: i16) -> Option<ScreenEdge> {
    edge_for_x(x, screen_width).or(if y <= 0 {
        Some(ScreenEdge::Top)
    } else if y >= screen_height - 1 {
        Some(ScreenEdge::Bottom)
    } else {
        None
    })
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
    if std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_some() {
        tracing::info!("receiver-only mode: local pointer capture disabled");
        return;
    }
    let Some(mut watcher) = EdgeWatcher::connect() else {
        return;
    };
    let shared = Rc::clone(shared);
    glib::source::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Some((edge, fraction)) = shared.backend.coordinator.take_pointer_return() {
            watcher.restore_pointer(edge, fraction);
        }
        let mut activation_started = false;
        if let Some(edge) = watcher.poll() {
            // A confined pointer may appear at a local edge. It must not
            // reseed the remote virtual cursor during an active session.
            if should_activate_edge(shared.backend.coordinator.active_receiver_sync()) {
                if let Some(conn_id) = shared.conn_id_for_edge(edge) {
                    if !shared.backend.coordinator.is_emergency_stopped() {
                        let ready = watcher.grab();
                        shared.backend.coordinator.set_controller_capture_ready(ready);
                        if ready {
                            activation_started = true;
                            activate(&shared, conn_id, edge);
                        }
                    }
                }
            }
        }
        // Grab/ungrab tracks `active_receiver` directly rather than
        // only the edge event above, so it also engages when the
        // Devices page's manual "Control this device" button sets it.
        //
        // MUST also check `is_emergency_stopped` — a real incident
        // during development found this the hard way: with only the
        // `active_receiver` check, Ctrl+Shift+Escape correctly paused
        // input forwarding/injection but left the local pointer
        // grabbed and invisible, because `emergency_stop()` doesn't
        // clear `active_receiver` (resuming should restore the same
        // target without re-selecting it) — so this loop kept
        // re-grabbing every 50 ms with no way to stop except killing
        // the process. A pointer lockout with no in-app escape hatch
        // is exactly the failure mode Emergency Stop exists to
        // prevent; it must never require killing the app to recover.
        let should_grab = should_grab_pointer(
            shared.backend.coordinator.is_emergency_stopped(),
            shared.backend.coordinator.active_receiver_sync(),
        ) || activation_started;
        if should_grab {
            shared.backend.coordinator.set_controller_capture_ready(watcher.grab());
        } else {
            shared.backend.coordinator.set_controller_capture_ready(false);
            watcher.ungrab();
        }
        glib::ControlFlow::Continue
    });
}

/// Extracted so this specific decision — the one that caused a real
/// "mouse locked with no way out but killing the app" incident during
/// development — is unit tested directly, not just read carefully.
fn should_grab_pointer(emergency_stopped: bool, active_receiver: Option<ConnId>) -> bool {
    !emergency_stopped && active_receiver.is_some()
}

/// An edge crossing starts a session only while control is local. The
/// confined X11 pointer must not reset the already-active remote cursor.
fn should_activate_edge(active_receiver: Option<ConnId>) -> bool {
    active_receiver.is_none()
}

fn activate(shared: &Rc<AppShared>, conn_id: ConnId, local_edge: ScreenEdge) {
    let coordinator = Arc::clone(&shared.backend.coordinator);
    let remote_entry_edge = match local_edge {
        ScreenEdge::Left => RemoteEntryEdge::Right,
        ScreenEdge::Right => RemoteEntryEdge::Left,
        ScreenEdge::Top => RemoteEntryEdge::Bottom,
        ScreenEdge::Bottom => RemoteEntryEdge::Top,
    };
    shared.backend.handle.spawn(async move {
        coordinator
            .set_active_receiver_from_edge(conn_id, remote_entry_edge)
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an isolated X11 server and ENTANGLO_X11_TEST=1"]
    fn isolated_x11_capture_confines_retries_and_restores_the_pointer() {
        assert_eq!(std::env::var("ENTANGLO_X11_TEST").as_deref(), Ok("1"));
        let mut watcher = EdgeWatcher::connect().expect("isolated X11 server");
        let (control, screen) = RustConnection::connect(None).unwrap();
        let root = control.setup().roots[screen].root;
        let warp = |x, y| {
            control.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x, y).unwrap().check().unwrap();
        };
        let position = || {
            let pointer = control.query_pointer(root).unwrap().reply().unwrap();
            (pointer.root_x, pointer.root_y)
        };
        warp(400, 300);
        let blocker = control.grab_pointer(false, root, EventMask::POINTER_MOTION,
            GrabMode::ASYNC, GrabMode::ASYNC, x11rb::NONE, x11rb::NONE, x11rb::CURRENT_TIME)
            .unwrap().reply().unwrap();
        assert_eq!(blocker.status, GrabStatus::SUCCESS);
        assert!(!watcher.grab(), "a competing X11 grab must pause forwarding");
        assert!(!watcher.grabbed);
        assert_eq!(position(), (400, 300));
        control.ungrab_pointer(x11rb::CURRENT_TIME).unwrap().check().unwrap();
        assert!(watcher.grab(), "retry must succeed after the competing grab ends");
        let confined = position();
        warp(900, 600);
        assert_eq!(position(), confined, "local pointer must not track remote motion");
        watcher.restore_pointer(RemoteEntryEdge::Left, 0.5);
        assert!(!watcher.grabbed && !watcher.keyboard_grabbed);
        assert_eq!(position(), return_position(RemoteEntryEdge::Left, 0.5, watcher.screen_width, watcher.screen_height));
        warp(500, 400);
        assert_eq!(position(), (500, 400), "local input must work after natural return");
        assert!(watcher.grab());
        watcher.ungrab();
        assert_eq!(position(), (500, 400), "emergency release must restore the pre-grab position");
        assert!(watcher.grab());
        drop(watcher);
        warp(700, 500);
        assert_eq!(position(), (700, 500), "X11 connection loss must release confinement");
    }

    #[test]
    fn denied_pointer_grabs_are_never_reported_as_success() {
        assert!(pointer_grab_succeeded(GrabStatus::SUCCESS));
        for status in [GrabStatus::ALREADY_GRABBED, GrabStatus::INVALID_TIME, GrabStatus::NOT_VIEWABLE, GrabStatus::FROZEN] {
            assert!(!pointer_grab_succeeded(status));
        }
    }

    #[test]
    fn return_uses_the_adjoining_edge_instead_of_the_hidden_local_position() {
        assert_eq!(return_position(RemoteEntryEdge::Left, 0.5, 2560, 1440), (2557, 720));
        assert_eq!(return_position(RemoteEntryEdge::Right, 0.25, 1920, 1080), (2, 270));
        assert_eq!(return_position(RemoteEntryEdge::Top, 0.5, 1920, 1080), (960, 1077));
        assert_eq!(return_position(RemoteEntryEdge::Bottom, 0.5, 1920, 1080), (960, 2));
        assert_eq!(edge_at(2557, 720, 2560, 1440), None, "return must not immediately trigger another crossing");
    }

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
    fn assignments_survive_a_save_and_load() {
        let assignments = EdgeAssignments::from([(ScreenEdge::Right,
            EdgeTarget { device_id: "A56B".into(), name: "iMac".into() })]);
        assert_eq!(parse_assignments(&serialize_assignments(&assignments)), assignments);
    }

    #[test]
    fn malformed_assignment_lines_are_ignored() {
        assert!(parse_assignments("center\tid\tname\nleft\tonly-id\n\n").is_empty());
    }

    #[test]
    fn saved_device_names_cannot_break_the_line_format() {
        let assignments = EdgeAssignments::from([(ScreenEdge::Left,
            EdgeTarget { device_id: "id".into(), name: "a\tb\nc".into() })]);
        assert_eq!(parse_assignments(&serialize_assignments(&assignments))[&ScreenEdge::Left].name, "a b c");
    }

    #[test]
    fn top_and_bottom_edges_are_detected_and_corners_stay_on_the_side() {
        assert_eq!(edge_at(960, 0, 1920, 1080), Some(ScreenEdge::Top));
        assert_eq!(edge_at(960, 1079, 1920, 1080), Some(ScreenEdge::Bottom));
        assert_eq!(edge_at(960, 540, 1920, 1080), None);
        assert_eq!(edge_at(0, 0, 1920, 1080), Some(ScreenEdge::Left));
    }

    #[test]
    fn middle_of_screen_is_no_edge() {
        assert_eq!(edge_for_x(960, 1920), None);
    }

    /// Regression test for a real incident: a user hit Ctrl+Shift+Escape
    /// while controlling a peer, expecting the local mouse back, but
    /// it stayed grabbed/invisible because the grab decision only
    /// checked `active_receiver` — `emergency_stop()` intentionally
    /// doesn't clear that (so resuming restores the same target), so
    /// nothing ever told the pointer to come back. Recovering required
    /// killing the whole app. This must never regress.
    #[test]
    fn emergency_stop_forces_ungrab_even_with_a_target_still_set() {
        assert!(!should_grab_pointer(
            /* emergency_stopped */ true,
            /* active_receiver */ Some(0),
        ));
    }

    #[test]
    fn grabs_when_targeting_a_peer_and_not_stopped() {
        assert!(should_grab_pointer(false, Some(0)));
    }

    #[test]
    fn does_not_grab_with_no_target() {
        assert!(!should_grab_pointer(false, None));
    }

    #[test]
    fn does_not_grab_when_stopped_with_no_target_either() {
        assert!(!should_grab_pointer(true, None));
    }

    #[test]
    fn edge_crossing_activates_only_while_control_is_local() {
        assert!(should_activate_edge(None));
        assert!(!should_activate_edge(Some(42)));
    }
}
