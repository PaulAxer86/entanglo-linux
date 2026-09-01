use serde::{Deserialize, Serialize};

/// See `PROTOCOL.md` §5.11 and §6 (error handling table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
}

pub mod codes {
    pub const UNKNOWN_PROTOCOL_VERSION: &str = "unknownProtocolVersion";
    pub const NOT_TRUSTED: &str = "notTrusted";
    pub const PERMISSION_MISSING: &str = "permissionMissing";
    pub const EMERGENCY_STOP_ACTIVE: &str = "emergencyStopActive";
}
