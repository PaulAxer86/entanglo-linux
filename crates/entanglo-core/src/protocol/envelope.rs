use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use super::message_type::MessageType;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// The `EntangloMessage` envelope. See `PROTOCOL.md` §3.
///
/// `payload` carries the type-specific payload as base64-encoded
/// JSON — a Swift `Codable` quirk baked into the wire format. Every
/// implementation, this one included, must mirror it: JSON-encode
/// the payload, base64 it, *then* put it in this field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntangloMessage {
    pub protocol_version: u32,
    pub message_type: MessageType,
    pub sender_device_id: String,
    pub session_id: String,
    pub timestamp: f64,
    pub payload: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u32),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("base64 error: {0}")]
    Base64(#[from] base64::DecodeError),
}

impl EntangloMessage {
    pub fn encode_payload<T: Serialize>(
        message_type: MessageType,
        sender_device_id: impl Into<String>,
        session_id: impl Into<String>,
        payload: &T,
    ) -> Result<Self, EnvelopeError> {
        let payload_bytes = serde_json::to_vec(payload)?;
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            message_type,
            sender_device_id: sender_device_id.into(),
            session_id: session_id.into(),
            timestamp: unix_timestamp_now(),
            payload: STANDARD.encode(payload_bytes),
        })
    }

    /// Decode the envelope's JSON bytes, rejecting an unsupported
    /// `protocolVersion` per §8. Callers reading frames off the wire
    /// should treat this error as "drop this one frame, keep the
    /// connection open" for anything except a version mismatch,
    /// which per §11 is the one case the Mac reference closes the
    /// connection over — mirror that here.
    pub fn decode(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        let env: Self = serde_json::from_slice(bytes)?;
        if env.protocol_version != PROTOCOL_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(env.protocol_version));
        }
        Ok(env)
    }

    pub fn decode_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, EnvelopeError> {
        let bytes = STANDARD.decode(&self.payload)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn unix_timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs_f64()
}
