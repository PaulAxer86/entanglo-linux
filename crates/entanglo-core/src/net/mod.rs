pub mod coordinator;
pub mod discovery;
pub mod session;
pub mod transport;
pub mod trust_store;

pub use coordinator::{ConnId, Coordinator, CoordinatorEvent, Direction};
pub use discovery::{DiscoveredPeer, DiscoveryService};
pub use session::{run_session, OutgoingMessage, SessionConfig, SessionEvent};
pub use transport::NetworkTransport;
pub use trust_store::TrustStore;

/// mDNS/Bonjour service type Entanglo advertises and browses.
/// See `PROTOCOL.md` §1.
pub const SERVICE_TYPE: &str = "_entanglo._tcp.local.";
