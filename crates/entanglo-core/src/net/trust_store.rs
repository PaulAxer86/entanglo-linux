//! Trusted-device persistence. See `PROTOCOL.md` §9.
//!
//! Primary path: Secret Service D-Bus API (GNOME Keyring / KWallet),
//! the desktop-Linux analogue of macOS Keychain / Windows DPAPI.
//! Fallback: a file under `$XDG_DATA_HOME/entanglo/trust.json`
//! encrypted with a key derived from `/etc/machine-id`, for headless
//! boxes with no Secret Service daemon running.
//!
//! This is a skeleton — wire up the actual `secret-service` crate
//! calls and the HKDF-based file fallback as part of Phase 1 pairing
//! UX work (`ROADMAP.md`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub friendly_name: String,
    pub trusted_since_unix: f64,
}

pub struct TrustStore {
    devices: HashMap<String, TrustedDevice>,
}

impl TrustStore {
    /// Load from Secret Service if available, else the encrypted
    /// fallback file. Skeleton: starts empty.
    pub async fn load() -> anyhow::Result<Self> {
        Ok(Self {
            devices: HashMap::new(),
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

    /// Persist to Secret Service / fallback file. Skeleton: no-op.
    pub async fn save(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn data_dir() -> std::path::PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                std::path::PathBuf::from(home).join(".local/share")
            })
            .join("entanglo")
    }
}
