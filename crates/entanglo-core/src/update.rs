//! Parses `https://entanglo.pages.dev/updates/latest-linux.json` and
//! drives the auto-update flow. Phase 2, see `ROADMAP.md`. New
//! manifest — mirrors the existing `latest.json` (Mac) /
//! `latest-win.json` (Windows) pattern in the `entanglo-website` repo.
//!
//! Install path: verify SHA-256 from the manifest, then
//! `pkexec dpkg -i <path>` so the user gets exactly one polkit
//! prompt — no silent root escalation.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(rename = "downloadUrl")]
    pub download_url: String,
    pub sha256: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
}

pub const MANIFEST_URL: &str = "https://entanglo.pages.dev/updates/latest-linux.json";

pub struct UpdateService;

impl UpdateService {
    /// Skeleton: not yet wired to an HTTP client (no `reqwest`
    /// dependency added yet — add it in Phase 2 alongside this).
    pub async fn check_for_update(
        &self,
        _current_version: &str,
    ) -> anyhow::Result<Option<UpdateManifest>> {
        anyhow::bail!("update check not yet implemented — see ROADMAP.md Phase 2")
    }

    pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        hex::encode(digest).eq_ignore_ascii_case(expected_hex)
    }
}
