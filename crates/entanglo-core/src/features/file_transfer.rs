//! Streaming file reception using the existing Mac/Windows/Android protocol.
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use base64::{engine::general_purpose::STANDARD, Engine};
use crate::protocol::payloads::{FileChunkPayload, FileOfferPayload};

/// Largest accepted chunk. Windows peers send 2 MiB chunks, which still fit
/// the 4 MiB frame limit once base64-encoded inside the message envelope.
const MAX_CHUNK_BYTES: u32 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct FileProgress {
    pub transfer_id: String,
    pub name: String,
    pub received: u64,
    pub total: u64,
    pub destination: Option<PathBuf>,
    pub error: Option<String>,
    pub complete: bool,
    pub updated_at: std::time::SystemTime,
}

struct IncomingTransfer {
    offer: FileOfferPayload,
    file: File,
    temporary: PathBuf,
    received: u64,
    next_sequence: u32,
}
impl Drop for IncomingTransfer {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.temporary); }
}

pub struct FileTransferService {
    directory: PathBuf,
    incoming: HashMap<String, IncomingTransfer>,
}
impl Default for FileTransferService { fn default() -> Self { Self::new() } }
impl FileTransferService {
    pub fn new() -> Self { Self::in_directory(Self::download_dir()) }
    pub fn in_directory(directory: PathBuf) -> Self { Self { directory, incoming: HashMap::new() } }

    pub fn download_dir() -> PathBuf {
        let base = std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from)
            .or_else(|| std::process::Command::new("xdg-user-dir").arg("DOWNLOAD").output().ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| PathBuf::from(s.trim())).filter(|p| p.is_absolute()))
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads"));
        base.join("Entanglo")
    }

    pub fn begin_incoming(&mut self, offer: FileOfferPayload) -> anyhow::Result<FileProgress> {
        anyhow::ensure!(!offer.name.is_empty() && offer.name != "." && offer.name != ".."
            && !offer.name.contains(['/', '\\']) && !offer.name.chars().any(char::is_control)
            && offer.name.len() <= 240, "Invalid file name");
        anyhow::ensure!(offer.chunk_size > 0 && offer.chunk_size <= MAX_CHUNK_BYTES, "Unsupported chunk size");
        let chunks = offer.size_bytes / u64::from(offer.chunk_size)
            + u64::from(offer.size_bytes % u64::from(offer.chunk_size) != 0);
        anyhow::ensure!(u64::from(offer.total_chunks) == chunks.max(1), "Invalid chunk count");
        anyhow::ensure!(self.incoming.len() < 8, "Too many simultaneous incoming transfers");
        anyhow::ensure!(!self.incoming.contains_key(&offer.transfer_id), "Duplicate transfer ID");
        anyhow::ensure!(offer.kind.as_deref() != Some("printJob"), "Printer jobs are not supported yet");
        std::fs::create_dir_all(&self.directory)?;
        let temporary = self.directory.join(format!(".entanglo-{}.part", uuid::Uuid::new_v4()));
        let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
        let progress = FileProgress {
            transfer_id: offer.transfer_id.clone(), name: offer.name.clone(), received: 0,
            total: offer.size_bytes, destination: None, error: None, complete: false,
            updated_at: std::time::SystemTime::now(),
        };
        self.incoming.insert(offer.transfer_id.clone(), IncomingTransfer { offer, file, temporary, received: 0, next_sequence: 0 });
        Ok(progress)
    }

    pub fn apply_chunk(&mut self, chunk: &FileChunkPayload) -> anyhow::Result<FileProgress> {
        // Removing the state first ensures any failed write/decode/order check
        // closes and cleans up the partial transfer immediately.
        let mut transfer = self.incoming.remove(&chunk.transfer_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown or already finished transfer"))?;
        anyhow::ensure!(chunk.sequence == transfer.next_sequence, "Out-of-order chunk");
        let bytes = STANDARD.decode(&chunk.data)?;
        let remaining = transfer.offer.size_bytes - transfer.received;
        let expected = remaining.min(u64::from(transfer.offer.chunk_size));
        anyhow::ensure!(bytes.len() as u64 == expected, "Chunk size does not match offer");
        transfer.file.write_all(&bytes)?;
        transfer.received += bytes.len() as u64;
        transfer.next_sequence += 1;
        let complete = transfer.next_sequence == transfer.offer.total_chunks;
        let mut progress = FileProgress {
            transfer_id: transfer.offer.transfer_id.clone(), name: transfer.offer.name.clone(),
            received: transfer.received, total: transfer.offer.size_bytes,
            destination: None, error: None, complete,
            updated_at: std::time::SystemTime::now(),
        };
        if complete {
            anyhow::ensure!(transfer.received == transfer.offer.size_bytes, "Incomplete file");
            transfer.file.sync_all()?;
            progress.destination = Some(publish_unique(&transfer.temporary, &self.directory, &transfer.offer.name)?);
        } else {
            self.incoming.insert(chunk.transfer_id.clone(), transfer);
        }
        Ok(progress)
    }
}

fn publish_unique(temporary: &Path, directory: &Path, name: &str) -> std::io::Result<PathBuf> {
    let name_path = Path::new(name);
    let stem = name_path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = name_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for index in 0..10_000 {
        let target = directory.join(if index == 0 { name.to_string() } else { format!("{stem} ({index}){extension}") });
        match std::fs::hard_link(temporary, &target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "Too many files with this name"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offer(name: &str, size: u64) -> FileOfferPayload {
        FileOfferPayload { transfer_id: "test".into(), name: name.into(), size_bytes: size,
            total_chunks: (size.div_ceil(3).max(1)) as u32, chunk_size: 3, kind: None }
    }
    fn chunk(sequence: u32, data: &[u8]) -> FileChunkPayload {
        FileChunkPayload { transfer_id: "test".into(), sequence, data: STANDARD.encode(data) }
    }
    fn directory() -> PathBuf { std::env::temp_dir().join(format!("entanglo-file-test-{}", uuid::Uuid::new_v4())) }
    #[test]
    fn receives_exact_bytes_without_overwriting_existing_files() {
        let dir = directory();let mut service = FileTransferService::in_directory(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();std::fs::write(dir.join("hello.txt"), b"keep").unwrap();
        service.begin_incoming(offer("hello.txt",5)).unwrap();
        assert!(!service.apply_chunk(&chunk(0,b"hel")).unwrap().complete);
        let done = service.apply_chunk(&chunk(1,b"lo")).unwrap();
        assert!(done.complete);assert_eq!(std::fs::read(done.destination.unwrap()).unwrap(), b"hello");
        assert_eq!(std::fs::read(dir.join("hello.txt")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),2);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_paths_bad_order_and_incomplete_chunks_and_cleans_partials() {
        let dir=directory();let mut service=FileTransferService::in_directory(dir.clone());
        assert!(service.begin_incoming(offer("../escape",3)).is_err());
        service.begin_incoming(offer("safe",3)).unwrap();
        assert!(service.apply_chunk(&chunk(1,b"abc")).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),0);
        service.begin_incoming(offer("safe",3)).unwrap();
        assert!(service.apply_chunk(&chunk(0,b"ab")).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_windows_chunk_size_and_rejects_larger_chunks() {
        let dir=directory();let mut service=FileTransferService::in_directory(dir.clone());
        let size=MAX_CHUNK_BYTES as usize;
        let windows=FileOfferPayload { transfer_id:"test".into(), name:"photo.jpg".into(),
            size_bytes:size as u64, total_chunks:1, chunk_size:MAX_CHUNK_BYTES, kind:None };
        service.begin_incoming(windows.clone()).unwrap();
        let done=service.apply_chunk(&chunk(0,&vec![7u8;size])).unwrap();
        assert!(done.complete);
        assert_eq!(std::fs::metadata(done.destination.unwrap()).unwrap().len(),size as u64);
        let too_large=FileOfferPayload { chunk_size:MAX_CHUNK_BYTES+1, ..windows };
        assert!(service.begin_incoming(too_large).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn empty_files_and_disconnected_transfer_cleanup() {
        let dir=directory();let mut service=FileTransferService::in_directory(dir.clone());
        service.begin_incoming(offer("empty",0)).unwrap();
        let done=service.apply_chunk(&chunk(0,b"")).unwrap();
        assert_eq!(std::fs::metadata(done.destination.unwrap()).unwrap().len(),0);
        service.begin_incoming(offer("unfinished",3)).unwrap();drop(service);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
