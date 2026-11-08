//! HTTPS Linux release checks and verified Debian package downloads.
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::io::Read;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(rename = "url", alias = "downloadUrl")]
    pub download_url: String,
    pub sha256: String,
    #[serde(rename = "size", alias = "sizeBytes")]
    pub size_bytes: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub highlights: Vec<String>,
}
pub const MANIFEST_URL: &str = "https://entanglo.pages.dev/updates/latest-linux.json";
pub struct UpdateService;

fn version_is_valid(version: &str) -> bool {
    !version.is_empty() && version.len() < 100 && version.as_bytes()[0].is_ascii_digit()
        && version.chars().all(|c| c.is_ascii_alphanumeric() || ".+-~:".contains(c))
}
impl UpdateManifest {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(version_is_valid(&self.version), "Invalid release version");
        anyhow::ensure!(self.download_url.starts_with("https://entanglo.pages.dev/downloads/")
            || self.download_url.starts_with("https://github.com/PaulAxer86/entanglo-linux/releases/download/"), "Unexpected update download host");
        anyhow::ensure!(self.sha256.len() == 64 && self.sha256.chars().all(|c| c.is_ascii_hexdigit()), "Invalid SHA-256");
        anyhow::ensure!(self.size_bytes > 0 && self.size_bytes <= 100 * 1024 * 1024, "Unsupported update package size");
        Ok(())
    }
}
impl UpdateService {
    pub async fn check_for_update(&self, current: &str) -> anyhow::Result<Option<UpdateManifest>> {
        let current = current.to_owned();
        tokio::task::spawn_blocking(move || {
            let output = Command::new("curl").args(["--fail", "--silent", "--show-error", "--location",
                "--proto", "=https", "--proto-redir", "=https", "--max-time", "30", "--max-filesize", "1048576", MANIFEST_URL]).output()?;
            anyhow::ensure!(output.status.success(), "Update server unavailable: {}", String::from_utf8_lossy(&output.stderr).trim());
            let manifest: UpdateManifest = serde_json::from_slice(&output.stdout)?;
            manifest.validate()?;
            anyhow::ensure!(version_is_valid(&current), "Invalid current version");
            let newer = Command::new("dpkg").args(["--compare-versions", &manifest.version, "gt", &current]).status()?;
            Ok(if newer.success() { Some(manifest) } else { None })
        }).await?
    }

    pub fn download(manifest: &UpdateManifest, cache: &Path) -> anyhow::Result<PathBuf> {
        manifest.validate()?;
        std::fs::create_dir_all(cache)?;
        let temporary = cache.join(format!("download-{}.part", uuid::Uuid::new_v4()));
        let result = (|| -> anyhow::Result<PathBuf> {
            let output = Command::new("curl").args(["--fail", "--silent", "--show-error", "--location",
                "--proto", "=https", "--proto-redir", "=https", "--max-time", "300", "--max-filesize", &manifest.size_bytes.to_string(),
                "--output"]).arg(&temporary).arg(&manifest.download_url).output()?;
            anyhow::ensure!(output.status.success(), "Download failed: {}", String::from_utf8_lossy(&output.stderr).trim());
            Self::verify_file(&temporary, manifest)?;
            let target = cache.join(format!("entanglo-linux_{}_amd64.deb", manifest.version));
            std::fs::rename(&temporary, &target)?;
            Ok(target)
        })();
        if result.is_err() { let _ = std::fs::remove_file(temporary); }
        result
    }

    pub fn verify_file(path: &Path, manifest: &UpdateManifest) -> anyhow::Result<()> {
        anyhow::ensure!(std::fs::metadata(path)?.len() == manifest.size_bytes, "Update size mismatch");
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop { let count = file.read(&mut buffer)?; if count == 0 { break; } hasher.update(&buffer[..count]); }
        anyhow::ensure!(hex::encode(hasher.finalize()).eq_ignore_ascii_case(&manifest.sha256), "Update checksum mismatch");
        Ok(())
    }

    pub fn verify_sha256(bytes: &[u8], expected: &str) -> bool {
        hex::encode(Sha256::digest(bytes)).eq_ignore_ascii_case(expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_accepts_existing_field_names_and_rejects_bad_hosts() {
        let mut manifest: UpdateManifest = serde_json::from_value(serde_json::json!({
            "version":"0.1.1", "url":"https://entanglo.pages.dev/downloads/Entanglo.deb",
            "sha256":"a".repeat(64),"size":5
        })).unwrap();
        assert!(manifest.validate().is_ok());
        manifest.download_url="http://example.com/file.deb".into();assert!(manifest.validate().is_err());
    }
    #[test]
    fn corrupted_or_truncated_packages_are_rejected() {
        let path=std::env::temp_dir().join(format!("entanglo-update-{}",uuid::Uuid::new_v4()));
        std::fs::write(&path,b"valid package").unwrap();
        let mut manifest=UpdateManifest { version:"0.1.1".into(),download_url:"https://entanglo.pages.dev/downloads/test.deb".into(),
            sha256:hex::encode(Sha256::digest(b"valid package")),size_bytes:13,title:String::new(),highlights:Vec::new() };
        assert!(UpdateService::verify_file(&path,&manifest).is_ok());
        manifest.size_bytes=12;assert!(UpdateService::verify_file(&path,&manifest).is_err());
        manifest.size_bytes=13;manifest.sha256="0".repeat(64);assert!(UpdateService::verify_file(&path,&manifest).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
