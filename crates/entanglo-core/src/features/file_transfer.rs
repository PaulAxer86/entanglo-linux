//! Chunked file send/receive between trusted peers. Phase 2, see
//! `ROADMAP.md`. Mirrors `entanglo-macos`'s `FileTransferService`;
//! wire shape is `PROTOCOL.md` §5.8 (`fileOffer`/`fileChunk`/`fileAck`).
//!
//! Files land in `$XDG_DOWNLOAD_DIR` (fallback `~/Downloads`), per
//! §5.8. `kind: "printJob"` spools to CUPS instead — see the Printer
//! Bridge Phase 3 plan in `ROADMAP.md`.

use crate::protocol::payloads::{FileAckPayload, FileChunkPayload, FileOfferPayload};

pub struct FileTransferService;

pub struct IncomingTransfer {
    pub offer: FileOfferPayload,
    pub received_bytes: Vec<u8>,
}

impl FileTransferService {
    pub fn new() -> Self {
        Self
    }

    pub fn begin_incoming(&self, offer: FileOfferPayload) -> IncomingTransfer {
        IncomingTransfer {
            received_bytes: Vec::with_capacity(offer.size_bytes as usize),
            offer,
        }
    }

    pub fn apply_chunk(
        &self,
        transfer: &mut IncomingTransfer,
        chunk: &FileChunkPayload,
    ) -> anyhow::Result<()> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD.decode(&chunk.data)?;
        transfer.received_bytes.extend_from_slice(&bytes);
        Ok(())
    }

    pub fn finish(&self, transfer: IncomingTransfer) -> FileAckPayload {
        // Real implementation writes `transfer.received_bytes` to
        // `download_dir().join(&transfer.offer.name)` and returns the
        // outcome. Skeleton: report success unconditionally.
        FileAckPayload {
            transfer_id: transfer.offer.transfer_id,
            ok: true,
            failure_reason: None,
        }
    }

    /// `$XDG_DOWNLOAD_DIR`, falling back to `~/Downloads` per the XDG
    /// user-dirs spec, matching `PROTOCOL.md` §5.8.
    pub fn download_dir() -> std::path::PathBuf {
        std::env::var_os("XDG_DOWNLOAD_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                std::path::PathBuf::from(home).join("Downloads")
            })
    }
}

impl Default for FileTransferService {
    fn default() -> Self {
        Self::new()
    }
}
