use serde::{Deserialize, Serialize};

/// The `messageType` discriminator. See `PROTOCOL.md` §4.
///
/// Unknown values on the wire MUST NOT be treated as a hard error —
/// decode the envelope with `#[serde(other)]` handling upstream (or
/// simply drop the frame and keep the connection open) rather than
/// letting an unrecognized variant abort the whole read loop. §8
/// requires forward-compatibility with future message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    Hello,
    PairRequest,
    PairResponse,
    Heartbeat,
    InputEvent,
    ReleaseControl,
    ClipboardText,
    FileOffer,
    FileChunk,
    FileAck,
    UrlPush,
    ScreenshotRequest,
    ScreenshotResult,
    Error,
}
