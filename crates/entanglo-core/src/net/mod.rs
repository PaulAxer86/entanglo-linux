pub mod coordinator;
pub mod discovery;
pub mod transport;
pub mod trust_store;

pub use discovery::{DiscoveredPeer, DiscoveryService};
pub use transport::NetworkTransport;
pub use trust_store::TrustStore;

/// mDNS/Bonjour service type Entanglo advertises and browses.
/// See `PROTOCOL.md` §1.
pub const SERVICE_TYPE: &str = "_entanglo._tcp.local.";
