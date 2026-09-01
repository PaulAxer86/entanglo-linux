//! Trusted-device persistence + local device identity. See
//! `PROTOCOL.md` §9.
//!
//! Primary path: Secret Service D-Bus API (GNOME Keyring / KWallet),
//! the desktop-Linux analogue of macOS Keychain / Windows DPAPI.
//! Fallback: a file under `$XDG_DATA_HOME/entanglo/trust.json.enc`
//! encrypted with a key derived (HKDF-SHA256) from `/etc/machine-id`,
//! for headless boxes with no Secret Service daemon running — mirrors
//! the DPAPI fallback story `entanglo-windows` doesn't need but
//! `entanglo-macos`'s Keychain doesn't either; Linux is the one
//! platform among the four where "no secure store available" is a
//! real, common case (minimal/server Debian installs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub friendly_name: String,
    pub trusted_since_unix: f64,
}

const SECRET_SERVICE_APPLICATION: &str = "entanglo";
const SECRET_SERVICE_PURPOSE: &str = "trust-store";
const SECRET_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
/// HKDF `info` string. Bumping this deliberately invalidates every
/// existing fallback-file encryption key, in case the derivation
/// scheme itself ever needs to change.
const HKDF_INFO: &[u8] = b"entanglo-trust-store-v1";

pub struct TrustStore {
    devices: HashMap<String, TrustedDevice>,
    /// Set once `load()` determines which backend answered, so
    /// `save()` writes back to the same place without re-probing
    /// Secret Service on every save.
    backend: Backend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    SecretService,
    File,
}

#[derive(Debug, thiserror::Error)]
pub enum TrustStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fallback file too short to contain a nonce")]
    FileTooShort,
    #[error("decryption failed — wrong key or corrupted file")]
    DecryptionFailed,
}

impl TrustStore {
    /// Try Secret Service first (bounded by `SECRET_SERVICE_TIMEOUT`
    /// so a broken/absent D-Bus session can't hang startup); fall
    /// back to the encrypted file on any error.
    pub async fn load() -> Result<Self, TrustStoreError> {
        match tokio::time::timeout(SECRET_SERVICE_TIMEOUT, secret_service_backend::load()).await {
            Ok(Ok(devices)) => {
                return Ok(Self {
                    devices,
                    backend: Backend::SecretService,
                })
            }
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "Secret Service unavailable, falling back to file")
            }
            Err(_) => tracing::debug!("Secret Service timed out, falling back to file"),
        }

        let devices = file_backend::load(&data_dir())?;
        Ok(Self {
            devices,
            backend: Backend::File,
        })
    }

    pub fn is_trusted(&self, device_id: &str) -> bool {
        self.devices.contains_key(device_id)
    }

    pub fn trust(&mut self, device: TrustedDevice) {
        self.devices.insert(device.device_id.clone(), device);
    }

    pub fn revoke(&mut self, device_id: &str) {
        self.devices.remove(device_id);
    }

    pub fn trusted_devices(&self) -> impl Iterator<Item = &TrustedDevice> {
        self.devices.values()
    }

    /// Persist to whichever backend `load()` used.
    pub async fn save(&self) -> Result<(), TrustStoreError> {
        match self.backend {
            Backend::SecretService => {
                match tokio::time::timeout(
                    SECRET_SERVICE_TIMEOUT,
                    secret_service_backend::save(&self.devices),
                )
                .await
                {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "Secret Service save failed, writing fallback file instead");
                        file_backend::save(&data_dir(), &self.devices)
                    }
                    Err(_) => {
                        tracing::warn!(
                            "Secret Service save timed out, writing fallback file instead"
                        );
                        file_backend::save(&data_dir(), &self.devices)
                    }
                }
            }
            Backend::File => file_backend::save(&data_dir(), &self.devices),
        }
    }
}

/// This installation's stable `senderDeviceId` (`PROTOCOL.md` §3),
/// generated once and persisted as plain text under the same data
/// directory as the trust store — it is an identifier, not a secret,
/// so it doesn't need Secret Service or encryption.
pub fn local_device_id() -> Result<String, TrustStoreError> {
    let path = data_dir().join("device_id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(&data_dir())?;
    std::fs::write(&path, &id)?;
    Ok(id)
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").expect("HOME must be set");
            PathBuf::from(home).join(".local/share")
        })
        .join("entanglo")
}

mod secret_service_backend {
    use super::*;
    use ::secret_service::{EncryptionType, SecretService};

    pub async fn load() -> anyhow::Result<HashMap<String, TrustedDevice>> {
        let ss = SecretService::connect(EncryptionType::Dh).await?;
        let search = ss
            .search_items(HashMap::from([
                ("application", SECRET_SERVICE_APPLICATION),
                ("purpose", SECRET_SERVICE_PURPOSE),
            ]))
            .await?;
        let secret = if let Some(item) = search.unlocked.first() {
            item.get_secret().await?
        } else if let Some(item) = search.locked.first() {
            item.unlock().await?;
            item.get_secret().await?
        } else {
            return Ok(HashMap::new());
        };
        Ok(serde_json::from_slice(&secret)?)
    }

    pub async fn save(devices: &HashMap<String, TrustedDevice>) -> anyhow::Result<()> {
        let ss = SecretService::connect(EncryptionType::Dh).await?;
        let collection = ss.get_default_collection().await?;
        let json = serde_json::to_vec(devices)?;
        collection
            .create_item(
                "Entanglo Trust Store",
                HashMap::from([
                    ("application", SECRET_SERVICE_APPLICATION),
                    ("purpose", SECRET_SERVICE_PURPOSE),
                ]),
                &json,
                true, // replace the existing item with these attributes
                "application/json",
            )
            .await?;
        Ok(())
    }
}

mod file_backend {
    use super::*;

    const FILE_NAME: &str = "trust.json.enc";

    pub fn load(dir: &std::path::Path) -> Result<HashMap<String, TrustedDevice>, TrustStoreError> {
        let path = dir.join(FILE_NAME);
        let Ok(bytes) = std::fs::read(&path) else {
            return Ok(HashMap::new());
        };
        let plaintext = decrypt(&bytes)?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    pub fn save(
        dir: &std::path::Path,
        devices: &HashMap<String, TrustedDevice>,
    ) -> Result<(), TrustStoreError> {
        std::fs::create_dir_all(dir)?;
        let plaintext = serde_json::to_vec(devices)?;
        let ciphertext = encrypt(&plaintext);
        std::fs::write(dir.join(FILE_NAME), ciphertext)?;
        Ok(())
    }

    fn derive_key() -> [u8; 32] {
        let machine_id = std::fs::read_to_string("/etc/machine-id")
            .unwrap_or_else(|_| "entanglo-no-machine-id-fallback".to_string());
        let hk = Hkdf::<Sha256>::new(None, machine_id.trim().as_bytes());
        let mut key = [0u8; 32];
        hk.expand(HKDF_INFO, &mut key)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        key
    }

    /// Layout: `[12-byte nonce][ciphertext+tag]`.
    fn encrypt(plaintext: &[u8]) -> Vec<u8> {
        let key = derive_key();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("key is exactly 32 bytes");
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .expect("in-memory AES-GCM encryption does not fail");
        let mut out = nonce.to_vec();
        out.append(&mut ciphertext);
        out
    }

    fn decrypt(data: &[u8]) -> Result<Vec<u8>, TrustStoreError> {
        if data.len() < 12 {
            return Err(TrustStoreError::FileTooShort);
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let key = derive_key();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("key is exactly 32 bytes");
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| TrustStoreError::DecryptionFailed)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn encrypt_decrypt_roundtrips() {
            let plaintext = br#"{"hello":"world"}"#;
            let ciphertext = encrypt(plaintext);
            let decrypted = decrypt(&ciphertext).unwrap();
            assert_eq!(decrypted, plaintext);
        }

        #[test]
        fn tampered_ciphertext_fails_to_decrypt() {
            let mut ciphertext = encrypt(b"secret data");
            let last = ciphertext.len() - 1;
            ciphertext[last] ^= 0xFF;
            assert!(decrypt(&ciphertext).is_err());
        }

        #[test]
        fn save_then_load_roundtrips_through_disk() {
            let dir = std::env::temp_dir().join(format!(
                "entanglo-trust-store-test-{}",
                uuid::Uuid::new_v4()
            ));
            let mut devices = HashMap::new();
            devices.insert(
                "device-1".to_string(),
                TrustedDevice {
                    device_id: "device-1".to_string(),
                    friendly_name: "Test iMac".to_string(),
                    trusted_since_unix: 1_700_000_000.0,
                },
            );

            save(&dir, &devices).unwrap();
            let loaded = load(&dir).unwrap();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded["device-1"].friendly_name, "Test iMac");

            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
