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

    /// Kernel TCP smoothed RTT, excluding the peer's heartbeat scheduling delay.
    pub fn kernel_rtt_ms(&self) -> Option<f64> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let mut info: libc::tcp_info = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<libc::tcp_info>() as libc::socklen_t;
            // TCP_INFO writes at most `size` bytes into this initialized buffer.
            let result = unsafe {
                libc::getsockopt(self.framed.get_ref().as_raw_fd(), libc::IPPROTO_TCP,
                    libc::TCP_INFO, (&mut info as *mut libc::tcp_info).cast(), &mut size)
            };
            if result == 0 && info.tcpi_rtt > 0 {
                return Some(f64::from(info.tcpi_rtt) / 1000.0);
            }
        }
        None
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

#[cfg(all(test, target_os = "linux"))]
mod rtt_tests {
    use super::*;
    #[tokio::test]
    async fn tcp_rtt_is_available_without_waiting_for_a_heartbeat_echo() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(listener.local_addr().unwrap()), listener.accept());
        let transport = NetworkTransport::new(client.unwrap());
        let _server = server.unwrap();
        assert!(transport.kernel_rtt_ms().is_some_and(|ms| ms > 0.0 && ms < 1000.0));
    }
}
