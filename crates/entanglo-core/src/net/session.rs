//! The real connection lifecycle: hello exchange, pairing, 1 Hz
//! heartbeat with RTT, and `inputEvent`/`releaseControl` forwarding.
//! Implements `PROTOCOL.md` §6 end to end for one peer connection —
//! this is the Phase 1 core everything else (UI, coordinator,
//! injection) plugs into.
//!
//! One `run_session` call owns one `NetworkTransport` for its whole
//! life and drives it via `tokio::select!`; the caller observes
//! progress through `events_tx` and pushes outgoing `inputEvent`s
//! through `outgoing_input_tx`. A device may run this twice for the
//! same peer (one inbound, one outbound) per `PROTOCOL.md` §6 —
//! reconciling that into a single logical link is
//! `coordinator.rs`'s job, not this module's.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::protocol::envelope::unix_timestamp_now;
use crate::protocol::message_type::MessageType;
use crate::protocol::payloads::{
    HeartbeatPayload, HelloPayload, InputEventMessage, PairRequestPayload, PairResponsePayload,
    ReleaseControlPayload,
};
use crate::protocol::EntangloMessage;

use super::transport::{NetworkTransport, TransportError, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT};
use super::trust_store::{TrustStore, TrustedDevice};

pub struct SessionConfig {
    pub local_device_id: String,
    pub local_session_id: String,
    pub local_hello: HelloPayload,
}

/// What a running session reports to its caller. The caller (UI or a
/// test harness) drives the Devices/Pairing pages and the input
/// injection safety gates off this stream — `session.rs` itself has
/// no UI and no `/dev/uinput` dependency.
#[derive(Debug)]
pub enum SessionEvent {
    /// The peer's `hello` arrived — carries its self-reported name,
    /// roles, and platform for display before trust is established.
    PeerHello {
        device_id: String,
        hello: HelloPayload,
    },
    /// An incoming `pairRequest` needs a user decision. Send the
    /// answer on `respond`; dropping it without a send is treated as
    /// a rejection.
    PairingRequested {
        device_id: String,
        request: PairRequestPayload,
        respond: oneshot::Sender<bool>,
    },
    /// This peer is now trusted (either it already was, or a
    /// `pairRequest`/`pairResponse` round trip just accepted it).
    Trusted {
        device_id: String,
    },
    PairingRejected {
        device_id: String,
    },
    Heartbeat {
        rtt_ms: Option<f64>,
    },
    /// Only ever emitted once `Trusted` has fired for this peer —
    /// see the `trusted` gate in the receive loop below.
    InputEvent(InputEventMessage),
    ReleaseControl {
        reason: String,
    },
    Closed {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("envelope error: {0}")]
    Envelope(#[from] crate::protocol::envelope::EnvelopeError),
}

/// Drives one peer connection to completion. Returns once the
/// connection closes (cleanly or via heartbeat timeout) — the caller
/// should treat any `Err` the same as a `SessionEvent::Closed`
/// (log and clean up), since best-effort event delivery means a
/// closing race can produce either.
pub async fn run_session(
    mut transport: NetworkTransport,
    config: SessionConfig,
    trust_store: Arc<Mutex<TrustStore>>,
    events_tx: mpsc::UnboundedSender<SessionEvent>,
    mut outgoing_input_rx: mpsc::UnboundedReceiver<InputEventMessage>,
) -> Result<(), SessionError> {
    let start = Instant::now();
    let now_ms = || start.elapsed().as_secs_f64() * 1000.0;

    send(
        &mut transport,
        &config,
        MessageType::Hello,
        &config.local_hello,
    )
    .await?;

    let mut peer_device_id: Option<String> = None;
    let mut peer_hello: Option<HelloPayload> = None;
    let mut trusted = false;
    let mut heartbeat_seq: u64 = 0;
    let mut last_seen_peer_sent_at_ms: Option<f64> = None;
    let mut last_traffic = Instant::now();
    let mut heartbeat_timer = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat_timer.tick().await; // first tick fires immediately; skip it

    loop {
        tokio::select! {
            frame = transport.recv() => {
                let msg = match frame {
                    Ok(m) => m,
                    Err(TransportError::Closed) => {
                        let _ = events_tx.send(SessionEvent::Closed { reason: "connection closed".into() });
                        return Ok(());
                    }
                    Err(e) => {
                        // Per PROTOCOL.md §8, a single bad frame (unknown
                        // messageType, malformed payload) must not end the
                        // connection — log and keep looping. `recv()`
                        // already special-cases the one connection-ending
                        // case (protocol version mismatch) by surfacing it
                        // as `EnvelopeError::UnsupportedVersion` here, which
                        // this arm still just logs; a future version-aware
                        // caller can pattern-match on `e` to close instead.
                        tracing::warn!(error = %e, "dropping unreadable frame");
                        continue;
                    }
                };
                last_traffic = Instant::now();

                if peer_device_id.is_none() {
                    let id = msg.sender_device_id.clone();
                    trusted = trust_store.lock().await.is_trusted(&id);
                    peer_device_id = Some(id.clone());
                    if trusted {
                        let _ = events_tx.send(SessionEvent::Trusted { device_id: id });
                    } else {
                        let request = PairRequestPayload {
                            requester_device_id: config.local_device_id.clone(),
                            requester_device_name: config.local_hello.device_name.clone(),
                            pin_hash: String::new(),
                        };
                        send(&mut transport, &config, MessageType::PairRequest, &request).await?;
                    }
                }
                let peer_id = peer_device_id.clone().expect("set immediately above");

                match msg.message_type {
                    MessageType::Hello => {
                        if let Ok(hello) = msg.decode_payload::<HelloPayload>() {
                            peer_hello = Some(hello.clone());
                            let _ = events_tx.send(SessionEvent::PeerHello { device_id: peer_id, hello });
                        }
                    }
                    MessageType::PairRequest => {
                        if let Ok(request) = msg.decode_payload::<PairRequestPayload>() {
                            let friendly_name = request.requester_device_name.clone();
                            let (respond_tx, respond_rx) = oneshot::channel();
                            let _ = events_tx.send(SessionEvent::PairingRequested {
                                device_id: peer_id.clone(),
                                request,
                                respond: respond_tx,
                            });
                            let accepted = respond_rx.await.unwrap_or(false);
                            if accepted {
                                let mut store = trust_store.lock().await;
                                store.trust(TrustedDevice {
                                    device_id: peer_id.clone(),
                                    friendly_name,
                                    trusted_since_unix: unix_timestamp_now(),
                                });
                                if let Err(e) = store.save().await {
                                    tracing::warn!(error = %e, "failed to persist trust store");
                                }
                                trusted = true;
                            }
                            let response = PairResponsePayload {
                                accepted,
                                trusted_device_id: accepted.then(|| peer_id.clone()),
                                rejection_reason: (!accepted).then(|| "rejected by user".to_string()),
                            };
                            send(&mut transport, &config, MessageType::PairResponse, &response).await?;
                            let _ = events_tx.send(if accepted {
                                SessionEvent::Trusted { device_id: peer_id }
                            } else {
                                SessionEvent::PairingRejected { device_id: peer_id }
                            });
                        }
                    }
                    MessageType::PairResponse => {
                        if let Ok(response) = msg.decode_payload::<PairResponsePayload>() {
                            if response.accepted {
                                let friendly_name = peer_hello
                                    .as_ref()
                                    .map(|h| h.device_name.clone())
                                    .unwrap_or_else(|| peer_id.clone());
                                let mut store = trust_store.lock().await;
                                store.trust(TrustedDevice {
                                    device_id: peer_id.clone(),
                                    friendly_name,
                                    trusted_since_unix: unix_timestamp_now(),
                                });
                                if let Err(e) = store.save().await {
                                    tracing::warn!(error = %e, "failed to persist trust store");
                                }
                                trusted = true;
                                let _ = events_tx.send(SessionEvent::Trusted { device_id: peer_id });
                            } else {
                                let _ = events_tx.send(SessionEvent::PairingRejected { device_id: peer_id });
                            }
                        }
                    }
                    MessageType::Heartbeat => {
                        if let Ok(hb) = msg.decode_payload::<HeartbeatPayload>() {
                            last_seen_peer_sent_at_ms = hb.sent_at_ms;
                            let rtt_ms = hb.echo_sent_at_ms.map(|echo| now_ms() - echo);
                            let _ = events_tx.send(SessionEvent::Heartbeat { rtt_ms });
                        }
                    }
                    MessageType::InputEvent => {
                        // Safety gate: only a trusted peer's input is ever
                        // surfaced. Callers apply further gates (receiver
                        // role enabled, emergency stop) before injecting.
                        if trusted {
                            if let Ok(event) = msg.decode_payload::<InputEventMessage>() {
                                let _ = events_tx.send(SessionEvent::InputEvent(event));
                            }
                        }
                    }
                    MessageType::ReleaseControl => {
                        if let Ok(rc) = msg.decode_payload::<ReleaseControlPayload>() {
                            let _ = events_tx.send(SessionEvent::ReleaseControl { reason: rc.reason });
                        }
                    }
                    other => {
                        // Clipboard/file/URL/screenshot/error — Phase 2+,
                        // see ROADMAP.md. Ignored, not connection-ending,
                        // per the forward-compat rule in PROTOCOL.md §8.
                        tracing::debug!(?other, "message type deferred to a later phase");
                    }
                }
            }

            _ = heartbeat_timer.tick() => {
                if peer_device_id.is_some() && last_traffic.elapsed() > HEARTBEAT_TIMEOUT {
                    let _ = events_tx.send(SessionEvent::Closed { reason: "heartbeat timeout".into() });
                    return Ok(());
                }
                let heartbeat = HeartbeatPayload {
                    sequence: heartbeat_seq,
                    last_rtt_ms: None,
                    sent_at_ms: Some(now_ms()),
                    echo_sent_at_ms: last_seen_peer_sent_at_ms,
                };
                heartbeat_seq += 1;
                send(&mut transport, &config, MessageType::Heartbeat, &heartbeat).await?;
            }

            Some(event) = outgoing_input_rx.recv() => {
                if trusted {
                    send(&mut transport, &config, MessageType::InputEvent, &event).await?;
                }
            }
        }
    }
}

async fn send<T: serde::Serialize>(
    transport: &mut NetworkTransport,
    config: &SessionConfig,
    message_type: MessageType,
    payload: &T,
) -> Result<(), SessionError> {
    let envelope = EntangloMessage::encode_payload(
        message_type,
        &config.local_device_id,
        &config.local_session_id,
        payload,
    )?;
    transport.send(&envelope).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::{TcpListener, TcpStream};

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
        // A fresh in-memory-only store: no Secret Service, no file —
        // point XDG_DATA_HOME at a throwaway dir so the file fallback
        // (which `TrustStore::load()` reaches immediately in this
        // sandboxed test environment, since no Secret Service is
        // running) never touches the real user data directory.
        let dir =
            std::env::temp_dir().join(format!("entanglo-session-test-{}", uuid::Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        Arc::new(Mutex::new(
            TrustStore::load().await.expect("load fresh trust store"),
        ))
    }

    async fn wait_for_trusted(rx: &mut mpsc::UnboundedReceiver<SessionEvent>) {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out waiting for Trusted")
                .expect("event stream closed")
            {
                SessionEvent::Trusted { .. } => return,
                _ => continue,
            }
        }
    }

    async fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client = TcpStream::connect(addr).await.unwrap();
        let server = accept.await.unwrap();
        (client, server)
    }

    /// Full round trip: two untrusted peers connect, pairing is
    /// auto-approved on both sides, and — the actual thing this test
    /// is for — an InputEventMessage sent by one side after trust is
    /// established arrives at the other's event stream. This is the
    /// backbone every later Phase 1 piece (safety gates, injection,
    /// UI) builds on, so it's worth covering end to end rather than
    /// unit-by-unit.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_peers_pair_and_forward_input() {
        let (client_sock, server_sock) = loopback_pair().await;

        let client_store = empty_trust_store().await;
        let server_store = empty_trust_store().await;

        let (client_events_tx, client_events_rx) = mpsc::unbounded_channel();
        let (server_events_tx, server_events_rx) = mpsc::unbounded_channel();
        let (client_out_tx, client_out_rx) = mpsc::unbounded_channel();
        let (_server_out_tx, server_out_rx) = mpsc::unbounded_channel();

        let client_config = SessionConfig {
            local_device_id: "client-device".to_string(),
            local_session_id: uuid::Uuid::new_v4().to_string(),
            local_hello: hello("Client"),
        };
        let server_config = SessionConfig {
            local_device_id: "server-device".to_string(),
            local_session_id: uuid::Uuid::new_v4().to_string(),
            local_hello: hello("Server"),
        };

        tokio::spawn(run_session(
            NetworkTransport::new(client_sock),
            client_config,
            client_store,
            client_events_tx,
            client_out_rx,
        ));
        tokio::spawn(run_session(
            NetworkTransport::new(server_sock),
            server_config,
            server_store,
            server_events_tx,
            server_out_rx,
        ));

        // Auto-approve any pairing request on both sides, exactly
        // like a UI callback would after the user clicks "Accept".
        let approve = |mut rx: mpsc::UnboundedReceiver<SessionEvent>,
                       tx: mpsc::UnboundedSender<SessionEvent>| {
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    if let SessionEvent::PairingRequested { respond, .. } = event {
                        let _ = respond.send(true);
                    } else {
                        let _ = tx.send(event);
                    }
                }
            })
        };
        let (relay_tx, mut relay_rx) = mpsc::unbounded_channel();
        approve(client_events_rx, relay_tx.clone());
        let (server_relay_tx, mut server_relay_rx) = mpsc::unbounded_channel();
        approve(server_events_rx, server_relay_tx);

        // Wait for both sides to report Trusted.
        wait_for_trusted(&mut relay_rx).await;
        wait_for_trusted(&mut server_relay_rx).await;

        // Now that the client believes the link is trusted, send an
        // InputEvent and confirm the server side observes it.
        let move_event = InputEventMessage {
            kind: crate::protocol::payloads::input_event::InputEventKind::MouseMove,
            x: Some(10.0),
            y: Some(20.0),
            delta_x: None,
            delta_y: None,
            button: None,
            key_code: None,
            media_key: None,
            modifier_flags: 0,
            pressed: None,
            click_state: None,
        };
        client_out_tx.send(move_event.clone()).unwrap();

        loop {
            match tokio::time::timeout(Duration::from_secs(5), server_relay_rx.recv())
                .await
                .expect("timed out waiting for InputEvent")
                .expect("event stream closed")
            {
                SessionEvent::InputEvent(received) => {
                    assert_eq!(received.x, move_event.x);
                    assert_eq!(received.y, move_event.y);
                    break;
                }
                _ => continue,
            }
        }
    }
}
