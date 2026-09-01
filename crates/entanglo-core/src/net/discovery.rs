//! mDNS advertise + browse for `_entanglo._tcp`, via the pure-Rust
//! `mdns-sd` crate — no dependency on `avahi-daemon` running.
//! See `PROTOCOL.md` §1 and `STACK.md`.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use super::SERVICE_TYPE;

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub device_name: String,
    pub host: String,
    pub port: u16,
}

pub struct DiscoveryService {
    daemon: ServiceDaemon,
}

impl DiscoveryService {
    pub fn new() -> Result<Self, mdns_sd::Error> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
        })
    }

    /// Advertise this instance on the LAN. `device_name` becomes the
    /// user-visible service name (e.g. "Debian Desktop"); `port` is
    /// the OS-assigned TCP listener port from `net::transport`.
    pub fn advertise(&self, device_name: &str, port: u16) -> Result<(), mdns_sd::Error> {
        let hostname = format!("{device_name}.local.");
        // Empty IP + `enable_addr_auto()` lets mdns-sd enumerate this
        // host's interfaces itself rather than us guessing which one
        // is "the" LAN address.
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            device_name,
            &hostname,
            "",
            port,
            None, // TXT record: reserved for future capability flags.
        )?
        .enable_addr_auto();
        self.daemon.register(info)?;
        Ok(())
    }

    /// Browse for peers. Returns a receiver of resolved
    /// `DiscoveredPeer`s; the caller decides whether to dial each one
    /// (typically: only if already trusted, per the connection
    /// lifecycle in `PROTOCOL.md` §6).
    pub fn browse(&self) -> Result<std::sync::mpsc::Receiver<DiscoveredPeer>, mdns_sd::Error> {
        let events = self.daemon.browse(SERVICE_TYPE)?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let Some(addr) = info.get_addresses().iter().next() else {
                        continue;
                    };
                    let _ = tx.send(DiscoveredPeer {
                        device_name: info.get_fullname().to_string(),
                        host: addr.to_string(),
                        port: info.get_port(),
                    });
                }
            }
        });
        Ok(rx)
    }
}
