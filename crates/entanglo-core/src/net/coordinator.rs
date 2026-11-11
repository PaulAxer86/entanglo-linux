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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::features::clipboard::ClipboardSyncService;
use crate::features::file_transfer::{outgoing_offer, FileProgress, TransferDirection, SEND_CHUNK_BYTES};
use crate::input::escape::{EscapeCombo, EscapeDetector, EscapeKey};
use crate::input::{InputCaptureService, InputInjectionService};
use crate::protocol::payloads::{
    FileAckPayload, FileChunkPayload, HelloPayload, InputEventMessage, PairRequestPayload,
    ReleaseControlPayload,
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

/// How long a sent file waits for its confirmation. Mac receivers can lose
/// theirs, so silence on a live connection does not mean the file failed.
const FILE_ACK_WAIT: std::time::Duration =
    std::time::Duration::from_millis(if cfg!(test) { 500 } else { 15_000 });
/// Pause between chunks, matching the Mac and Windows senders.
const FILE_CHUNK_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);
const FILE_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const CONNECTION_LOST: &str = "Connection closed before transfer completed";

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

fn file_ack_outcome(ack: Result<FileAckPayload, oneshot::error::RecvError>) -> anyhow::Result<()> {
    let ack = ack.map_err(|_| anyhow::anyhow!(CONNECTION_LOST))?;
    anyhow::ensure!(ack.ok, ack.failure_reason.unwrap_or_else(|| "The device rejected the file".into()));
    Ok(())
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
    PeerDiscoveryEndpointRemoved(super::discovery::DiscoveryEvent),
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
    FileAck {
        conn_id: ConnId,
        ack: FileAckPayload,
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
    local_addr: Option<SocketAddr>,
    expected_device_id: Option<String>,
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
    pointer_return: std::sync::Mutex<Option<(RemoteEntryEdge, f64)>>,
    remote_pointer_speed: AtomicU64,
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
    controller_capture_ready: std::sync::atomic::AtomicBool,
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
    discovery_connect: Mutex<()>,
    discovery_attempts: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    pending_file_acks: std::sync::Mutex<HashMap<String, oneshot::Sender<FileAckPayload>>>,
    /// When evdev capture last produced any event. The X11 desktop compares
    /// this with clicks and key presses it receives during a pointer grab to
    /// notice that capture has lost a device and must not keep the grab.
    last_local_input: std::sync::Mutex<LocalInputTimes>,
    /// When an escape combination last returned control. Both evdev and the
    /// X11 grab see the same key press; the second one must not toggle
    /// Emergency Stop after the first already released control.
    last_escape_release: std::sync::Mutex<Option<std::time::Instant>>,
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
            pointer_return: std::sync::Mutex::new(None),
            remote_pointer_speed: AtomicU64::new(saved_pointer_speed().to_bits()),
            injector: Mutex::new(None),
            receiver_enabled_flag: std::sync::atomic::AtomicBool::new(false),
            controller_enabled: std::sync::atomic::AtomicBool::new(false),
            controller_capture_ready: std::sync::atomic::AtomicBool::new(false),
            controller_device_count: std::sync::atomic::AtomicUsize::new(0),
            emergency_stopped: std::sync::atomic::AtomicBool::new(false),
            being_controlled_by: Mutex::new(None),
            preferred_controller: std::sync::Mutex::new(super::trust_store::preferred_controller()),
            discovery_connect: Mutex::new(()),
            discovery_attempts: Mutex::new(std::collections::HashMap::new()),
            pending_file_acks: std::sync::Mutex::new(HashMap::new()),
            last_local_input: std::sync::Mutex::new(LocalInputTimes::default()),
            last_escape_release: std::sync::Mutex::new(None),
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
    /// - **Escape combinations** (`input::escape`): Left Shift+Escape,
    ///   triple Escape and Ctrl+Shift+Escape return control to this
    ///   computer while a peer is being controlled. With no peer under
    ///   control, Ctrl+Shift+Escape toggles `emergency_stop`/`resume`.
    ///   They are consumed here rather than forwarded, and work even when
    ///   Entanglo's window doesn't have focus, since they're caught at the
    ///   evdev level. The X11 grab in the desktop crate runs the same
    ///   detector as a fallback.
    ///
    /// Input devices are rescanned every second. A Bluetooth mouse or
    /// keyboard that sleeps and reconnects gets a new `/dev/input/event*`
    /// node; without the rescan, X11 still moves the cursor to an edge
    /// and the pointer is grabbed, but nothing is forwarded and control
    /// can never come back.
    pub async fn enable_controller(self: &Arc<Self>) -> std::io::Result<()> {
        let devices = InputCaptureService::enumerate_devices()?;
        self.controller_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let capture = Arc::new(Mutex::new(InputCaptureService::new()));
        let escape = Arc::new(std::sync::Mutex::new(EscapeDetector::new()));
        let open = Arc::new(std::sync::Mutex::new(std::collections::HashSet::<PathBuf>::new()));
        for (path, device) in devices {
            self.capture_device(path, device, &capture, &escape, &open);
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(INPUT_DEVICE_RESCAN);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let devices = match tokio::task::spawn_blocking(InputCaptureService::enumerate_devices).await {
                    Ok(Ok(devices)) => devices,
                    Ok(Err(e)) => {
                        tracing::debug!(error = %e, "input device rescan failed");
                        continue;
                    }
                    Err(_) => continue,
                };
                for (path, device) in devices {
                    if open.lock().unwrap().contains(&path) {
                        continue;
                    }
                    tracing::info!(?path, name = ?device.name(), "input device connected; capturing it");
                    this.capture_device(path, device, &capture, &escape, &open);
                }
            }
        });
        Ok(())
    }

    fn capture_device(
        self: &Arc<Self>,
        path: PathBuf,
        device: evdev::Device,
        capture: &Arc<Mutex<InputCaptureService>>,
        escape: &Arc<std::sync::Mutex<EscapeDetector>>,
        open: &Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
    ) {
        let name = device.name().unwrap_or_default().to_owned();
        let mut stream = match device.into_event_stream() {
            Ok(stream) => stream,
            Err(e) => {
                tracing::debug!(?path, error = %e, "input device could not be streamed");
                return;
            }
        };
        if !open.lock().unwrap().insert(path.clone()) {
            return;
        }
        let device_count = open.lock().unwrap().len();
        self.controller_device_count.store(device_count, Ordering::Relaxed);
        tracing::debug!(?path, %name, "capturing input device");
        let capture = Arc::clone(capture);
        let escape = Arc::clone(escape);
        let open = Arc::clone(open);
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let event = match stream.next_event().await {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::info!(?path, %name, error = %e, "input device disconnected");
                        let mut open = open.lock().unwrap();
                        open.remove(&path);
                        this.controller_device_count.store(open.len(), Ordering::Relaxed);
                        return;
                    }
                };
                let translated = capture.lock().await.translate(event);
                let Some(input_event) = translated else {
                    continue;
                };
                this.last_local_input.lock().unwrap().observe(&input_event);

                if let Some(key_code) = input_event.key_code {
                    use crate::protocol::payloads::input_event::InputEventKind;
                    let pressed = match input_event.kind {
                        InputEventKind::KeyDown => Some(true),
                        InputEventKind::KeyUp => Some(false),
                        _ => None,
                    };
                    if let Some(pressed) = pressed {
                        let combo = escape.lock().unwrap().observe(
                            EscapeKey::from_mac_key_code(key_code), pressed, std::time::Instant::now());
                        if let Some(combo) = combo {
                            if this.handle_escape_combo(combo, "keyboard capture").await {
                                continue; // consumed, not forwarded
                            }
                        }
                    }
                }

                if let Some(controller_conn_id) = this.being_controlled_by.lock().await.take() {
                    this.send_release_control(controller_conn_id, "local_input")
                        .await;
                }

                if this.controller_capture_ready.load(Ordering::Relaxed) {
                    this.send_to_active_receiver(input_event).await;
                }
            }
        });
    }

    /// Returns true when the key press was used and must not be forwarded.
    pub async fn handle_escape_combo(&self, combo: EscapeCombo, source: &str) -> bool {
        if combo == EscapeCombo::ControlShiftEscape && self.is_emergency_stopped() {
            self.take_control_back(source);
            self.resume();
            tracing::info!(source, "emergency stop lifted by keyboard");
            return true;
        }
        if let Some(conn_id) = self.take_control_back(source) {
            tracing::info!(conn_id, ?combo, source, "escape keys returned control to this computer");
            self.release_peer_modifiers(conn_id).await;
            return true;
        }
        let recently_released = self.last_escape_release.lock().unwrap()
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1));
        if recently_released {
            return true;
        }
        if combo == EscapeCombo::ControlShiftEscape {
            self.emergency_stop();
            tracing::info!(source, "emergency stop engaged by keyboard");
            return true;
        }
        false
    }

    /// Ends the current remote-control session without waiting for the
    /// virtual cursor to cross back. Synchronous so the X11 poll loop
    /// can release its grab on the same tick.
    pub fn take_control_back(&self, reason: &str) -> Option<ConnId> {
        let conn_id = self.active_receiver.lock().unwrap().take()?;
        *self.remote_cursor.lock().unwrap() = None;
        *self.pointer_return.lock().unwrap() = None;
        *self.last_escape_release.lock().unwrap() = Some(std::time::Instant::now());
        self.controller_capture_ready.store(false, Ordering::Relaxed);
        tracing::info!(conn_id, reason, "control returned to this computer");
        Some(conn_id)
    }

    /// The modifiers of an escape combination were already forwarded to
    /// the peer; release them so they are not left held there.
    pub async fn release_peer_modifiers(&self, conn_id: ConnId) {
        use crate::protocol::payloads::input_event::InputEventKind;
        for key_code in [56, 60, 59, 62] {
            self.send_input(conn_id, InputEventMessage {
                kind: InputEventKind::KeyUp,
                x: None,
                y: None,
                delta_x: None,
                delta_y: None,
                button: None,
                key_code: Some(key_code),
                media_key: None,
                modifier_flags: 0,
                pressed: Some(false),
                click_state: None,
            }).await;
        }
    }

    /// Time since evdev capture last saw a key or pointer event, if ever.
    pub fn local_input_age(&self, kind: LocalInputKind) -> Option<std::time::Duration> {
        let times = self.last_local_input.lock().unwrap();
        match kind {
            LocalInputKind::Keyboard => times.keyboard,
            LocalInputKind::Pointer => times.pointer,
        }.map(|at| at.elapsed())
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
        self.spawn_identified_candidate(socket, direction, None).await;
    }

    async fn spawn_identified_candidate(self: Arc<Self>, socket: TcpStream, direction: Direction, expected_device_id: Option<String>) {
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
                local_addr: socket.local_addr().ok(),
                expected_device_id,
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
                if let SessionEvent::Trusted { ref device_id } = event {
                    let peers = this.peers.lock().await;
                    if let Some(current) = peers.get(&conn_id) {
                        let priority = super::path::address_priority(current.local_addr.map(|addr| addr.ip()));
                        let old_paths: Vec<_> = peers.iter().filter(|(id, peer)| **id != conn_id
                            && peer.device_id.as_ref() == Some(device_id)
                            && super::path::address_priority(peer.local_addr.map(|addr| addr.ip())) > priority)
                            .map(|(id, peer)| (*id, peer.outgoing_tx.clone())).collect();
                        drop(peers);
                        for (old_id, sender) in old_paths {
                            {
                                let mut active = this.active_receiver.lock().unwrap();
                                if *active == Some(old_id) {
                                    *active = None;
                                    *this.remote_cursor.lock().unwrap() = None;
                                }
                            }
                            let mut holder = this.being_controlled_by.lock().await;
                            if *holder == Some(old_id) { *holder = None; }
                            drop(holder);
                            let _ = sender.send(OutgoingMessage::ReleaseControl(ReleaseControlPayload {
                                reason: "A faster network path is available".into(),
                            }));
                        }
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
                if let SessionEvent::FileAck(ref ack) = event {
                    this.deliver_file_ack(ack.clone());
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
        let mut selected: HashMap<&str, (&PeerHandle, u8)> = HashMap::new();
        for handle in peers.values() {
            let Some(id) = handle.device_id.as_deref() else { continue; };
            let priority = super::path::address_priority(handle.local_addr.map(|addr| addr.ip()));
            if selected.get(id).is_none_or(|(_, current)| priority < *current) {
                selected.insert(id, (handle, priority));
            }
        }
        for (handle, _) in selected.values() {
            let _ = handle.outgoing_tx.send(OutgoingMessage::ClipboardText(payload.clone()));
        }
    }

    /// Sends `paths` to a trusted peer one after another, reporting progress
    /// through `CoordinatorEvent::FileTransfer`.
    pub fn send_files(self: &Arc<Self>, conn_id: ConnId, paths: Vec<PathBuf>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            for path in paths {
                this.send_file(conn_id, &path).await;
            }
        });
    }

    async fn send_file(&self, conn_id: ConnId, path: &Path) {
        let mut progress = FileProgress {
            transfer_id: uuid::Uuid::new_v4().to_string(),
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            received: 0,
            total: 0,
            destination: None,
            error: None,
            complete: false,
            updated_at: std::time::SystemTime::now(),
            direction: TransferDirection::Outgoing,
            unconfirmed: false,
        };
        self.report_outgoing(conn_id, &progress);
        match self.stream_file(conn_id, path, &mut progress).await {
            Ok(()) => {
                progress.received = progress.total;
                progress.complete = true;
                tracing::info!(name = %progress.name, bytes = progress.total, "file sent");
            }
            Err(error) => {
                tracing::warn!(name = %progress.name, %error, "file send failed");
                progress.error = Some(error.to_string());
            }
        }
        self.pending_file_acks.lock().unwrap().remove(&progress.transfer_id);
        progress.updated_at = std::time::SystemTime::now();
        self.report_outgoing(conn_id, &progress);
    }

    async fn stream_file(&self, conn_id: ConnId, path: &Path, progress: &mut FileProgress) -> anyhow::Result<()> {
        let mut file = tokio::fs::File::open(path).await?;
        let metadata = file.metadata().await?;
        anyhow::ensure!(metadata.is_file(), "Folders cannot be sent");
        progress.total = metadata.len();
        let offer = outgoing_offer(progress.transfer_id.clone(), progress.name.clone(), progress.total)?;
        let outgoing = self.peers.lock().await.get(&conn_id).map(|peer| peer.outgoing_tx.clone())
            .ok_or_else(|| anyhow::anyhow!(CONNECTION_LOST))?;

        let (ack_tx, mut ack_rx) = oneshot::channel();
        self.pending_file_acks.lock().unwrap().insert(progress.transfer_id.clone(), ack_tx);
        let total_chunks = offer.total_chunks;
        outgoing.send(OutgoingMessage::FileOffer(offer)).map_err(|_| anyhow::anyhow!(CONNECTION_LOST))?;
        tracing::info!(name = %progress.name, bytes = progress.total, "sending file");

        let mut last_report = std::time::Instant::now();
        for sequence in 0..total_chunks {
            let length = (progress.total - progress.received).min(u64::from(SEND_CHUNK_BYTES)) as usize;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await.map_err(|_| anyhow::anyhow!("The file changed while it was being sent"))?;
            let (written_tx, written_rx) = oneshot::channel();
            let payload = FileChunkPayload {
                transfer_id: progress.transfer_id.clone(),
                sequence,
                data: STANDARD.encode(&bytes),
            };
            outgoing.send(OutgoingMessage::FileChunk { payload, written: written_tx })
                .map_err(|_| anyhow::anyhow!(CONNECTION_LOST))?;
            tokio::select! {
                written = written_rx => written.map_err(|_| anyhow::anyhow!(CONNECTION_LOST))?,
                // A receiver that gives up answers before the last chunk.
                ack = &mut ack_rx => return file_ack_outcome(ack),
            }
            progress.received += length as u64;
            if last_report.elapsed() >= FILE_PROGRESS_INTERVAL {
                last_report = std::time::Instant::now();
                self.report_outgoing(conn_id, progress);
            }
            tokio::time::sleep(FILE_CHUNK_PAUSE).await;
        }
        match tokio::time::timeout(FILE_ACK_WAIT, ack_rx).await {
            Ok(ack) => file_ack_outcome(ack),
            Err(_) => {
                anyhow::ensure!(self.peers.lock().await.contains_key(&conn_id), CONNECTION_LOST);
                tracing::info!(name = %progress.name, "device did not confirm the file");
                progress.unconfirmed = true;
                Ok(())
            }
        }
    }

    fn report_outgoing(&self, conn_id: ConnId, progress: &FileProgress) {
        let _ = self.events_tx.send(CoordinatorEvent::FileTransfer { conn_id, progress: progress.clone() });
    }

    fn deliver_file_ack(&self, ack: FileAckPayload) {
        if let Some(waiting) = self.pending_file_acks.lock().unwrap().remove(&ack.transfer_id) {
            let _ = waiting.send(ack);
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
        *self.pointer_return.lock().unwrap() = None;
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
            event @ super::discovery::DiscoveryEvent::RemovedOnInterface { .. } => CoordinatorEvent::PeerDiscoveryEndpointRemoved(event),
        };
        let _ = self.events_tx.send(event);
    }

    pub async fn connect_discovered(self: &Arc<Self>, peer: &super::discovery::DiscoveredPeer) -> std::io::Result<()> {
        self.connect_candidate(peer, false).await
    }

    pub async fn connect_first_contact(self: &Arc<Self>, peer: &super::discovery::DiscoveredPeer) -> std::io::Result<()> {
        self.connect_candidate(peer, true).await
    }

    async fn connect_candidate(self: &Arc<Self>, peer: &super::discovery::DiscoveredPeer, allow_untrusted: bool) -> std::io::Result<()> {
        // Serialize discovery dialing so simultaneous IPv4/IPv6 announcements
        // cannot each open a socket before the first hello has arrived.
        let _connecting = self.discovery_connect.lock().await;
        let id = match peer.device_id.as_deref() {
            Some(id) => id,
            None if allow_untrusted => &peer.device_name,
            None => return Ok(()),
        };
        if id == self.local_device_id { return Ok(()); }
        let trusted = self.trust_store.lock().await.is_trusted(id);
        if !trusted && !allow_untrusted { return Ok(()); }
        let priority = peer.priority();
        {
            let peers = self.peers.lock().await;
            let existing = peers.values().filter(|handle| handle.device_id.as_deref()
                .or(handle.expected_device_id.as_deref()) == Some(id));
            if existing.into_iter().any(|handle| path_blocks_candidate(trusted, priority,
                super::path::address_priority(handle.local_addr.map(|addr| addr.ip())))) {
                return Ok(());
            }
        }
        let Some(endpoint) = peer.endpoint() else { return Ok(()); };
        let key = format!("{id}@{endpoint}");
        {
            let mut attempts = self.discovery_attempts.lock().await;
            if attempts.get(&key).is_some_and(|last| last.elapsed() < std::time::Duration::from_secs(5)) { return Ok(()); }
            attempts.insert(key, std::time::Instant::now());
        }
        let socket = if let Some(interface) = peer.interface.as_deref() {
            let source = super::path::source_address(interface, endpoint.ip())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "Bonjour interface has no matching local address"))?;
            let socket = if endpoint.is_ipv4() { tokio::net::TcpSocket::new_v4()? } else { tokio::net::TcpSocket::new_v6()? };
            let source = match (source, endpoint) {
                (std::net::IpAddr::V6(ip), SocketAddr::V6(target)) if ip.is_unicast_link_local() =>
                    SocketAddr::V6(std::net::SocketAddrV6::new(ip, 0, 0, target.scope_id())),
                (ip, _) => SocketAddr::new(ip, 0),
            };
            socket.bind(source)?;
            socket.connect(endpoint).await?
        } else { TcpStream::connect(endpoint).await? };
        tracing::info!(device_id = id, interface = ?peer.interface, %endpoint, "connecting discovered peer on selected interface");
        Arc::clone(self).spawn_identified_candidate(socket, Direction::Outbound, Some(id.to_owned())).await;
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

    /// The X11 desktop sets this only after pointer and keyboard capture
    /// have both succeeded. Emergency-stop detection remains active regardless.
    pub fn set_controller_capture_ready(&self, ready: bool) {
        self.controller_capture_ready.store(ready, Ordering::Relaxed);
    }

    pub fn active_receiver_sync(&self) -> Option<ConnId> {
        *self.active_receiver.lock().unwrap()
    }

    /// Forwards one input event to whichever peer `set_active_receiver`
    /// last selected, if any. No-op if nothing is currently targeted,
    /// or while `emergency_stop` is active.
    pub fn remote_pointer_speed(&self) -> f64 {
        f64::from_bits(self.remote_pointer_speed.load(Ordering::Relaxed))
    }

    pub fn set_remote_pointer_speed(&self, speed: f64) -> std::io::Result<()> {
        let speed = valid_pointer_speed(speed);
        super::trust_store::write_setting("remote_pointer_speed", &speed.to_string())?;
        self.remote_pointer_speed.store(speed.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    /// Consumed by the desktop thread before revealing the local pointer.
    pub fn take_pointer_return(&self) -> Option<(RemoteEntryEdge, f64)> {
        self.pointer_return.lock().unwrap().take()
    }

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
            scale_pointer_motion(&mut event, self.remote_pointer_speed());
            let returned_local = {
                let mut cursor = self.remote_cursor.lock().unwrap();
                if let Some(state) = cursor.as_mut().filter(|state| state.conn_id == conn_id) {
                    if state.observe(&mut event) {
                        let fraction = match state.entry_edge {
                            RemoteEntryEdge::Left | RemoteEntryEdge::Right => state.y / (state.screen_height - 1.0).max(1.0),
                            RemoteEntryEdge::Top | RemoteEntryEdge::Bottom => state.x / (state.screen_width - 1.0).max(1.0),
                        };
                        *self.pointer_return.lock().unwrap() = Some((state.entry_edge, fraction.clamp(0.0, 1.0)));
                        true
                    } else { false }
                } else { false }
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
        SessionEvent::FileAck(ack) => CoordinatorEvent::FileAck { conn_id, ack },
        SessionEvent::ReleaseControl { reason } => {
            CoordinatorEvent::ReleaseControl { conn_id, reason }
        }
        SessionEvent::Closed { reason } => CoordinatorEvent::PeerDisconnected { conn_id, reason },
    }
}

fn valid_pointer_speed(speed: f64) -> f64 {
    if speed.is_finite() { speed.clamp(0.25, 4.0) } else { 1.0 }
}

fn saved_pointer_speed() -> f64 {
    valid_pointer_speed(super::trust_store::read_setting("remote_pointer_speed")
        .and_then(|value| value.parse().ok()).unwrap_or(1.0))
}

fn scale_pointer_motion(event: &mut InputEventMessage, speed: f64) {
    if event.kind == crate::protocol::payloads::input_event::InputEventKind::MouseMove {
        event.delta_x = event.delta_x.map(|delta| delta * valid_pointer_speed(speed));
        event.delta_y = event.delta_y.map(|delta| delta * valid_pointer_speed(speed));
    }
}

/// Keep an established path unless a trusted device has a better one.
fn path_blocks_candidate(trusted: bool, candidate: u8, existing: u8) -> bool {
    !trusted || existing <= candidate
}

fn reconnect_required(preferred: Option<&str>, connected: &std::collections::HashSet<String>, trusted: &std::collections::HashSet<String>) -> bool {
    if connected.is_empty() { return true; }
    if let Some(preferred)=preferred.filter(|id|trusted.contains(*id)) {
        return !connected.contains(preferred);
    }
    trusted.iter().any(|id|!connected.contains(id))
}

const INPUT_DEVICE_RESCAN: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInputKind {
    Keyboard,
    Pointer,
}

#[derive(Debug, Default)]
struct LocalInputTimes {
    keyboard: Option<std::time::Instant>,
    pointer: Option<std::time::Instant>,
}

impl LocalInputTimes {
    fn observe(&mut self, event: &InputEventMessage) {
        use crate::protocol::payloads::input_event::InputEventKind;
        let now = Some(std::time::Instant::now());
        match event.kind {
            InputEventKind::KeyDown | InputEventKind::KeyUp | InputEventKind::MediaKey => self.keyboard = now,
            InputEventKind::MouseMove | InputEventKind::MouseDown | InputEventKind::MouseUp
                | InputEventKind::Scroll => self.pointer = now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::payloads::input_event::InputEventKind;
    use std::time::Duration;

    #[test]
    fn pointer_speed_changes_only_remote_motion_and_rejects_invalid_settings() {
        let mut event = sample_delta_event(3.0, -2.0);
        scale_pointer_motion(&mut event, 2.0);
        assert_eq!((event.delta_x, event.delta_y), (Some(6.0), Some(-4.0)));
        let mut scroll = sample_delta_event(1.0, 10.0);
        scroll.kind = InputEventKind::Scroll;
        scale_pointer_motion(&mut scroll, 2.0);
        assert_eq!(scroll.delta_y, Some(10.0), "pointer speed must not accelerate scrolling");
        assert_eq!(valid_pointer_speed(f64::NAN), 1.0);
        assert_eq!(valid_pointer_speed(100.0), 4.0);
        assert_eq!(valid_pointer_speed(0.0), 0.25);
    }

    #[test]
    fn wired_upgrade_preserves_fallback_and_never_downgrades() {
        assert!(!path_blocks_candidate(true, 0, 1), "trusted Wi-Fi peer can gain an Ethernet path");
        assert!(path_blocks_candidate(true, 1, 0), "later Wi-Fi announcements must not displace Ethernet");
        assert!(path_blocks_candidate(true, 0, 0), "IPv4/IPv6 announcements must not duplicate a wired session");
        assert!(path_blocks_candidate(false, 0, 1), "discovery must not bypass trust during an upgrade");
        assert!(!path_blocks_candidate(true, 1, 2), "a known path can replace an unclassified path");
    }

    #[tokio::test]
    async fn discovery_dials_the_announced_interface_without_duplicate_sockets() {
        let (coordinator, _events) = Coordinator::new("local-discovery-test".into(), hello("Local"),
            Arc::new(Mutex::new(TrustStore::empty_for_test())));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = super::super::discovery::DiscoveredPeer {
            device_name: "Peer._entanglo._tcp.local.".into(), host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(), addresses: vec!["127.0.0.1".parse().unwrap()],
            device_id: Some("remote-discovery-test".into()), interface: Some("lo".into()),
        };
        coordinator.connect_first_contact(&peer).await.unwrap();
        let (_socket, address) = listener.accept().await.unwrap();
        assert_eq!(address.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
        coordinator.connect_first_contact(&peer).await.unwrap();
        assert_eq!(coordinator.peers.lock().await.len(), 1, "pending hello must also suppress duplicate dialing");
        assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err());
    }

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

    async fn coordinator_with_fake_peer() -> (
        Arc<Coordinator>,
        mpsc::UnboundedReceiver<CoordinatorEvent>,
        mpsc::UnboundedReceiver<OutgoingMessage>,
    ) {
        let (coordinator, events) =
            Coordinator::new("local".into(), hello("Local"), empty_trust_store().await);
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        coordinator.peers.lock().await.insert(
            42,
            PeerHandle { outgoing_tx, device_id: Some("peer".into()), screen_size: None, is_linux: false, local_addr: None, expected_device_id: None },
        );
        (coordinator, events, outgoing_rx)
    }

    fn temporary_directory() -> PathBuf {
        let directory = std::env::temp_dir().join(format!("entanglo-send-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    async fn final_file_progress(events: &mut mpsc::UnboundedReceiver<CoordinatorEvent>) -> FileProgress {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await.expect("no final progress").expect("events closed");
            if let CoordinatorEvent::FileTransfer { progress, .. } = event {
                if progress.complete || progress.error.is_some() {
                    return progress;
                }
            }
        }
    }

    #[tokio::test]
    async fn sent_file_arrives_intact_and_completes_on_ack() {
        use crate::features::file_transfer::FileTransferService;

        let (coordinator, mut events, mut outgoing) = coordinator_with_fake_peer().await;
        let directory = temporary_directory();
        let source = directory.join("source.bin");
        let bytes: Vec<u8> = (0..1_200_000u32).map(|value| value as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let received = directory.join("received");
        let mut receiver = FileTransferService::in_directory(received.clone());

        coordinator.send_files(42, vec![source]);
        let destination = loop {
            match outgoing.recv().await.expect("session closed") {
                OutgoingMessage::FileOffer(offer) => {
                    assert_eq!(offer.total_chunks, 3);
                    receiver.begin_incoming(offer).unwrap();
                }
                OutgoingMessage::FileChunk { payload, written } => {
                    let progress = receiver.apply_chunk(&payload).unwrap();
                    written.send(()).unwrap();
                    if progress.complete {
                        coordinator.deliver_file_ack(FileAckPayload {
                            transfer_id: payload.transfer_id, ok: true, failure_reason: None,
                        });
                        break progress.destination.unwrap();
                    }
                }
                other => panic!("unexpected message {other:?}"),
            }
        };

        assert_eq!(std::fs::read(destination).unwrap(), bytes);
        let progress = final_file_progress(&mut events).await;
        assert!(progress.complete && progress.error.is_none());
        assert_eq!(progress.direction, TransferDirection::Outgoing);
        assert_eq!(progress.received, 1_200_000);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Feeds every chunk to the socket stand-in without ever acknowledging.
    fn accept_chunks_without_ack(mut outgoing: mpsc::UnboundedReceiver<OutgoingMessage>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if let OutgoingMessage::FileChunk { written, .. } = message {
                    let _ = written.send(());
                }
            }
        })
    }

    #[tokio::test]
    async fn silence_on_a_live_connection_counts_as_sent_but_unconfirmed() {
        let (coordinator, mut events, outgoing) = coordinator_with_fake_peer().await;
        let directory = temporary_directory();
        let source = directory.join("source.bin");
        std::fs::write(&source, vec![1; 1_200_000]).unwrap();
        let session = accept_chunks_without_ack(outgoing);

        coordinator.send_files(42, vec![source]);
        let progress = final_file_progress(&mut events).await;

        assert!(progress.complete && progress.unconfirmed);
        assert_eq!(progress.error, None);
        session.abort();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn silence_after_the_device_left_is_a_failure() {
        let (coordinator, mut events, outgoing) = coordinator_with_fake_peer().await;
        let directory = temporary_directory();
        let source = directory.join("source.bin");
        std::fs::write(&source, vec![1; 1_200_000]).unwrap();
        let session = accept_chunks_without_ack(outgoing);

        coordinator.send_files(42, vec![source]);
        // Every chunk is out well before the confirmation wait ends.
        tokio::time::sleep(FILE_ACK_WAIT / 2).await;
        coordinator.peers.lock().await.remove(&42);
        let progress = final_file_progress(&mut events).await;

        assert_eq!(progress.error.as_deref(), Some(CONNECTION_LOST));
        session.abort();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn receiver_rejection_fails_the_transfer_with_its_reason() {
        let (coordinator, mut events, mut outgoing) = coordinator_with_fake_peer().await;
        let directory = temporary_directory();
        let source = directory.join("source.bin");
        std::fs::write(&source, vec![1; 1_200_000]).unwrap();

        coordinator.send_files(42, vec![source]);
        match outgoing.recv().await.unwrap() {
            OutgoingMessage::FileOffer(offer) => coordinator.deliver_file_ack(FileAckPayload {
                transfer_id: offer.transfer_id, ok: false, failure_reason: Some("Disk full".into()),
            }),
            other => panic!("unexpected message {other:?}"),
        }

        assert_eq!(final_file_progress(&mut events).await.error.as_deref(), Some("Disk full"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn folders_and_missing_files_fail_without_reaching_the_peer() {
        let (coordinator, mut events, mut outgoing) = coordinator_with_fake_peer().await;
        let directory = temporary_directory();

        coordinator.send_files(42, vec![directory.clone()]);
        assert!(final_file_progress(&mut events).await.error.is_some());
        coordinator.send_files(42, vec![directory.join("missing")]);
        assert!(final_file_progress(&mut events).await.error.is_some());
        assert!(outgoing.try_recv().is_err());
        std::fs::remove_dir_all(directory).unwrap();
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
                local_addr: None,
                expected_device_id: None,
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
        let (edge, fraction) = coordinator.take_pointer_return().expect("local cursor return");
        assert_eq!(edge, RemoteEntryEdge::Right);
        assert!((0.49..0.51).contains(&fraction));
        assert!(coordinator.take_pointer_return().is_none(), "cursor return is consumed once");
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
    async fn file_travels_between_coordinators_over_tcp() {
        let directory = temporary_directory();
        let old_download = std::env::var_os("XDG_DOWNLOAD_DIR");
        std::env::set_var("XDG_DOWNLOAD_DIR", directory.join("downloads"));
        let (coordinator_a, _coordinator_b, mut relay_a_rx, mut relay_b_rx) =
            paired_coordinators().await;
        if let Some(old) = old_download {
            std::env::set_var("XDG_DOWNLOAD_DIR", old);
        } else {
            std::env::remove_var("XDG_DOWNLOAD_DIR");
        }

        let source = directory.join("report.bin");
        let bytes: Vec<u8> = (0..1_700_000u32).map(|value| (value % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let conn_id = coordinator_a.active_receiver().await.expect("peer selected");
        coordinator_a.send_files(conn_id, vec![source]);

        let sent = final_file_progress(&mut relay_a_rx).await;
        assert_eq!(sent.error, None);
        assert!(sent.complete);
        let received = final_file_progress(&mut relay_b_rx).await;
        assert_eq!(received.direction, TransferDirection::Incoming);
        assert_eq!(std::fs::read(received.destination.unwrap()).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn escape_keys_return_control_before_toggling_emergency_stop() {
        let (coordinator_a, _coordinator_b, _relay_a_rx, _relay_b_rx) =
            paired_coordinators().await;
        let conn_id = coordinator_a.active_receiver().await.expect("a controls b");

        assert!(coordinator_a.handle_escape_combo(EscapeCombo::LeftShiftEscape, "test").await);
        assert_eq!(coordinator_a.active_receiver().await, None);
        assert!(!coordinator_a.is_emergency_stopped());

        // The X11 grab reports the same press after evdev already released
        // control: it is swallowed instead of engaging Emergency Stop.
        assert!(coordinator_a.handle_escape_combo(EscapeCombo::ControlShiftEscape, "x11").await);
        assert!(!coordinator_a.is_emergency_stopped());

        *coordinator_a.last_escape_release.lock().unwrap() = None;
        assert!(!coordinator_a.handle_escape_combo(EscapeCombo::TripleEscape, "test").await,
            "with nothing under control, Escape keys stay ordinary keys");
        assert!(coordinator_a.handle_escape_combo(EscapeCombo::ControlShiftEscape, "test").await);
        assert!(coordinator_a.is_emergency_stopped());

        coordinator_a.set_active_receiver(Some(conn_id)).await;
        assert!(coordinator_a.handle_escape_combo(EscapeCombo::ControlShiftEscape, "test").await);
        assert!(!coordinator_a.is_emergency_stopped(), "Ctrl+Shift+Escape lifts Emergency Stop");
        assert_eq!(coordinator_a.active_receiver().await, None, "and does not re-grab the old peer");
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
