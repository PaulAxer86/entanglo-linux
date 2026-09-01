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

use crate::input::{InputCaptureService, InputInjectionService};
use crate::protocol::payloads::{
    HelloPayload, InputEventMessage, PairRequestPayload, ReleaseControlPayload,
};

use super::session::{run_session, OutgoingMessage, SessionConfig, SessionEvent};
use super::transport::NetworkTransport;
use super::trust_store::TrustStore;

pub type ConnId = u64;

/// What the coordinator reports, one connection at a time. Same
/// shape as `SessionEvent` with a `conn_id` attached so the caller
/// can tell peers apart before any of them have a known device id.
#[derive(Debug)]
pub enum CoordinatorEvent {
    PeerConnected {
        conn_id: ConnId,
        direction: Direction,
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
}

pub struct Coordinator {
    local_device_id: String,
    local_hello: HelloPayload,
    trust_store: Arc<Mutex<TrustStore>>,
    peers: Mutex<HashMap<ConnId, PeerHandle>>,
    next_conn_id: AtomicU64,
    events_tx: mpsc::UnboundedSender<CoordinatorEvent>,
    /// The peer currently receiving this device's input, if this
    /// device is acting as controller. Real edge-detection is still
    /// ⬜ (`ROADMAP.md`); today the only setter is the Devices page's
    /// manual "Control this device" button. Cleared automatically on
    /// that peer's disconnect, or when it sends us `releaseControl`.
    active_receiver: Mutex<Option<ConnId>>,
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
            active_receiver: Mutex::new(None),
            injector: Mutex::new(None),
            receiver_enabled_flag: std::sync::atomic::AtomicBool::new(false),
            controller_enabled: std::sync::atomic::AtomicBool::new(false),
            controller_device_count: std::sync::atomic::AtomicUsize::new(0),
            emergency_stopped: std::sync::atomic::AtomicBool::new(false),
            being_controlled_by: Mutex::new(None),
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
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        let (session_events_tx, mut session_events_rx) = mpsc::unbounded_channel();

        self.peers.lock().await.insert(
            conn_id,
            PeerHandle {
                outgoing_tx,
                device_id: None,
            },
        );
        let _ = self
            .events_tx
            .send(CoordinatorEvent::PeerConnected { conn_id, direction });

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
                                *holder = Some(conn_id);
                                true
                            }
                            Some(existing) => existing == conn_id,
                        };
                        drop(holder);

                        if holds_control {
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
                if matches!(event, SessionEvent::ReleaseControl { .. }) {
                    // PROTOCOL.md §5.6: the controller MUST immediately
                    // stop forwarding inputEvents on releaseControl.
                    // Only clear if `conn_id` is actually who we were
                    // controlling — a stray releaseControl from
                    // someone else is not this peer's business.
                    let mut active = this.active_receiver.lock().await;
                    if *active == Some(conn_id) {
                        *active = None;
                    }
                }
                let _ = this.events_tx.send(translate(conn_id, event));
                if closed {
                    break;
                }
            }
            this.peers.lock().await.remove(&conn_id);
            let mut active = this.active_receiver.lock().await;
            if *active == Some(conn_id) {
                *active = None;
            }
            drop(active);
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
        *self.active_receiver.lock().await = conn_id;
    }

    pub async fn active_receiver(&self) -> Option<ConnId> {
        *self.active_receiver.lock().await
    }

    /// Forwards one input event to whichever peer `set_active_receiver`
    /// last selected, if any. No-op if nothing is currently targeted,
    /// or while `emergency_stop` is active.
    pub async fn send_to_active_receiver(&self, event: InputEventMessage) {
        if self
            .emergency_stopped
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if let Some(conn_id) = *self.active_receiver.lock().await {
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
        SessionEvent::ReleaseControl { reason } => {
            CoordinatorEvent::ReleaseControl { conn_id, reason }
        }
        SessionEvent::Closed { reason } => CoordinatorEvent::PeerDisconnected { conn_id, reason },
    }
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

    fn hello(name: &str) -> HelloPayload {
        HelloPayload {
            device_name: name.to_string(),
            device_model: "Linux".to_string(),
            app_version: "0.1.0-test".to_string(),
            roles: vec!["controller".to_string(), "receiver".to_string()],
            platform: Some("Linux".to_string()),
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
