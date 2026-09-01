//! Wire framing: 4-byte big-endian length prefix + JSON body.
//! See `PROTOCOL.md` §2.

use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::envelope::MAX_MESSAGE_BYTES;

pub type FramedTransport = Framed<TcpStream, LengthDelimitedCodec>;

/// Wrap a raw `TcpStream` in the length-delimited framing the
/// protocol uses: 4-byte big-endian length header, no adjustment,
/// capped at `MAX_MESSAGE_BYTES` (4 MiB). Both peers MUST drop the
/// connection if a frame claims to exceed that cap — `Framed`
/// enforces this for us by erroring the stream.
pub fn framed_transport(socket: TcpStream) -> FramedTransport {
    let codec = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .big_endian()
        .max_frame_length(MAX_MESSAGE_BYTES)
        .new_codec();
    Framed::new(socket, codec)
}
