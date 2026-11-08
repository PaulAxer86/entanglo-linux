//! Multi-peer connection manager built on `session.rs`. Owns the
//! inbound TCP listener, dials outbound connections, spawns one
//! `session::run_session` task per connection, and merges every
//! peer's `SessionEvent` stream into one `CoordinatorEvent` stream
//! tagged by connection id for the caller (the GTK app, or a test
//! harness) to consume.
//!
//! Mirrors `entanglo-macos`'s `ConnectionCoordinator` and
//! `entanglo-windows`'s `ConnectionCoordinator.cs`: a device may hold
//! simultaneous outbound (self-initiated) and inbound (peer-initiated)
//! connections to the same peer — this is why peers are keyed by an
//! internal connection id here, not by device id, which isn't known
//! until the peer's `hello` arrives (see `PROTOCOL.md` §6).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::features::clipboard::ClipboardSyncService;
use crate::input::{InputCaptureService, InputInjectionService};
use crate::protocol::payloads::{
    HelloPayload, InputEventMessage, PairRequestPayload, ReleaseControlPayload,
};

use super::session::{run_session, OutgoingMessage, SessionConfig, SessionEvent};
use super::transport::NetworkTransport;
use super::trust_store::TrustStore;

pub type ConnId = u64;

/// Edge of the remote screen through which control entered. Crossing
/// a local edge enters the remote screen from the opposite edge. Only
/// that same remote edge returns control locally, matching
/// `entanglo-macos`'s anti-loop behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteEntryEdge {
    Left,
    Right,
    Top,
    Bottom,
}

const FALLBACK_REMOTE_SCREEN_WIDTH: f64 = 2560.0;
const FALLBACK_REMOTE_SCREEN_HEIGHT: f64 = 1440.0;

#[derive(Debug)]
struct RemoteCursorState {
    conn_id: ConnId,
    entry_edge: RemoteEntryEdge,
    x: f64,
    y: f64,
    screen_width: f64,
    screen_height: f64,
    buttons_held: u8,
    /// Linux receivers read coordinates on a fixed 2560 x 1440 canvas
    /// rather than in their own pixels; see `input::inject::map_absolute`.
    linux_canvas: bool,
}

impl RemoteCursorState {
    fn new(
        conn_id: ConnId,
        entry_edge: RemoteEntryEdge,
        screen_width: f64,
        screen_height: f64,
    ) -> Self {
        let (x, y) = match entry_edge {
            RemoteEntryEdge::Left => (0.0, screen_height / 2.0),
            RemoteEntryEdge::Right => (screen_width - 1.0, screen_height / 2.0),
            RemoteEntryEdge::Top => (screen_width / 2.0, 0.0),
            RemoteEntryEdge::Bottom => (screen_width / 2.0, screen_height - 1.0),
        };
        Self {
            conn_id,
            entry_edge,
            x,
            y,
            screen_width,
            screen_height,
            buttons_held: 0,
            linux_canvas: false,
        }
    }

    fn with_linux_canvas(mut self, linux_canvas: bool) -> Self {
        self.linux_canvas = linux_canvas;
        self
    }

    fn wire_position(&self) -> (f64, f64) {
        if self.linux_canvas {
            (
                self.x / (self.screen_width - 1.0) * 2559.0,
                self.y / (self.screen_height - 1.0) * 1439.0,
            )
        } else {
            (self.x, self.y)
        }
    }

    /// Updates the sender-side virtual cursor. Returns true when it
    /// crosses back out through the same edge it entered from. The
    /// opposite edge is a wall, preventing an immediate two-machine
    /// bounce loop. Do not return during a drag: releasing control
    /// mid-drag can leave a mouse button logically held on the peer.
    fn observe(&mut self, event: &mut InputEventMessage) -> bool {
        use crate::protocol::payloads::input_event::InputEventKind;

        match event.kind {
            InputEventKind::MouseDown => {
                if let Some(button) = event.button.filter(|button| *button < 8) {
                    self.buttons_held |= 1 << button;
                }
                return false;
            }
            InputEventKind::MouseUp => {
                if let Some(button) = event.button.filter(|button| *button < 8) {
                    self.buttons_held &= !(1 << button);
                }
                return false;
            }
            InputEventKind::MouseMove => {
                self.x += event.delta_x.unwrap_or(0.0);
                self.y += event.delta_y.unwrap_or(0.0);
            }
            _ => return false,
        }

        if self.buttons_held == 0 {
            match self.entry_edge {
                RemoteEntryEdge::Left if self.x < 0.0 => return true,
                RemoteEntryEdge::Right if self.x >= self.screen_width => return true,
                RemoteEntryEdge::Top if self.y < 0.0 => return true,
                RemoteEntryEdge::Bottom if self.y >= self.screen_height => return true,
                _ => {}
            }
        }

        self.x = self.x.clamp(0.0, self.screen_width - 1.0);
        self.y = self.y.clamp(0.0, self.screen_height - 1.0);
        // macOS uses absolute coordinates when present. Sending our
        // sender-side virtual cursor makes the pointer enter at the
        // physical adjoining edge instead of continuing from the
        // receiver's stale cursor position (its relative fallback).
        let (x, y) = self.wire_position();
        event.x = Some(x);
        event.y = Some(y);
        event.delta_x = None;
        event.delta_y = None;
        false
    }
}

fn peer_screen_size(hello: &HelloPayload) -> Option<(f64, f64)> {
    hello.screen_size()
}

/// What the coordinator reports, one connection at a time. Same
/// shape as `SessionEvent` with a `conn_id` attached so the caller
/// can tell peers apart before any of them have a known device id.
#[derive(Debug)]
pub enum CoordinatorEvent {
    PeerDiscovered(super::discovery::DiscoveredPeer),
    PeerDiscoveryRemoved(String),
    PeerConnected {
        conn_id: ConnId,
        direction: Direction,
        local_addr: Option<SocketAddr>,
        peer_addr: Option<SocketAddr>,
    },
    /// Fires as soon as the peer's device id is known, before any
    /// trust decision — lets the UI offer manual trust
    /// (`Coordinator::trust_manually`) for peers that never send a
    /// `pairRequest`, which turns out to include the real
    /// `entanglo-macos` v0.1.58 — see `session::OutgoingMessage::
    /// TrustManually`'s doc comment for how that was discovered.
    PeerIdentified {
        conn_id: ConnId,
        device_id: String,
    },
    PeerHello {
        conn_id: ConnId,
        hello: HelloPayload,
    },
    PairingRequested {
        conn_id: ConnId,
        request: PairRequestPayload,
        respond: oneshot::Sender<bool>,
    },
    Trusted {
        conn_id: ConnId,
        device_id: String,
    },
    PairingRejected {
        conn_id: ConnId,
    },
    Heartbeat {
        conn_id: ConnId,
        rtt_ms: Option<f64>,
    },
    InputEvent {
        conn_id: ConnId,
        event: InputEventMessage,
    },
    ClipboardText {
        conn_id: ConnId,
        text: String,
    },
    FileTransfer {
        conn_id: ConnId,
        progress: crate::features::file_transfer::FileProgress,
    },
    ReleaseControl {
        conn_id: ConnId,
        reason: String,
    },
    PeerDisconnected {
        conn_id: ConnId,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

struct PeerHandle {
    outgoing_tx: mpsc::UnboundedSender<OutgoingMessage>,
    device_id: Option<String>,
    screen_size: Option<(f64, f64)>,
    is_linux: bool,
}

pub struct Coordinator {
    local_device_id: String,
    local_hello: HelloPayload,
    trust_store: Arc<Mutex<TrustStore>>,
    peers: Mutex<HashMap<ConnId, PeerHandle>>,
    next_conn_id: AtomicU64,
    events_tx: mpsc::UnboundedSender<CoordinatorEvent>,
    /// The peer currently receiving this device's input, if this
    /// device is acting as controller. Set either by the Devices page's
    /// manual button or by the X11 edge watcher. Cleared automatically
    /// when the sender-side virtual cursor returns through its remote
    /// entry edge, on disconnect, or when the peer sends `releaseControl`.
    // Plain std Mutex, not tokio's — nothing here ever holds this
    // lock across an `.await`, so a sync lock lets GTK code (the
    // pointer-grab poll loop in `edge.rs`) read it directly without
    // needing an async round trip through the backend runtime.
    active_receiver: std::sync::Mutex<Option<ConnId>>,
    /// Sender-side remote cursor used to detect the natural edge return
    /// to this Linux desktop. Present only when control was activated by
    /// an assigned screen edge; the manual Control button has no known
    /// geometric entry edge and still relies on Emergency Stop.
    remote_cursor: std::sync::Mutex<Option<RemoteCursorState>>,
    /// `Some` once `enable_receiver` has successfully opened
    /// `/dev/uinput` — every trusted peer's `InputEvent`s are then
    /// injected through it. `None` (the default) means this device
    /// can still act as controller and observe `InputEvent`s via
    /// `CoordinatorEvent`, it just never writes them to a virtual
    /// device — the safe default until a caller opts in.
    injector: Mutex<Option<InputInjectionService>>,
    /// Mirrors "`injector` is `Some`" as a plain atomic so UI code can
    /// check receiver status synchronously (`receiver_enabled`)
    /// without an async lock, from any thread.
    receiver_enabled_flag: std::sync::atomic::AtomicBool,
    /// Set once `enable_controller` has successfully enumerated and
    /// started reading local input devices. Doesn't mean any device
    /// was actually found — see `controller_device_count`.
    controller_enabled: std::sync::atomic::AtomicBool,
    controller_device_count: std::sync::atomic::AtomicUsize,
    /// The Input Sharing page's Emergency Stop, per `ROADMAP.md`
    /// Phase 1 ("matches the Mac's triple-Escape + explicit button").
    /// While set, both directions of input sharing are inert: this
    /// device stops forwarding its own captured input to
    /// `active_receiver`, *and* stops injecting a trusted peer's
    /// incoming `InputEvent`s — see the checks in `send_to_active_receiver`
    /// and the `spawn_peer` relay loop.
    emergency_stopped: std::sync::atomic::AtomicBool,
    /// The peer whose `InputEvent`s we're currently injecting, if
    /// any — set on the *first* `InputEvent` from a trusted peer
    /// while this is `None`, and cleared the instant local hardware
    /// input is detected (`enable_controller`'s per-device loop).
    /// That clear is what actually sends `releaseControl` — see the
    /// checks in `spawn_peer`'s relay loop and in `enable_controller`.
    /// Single-target by design: Phase 1's goal is "share one mouse +
    /// keyboard" (`ROADMAP.md`), not arbitrate multiple simultaneous
    /// controllers.
    being_controlled_by: Mutex<Option<ConnId>>,
    preferred_controller: std::sync::Mutex<Option<String>>,
    discovery_attempts: Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl Coordinator {
    /// Returns the coordinator and the merged event stream. Keep the
    /// receiver draining — a full channel just grows unboundedly
    /// rather than blocking senders, but nothing reads a dropped
    /// receiver's events, so the UI (or a test) must actually consume
    /// this.
    pub fn new(
        local_device_id: String,
        local_hello: HelloPayload,
        trust_store: Arc<Mutex<TrustStore>>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<CoordinatorEvent>) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let coordinator = Arc::new(Self {
            local_device_id,
            local_hello,
            trust_store,
            peers: Mutex::new(HashMap::new()),
            next_conn_id: AtomicU64::new(0),
            events_tx,
            active_receiver: std::sync::Mutex::new(None),
            remote_cursor: std::sync::Mutex::new(None),
            injector: Mutex::new(None),
            receiver_enabled_flag: std::sync::atomic::AtomicBool::new(false),
            controller_enabled: std::sync::atomic::AtomicBool::new(false),
            controller_device_count: std::sync::atomic::AtomicUsize::new(0),
            emergency_stopped: std::sync::atomic::AtomicBool::new(false),
            being_controlled_by: Mutex::new(None),
            preferred_controller: std::sync::Mutex::new(super::trust_store::preferred_controller()),
            discovery_attempts: Mutex::new(std::collections::HashMap::new()),
        });
        (coordinator, events_rx)
    }

    /// Opens `/dev/uinput` so incoming `InputEvent`s from trusted
    /// peers actually move the mouse / press keys, instead of only
    /// being visible as `CoordinatorEvent::InputEvent`s. Requires the
    /// process to be in the `input` group — see
    /// `packaging/60-entanglo-uinput.rules`. Idempotent: calling it
    /// again replaces the previous virtual device.
    pub async fn enable_receiver(&self) -> Result<(), crate::input::inject::InjectionError> {
        let injector = InputInjectionService::open()?;
        tracing::info!("receiver ready: virtual mouse and keyboard opened");
        *self.injector.lock().await = Some(injector);
        self.receiver_enabled_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Enumerates local keyboard/mouse devices via evdev and starts
    /// forwarding every event to whichever peer `set_active_receiver`
    /// currently targets. Requires the process to be in the `input`
    /// group. One task per physical device; they share one
    /// `InputCaptureService` (behind a mutex) so modifier-key state
    /// stays consistent across e.g. a keyboard and a separate
    /// USB-mouse-with-buttons device.
    ///
    /// Two side effects live in this same loop, since it's the one
    /// place that sees every piece of genuine local hardware input:
    ///
    /// - **`releaseControl`** (`PROTOCOL.md` §5.6): any local input at
    ///   all, while a trusted peer is currently injecting into us
    ///   (`being_controlled_by.is_some()`), hands control back —
    ///   clears `being_controlled_by` and notifies that peer. This is
    ///   `entanglo-macos`'s `LocalInputWatcher` equivalent. Global
    ///   `Emergency Stop` is a separate, deliberate action
    ///   (`emergency_stop`/`resume`) — touching your own mouse doesn't
    ///   need a manual "Resume" click to hand control back next time.
    /// - **Emergency Stop hotkey**: Ctrl+Shift+Escape toggles
    ///   `emergency_stop`/`resume` and is consumed here rather than
    ///   forwarded — this is `ROADMAP.md`'s "global hotkey toggle"
    ///   fallback for edge-detection (⬜), and unlike a GTK
    ///   `ShortcutController` it works even when Entanglo's window
    ///   doesn't have focus, since it's caught at the evdev level.
    pub async fn enable_controller(self: &Arc<Self>) -> std::io::Result<()> {
        let devices = InputCaptureService::enumerate_devices()?;
        self.controller_device_count
            .store(devices.len(), std::sync::atomic::Ordering::Relaxed);
        self.controller_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let capture = Arc::new(Mutex::new(InputCaptureService::new()));
        for device in devices {
            let mut stream = device.into_event_stream()?;
            let capture = Arc::clone(&capture);
            let this = Arc::clone(self);
            tokio::spawn(async move {
                loop {
                    let event = match stream.next_event().await {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::debug!(error = %e, "input device stream ended");
                            return;
                        }
                    };
                    let translated = capture.lock().await.translate(event);
                    let Some(input_event) = translated else {
                        continue;
                    };

                    if is_emergency_stop_hotkey(&input_event) {
                        if this.is_emergency_stopped() {
                            this.resume();
                        } else {
                            this.emergency_stop();
                        }
                        continue; // consumed, not forwarded
                    }

                    if let Some(controller_conn_id) = this.being_controlled_by.lock().await.take() {
                        this.send_release_control(controller_conn_id, "local_input")
                            .await;
                    }

                    this.send_to_active_receiver(input_event).await;
                }
            });
        }
        Ok(())
    }

    /// Binds a TCP listener and spawns a task that accepts
    /// connections forever, handing each to `spawn_peer`. Returns the
    /// bound port so the caller can pass it to
    /// `DiscoveryService::advertise`.
    pub async fn listen(self: &Arc<Self>, bind_addr: &str) -> std::io::Result<u16> {
        let listener = TcpListener::bind(bind_addr).await?;
        let port = listener.local_addr()?.port();
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, _addr)) => {
                        tokio::spawn(this.clone().spawn_peer(socket, Direction::Inbound));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed, listener still running");
                    }
                }
            }
        });
        Ok(port)
    }

    /// Dials an outbound connection and spawns it the same way an
    /// accepted one would be. Typical caller: the discovery browse
    /// loop, dialing a newly-resolved `_entanglo._tcp` peer.
    pub async fn dial(self: &Arc<Self>, addr: SocketAddr) -> std::io::Result<()> {
        let socket = TcpStream::connect(addr).await?;
        Arc::clone(self)
            .spawn_peer(socket, Direction::Outbound)
            .await;
        Ok(())
    }

    async fn spawn_peer(self: Arc<Self>, socket: TcpStream, direction: Direction) {
        tracing::info!(?direction, peer = ?socket.peer_addr().ok(), local = ?socket.local_addr().ok(), "peer transport connected");
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        let (session_events_tx, mut session_events_rx) = mpsc::unbounded_channel();

        self.peers.lock().await.insert(
            conn_id,
            PeerHandle {
                outgoing_tx,
                device_id: None,
                screen_size: None,
                is_linux: false,
            },
        );
        let _ = self
            .events_tx
            .send(CoordinatorEvent::PeerConnected {
                conn_id, direction, local_addr: socket.local_addr().ok(), peer_addr: socket.peer_addr().ok(),
            });

        let config = SessionConfig {
            local_device_id: self.local_device_id.clone(),
            local_session_id: uuid::Uuid::new_v4().to_string(),
            local_hello: self.local_hello.clone(),
        };
        let trust_store = Arc::clone(&self.trust_store);
        tokio::spawn(async move {
            if let Err(e) = run_session(
                NetworkTransport::new(socket),
                config,
                trust_store,
                session_events_tx,
                outgoing_rx,
            )
            .await
            {
                tracing::warn!(error = %e, conn_id, "session ended with an error");
            }
        });

        let this = Arc::clone(&self);
        tokio::spawn(async move {
            let mut disconnect_reason = "connection closed".to_string();
            while let Some(event) = session_events_rx.recv().await {
                let closed = matches!(event, SessionEvent::Closed { .. });
                if let SessionEvent::Closed { ref reason } = event {
                    disconnect_reason = reason.clone();
                }
                if let SessionEvent::InputEvent(ref input_event) = event {
                    // `session.rs` already gates InputEvent delivery
                    // on trust — see the `trusted` check in
                    // `run_session`'s receive loop — so anything that
                    // reaches here is from a trusted peer. Two more
                    // gates before it's safe to inject:
                    //  - not emergency-stopped, and
                    //  - this conn_id is the one currently "holding"
                    //    control (claims it on the first InputEvent
                    //    seen while nobody else holds it; a second
                    //    peer can't inject over an active controller
                    //    — Phase 1 is single-target by design, see
                    //    `being_controlled_by`'s doc comment).
                    // `enable_controller`'s local-input detection is
                    // what releases the claim (`releaseControl`).
                    // Only actually writes to a virtual device if
                    // `enable_receiver` was called; otherwise this is
                    // a no-op and the event still reaches the UI below.
                    if !this.is_emergency_stopped() {
                        let mut holder = this.being_controlled_by.lock().await;
                        let holds_control = match *holder {
                            None => {
                                tracing::info!(conn_id, "trusted peer input received; receiver control started");
                                *holder = Some(conn_id);
                                true
                            }
                            Some(existing) => existing == conn_id,
                        };
                        drop(holder);

                        if holds_control {
                            if let Some(id) = this.peers.lock().await.get(&conn_id).and_then(|peer| peer.device_id.clone()) {
                                let mut preferred = this.preferred_controller.lock().unwrap();
                                if preferred.as_ref() != Some(&id) {
                                    *preferred = Some(id.clone());
                                    if let Err(error) = super::trust_store::save_preferred_controller(&id) {
                                        tracing::warn!(%error, "could not remember preferred controller");
                                    }
                                }
                            }
                            if let Some(injector) = this.injector.lock().await.as_mut() {
                                if let Err(e) = injector.inject(input_event) {
                                    tracing::warn!(error = %e, conn_id, "input injection failed");
                                }
                            }
                        }
                    }
                }
                let identified_device_id = match &event {
                    SessionEvent::PeerIdentified { device_id }
                    | SessionEvent::Trusted { device_id } => Some(device_id.clone()),
                    _ => None,
                };
                if let Some(device_id) = identified_device_id {
                    if let Some(handle) = this.peers.lock().await.get_mut(&conn_id) {
                        handle.device_id = Some(device_id);
                    }
                }
                if let SessionEvent::PeerHello { ref hello, .. } = event {
                    let negotiated_screen_size = peer_screen_size(hello);
                    if let Some(handle) = this.peers.lock().await.get_mut(&conn_id) {
                        handle.screen_size = negotiated_screen_size;
                        handle.is_linux = hello.platform.as_deref() == Some("Linux");
                    }
                    tracing::info!(
                        conn_id,
                        device_model = %hello.device_model,
                        screen_width = negotiated_screen_size.map(|size| size.0),
                        screen_height = negotiated_screen_size.map(|size| size.1),
                        "peer pointer coordinate space selected"
                    );
                }
                if matches!(event, SessionEvent::ReleaseControl { .. }) {
                    // PROTOCOL.md §5.6: the controller MUST immediately
                    // stop forwarding inputEvents on releaseControl.
                    // Only clear if `conn_id` is actually who we were
                    // controlling — a stray releaseControl from
                    // someone else is not this peer's business.
                    let mut active = this.active_receiver.lock().unwrap();
                    if *active == Some(conn_id) {
                        *active = None;
                        drop(active);
                        *this.remote_cursor.lock().unwrap() = None;
                    }
                }
                let _ = this.events_tx.send(translate(conn_id, event));
                if closed {
                    break;
                }
            }
            tracing::info!(conn_id, reason = %disconnect_reason, "peer disconnected");
            this.peers.lock().await.remove(&conn_id);
            {
                let mut active = this.active_receiver.lock().unwrap();
                if *active == Some(conn_id) {
                    *active = None;
                }
            }
            {
                let mut cursor = this.remote_cursor.lock().unwrap();
                if cursor
                    .as_ref()
                    .is_some_and(|state| state.conn_id == conn_id)
                {
                    *cursor = None;
                }
            }
            let mut holder = this.being_controlled_by.lock().await;
            if *holder == Some(conn_id) {
                *holder = None;
            }
            drop(holder);
            let _ = this.events_tx.send(CoordinatorEvent::PeerDisconnected {
                conn_id,
                reason: disconnect_reason,
            });
        });
    }

    pub async fn send_input(&self, conn_id: ConnId, event: InputEventMessage) {
        if let Some(handle) = self.peers.lock().await.get(&conn_id) {
            let _ = handle.outgoing_tx.send(OutgoingMessage::Input(event));
        }
    }

    /// Sends `releaseControl` to `conn_id` — see `PROTOCOL.md` §5.6.
    /// Called from `enable_controller`'s local-input detection; also
    /// usable directly (e.g. a future "release control" UI button).
    pub async fn send_release_control(&self, conn_id: ConnId, reason: &str) {
        if let Some(handle) = self.peers.lock().await.get(&conn_id) {
            let _ =
                handle
                    .outgoing_tx
                    .send(OutgoingMessage::ReleaseControl(ReleaseControlPayload {
                        reason: reason.to_string(),
                    }));
        }
    }

    /// Broadcasts a local plain-text clipboard change to every live
    /// session. Each session applies its own trust gate before putting
    /// anything on the wire, so untrusted peers never receive it.
    /// Sender-side UTF-8 truncation mirrors the Mac's 1 MiB cap.
    pub async fn broadcast_clipboard(&self, text: &str) {
        let mut service = ClipboardSyncService::new();
        service.set_enabled(true);
        let Some(payload) = service.build_payload(text) else {
            return;
        };
        let peers = self.peers.lock().await;
        for handle in peers.values() {
            let _ = handle
                .outgoing_tx
                .send(OutgoingMessage::ClipboardText(payload.clone()));
        }
    }

    /// Manually trusts `conn_id` right now, without a `pairRequest`/
    /// `pairResponse` round trip — see `session::OutgoingMessage::
    /// TrustManually`'s doc comment for why this exists (the real Mac
    /// app doesn't implement that round trip). This is the Devices
    /// page's "Trust" button for a peer we've said hello to but that
    /// never sent us a `pairRequest`.
    pub async fn trust_manually(&self, conn_id: ConnId) {
        if let Some(handle) = self.peers.lock().await.get(&conn_id) {
            let _ = handle.outgoing_tx.send(OutgoingMessage::TrustManually);
        }
    }

    /// Sets which connection this device is currently controlling (if
    /// acting as controller). `None` clears it. The edge-detection /
    /// hotkey logic that decides *when* to call this is ⬜ in
    /// `ROADMAP.md` Phase 1; today the only caller is the Devices
    /// page's manual "Control this device" button.
    pub async fn set_active_receiver(&self, conn_id: Option<ConnId>) {
        *self.active_receiver.lock().unwrap() = conn_id;
        *self.remote_cursor.lock().unwrap() = None;
    }

    /// Activates a peer from a physical screen-edge crossing and seeds
    /// the virtual cursor at the corresponding edge of the remote iMac.
    /// This geometry is what lets raw evdev deltas detect a later return
    /// through the same edge without cooperation from the receiver.
    pub async fn set_active_receiver_from_edge(
        &self,
        conn_id: ConnId,
        remote_entry_edge: RemoteEntryEdge,
    ) {
        let (peer_screen, is_linux) = self
            .peers
            .lock()
            .await
            .get(&conn_id)
            .map_or((None, false), |peer| (peer.screen_size, peer.is_linux));
        let (screen_width, screen_height) =
            peer_screen.unwrap_or((FALLBACK_REMOTE_SCREEN_WIDTH, FALLBACK_REMOTE_SCREEN_HEIGHT));
        tracing::info!(
            conn_id,
            screen_width,
            screen_height,
            "remote pointer coordinate space activated"
        );
        *self.active_receiver.lock().unwrap() = Some(conn_id);
        *self.remote_cursor.lock().unwrap() = Some(RemoteCursorState::new(
            conn_id,
            remote_entry_edge,
            screen_width,
            screen_height,
        ).with_linux_canvas(is_linux));
    }

    pub async fn active_receiver(&self) -> Option<ConnId> {
        *self.active_receiver.lock().unwrap()
    }

    /// Same as `active_receiver`, but synchronous — for GTK-thread
    /// polling (`edge.rs`'s pointer-grab loop) that can't await the
    /// backend runtime on every 50 ms tick without real overhead.
    pub fn report_discovery(&self, event: super::discovery::DiscoveryEvent) {
        let event = match event {
            super::discovery::DiscoveryEvent::Resolved(peer) => CoordinatorEvent::PeerDiscovered(peer),
            super::discovery::DiscoveryEvent::Removed(name) => CoordinatorEvent::PeerDiscoveryRemoved(name),
        };
        let _ = self.events_tx.send(event);
    }

    pub async fn connect_discovered(self: &Arc<Self>, peer: &super::discovery::DiscoveredPeer) -> std::io::Result<()> {
        let Some(id) = peer.device_id.as_deref() else { return Ok(()); };
        if !self.should_connect_discovered(id).await { return Ok(()); }
        let Some(endpoint)=peer.endpoint() else {return Ok(());};
        let key=format!("{id}@{endpoint}");
        {
            let mut attempts = self.discovery_attempts.lock().await;
            if attempts.get(&key).is_some_and(|last| last.elapsed() < std::time::Duration::from_secs(5)) { return Ok(()); }
            attempts.insert(key, std::time::Instant::now());
        }
        self.dial(endpoint).await?;
        Ok(())
    }

    pub async fn should_connect_discovered(&self, device_id: &str) -> bool {
        if device_id == self.local_device_id { return false; }
        if self.peers.lock().await.values().any(|peer|peer.device_id.as_deref()==Some(device_id)) { return false; }
        self.trust_store.lock().await.is_trusted(device_id)
    }

    pub async fn needs_reconnect(&self) -> bool {
        let connected: std::collections::HashSet<String> = self.peers.lock().await.values()
            .filter_map(|peer| peer.device_id.clone()).collect();
        let trusted: std::collections::HashSet<String> = self.trust_store.lock().await.trusted_devices()
            .map(|peer|peer.device_id.clone()).collect();
        reconnect_required(self.preferred_controller.lock().unwrap().as_deref(), &connected, &trusted)
    }

    pub fn receiving_controller_sync(&self) -> Option<ConnId> {
        self.being_controlled_by.try_lock().ok().and_then(|holder| *holder)
    }

    pub fn active_receiver_sync(&self) -> Option<ConnId> {
        *self.active_receiver.lock().unwrap()
    }

    /// Forwards one input event to whichever peer `set_active_receiver`
    /// last selected, if any. No-op if nothing is currently targeted,
    /// or while `emergency_stop` is active.
    pub async fn send_to_active_receiver(&self, mut event: InputEventMessage) {
        if self
            .emergency_stopped
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        // Copy the target out and let the guard drop *before* the
        // `.await` below — `if let Some(x) = *mutex.lock().unwrap() {
        // ...await... }` would extend the temporary guard's lifetime
        // across the whole body per Rust's "if let" temporary rules,
        // holding a `!Send` std MutexGuard across an await point.
        let target = *self.active_receiver.lock().unwrap();
        if let Some(conn_id) = target {
            let returned_local = {
                let mut cursor = self.remote_cursor.lock().unwrap();
                cursor
                    .as_mut()
                    .filter(|state| state.conn_id == conn_id)
                    .is_some_and(|state| state.observe(&mut event))
            };
            if returned_local {
                *self.active_receiver.lock().unwrap() = None;
                *self.remote_cursor.lock().unwrap() = None;
                tracing::info!(
                    conn_id,
                    "remote cursor exited via entry edge; control returned locally"
                );
                return;
            }
            self.send_input(conn_id, event).await;
        }
    }

    /// Input Sharing page's Emergency Stop — matches the Mac's
    /// triple-Escape + explicit button (`ROADMAP.md` Phase 1). Halts
    /// both directions immediately: this device stops forwarding its
    /// own input to `active_receiver`, and stops injecting any
    /// trusted peer's incoming `InputEvent`s. Does **not** clear
    /// `active_receiver` or disconnect anyone — it's a pause, not a
    /// teardown; `resume` lifts it with everything else unchanged.
    pub fn emergency_stop(&self) {
        self.emergency_stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.emergency_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_emergency_stopped(&self) -> bool {
        self.emergency_stopped
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether `enable_receiver` successfully opened `/dev/uinput`.
    pub fn receiver_enabled(&self) -> bool {
        self.receiver_enabled_flag
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `(enabled, device_count)` from the last `enable_controller`
    /// call. `enabled` can be true with a `device_count` of 0 (evdev
    /// enumeration succeeded but found no keyboard/pointer devices).
    pub fn controller_status(&self) -> (bool, usize) {
        (
            self.controller_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
            self.controller_device_count
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    pub async fn known_device_ids(&self) -> Vec<String> {
        self.peers
            .lock()
            .await
            .values()
            .filter_map(|p| p.device_id.clone())
            .collect()
    }
}

fn translate(conn_id: ConnId, event: SessionEvent) -> CoordinatorEvent {
    match event {
        SessionEvent::PeerIdentified { device_id } => {
            CoordinatorEvent::PeerIdentified { conn_id, device_id }
        }
        SessionEvent::PeerHello { hello, .. } => CoordinatorEvent::PeerHello { conn_id, hello },
        SessionEvent::PairingRequested {
            request, respond, ..
        } => CoordinatorEvent::PairingRequested {
            conn_id,
            request,
            respond,
        },
        SessionEvent::Trusted { device_id } => CoordinatorEvent::Trusted { conn_id, device_id },
        SessionEvent::PairingRejected { .. } => CoordinatorEvent::PairingRejected { conn_id },
        SessionEvent::Heartbeat { rtt_ms } => CoordinatorEvent::Heartbeat { conn_id, rtt_ms },
        SessionEvent::InputEvent(event) => CoordinatorEvent::InputEvent { conn_id, event },
        SessionEvent::ClipboardText { text } => CoordinatorEvent::ClipboardText { conn_id, text },
        SessionEvent::FileTransfer(progress) => CoordinatorEvent::FileTransfer { conn_id, progress },
        SessionEvent::ReleaseControl { reason } => {
            CoordinatorEvent::ReleaseControl { conn_id, reason }
        }
        SessionEvent::Closed { reason } => CoordinatorEvent::PeerDisconnected { conn_id, reason },
    }
}

fn reconnect_required(preferred: Option<&str>, connected: &std::collections::HashSet<String>, trusted: &std::collections::HashSet<String>) -> bool {
    if connected.is_empty() { return true; }
    if let Some(preferred)=preferred.filter(|id|trusted.contains(*id)) {
        return !connected.contains(preferred);
    }
    trusted.iter().any(|id|!connected.contains(id))
}

/// Ctrl+Shift+Escape — the global Emergency Stop toggle,
/// `ROADMAP.md`'s "hotkey toggle" fallback for edge-detection. Fixed
/// combo for Phase 1; a Settings-page rebind is future work.
fn is_emergency_stop_hotkey(event: &InputEventMessage) -> bool {
    use crate::protocol::payloads::input_event::{modifier_flags, InputEventKind};
    const MAC_ESCAPE_KEYCODE: u16 = 53;
    event.kind == InputEventKind::KeyDown
        && event.key_code == Some(MAC_ESCAPE_KEYCODE)
        && event.modifier_flags & modifier_flags::SHIFT != 0
        && event.modifier_flags & modifier_flags::CONTROL != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::payloads::input_event::InputEventKind;
    use std::time::Duration;

    #[test]
    fn a_connected_receiver_does_not_hide_a_missing_controller() {
        let trusted=std::collections::HashSet::from(["controller".into(),"receiver".into()]);
        let connected=std::collections::HashSet::from(["receiver".into()]);
        assert!(reconnect_required(Some("controller"),&connected,&trusted));
        assert!(reconnect_required(None,&connected,&trusted));
    }
    #[test]
    fn an_offline_receiver_or_revoked_controller_does_not_flap_a_live_connection() {
        let trusted=std::collections::HashSet::from(["controller".into(),"receiver".into()]);
        let connected=std::collections::HashSet::from(["controller".into()]);
        assert!(!reconnect_required(Some("controller"),&connected,&trusted));
        let trusted=std::collections::HashSet::from(["receiver".into()]);
        let connected=std::collections::HashSet::from(["receiver".into()]);
        assert!(!reconnect_required(Some("revoked"),&connected,&trusted));
    }

    fn hello(name: &str) -> HelloPayload {
        HelloPayload {
            device_name: name.to_string(),
            device_model: "Linux".to_string(),
            app_version: "0.1.0-test".to_string(),
            roles: vec!["controller".to_string(), "receiver".to_string()],
            platform: Some("Linux".to_string()),
            screen_width: Some(1920),
            screen_height: Some(1080),
        }
    }

    async fn empty_trust_store() -> Arc<Mutex<TrustStore>> {
        let dir = std::env::temp_dir().join(format!(
            "entanglo-coordinator-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::env::set_var("XDG_DATA_HOME", &dir);
        Arc::new(Mutex::new(
            TrustStore::load().await.expect("load fresh trust store"),
        ))
    }

    /// Auto-approves any pairing request seen on `rx` and forwards
    /// every other event onto `tx`, exactly like a GTK Pairing page
    /// would after the user clicks "Accept" — but immediately, so
    /// tests don't need a UI.
    fn auto_approve(
        mut rx: mpsc::UnboundedReceiver<CoordinatorEvent>,
        tx: mpsc::UnboundedSender<CoordinatorEvent>,
    ) {
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let CoordinatorEvent::PairingRequested { respond, .. } = event {
                    let _ = respond.send(true);
                } else {
                    let _ = tx.send(event);
                }
            }
        });
    }

    async fn wait_for_trusted(rx: &mut mpsc::UnboundedReceiver<CoordinatorEvent>) -> ConnId {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out waiting for Trusted")
                .expect("event stream closed")
            {
                CoordinatorEvent::Trusted { conn_id, .. } => return conn_id,
                _ => continue,
            }
        }
    }

    fn sample_move_event(x: f64, y: f64) -> InputEventMessage {
        InputEventMessage {
            kind: InputEventKind::MouseMove,
            x: Some(x),
            y: Some(y),
            delta_x: None,
            delta_y: None,
            button: None,
            key_code: None,
            media_key: None,
            modifier_flags: 0,
            pressed: None,
            click_state: None,
        }
    }

    fn sample_delta_event(delta_x: f64, delta_y: f64) -> InputEventMessage {
        InputEventMessage {
            kind: InputEventKind::MouseMove,
            x: None,
            y: None,
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
            button: None,
            key_code: None,
            media_key: None,
            modifier_flags: 0,
            pressed: None,
            click_state: None,
        }
    }

    #[test]
    fn virtual_cursor_returns_only_through_its_entry_edge() {
        let mut from_right = RemoteCursorState::new(7, RemoteEntryEdge::Right, 3840.0, 2160.0);
        assert!(!from_right.observe(&mut sample_delta_event(-5000.0, 0.0)));
        assert_eq!(from_right.x, 0.0, "opposite edge is a wall");
        assert!(!from_right.observe(&mut sample_delta_event(3839.0, 0.0)));
        assert!(from_right.observe(&mut sample_delta_event(1.0, 0.0)));

        let mut from_left = RemoteCursorState::new(8, RemoteEntryEdge::Left, 3840.0, 2160.0);
        assert!(!from_left.observe(&mut sample_delta_event(5000.0, 0.0)));
        assert_eq!(from_left.x, 3839.0);
        assert!(!from_left.observe(&mut sample_delta_event(-3839.0, 0.0)));
        assert!(from_left.observe(&mut sample_delta_event(-1.0, 0.0)));
    }

    #[test]
    fn virtual_cursor_does_not_return_mid_drag() {
        let mut cursor = RemoteCursorState::new(7, RemoteEntryEdge::Right, 2560.0, 1440.0);
        let mut button = sample_delta_event(0.0, 0.0);
        button.kind = InputEventKind::MouseDown;
        button.button = Some(0);
        assert!(!cursor.observe(&mut button));
        assert!(!cursor.observe(&mut sample_delta_event(10.0, 0.0)));

        button.kind = InputEventKind::MouseUp;
        assert!(!cursor.observe(&mut button));
        assert!(cursor.observe(&mut sample_delta_event(1.0, 0.0)));
    }

    #[test]
    fn edge_control_rewrites_relative_motion_to_remote_absolute_coordinates() {
        let mut cursor = RemoteCursorState::new(7, RemoteEntryEdge::Right, 3840.0, 2160.0);
        let mut event = sample_delta_event(-12.0, 5.0);

        assert!(!cursor.observe(&mut event));
        assert_eq!(event.x, Some(3827.0));
        assert_eq!(event.y, Some(1085.0));
        assert_eq!(event.delta_x, None);
        assert_eq!(event.delta_y, None);
    }

    #[test]
    fn legacy_imac_pro_hello_uses_the_logical_fallback() {
        let mut legacy = hello("iMac Pro (2)");
        legacy.platform = Some("macOS".into());
        legacy.device_model = "iMacPro1,1".into();
        legacy.app_version = "0.1.58".into();
        legacy.screen_width = None;
        legacy.screen_height = None;

        assert_eq!(peer_screen_size(&legacy), None);
    }

    #[test]
    fn advertised_size_is_used_for_a_current_peer() {
        let mut current = hello("iMac Pro (2)");
        current.platform = Some("macOS".into());
        current.device_model = "iMacPro1,1".into();
        current.screen_width = Some(3008);
        current.screen_height = Some(1692);

        assert_eq!(peer_screen_size(&current), Some((3008.0, 1692.0)));
    }

    #[test]
    fn control_entered_from_the_bottom_returns_only_through_the_bottom() {
        let mut cursor = RemoteCursorState::new(7, RemoteEntryEdge::Bottom, 1920.0, 1080.0);
        assert_eq!((cursor.x, cursor.y), (960.0, 1079.0));
        assert!(!cursor.observe(&mut sample_delta_event(0.0, -5000.0)));
        assert_eq!(cursor.y, 0.0, "the opposite edge is a wall");
        assert!(!cursor.observe(&mut sample_delta_event(0.0, 1079.0)));
        assert!(cursor.observe(&mut sample_delta_event(0.0, 1.0)));
    }

    #[test]
    fn control_entered_from_the_top_returns_through_the_top() {
        let mut cursor = RemoteCursorState::new(7, RemoteEntryEdge::Top, 1920.0, 1080.0);
        assert_eq!((cursor.x, cursor.y), (960.0, 0.0));
        assert!(!cursor.observe(&mut sample_delta_event(0.0, 10.0)));
        assert!(cursor.observe(&mut sample_delta_event(0.0, -11.0)));
    }

    #[test]
    fn linux_receivers_get_coordinates_on_their_fixed_canvas() {
        let mut cursor = RemoteCursorState::new(7, RemoteEntryEdge::Left, 1920.0, 1080.0)
            .with_linux_canvas(true);
        let mut event = sample_delta_event(1919.0, 539.5);

        assert!(!cursor.observe(&mut event));
        assert_eq!(event.x, Some(2559.0));
        assert_eq!(event.y, Some(1439.0));
    }

    #[tokio::test]
    async fn edge_control_uses_the_peer_advertised_screen_size() {
        let (coordinator, _events) =
            Coordinator::new("local".into(), hello("Local"), empty_trust_store().await);
        let (outgoing_tx, _outgoing_rx) = mpsc::unbounded_channel();
        coordinator.peers.lock().await.insert(
            42,
            PeerHandle {
                outgoing_tx,
                device_id: Some("wide-peer".into()),
                screen_size: Some((5120.0, 2880.0)),
                is_linux: false,
            },
        );

        coordinator
            .set_active_receiver_from_edge(42, RemoteEntryEdge::Right)
            .await;

        let cursor = coordinator.remote_cursor.lock().unwrap();
        let cursor = cursor.as_ref().expect("remote cursor");
        assert_eq!(cursor.x, 5119.0);
        assert_eq!(cursor.y, 1440.0);
        assert_eq!(cursor.screen_width, 5120.0);
        assert_eq!(cursor.screen_height, 2880.0);
    }

    #[tokio::test]
    async fn edge_exit_clears_the_active_receiver() {
        let (coordinator, _events) =
            Coordinator::new("local".into(), hello("Local"), empty_trust_store().await);
        coordinator
            .set_active_receiver_from_edge(42, RemoteEntryEdge::Right)
            .await;
        assert_eq!(coordinator.active_receiver().await, Some(42));

        coordinator
            .send_to_active_receiver(sample_delta_event(1.0, 0.0))
            .await;
        assert_eq!(coordinator.active_receiver().await, None);
    }

    /// Two full `Coordinator`s — one listening, one dialing — dialed,
    /// auto-paired over real loopback TCP, with `a`'s active receiver
    /// already set to `b`. Returns both coordinators and each side's
    /// relayed (non-pairing) event stream.
    async fn paired_coordinators() -> (
        Arc<Coordinator>,
        Arc<Coordinator>,
        mpsc::UnboundedReceiver<CoordinatorEvent>,
        mpsc::UnboundedReceiver<CoordinatorEvent>,
    ) {
        let (coordinator_a, events_a) =
            Coordinator::new("device-a".into(), hello("A"), empty_trust_store().await);
        let (coordinator_b, events_b) =
            Coordinator::new("device-b".into(), hello("B"), empty_trust_store().await);

        let port_a = coordinator_a
            .listen("127.0.0.1:0")
            .await
            .expect("bind coordinator A");

        let (relay_a_tx, mut relay_a_rx) = mpsc::unbounded_channel();
        auto_approve(events_a, relay_a_tx);
        let (relay_b_tx, mut relay_b_rx) = mpsc::unbounded_channel();
        auto_approve(events_b, relay_b_tx);

        coordinator_b
            .dial(format!("127.0.0.1:{port_a}").parse().unwrap())
            .await
            .expect("dial coordinator A");

        let conn_id_on_a = wait_for_trusted(&mut relay_a_rx).await;
        let _conn_id_on_b = wait_for_trusted(&mut relay_b_rx).await;
        coordinator_a.set_active_receiver(Some(conn_id_on_a)).await;

        (coordinator_a, coordinator_b, relay_a_rx, relay_b_rx)
    }

    /// This is the multi-peer management layer `session.rs`'s own
    /// test doesn't cover: listener/accept, dial, and the
    /// conn-id-keyed peer map, exercised end to end over real
    /// loopback TCP.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_coordinators_connect_and_forward_input() {
        let (coordinator_a, coordinator_b, _relay_a_rx, mut relay_b_rx) =
            paired_coordinators().await;

        let move_event = sample_move_event(42.0, 7.0);
        coordinator_a
            .send_to_active_receiver(move_event.clone())
            .await;

        loop {
            match tokio::time::timeout(Duration::from_secs(5), relay_b_rx.recv())
                .await
                .expect("timed out waiting for InputEvent")
                .expect("event stream closed")
            {
                CoordinatorEvent::InputEvent { event, .. } => {
                    assert_eq!(event.x, move_event.x);
                    assert_eq!(event.y, move_event.y);
                    break;
                }
                _ => continue,
            }
        }

        assert_eq!(coordinator_a.known_device_ids().await, vec!["device-b"]);
        assert_eq!(coordinator_b.known_device_ids().await, vec!["device-a"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trusted_coordinators_exchange_clipboard_text() {
        let (coordinator_a, _coordinator_b, _relay_a_rx, mut relay_b_rx) =
            paired_coordinators().await;

        coordinator_a.broadcast_clipboard("hello from Linux").await;

        loop {
            match tokio::time::timeout(Duration::from_secs(5), relay_b_rx.recv())
                .await
                .expect("timed out waiting for ClipboardText")
                .expect("event stream closed")
            {
                CoordinatorEvent::ClipboardText { text, .. } => {
                    assert_eq!(text, "hello from Linux");
                    break;
                }
                _ => continue,
            }
        }
    }

    /// Emergency Stop (`ROADMAP.md` Phase 1, "matches the Mac's
    /// triple-Escape + explicit button") must actually stop input
    /// from flowing, and `resume` must let it flow again — this is
    /// the one safety-critical UI action in the whole app, so it gets
    /// its own test rather than relying on `Coordinator::emergency_stop`
    /// only ever being checked by inspection.
    #[tokio::test(flavor = "multi_thread")]
    async fn emergency_stop_blocks_input_until_resumed() {
        let (coordinator_a, _coordinator_b, _relay_a_rx, mut relay_b_rx) =
            paired_coordinators().await;

        coordinator_a.emergency_stop();
        assert!(coordinator_a.is_emergency_stopped());
        coordinator_a
            .send_to_active_receiver(sample_move_event(1.0, 1.0))
            .await;
        // Drain whatever's already queued (e.g. a second `Trusted`
        // event — both sides pairing each other simultaneously, since
        // neither trusted the other yet, means each side's `Trusted`
        // can fire twice: once handling the peer's `pairRequest`,
        // once handling the peer's `pairResponse` to its own) without
        // treating that as "input leaked while stopped" — only an
        // actual `InputEvent` counts as a failure here.
        loop {
            match tokio::time::timeout(Duration::from_millis(200), relay_b_rx.recv()).await {
                Ok(Some(CoordinatorEvent::InputEvent { .. })) => {
                    panic!("an InputEvent was forwarded while emergency-stopped")
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }

        coordinator_a.resume();
        assert!(!coordinator_a.is_emergency_stopped());
        let resumed_event = sample_move_event(2.0, 2.0);
        coordinator_a
            .send_to_active_receiver(resumed_event.clone())
            .await;
        loop {
            match tokio::time::timeout(Duration::from_secs(5), relay_b_rx.recv())
                .await
                .expect("timed out waiting for InputEvent after resume")
                .expect("event stream closed")
            {
                CoordinatorEvent::InputEvent { event, .. } => {
                    assert_eq!(event.x, resumed_event.x);
                    break;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn hotkey_requires_exact_combo() {
        use crate::protocol::payloads::input_event::{modifier_flags, InputEventKind};
        let hotkey = |kind, key_code, flags| InputEventMessage {
            kind,
            x: None,
            y: None,
            delta_x: None,
            delta_y: None,
            button: None,
            key_code,
            media_key: None,
            modifier_flags: flags,
            pressed: Some(true),
            click_state: None,
        };
        let ctrl_shift = modifier_flags::CONTROL | modifier_flags::SHIFT;

        assert!(is_emergency_stop_hotkey(&hotkey(
            InputEventKind::KeyDown,
            Some(53),
            ctrl_shift
        )));
        // Plain Escape (no modifiers) must not trigger it — this is
        // the whole reason it's Ctrl+Shift+Escape and not bare Escape.
        assert!(!is_emergency_stop_hotkey(&hotkey(
            InputEventKind::KeyDown,
            Some(53),
            0
        )));
        // Only one of the two required modifiers.
        assert!(!is_emergency_stop_hotkey(&hotkey(
            InputEventKind::KeyDown,
            Some(53),
            modifier_flags::CONTROL
        )));
        // Right combo, wrong key.
        assert!(!is_emergency_stop_hotkey(&hotkey(
            InputEventKind::KeyDown,
            Some(0),
            ctrl_shift
        )));
        // Right combo, wrong kind (key-up shouldn't re-trigger it).
        assert!(!is_emergency_stop_hotkey(&hotkey(
            InputEventKind::KeyUp,
            Some(53),
            ctrl_shift
        )));
    }

    /// PROTOCOL.md §5.6: receiving `releaseControl` MUST make the
    /// controller stop forwarding input immediately. Simulates "b
    /// detected local input" via `send_release_control` directly
    /// (the evdev-triggered path is exercised live, not by this
    /// sandboxed test — see ROADMAP.md/docs/DEV.md) and checks the
    /// effect on a's `active_receiver`, not just that a message went
    /// out.
    #[tokio::test(flavor = "multi_thread")]
    async fn release_control_clears_active_receiver_on_controller_side() {
        let (coordinator_a, coordinator_b, mut relay_a_rx, _relay_b_rx) =
            paired_coordinators().await;

        let conn_id_on_a = coordinator_a
            .active_receiver()
            .await
            .expect("paired_coordinators leaves a's active_receiver set");

        // `Coordinator` allocates conn ids from 0 per instance, and
        // `paired_coordinators` gives b exactly one connection (its
        // one `dial` call) — so that connection is conn_id 0 on b's
        // side, with no public API needed to look it up.
        coordinator_b.send_release_control(0, "local_input").await;

        loop {
            match tokio::time::timeout(Duration::from_secs(5), relay_a_rx.recv())
                .await
                .expect("timed out waiting for ReleaseControl")
                .expect("event stream closed")
            {
                CoordinatorEvent::ReleaseControl { conn_id, .. } => {
                    assert_eq!(conn_id, conn_id_on_a);
                    break;
                }
                _ => continue,
            }
        }

        // Give the coordinator's own event-handling task a moment to
        // process the ReleaseControl it just relayed before checking
        // the side effect (active_receiver is cleared in the same
        // loop iteration that forwards the event, but from a
        // different task than this test).
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(coordinator_a.active_receiver().await, None);
    }
}
