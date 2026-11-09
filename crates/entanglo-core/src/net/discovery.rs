//! Bonjour discovery. Prefer Debian's Avahi for network-interface changes;
//! retain the Rust mDNS backend when Avahi is unavailable.
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use std::process::{Child, Command, Stdio};
use std::net::{IpAddr, SocketAddr};
use super::SERVICE_TYPE;

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub device_name: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
    pub device_id: Option<String>,
    pub interface: Option<String>,
}
impl DiscoveredPeer {
    pub fn cache_key(&self) -> String {
        format!("{}|{}|{}", self.device_name, self.interface.as_deref().unwrap_or(""), self.host)
    }
    pub fn priority(&self) -> u8 {
        super::path::interface_priority(self.interface.as_deref())
    }
    pub fn endpoint(&self) -> Option<SocketAddr> {
        let ip = self.host.parse::<IpAddr>().ok()?;
        if let IpAddr::V6(ip) = ip {
            if ip.is_unicast_link_local() {
                let interface = std::ffi::CString::new(self.interface.as_deref()?).ok()?;
                let scope = unsafe { libc::if_nametoindex(interface.as_ptr()) };
                if scope == 0 { return None; }
                return Some(SocketAddr::V6(std::net::SocketAddrV6::new(ip, self.port, 0, scope)));
            }
        }
        Some(SocketAddr::new(ip, self.port))
    }
}
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Resolved(DiscoveredPeer),
    Removed(String),
    RemovedOnInterface { name: String, interface: String, ipv4: bool },
}
impl DiscoveryEvent {
    pub fn removes(&self, peer: &DiscoveredPeer) -> bool {
        match self {
            Self::Removed(name) => peer.device_name == *name,
            Self::RemovedOnInterface { name, interface, ipv4 } => peer.device_name == *name
                && peer.interface.as_ref() == Some(interface)
                && peer.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv4() == *ipv4),
            Self::Resolved(_) => false,
        }
    }
}

pub struct DiscoveryService {
    daemon: Mutex<Option<ServiceDaemon>>,
    publisher: Mutex<Option<Child>>,
    browser: Mutex<Option<Child>>,
    advertised: Mutex<Option<ServiceInfo>>,
    native: AtomicBool,
    stopping: AtomicBool,
}
fn bind_child_lifetime(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // Ensure a forced app exit cannot leave an orphan Bonjour publisher.
        unsafe { command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }); }
    }
}
fn native_publisher(info: &ServiceInfo) -> Option<Child> {
    let suffix = format!(".{SERVICE_TYPE}");
    let name = info.get_fullname().strip_suffix(&suffix).unwrap_or(info.get_fullname());
    let mut command = Command::new("avahi-publish-service");
    command.args([name, "_entanglo._tcp", &info.get_port().to_string()]);
    for key in ["id", "version", "txtvers"] {
        if let Some(value) = info.get_property_val_str(key) { command.arg(format!("{key}={value}")); }
    }
    bind_child_lifetime(&mut command);
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    if child.try_wait().ok()?.is_some() { return None; }
    Some(child)
}
fn stop(child: &mut Option<Child>) { if let Some(mut child) = child.take() { let _ = child.kill(); let _ = child.wait(); } }
impl DiscoveryService {
    pub fn new() -> Result<Self, mdns_sd::Error> {
        Ok(Self { daemon: Mutex::new(None),publisher: Mutex::new(None),browser: Mutex::new(None),
            advertised: Mutex::new(None),native: AtomicBool::new(false),stopping: AtomicBool::new(false) })
    }
    fn rust_daemon(&self) -> Result<ServiceDaemon, mdns_sd::Error> {
        let mut daemon=self.daemon.lock().unwrap();
        if daemon.is_none() { *daemon=Some(ServiceDaemon::new()?); }
        Ok(daemon.as_ref().unwrap().clone())
    }
    pub fn advertise(&self, name: &str, port: u16) -> Result<(), mdns_sd::Error> { self.advertise_identity(name,port,None) }
    pub fn advertise_identity(&self, name: &str, port: u16, device_id: Option<&str>) -> Result<(), mdns_sd::Error> {
        let mut properties=std::collections::HashMap::from([("txtvers".to_string(),"1".to_string())]);
        if let Some(id)=device_id { properties.insert("id".into(),id.into()); }
        let info=ServiceInfo::new(SERVICE_TYPE,name,&format!("{name}.local."),"",port,properties)?.enable_addr_auto();
        if let Some(child)=native_publisher(&info) {
            *self.publisher.lock().unwrap()=Some(child);self.native.store(true,Ordering::Release);
            tracing::info!("Bonjour advertising through Avahi (interface-aware)");
        } else {
            self.rust_daemon()?.register(info.clone())?;
            tracing::info!("Bonjour advertising through Rust mDNS fallback");
        }
        *self.advertised.lock().unwrap()=Some(info);
        Ok(())
    }
    pub fn refresh(&self) -> Result<(), mdns_sd::Error> {
        let saved=self.advertised.lock().unwrap();
        if self.stopping.load(Ordering::Acquire) { return Ok(()); }
        if let Some(info)=saved.as_ref() {
            if self.native.load(Ordering::Acquire) {
                let mut publisher=self.publisher.lock().unwrap();stop(&mut publisher);
                std::thread::sleep(std::time::Duration::from_millis(500));
                if !self.stopping.load(Ordering::Acquire) { *publisher=native_publisher(info); }
            } else {
                let daemon=self.rust_daemon()?;
                if let Ok(response)=daemon.unregister(info.get_fullname()) { let _=response.recv_timeout(std::time::Duration::from_millis(300)); }
                std::thread::sleep(std::time::Duration::from_millis(500));
                if !self.stopping.load(Ordering::Acquire) { daemon.register(info.clone())?; }
            }
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        if self.stopping.swap(true,Ordering::AcqRel) { return; }
        let saved=self.advertised.lock().unwrap().take();
        stop(&mut self.publisher.lock().unwrap());stop(&mut self.browser.lock().unwrap());
        if let Some(daemon)=self.daemon.lock().unwrap().take() {
            if let Some(info)=saved {
                if let Ok(response)=daemon.unregister(info.get_fullname()) { let _=response.recv_timeout(std::time::Duration::from_millis(300)); }
            }
            if let Ok(response)=daemon.shutdown() { let _=response.recv_timeout(std::time::Duration::from_millis(300)); }
        }
    }
    pub fn browse_events(&self) -> Result<std::sync::mpsc::Receiver<DiscoveryEvent>, mdns_sd::Error> {
        let (tx,rx)=std::sync::mpsc::channel();
        if self.native.load(Ordering::Acquire) {
            use std::io::BufRead;
            let mut command=Command::new("avahi-browse");
            command.args(["--resolve","--parsable","_entanglo._tcp"])
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            bind_child_lifetime(&mut command);
            if let Ok(mut child)=command.spawn() {
                let stdout=child.stdout.take().unwrap();*self.browser.lock().unwrap()=Some(child);
                std::thread::spawn(move || {
                    for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                        if let Some(event)=parse_avahi_line(&line) { if tx.send(event).is_err() { break; } }
                    }
                });
                return Ok(rx);
            }
        }
        let events=self.rust_daemon()?.browse(SERVICE_TYPE)?;
        std::thread::spawn(move || {
            while let Ok(event)=events.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let mut addresses: Vec<_>=info.get_addresses().iter().copied().collect();
                        addresses.sort_by_key(|ip| (!ip.is_ipv4(),ip.to_string()));
                        let Some(addr)=addresses.first() else {continue;};
                        let _=tx.send(DiscoveryEvent::Resolved(DiscoveredPeer { device_name:info.get_fullname().to_string(),host:addr.to_string(),port:info.get_port(),
                            device_id:info.get_property_val_str("id").map(str::to_owned),addresses,interface:None }));
                    }
                    ServiceEvent::ServiceRemoved(_,name)=>{let _=tx.send(DiscoveryEvent::Removed(name));}
                    _=>{}
                }
            }
        });Ok(rx)
    }
    pub fn browse(&self) -> Result<std::sync::mpsc::Receiver<DiscoveredPeer>,mdns_sd::Error> {
        let events=self.browse_events()?;let(tx,rx)=std::sync::mpsc::channel();
        std::thread::spawn(move || {while let Ok(event)=events.recv(){if let DiscoveryEvent::Resolved(peer)=event{if tx.send(peer).is_err(){break;}}}});Ok(rx)
    }
}
fn decode_name(name: &str) -> String {
    let bytes=name.as_bytes();let mut output=Vec::new();let mut index=0;
    while index<bytes.len() {
        if bytes[index]==b'\\' && index+3<bytes.len() {
            if let Some(value)=std::str::from_utf8(&bytes[index+1..index+4]).ok().and_then(|part|part.parse::<u8>().ok()) {output.push(value);index+=4;continue;}
        }
        output.push(bytes[index]);index+=1;
    }
    String::from_utf8_lossy(&output).into_owned()
}
pub fn parse_avahi_line(line: &str) -> Option<DiscoveryEvent> {
    let fields:Vec<_>=line.split(';').collect();
    if fields.len()<6 || fields[4]!="_entanglo._tcp" {return None;}
    let name=decode_name(fields[3]);let service=format!("{name}._entanglo._tcp.local.");
    if fields[0]=="-" {return Some(DiscoveryEvent::RemovedOnInterface { name: service, interface: fields[1].into(), ipv4: fields[2] == "IPv4" });}
    if fields[0]!="=" || fields.len()<9 {return None;}
    let address=fields[7].split('%').next()?.parse::<IpAddr>().ok()?;
    let id=fields.get(9).and_then(|txt|txt.split_whitespace().find_map(|part|part.trim_matches('"').strip_prefix("id=").map(str::to_owned)));
    Some(DiscoveryEvent::Resolved(DiscoveredPeer {device_name:service,host:address.to_string(),port:fields[8].parse().ok()?,addresses:vec![address],device_id:id,interface:Some(fields[1].into())}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removing_wifi_keeps_the_same_devices_ethernet_and_other_address_family() {
        let resolve = |interface: &str, family: &str, address: &str| {
            let line = format!("=;{interface};{family};Peer;_entanglo._tcp;local;peer.local;{address};55555;\"id=one-device\"");
            let DiscoveryEvent::Resolved(peer) = parse_avahi_line(&line).unwrap() else { panic!() };
            peer
        };
        let wired = resolve("eth0", "IPv4", "169.254.1.2");
        let wifi = resolve("wlan0", "IPv4", "192.0.2.7");
        let wifi6 = resolve("wlan0", "IPv6", "fe80::2");
        let event = parse_avahi_line("-;wlan0;IPv4;Peer;_entanglo._tcp;local").unwrap();
        assert!(event.removes(&wifi));
        assert!(!event.removes(&wired));
        assert!(!event.removes(&wifi6));
        assert_ne!(wired.cache_key(), wifi.cache_key());
    }

    #[test]
    fn link_local_ipv6_keeps_its_interface_scope() {
        let DiscoveryEvent::Resolved(peer) = parse_avahi_line("=;lo;IPv6;Peer;_entanglo._tcp;local;peer.local;fe80::2;55555;\"id=peer\"").unwrap() else { panic!() };
        let SocketAddr::V6(endpoint) = peer.endpoint().unwrap() else { panic!() };
        assert_ne!(endpoint.scope_id(), 0);
    }

    #[test]
    fn avahi_resolution_preserves_identity_and_interface() {
        let DiscoveryEvent::Resolved(peer)=parse_avahi_line("=;eth0;IPv4;Office\\032Mac;_entanglo._tcp;local;mac.local;192.0.2.7;55555;\"id=stable-uuid\"").unwrap() else {panic!()};
        assert_eq!(peer.device_name,"Office Mac._entanglo._tcp.local.");assert_eq!(peer.device_id.as_deref(),Some("stable-uuid"));
        assert_eq!(peer.endpoint().unwrap().to_string(),"192.0.2.7:55555");assert_eq!(peer.interface.as_deref(),Some("eth0"));
    }
    #[test]
    fn avahi_removal_is_not_a_new_device_and_invalid_records_are_ignored() {
        assert!(matches!(parse_avahi_line("-;eth0;IPv4;Mac;_entanglo._tcp;local"),Some(DiscoveryEvent::RemovedOnInterface { .. })));
        assert!(parse_avahi_line("=;eth0;IPv4;Printer;_ipp._tcp;local;printer;192.0.2.1;631;").is_none());
        assert!(parse_avahi_line("=;bad").is_none());
    }
}
