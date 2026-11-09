//! Resolve the interface used by a socket rather than the first Bonjour result.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn interface_addresses() -> Vec<(IpAddr, String)> {
    let mut head = std::ptr::null_mut();
    // getifaddrs owns the linked list until freeifaddrs below. Null addresses
    // and non-IP records are skipped before casting the sockaddr.
    if unsafe { libc::getifaddrs(&mut head) } != 0 { return Vec::new(); }
    let mut result = Vec::new();
    let mut current = head;
    while !current.is_null() {
        let item = unsafe { &*current };
        if !item.ifa_addr.is_null() && !item.ifa_name.is_null() {
            let name = unsafe { std::ffi::CStr::from_ptr(item.ifa_name) }.to_string_lossy().into_owned();
            let ip = match unsafe { (*item.ifa_addr).sa_family } as i32 {
                libc::AF_INET => {
                    let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in) };
                    Some(IpAddr::V4(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())))
                }
                libc::AF_INET6 => {
                    let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in6) };
                    Some(IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)))
                }
                _ => None,
            };
            if let Some(ip) = ip { result.push((ip, name)); }
        }
        current = item.ifa_next;
    }
    unsafe { libc::freeifaddrs(head); }
    result
}

pub fn interface_for_address(ip: IpAddr) -> Option<String> {
    let ip = ip.to_canonical();
    interface_addresses().into_iter().find(|(address, _)| *address == ip).map(|(_, name)| name)
}

/// Lower values are preferred. Bridges and Thunderbolt network interfaces
/// have Ethernet framing too; unavailable or unclassified paths rank last.
pub fn interface_priority(name: Option<&str>) -> u8 {
    let Some(name) = name else { return 2; };
    let root = std::path::Path::new("/sys/class/net").join(name);
    if root.join("wireless").exists() || root.join("phy80211").exists() { return 1; }
    if std::fs::read_to_string(root.join("type")).is_ok_and(|kind| kind.trim() == "1") { return 0; }
    2
}

pub fn address_priority(ip: Option<IpAddr>) -> u8 {
    interface_priority(ip.and_then(interface_for_address).as_deref())
}

/// A link-local destination must use a link-local source on that interface.
pub fn source_address(interface: &str, destination: IpAddr) -> Option<IpAddr> {
    interface_addresses().into_iter().filter(|(ip, name)| name == interface && ip.is_ipv4() == destination.is_ipv4())
        .map(|(ip, _)| ip).min_by_key(|ip| match (ip, destination) {
            (IpAddr::V4(source), IpAddr::V4(target)) => u8::from(source.is_link_local() != target.is_link_local()),
            (IpAddr::V6(source), IpAddr::V6(target)) => u8::from(source.is_unicast_link_local() != target.is_unicast_link_local()),
            _ => 2,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_address_resolves_to_loopback_interface() {
        assert_eq!(interface_for_address("127.0.0.1".parse().unwrap()).as_deref(), Some("lo"));
        assert_eq!(source_address("lo", "127.0.0.1".parse().unwrap()), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(interface_priority(Some("lo")), 2);
        assert!(source_address("missing-entanglo-interface", "192.0.2.1".parse().unwrap()).is_none());
    }
}
