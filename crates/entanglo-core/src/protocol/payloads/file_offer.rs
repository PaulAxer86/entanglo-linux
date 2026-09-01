use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.8. `kind` is an optional discriminator —
/// today only `"printJob"` is used (Printer Bridge). Receivers MUST
/// treat any other/unknown `kind` as a regular file drop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOfferPayload {
    pub transfer_id: String,
    pub name: String,
    pub size_bytes: u64,
    pub total_chunks: u32,
    pub chunk_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

pub const PRINT_JOB_KIND: &str = "printJob";
