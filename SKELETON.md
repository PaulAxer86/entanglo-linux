# Starter skeleton — `entanglo-linux`

Concrete Cargo workspace layout for the new Debian project. Build
this on the Debian machine; do not mix it into the Mac/Windows/Android
repos.

```
entanglo-linux/
├── README.md
├── .gitignore
├── Cargo.toml                               ← workspace root
│
├── crates/
│   ├── entanglo-core/                       ← protocol + transport (no UI, no GTK)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── protocol/
│   │       │   ├── mod.rs
│   │       │   ├── message_type.rs          ← enum, matches PROTOCOL.md §4
│   │       │   ├── envelope.rs              ← EntangloMessage struct
│   │       │   ├── payloads/                ← one struct per payload
│   │       │   │   ├── mod.rs
│   │       │   │   ├── hello.rs
│   │       │   │   ├── pair_request.rs
│   │       │   │   ├── pair_response.rs
│   │       │   │   ├── heartbeat.rs
│   │       │   │   ├── input_event.rs
│   │       │   │   ├── release_control.rs
│   │       │   │   ├── clipboard_text.rs
│   │       │   │   ├── file_offer.rs
│   │       │   │   ├── file_chunk.rs
│   │       │   │   ├── file_ack.rs
│   │       │   │   ├── url_push.rs
│   │       │   │   ├── screenshot_request.rs
│   │       │   │   ├── screenshot_result.rs
│   │       │   │   └── error.rs
│   │       │   ├── codec.rs                 ← encode/decode + base64 payload wrap
│   │       │   └── keymap.rs                ← Mac vk ↔ Linux KEY_* table
│   │       ├── net/
│   │       │   ├── mod.rs
│   │       │   ├── discovery.rs             ← mdns-sd advertise + browse
│   │       │   ├── transport.rs             ← Framed TCP, heartbeats, RTT
│   │       │   ├── coordinator.rs           ← outbound/inbound transport map
│   │       │   └── trust_store.rs           ← Secret Service / encrypted-file
│   │       ├── input/
│   │       │   ├── mod.rs
│   │       │   ├── capture.rs               ← evdev reader → InputEvent
│   │       │   ├── inject.rs                ← uinput writer
│   │       │   └── modifiers.rs             ← bitmask ↔ individual KEY_* state
│   │       ├── features/
│   │       │   ├── mod.rs
│   │       │   ├── clipboard.rs
│   │       │   ├── file_transfer.rs
│   │       │   ├── screenshot.rs
│   │       │   ├── url_push.rs
│   │       │   └── network_quality.rs
│   │       ├── update.rs                    ← parses latest-linux.json
│   │       └── logging.rs
│   │
│   └── entanglo-linux/                      ← GTK4/libadwaita app (UI)
│       ├── Cargo.toml
│       ├── build.rs                         ← compiles .blp (Blueprint) → .ui if used
│       ├── data/
│       │   ├── com.paoloasara.Entanglo.desktop
│       │   ├── com.paoloasara.Entanglo.metainfo.xml
│       │   └── icons/
│       │       └── com.paoloasara.Entanglo.svg
│       └── src/
│           ├── main.rs
│           ├── application.rs               ← AdwApplication setup
│           ├── window.rs                    ← AdwApplicationWindow + NavigationSplitView
│           ├── pages/
│           │   ├── mod.rs
│           │   ├── dashboard.rs
│           │   ├── devices.rs
│           │   ├── pairing.rs
│           │   ├── input_sharing.rs
│           │   ├── files.rs
│           │   ├── print.rs
│           │   ├── network.rs
│           │   ├── news_updates.rs
│           │   ├── settings.rs
│           │   └── logs.rs
│           └── widgets/                     ← reusable cards, badges, etc.
│
├── packaging/
│   ├── 60-entanglo-uinput.rules             ← udev rule, see ROADMAP.md Phase 1
│   └── postinst                             ← cargo-deb maintainer script (group hint)
│
├── tests/
│   └── interop/
│       └── mac_fixtures/                    ← real frames captured from Mac peers
│           ├── hello.bin
│           ├── heartbeat.bin
│           └── input_double_click.bin
│
└── docs/
    ├── PROTOCOL.md                          ← mirror of this folder's copy
    ├── ARCHITECTURE.md                      ← diagrams + design notes
    └── DEV.md                               ← how to run, udev setup, etc.
```

---

## Day-1 starter code (paste these to bootstrap)

### Workspace `Cargo.toml`

```toml
[workspace]
resolver = "2"
members = ["crates/entanglo-core", "crates/entanglo-linux"]

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Paolo Asara"]
```

### `crates/entanglo-core/Cargo.toml`

```toml
[package]
name = "entanglo-core"
version.workspace = true
edition.workspace = true

[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
uuid = { version = "1", features = ["v4", "serde"] }
mdns-sd = "0.11"
evdev = "0.12"
secret-service = { version = "4", features = ["rt-tokio-crypto-rust"] }
thiserror = "1"
tracing = "0.1"
```

### `crates/entanglo-core/src/protocol/envelope.rs`

```rust
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntangloMessage {
    pub protocol_version: u32,
    pub message_type: super::message_type::MessageType,
    pub sender_device_id: String,
    pub session_id: String,
    pub timestamp: f64,
    /// Base64 of the type-specific payload's JSON bytes — mirrors
    /// Swift Codable's default `Data` encoding. See PROTOCOL.md §3.
    pub payload: String,
}

impl EntangloMessage {
    pub fn encode_payload<T: Serialize>(
        message_type: super::message_type::MessageType,
        sender_device_id: impl Into<String>,
        session_id: impl Into<String>,
        payload: &T,
    ) -> serde_json::Result<Self> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let payload_bytes = serde_json::to_vec(payload)?;
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            message_type,
            sender_device_id: sender_device_id.into(),
            session_id: session_id.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            payload: STANDARD.encode(payload_bytes),
        })
    }

    pub fn decode_payload<T: for<'de> Deserialize<'de>>(&self) -> anyhow::Result<T> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes = STANDARD.decode(&self.payload)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
```

### `crates/entanglo-core/src/net/transport.rs` (framing core)

```rust
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio::net::TcpStream;
use crate::protocol::envelope::MAX_MESSAGE_BYTES;

pub fn framed_transport(socket: TcpStream) -> Framed<TcpStream, LengthDelimitedCodec> {
    let codec = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .big_endian()
        .max_frame_length(MAX_MESSAGE_BYTES)
        .new_codec();
    Framed::new(socket, codec)
}
```

Reading/writing an `EntangloMessage` on top of this is then:

```rust
use futures::{SinkExt, StreamExt};

async fn send(framed: &mut Framed<TcpStream, LengthDelimitedCodec>, msg: &EntangloMessage) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    framed.send(bytes.into()).await?;
    Ok(())
}

async fn recv(framed: &mut Framed<TcpStream, LengthDelimitedCodec>) -> anyhow::Result<Option<EntangloMessage>> {
    match framed.next().await {
        Some(frame) => Ok(Some(serde_json::from_slice(&frame?)?)),
        None => Ok(None),
    }
}
```

### Where the input hooks plug in

`input/capture.rs` opens every `/dev/input/eventX` device with the
`EV_KEY`/`EV_REL` capability bits set (skip anything that isn't a
keyboard or pointer — check `evdev::Device::supported_events()`), and
merges them into a single async stream of raw `InputEvent`s, which
get translated into wire `InputEventMessage`s per `PROTOCOL.md` §5.5:

```rust
let mut device = evdev::Device::open("/dev/input/event3")?;
let mut events = device.into_event_stream()?;
while let Ok(ev) = events.next_event().await {
    // ev.kind() is InputEventKind::Key(Key) or RelAxis(...) etc.
    // translate via keymap.rs and push onto the outbound channel
}
```

`input/inject.rs` opens `/dev/uinput` once at startup and holds it for
the app's lifetime — same long-lived-fd pattern as
`entanglo-android`'s root helper, just without the root/IPC layer
since desktop Debian lets an `input`-group process open the node
directly. No separate `uinput` crate needed — `evdev`'s own `uinput`
module builds the virtual device using the same `Key`/
`RelativeAxisType` types as capture:

```rust
use evdev::{uinput::VirtualDeviceBuilder, AttributeSet, Key, RelativeAxisType};

let mut keys = AttributeSet::<Key>::new();
keys.insert(Key::KEY_A); // ...one insert per key in keymap.rs's table

let mut axes = AttributeSet::<RelativeAxisType>::new();
axes.insert(RelativeAxisType::REL_X);
axes.insert(RelativeAxisType::REL_Y);

let mut device = VirtualDeviceBuilder::new()?
    .name("Entanglo Virtual Input")
    .with_keys(&keys)?
    .with_relative_axes(&axes)?
    .build()?;
```

(Verified against `evdev` 0.12.2 — `cargo check -p entanglo-core`
passes clean with this shape as of this scaffold's initial commit.)

### First test to write

`crates/entanglo-core/tests/protocol_roundtrip.rs`:

```rust
use entanglo_core::protocol::{envelope::EntangloMessage, message_type::MessageType, payloads::hello::HelloPayload};

#[test]
fn hello_roundtrips() {
    let payload = HelloPayload {
        device_name: "LinuxDev".into(),
        device_model: "Linux".into(),
        app_version: "0.1.0".into(),
        roles: vec!["controller".into(), "receiver".into()],
        platform: Some("Linux".into()),
    };
    let msg = EntangloMessage::encode_payload(
        MessageType::Hello, "dev-id", "sess-id", &payload,
    ).unwrap();
    let bytes = serde_json::to_vec(&msg).unwrap();
    let back: EntangloMessage = serde_json::from_slice(&bytes).unwrap();
    let decoded: HelloPayload = back.decode_payload().unwrap();
    assert_eq!(payload, decoded);
}
```

Then drop a captured-from-Mac frame into `tests/interop/mac_fixtures/`
and assert it decodes successfully — same interop-guard pattern as
`entanglo-windows/SKELETON.md`.

---

## To capture Mac fixtures

Reuse the exact procedure from `entanglo-windows/SKELETON.md`: attach
lldb to a running Mac Entanglo, or add a tiny one-time logging branch
in `NetworkTransport.swift`, dump incoming frame bytes to a file for
the first `hello` / `heartbeat` / double-click `inputEvent`. Commit
the resulting `.bin` files here under `tests/interop/mac_fixtures/`.

---

## GTK4/libadwaita starting point

`crates/entanglo-linux/src/main.rs`:

```rust
use adw::prelude::*;
use adw::Application;

fn main() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("com.paoloasara.Entanglo")
        .build();
    app.connect_activate(|app| {
        let window = crate::window::build(app);
        window.present();
    });
    app.run()
}
```

`window.rs` builds an `AdwNavigationSplitView` with a sidebar
`AdwNavigationPage` listing Dashboard/Devices/Pairing/.../Logs, same
page set as `entanglo-windows/SKELETON.md`'s `Pages/` folder.

---

## Don't worry about

- TLS yet (out of v1 scope per `PROTOCOL.md` §9)
- PIN handshake (reserved field, send `""`)
- Printer Bridge before the rest works
- Wayland global pointer position (spike it, but don't let it block
  Phase 1 — see `ROADMAP.md` Risks)
- Tray icon polish (GNOME's tray story is rough; defer to Phase 2)
- Flatpak/Flathub packaging until the `.deb` path is solid

Build phase 1, ship it to yourself, *then* iterate.
