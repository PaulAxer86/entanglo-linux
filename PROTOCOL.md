# Entanglo Wire Protocol — v1

This document is the **authoritative specification** of the Entanglo
peer-to-peer network protocol, extracted from the reference Swift
implementation (macOS). Any compatible client (Windows, Linux,
Android, iOS, …) that conforms to this spec can pair, hold a link,
and exchange every feature the Mac app supports today.

Protocol version: `1`
Document version: 2026-09-01 (matches Mac app `0.1.58`, Win `0.1.55`,
Android v0.1). Mirrors `entanglo-windows/PROTOCOL.md` with the
Windows-specific §1 and §7 sections replaced by Linux equivalents.

---

## 1. Discovery

Entanglo peers find each other via **multicast DNS / Bonjour** on the
local network. No central server, no internet required.

| Field | Value |
|---|---|
| Service type | `_entanglo._tcp` |
| Service name | The user-visible device name (e.g. `Debian Desktop`) |
| Domain | `local.` |
| Port | Dynamic — chosen by the OS, advertised via the SRV record |
| TXT record | None required today (reserved for future capability flags) |

Each running Entanglo instance simultaneously **advertises** its own
service and **browses** for peers' services. When a service is
resolved, the client extracts `<host, port>` and may attempt a TCP
connection if the peer is already trusted (see §3).

On Linux the recommended approach is a **pure-Rust mDNS responder**
(the `mdns-sd` crate) so the app has no runtime dependency on
`avahi-daemon` being installed or running — important since some
minimal Debian installs (headless, server, containers) don't ship
Avahi. If `avahi-daemon` is present, `mdns-sd` and it coexist fine
(both just speak standard mDNS on port 5353); no D-Bus integration is
required. The wire-level mDNS protocol is standard RFC 6762/6763,
same as Windows (`Windows.Networking.ServiceDiscovery.Dnssd`) and Mac
(`NetService`/Bonjour).

---

## 2. Transport framing

All peer traffic flows over **plain TCP** (no TLS today — security
relies on LAN trust + the pairing handshake; TLS is on the roadmap).

Every TCP message is a **single framed packet**:

```
+--------------------------+--------------------------+
|   4 bytes (big-endian)   |   N bytes UTF-8 JSON     |
|     length = N           |   EntangloMessage        |
+--------------------------+--------------------------+
```

- **Length prefix**: unsigned 32-bit big-endian integer, the byte
  length of the JSON body that follows. Reading the frame is a
  two-stage read: 4 bytes → parse length → read exactly `length`
  bytes.
- **Maximum frame size**: `4 * 1024 * 1024` bytes (4 MiB). Both sides
  MUST drop the connection if they see a length above this.
- **No keepalive at TCP level** — see heartbeats in §6.

In Rust this maps directly onto `tokio::net::TcpStream` +
`AsyncReadExt`/`AsyncWriteExt`, or a `tokio_util::codec::Framed` with
a custom `LengthDelimitedCodec` configured for a 4-byte big-endian
header (`length_field_type::u32`, `big_endian()`, no
length-adjustment, `max_frame_length(4 * 1024 * 1024)`).

---

## 3. Envelope

Every JSON body is an `EntangloMessage` envelope:

```json
{
  "protocolVersion": 1,
  "messageType": "hello",
  "senderDeviceId": "A56B8029-C69E-4D97-8589-99C81524093B",
  "sessionId": "5F2C9C0A-...",
  "timestamp": 1751120480.213,
  "payload": "<base64 of inner-JSON bytes>"
}
```

| Field | Type | Notes |
|---|---|---|
| `protocolVersion` | int | Must equal `1`. Receivers MUST drop frames with any other value. |
| `messageType` | string | One of the enum values in §4. Unknown values MUST be dropped. |
| `senderDeviceId` | string | Stable per-installation UUID (UUID v4 string). |
| `sessionId` | string | UUID v4 generated once per app run. Lets a peer detect crash/restart. |
| `timestamp` | number | Unix epoch seconds, fractional. Sender's wall clock — informational only, never used for correctness. |
| `payload` | string (base64) | The inner per-type payload, JSON-encoded then base64'd. Swift's `Data` Codable defaults to base64 when serialising to JSON — every other implementation MUST mirror that. |

**About the double-encoding** (envelope JSON wraps base64-JSON): this
is a Swift `Codable` quirk that's part of the wire today. To stay
compatible:

1. Serialize the typed payload struct to UTF-8 JSON → `payloadBytes`.
2. Base64-encode `payloadBytes` (standard base64, no URL-safe).
3. Put the base64 string into the envelope's `payload` field.
4. Serialize the envelope to JSON.

In Rust: `serde_json::to_vec(&payload)` → `base64::engine::general_purpose::STANDARD.encode(...)`
→ set on the envelope struct → `serde_json::to_vec(&envelope)`.

A future v2 of the protocol may flatten this; v1 clients must do it.

---

## 4. Message types

The `messageType` discriminator. Each message has a typed payload
struct decoded from `EntangloMessage.payload`.

| Type | Direction | Payload struct |
|---|---|---|
| `hello` | both | `HelloPayload` |
| `pairRequest` | requester → responder | `PairRequestPayload` |
| `pairResponse` | responder → requester | `PairResponsePayload` |
| `heartbeat` | both | `HeartbeatMessage` |
| `inputEvent` | controller → receiver | `InputEventMessage` |
| `releaseControl` | receiver → controller | `ReleaseControlPayload` |
| `clipboardText` | both | `ClipboardTextPayload` |
| `fileOffer` | sender → receiver | `FileOfferPayload` |
| `fileChunk` | sender → receiver | `FileChunkPayload` |
| `fileAck` | receiver → sender | `FileAckPayload` |
| `urlPush` | controller → peer | `UrlPushPayload` |
| `screenshotRequest` | requester → target | `ScreenshotRequestPayload` |
| `screenshotResult` | target → requester | `ScreenshotResultPayload` |
| `error` | both | `ErrorPayload` |

---

## 5. Payload schemas

All payloads are JSON objects. Unknown extra fields MUST be ignored
(forward compatibility). Optional fields are marked `?`.

### 5.1 `HelloPayload`

Sent immediately after the TCP connect, before any other traffic.

```json
{
  "deviceName": "Debian Desktop",
  "deviceModel": "Linux",
  "appVersion": "0.1.0",
  "roles": ["controller", "receiver"],
  "platform": "Linux"
}
```

`roles` is an array of one or more of `"controller"` or `"receiver"`.

`platform` is optional. Coarse OS identifier — `"macOS"`,
`"Windows"`, `"Linux"`, … — so peers can branch on OS without parsing
`deviceModel`. The Linux client MUST send `"Linux"`.

### 5.2 `PairRequestPayload`

```json
{
  "requesterDeviceId": "A56B8029-...",
  "requesterDeviceName": "Debian Desktop",
  "pinHash": ""
}
```

> **Note on `pinHash`**: the field is reserved. In the current Mac
> implementation pairing is confirmed by the user via the Pairing UI
> on the *responder* side; no PIN is challenged on the wire. Send
> `""` to interop with v1 peers. Responders MUST NOT reject empty
> `pinHash` in v1.

### 5.3 `PairResponsePayload`

```json
{
  "accepted": true,
  "trustedDeviceId": "A56B8029-...",
  "rejectionReason": null
}
```

If `accepted == false`, `trustedDeviceId` is null and
`rejectionReason` is a short human string.

### 5.4 `HeartbeatMessage`

Sent every **1.0 s** while a transport is open, both directions.

```json
{
  "sequence": 142,
  "lastRttMs": 1.8,
  "sentAtMs": 27429103.4,
  "echoSentAtMs": 27429102.6
}
```

| Field | Notes |
|---|---|
| `sequence` | monotonic counter per side |
| `lastRttMs?` | last computed round-trip, ms — informational |
| `sentAtMs?` | sender's monotonic clock reading (ms) at send time. **Opaque** — the peer never interprets it, only echoes. |
| `echoSentAtMs?` | the most recent `sentAtMs` this side saw from the peer. Used by the peer to compute `RTT = nowMs() − echoSentAtMs`. |

RTT computation needs **no clock sync** between machines: each side
reads its own monotonic clock. On Linux use `std::time::Instant` (or
`tokio::time::Instant`) for the monotonic source.

If two consecutive heartbeats are missed (no traffic for ≥ 2 s) the
controller MUST stop forwarding input and mark the link unhealthy.

### 5.5 `InputEventMessage`

Sent only by the side currently controlling the cursor. **Never**
contains Unicode text — only low-level key codes and modifier
bitmasks, so logs and packet dumps don't leak what the user typed.

```json
{
  "kind": "mouseMove",
  "x": 1042.0,
  "y": 318.0,
  "deltaX": null,
  "deltaY": null,
  "button": null,
  "keyCode": null,
  "mediaKey": null,
  "modifierFlags": 0,
  "pressed": null,
  "clickState": null
}
```

`kind` values: `"mouseMove" | "mouseDown" | "mouseUp" | "scroll" |
"keyDown" | "keyUp" | "mediaKey"`.

Field semantics by kind:

| Kind | Required fields | Notes |
|---|---|---|
| `mouseMove` | `x`,`y` **or** `deltaX`,`deltaY` | Absolute coords are receiver-screen points (logical pixels). |
| `mouseDown` / `mouseUp` | `button`, `pressed`, `clickState?` | `button` 0 = primary, 1 = secondary, 2 = middle. `clickState` 1=single, 2=double, 3=triple. **Receiver MUST treat missing `clickState` as 1** (older peers omit it). |
| `scroll` | `deltaX` and/or `deltaY` | Line-scaled deltas (≈ pixels, divide by ~10 for "ticks"). |
| `keyDown` / `keyUp` | `keyCode` | `keyCode` is the **macOS virtual key code** (HIToolbox/Events.h). Cross-platform implementations MUST translate to/from their native code. A mapping table is provided in §7. |
| `mediaKey` | `mediaKey`, `pressed` | `mediaKey` string enum: see below. |

`mediaKey` enum: `"volumeUp" | "volumeDown" | "mute" | "brightnessUp"
| "brightnessDown" | "playPause" | "next" | "previous" | "fastForward"
| "rewind" | "eject"`.

`modifierFlags` is a `UInt64` bitmask using **macOS CGEventFlags**
values:

| Flag | Bit |
|---|---|
| Caps Lock | `1 << 16` (`0x10000`) |
| Shift | `1 << 17` (`0x20000`) |
| Control | `1 << 18` (`0x40000`) |
| Option (Alt) | `1 << 19` (`0x80000`) |
| Command (Meta/Win/Super) | `1 << 20` (`0x100000`) |
| Numeric pad | `1 << 21` (`0x200000`) |
| Help | `1 << 22` (`0x400000`) |
| Function (Fn) | `1 << 23` (`0x800000`) |

**Linux has no native "modifier bitmask" concept** — evdev/X11/
Wayland all model modifiers as ordinary key-down/key-up events for
`KEY_LEFTSHIFT`, `KEY_LEFTCTRL`, etc. The Linux client MUST therefore:

- **On capture (controller role)**: maintain its own modifier state by
  watching `KEY_LEFTSHIFT`/`KEY_RIGHTSHIFT`/`KEY_LEFTCTRL`/… key
  events from evdev, and compute `modifierFlags` from that state to
  attach to every outgoing `mouseMove`/`mouseDown`/scroll/etc. frame.
- **On injection (receiver role)**: before injecting a non-modifier
  event, diff the incoming `modifierFlags` against the uinput
  device's currently-held modifier keys and synthesize the missing
  `KEY_*` down/up events first (uinput has no bitmask API — it's
  individual key events all the way down, same principle as
  `SendInput` on Windows or `CGEventPost` on Mac, just more manual).

### 5.6 `ReleaseControlPayload`

Sent by the receiver to the controller when the receiver detects
local input (its user touched its own mouse/keyboard). The controller
must immediately stop sending `inputEvent`s and hand the cursor back.

```json
{ "reason": "local_input" }
```

### 5.7 `ClipboardTextPayload`

```json
{ "text": "hello from the other machine" }
```

UTF-8. Truncation policy is sender-side (Mac currently caps at
~1 MiB).

### 5.8 File transfer (`fileOffer`, `fileChunk`, `fileAck`)

Three-message flow:

1. Sender → receiver: `FileOfferPayload`
   ```json
   {
     "transferId": "ce3e6d3a-...",
     "name": "report.pdf",
     "sizeBytes": 184320,
     "totalChunks": 12,
     "chunkSize": 16384,
     "kind": null
   }
   ```
   `kind` is an optional discriminator. Today only `"printJob"` is
   used (Printer Bridge). Receivers MUST treat unknown `kind` as a
   regular file drop.
2. Sender streams `FileChunkPayload` messages in order. `data` is a
   base64-encoded byte slice of exactly `chunkSize` bytes (last chunk
   may be smaller). `sequence` is 0-indexed.
3. After the last chunk the receiver sends `FileAckPayload`:
   ```json
   { "transferId": "...", "ok": true, "failureReason": null }
   ```

The receiver writes the assembled file to `~/Downloads` by default
(`$XDG_DOWNLOAD_DIR` if set, else `~/Downloads`, matching the XDG user
dirs spec). For `kind: "printJob"` it spools to the printer instead.

### 5.9 `UrlPushPayload`

```json
{ "url": "https://example.com" }
```

Receiver opens the URL in the default browser (`xdg-open` on Linux,
matching `NSWorkspace.shared.open` on Mac and `ShellExecute`/
`Process.Start` on Windows). Receiver MUST sanity-check the URL and
refuse non-`http(s)`/`file` schemes unless explicitly enabled.

### 5.10 `ScreenshotRequestPayload` / `ScreenshotResultPayload`

```json
// request
{ "requestId": "9d20...." }

// result (success)
{
  "requestId": "9d20....",
  "pngData": "<base64 PNG>",
  "error": null
}

// result (failure)
{
  "requestId": "9d20....",
  "pngData": null,
  "error": "capture failed — screenshot portal denied"
}
```

Receiver captures the main display, scales the long side down to
2048 px, encodes as PNG, returns inline. **Capture target on Linux**:
`org.freedesktop.portal.Screenshot` via `xdg-desktop-portal` (works on
both GNOME/Wayland and KDE/Wayland, and falls back through the portal
on X11 too) — see `ashpd` crate. Direct `XGetImage` is an X11-only
fallback for sessions without a portal.

**Timeout**: the requester MUST set its own deadline (Mac uses 8 s)
and surface an error if no result arrives. The target SHOULD never
block forever — the portal call in particular must be wrapped in a
timeout since it waits on a user consent dialog.

### 5.11 `ErrorPayload`

Generic out-of-band error report.

```json
{ "code": "PROTOCOL_VERSION", "message": "Unsupported protocol version 2" }
```

---

## 6. Connection lifecycle

```
┌─────────────────────┐
│  Bonjour discover   │
└──────────┬──────────┘
           │
           v
┌─────────────────────┐
│  TCP connect        │
└──────────┬──────────┘
           │
           v
┌─────────────────────┐
│  send `hello`       │ ← both sides, independently
│  receive `hello`    │
└──────────┬──────────┘
           │
           v
        already           ┌─────────────────────┐
        trusted? ── no ──>│ pairRequest /       │
           │              │ pairResponse        │
          yes             └──────────┬──────────┘
           │                         │
           └─────────────┬───────────┘
                         v
              ┌─────────────────────┐
              │  heartbeat 1 Hz     │
              │  + feature traffic  │
              └─────────────────────┘
```

The Mac listener accepts incoming connections on the advertised port.
A device may simultaneously hold an outbound connection (it
initiated) and an inbound connection (the peer initiated) to the same
peer. The controller-side direction is whichever side is sending
`inputEvent`s.

---

## 7. Virtual key code mapping (Linux evdev ↔ macOS)

Mac sends `keyCode` as a **macOS HIToolbox virtual key code**. A
Linux client must convert its `evdev` key code (from
`struct input_event { .code }`, i.e. the `KEY_*` constants in
`<linux/input-event-codes.h>`) to the Mac equivalent before sending
as a controller, and convert incoming Mac codes back to `KEY_*` before
writing `EV_KEY` events to `/dev/uinput` as a receiver.

A minimal mapping (extend as needed — full table is ~120 entries):

| Key | Mac code (dec) | Linux `KEY_*` (dec) |
|---|---|---|
| A | 0 | `KEY_A` (30) |
| S | 1 | `KEY_S` (31) |
| Return | 36 | `KEY_ENTER` (28) |
| Tab | 48 | `KEY_TAB` (15) |
| Space | 49 | `KEY_SPACE` (57) |
| Delete (backspace) | 51 | `KEY_BACKSPACE` (14) |
| Escape | 53 | `KEY_ESC` (1) |
| Command (left) | 55 | `KEY_LEFTMETA` (125) |
| Shift (left) | 56 | `KEY_LEFTSHIFT` (42) |
| Caps Lock | 57 | `KEY_CAPSLOCK` (58) |
| Option (left) | 58 | `KEY_LEFTALT` (56) |
| Control (left) | 59 | `KEY_LEFTCTRL` (29) |
| Left | 123 | `KEY_LEFT` (105) |
| Right | 124 | `KEY_RIGHT` (106) |
| Down | 125 | `KEY_DOWN` (108) |
| Up | 126 | `KEY_UP` (103) |
| F1 | 122 | `KEY_F1` (59) |
| F2 | 120 | `KEY_F2` (60) |

For the full table, the reference source on macOS is
`<HIToolbox/Events.h>`; on Linux it's
[`linux/input-event-codes.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/input-event-codes.h).
Recommendation: ship a single `keymap.rs` table (a `const [(u16, u16); N]`
or two `phf` maps) covering all keys of a standard US ANSI keyboard.
Non-ANSI layouts inherit OS-level remapping — evdev codes are
physical-position codes, not layout-aware, exactly like Mac's virtual
key codes, so this property holds symmetrically on both ends.

Mouse buttons map to `BTN_LEFT` (0x110), `BTN_RIGHT` (0x111),
`BTN_MIDDLE` (0x112) for `button` 0/1/2 respectively.

Media keys (`mediaKey` enum, §5.5) map to
`KEY_VOLUMEUP`/`KEY_VOLUMEDOWN`/`KEY_MUTE`/`KEY_BRIGHTNESSUP`/
`KEY_BRIGHTNESSDOWN`/`KEY_PLAYPAUSE`/`KEY_NEXTSONG`/`KEY_PREVIOUSSONG`/
`KEY_FASTFORWARD`/`KEY_REWIND`/`KEY_EJECTCD`.

---

## 8. Forward-compat rules

Any compliant client MUST:

- Reject frames with `protocolVersion != 1`.
- Reject frames longer than 4 MiB.
- Ignore unknown fields on payload objects.
- Ignore unknown `mediaKey` values.
- Ignore unknown `messageType` values without closing the connection
  (just log + skip the frame).
- Treat absent optional fields (e.g. `clickState`, `kind`) per the
  defaults specified above.

These rules let v1.1 add new message types or fields without breaking
v1 peers.

> **Mac status**: enforced from 0.1.38. Earlier Mac builds (≤ 0.1.37)
> would log a generic decode error and drop the individual frame on
> an unknown `messageType` or `mediaKey`, but the connection stayed
> open thanks to a catch-all in `NetworkTransport.handlePayload`.
> From 0.1.38 the codec returns a distinct `unknownMessageType` error
> and `InputEventMessage` decodes unknown `mediaKey` to nil, matching
> the spec exactly.

---

## 9. Security posture (v1)

- **No TLS** on the wire. Trust model = "if you can reach the LAN you
  can sniff and inject Entanglo traffic." Adequate for home LANs
  only.
- **Trust store** is local: trusted device IDs and their friendly
  names. On Mac it lives in Keychain; on Windows, DPAPI. On Linux,
  use the **Secret Service D-Bus API** (GNOME Keyring / KWallet both
  implement it) via the `secret-service` crate to encrypt a small
  JSON blob, keyed the same as the Mac/Windows trust store. Fall back
  to a file under `$XDG_DATA_HOME/entanglo/trust.json` encrypted with
  a key derived from `/etc/machine-id` (via `ring`/HKDF) on headless
  systems with no Secret Service daemon running (rare on desktop
  Debian, common on a Debian server box).
- **Pairing UX** is the security barrier: the user explicitly
  approves each peer once on the receiver-side dialog. After that the
  device ID is trusted forever (until revoked from the Pairing view).
- **PIN challenge**: reserved (`pinHash` field). Not enforced in v1.

v2 will add TLS via a self-signed cert per device, pinned at pair
time. Design TBD.

---

## 10. Concrete examples

A complete `hello` frame, byte-by-byte:

```
   00 00 01 47          ← length = 327
   { "protocolVersion": 1,
     "messageType": "hello",
     "senderDeviceId": "A56B8029-C69E-4D97-8589-99C81524093B",
     "sessionId":      "5F2C9C0A-AA31-4D80-BCEA-2C7B0E0D0AFB",
     "timestamp": 1751120480.213,
     "payload": "eyJkZXZpY2VOYW1lIjoiaU1hYyBQcm8iLCJkZXZpY2VNb2RlbCI6ImlNYWMyMCwxIiwiYXBwVmVyc2lvbiI6IjAuMS4zNyIsInJvbGVzIjpbImNvbnRyb2xsZXIiLCJyZWNlaXZlciJdfQ=="
   }
```

The base64 payload decodes to:

```json
{"deviceName":"iMac Pro","deviceModel":"iMac20,1","appVersion":"0.1.37","roles":["controller","receiver"]}
```

---

## 11. Reference sources

If anything in this document is ambiguous, the reference sources are
the ground truth:

- `entanglo-macos/Entanglo/Protocol/EntangloMessage.swift` — envelope + payload structs
- `entanglo-macos/Entanglo/Protocol/MessageCodec.swift` — framing rules
- `entanglo-macos/Entanglo/Protocol/InputEventMessage.swift` — input events
- `entanglo-macos/Entanglo/Protocol/HeartbeatMessage.swift` — heartbeat + RTT
- `entanglo-macos/Entanglo/Services/DiscoveryService.swift` — Bonjour
- `entanglo-macos/Entanglo/Services/NetworkTransport.swift` — TCP framing
- `entanglo-android/app/src/main/kotlin/.../net/ProtocolMessages.kt` +
  `MessageCodec.kt` — a second-language (Kotlin) implementation to
  diff against if the Swift source is ambiguous on a JSON edge case
- `entanglo-android/app/src/main/cpp/uinput.c` — working `/dev/uinput`
  injection code; the Linux uinput approach in this document is the
  desktop evolution of this exact file

Any change to the Swift sources in a future Mac release will be
accompanied by a bump of this document's date and (if breaking) the
`protocolVersion`.
