//! TCP framing + send/recv of `EntangloMessage`s, plus the 1 Hz
//! heartbeat loop. See `PROTOCOL.md` §2 and §5.4.

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;

use crate::protocol::codec::{framed_transport, FramedTransport};
use crate::protocol::envelope::{EntangloMessage, EnvelopeError};

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// Two missed heartbeats (~2s of silence) marks the link unhealthy,
/// per `PROTOCOL.md` §5.4. The Mac reference additionally times out
/// at 3s server-side — see `entanglo-macos/docs/PROTOCOL.md`.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct NetworkTransport {
    framed: FramedTransport,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("connection closed")]
    Closed,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("envelope error: {0}")]
    Envelope(#[from] EnvelopeError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl NetworkTransport {
    pub fn new(socket: TcpStream) -> Self {
        // Input latency is sensitive to Nagle's algorithm batching
        // small frames — disable it, mirroring the low-latency intent
        // of `NWConnection`/`TcpClient` on the Mac/Windows sides.
        let _ = socket.set_nodelay(true);
        Self {
            framed: framed_transport(socket),
        }
    }

    pub async fn send(&mut self, msg: &EntangloMessage) -> Result<(), TransportError> {
        let bytes = serde_json::to_vec(msg)?;
        self.framed.send(Bytes::from(bytes)).await?;
        Ok(())
    }

    /// Reads and decodes the next envelope. Per `PROTOCOL.md` §8, a
    /// decode failure on a single frame (unknown `messageType`,
    /// malformed payload) should be logged and the frame skipped —
    /// NOT propagated as a connection-ending error — except for a
    /// `protocolVersion` mismatch, which the Mac reference treats as
    /// connection-ending. Callers should loop on `recv()` and match
    /// on `EnvelopeError` accordingly rather than bailing out on any
    /// `Err`.
    pub async fn recv(&mut self) -> Result<EntangloMessage, TransportError> {
        let frame = self.framed.next().await.ok_or(TransportError::Closed)??;
        Ok(EntangloMessage::decode(&frame)?)
    }
}
