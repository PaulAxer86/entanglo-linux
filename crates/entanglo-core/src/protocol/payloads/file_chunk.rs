use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.8. `data` is base64 of exactly `chunk_size`
/// bytes from the offer (last chunk may be smaller). `sequence` is
/// 0-indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChunkPayload {
    pub transfer_id: String,
    pub sequence: u32,
    pub data: String,
}
