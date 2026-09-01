//! Platform-agnostic core: wire protocol, transport, input translation.
//! See `PROTOCOL.md` at the repo root for the spec this crate implements.

pub mod features;
pub mod input;
pub mod logging;
pub mod net;
pub mod protocol;
pub mod update;
