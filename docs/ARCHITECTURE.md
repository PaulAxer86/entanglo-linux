# Entanglo for Linux — Architecture (v0.1 scaffold)

## Layers

```
┌──────────────────────────────────────────────────────────┐
│  crates/entanglo-linux/    GTK4/libadwaita app shell,    │
│    src/pages/, src/window.rs   observes core state       │
├──────────────────────────────────────────────────────────┤
│  crates/entanglo-core/net/coordinator   glue: dispatches │
│    events between transport and input                     │
├──────────────────────────────────────────────────────────┤
│  crates/entanglo-core/net/     Discovery, Transport,      │
│    net/trust_store, features/  Trust, Clipboard, Files,   │
│                                 Screenshot, Network Quality│
├──────────────────────────────────────────────────────────┤
│  crates/entanglo-core/input/   evdev capture, uinput      │
│                                 injection, modifier state  │
├──────────────────────────────────────────────────────────┤
│  crates/entanglo-core/protocol/  Envelope + codec + all   │
│                                   payload structs          │
└──────────────────────────────────────────────────────────┘
```

This mirrors `entanglo-macos/docs/ARCHITECTURE.md`'s layer split
(Views / AppState / Services / Protocol / Models), with two
differences forced by the platform:

- **No AppKit-equivalent split.** GTK4 doesn't separate a
  "platform-agnostic core buildable without the IDE" story the way
  `Package.swift` does for the Mac's `Models`/`Protocol`/pure-logic
  `Services` — but `entanglo-core` still enforces that boundary as a
  *crate* boundary: it has zero GTK dependency and builds/tests on a
  headless CI runner with no display server at all.
- **Input is a first-class module of the core crate**, not a
  UI-adjacent service. On Mac, `InputCaptureService`/
  `InputInjectionService` sit under `Services/` because CGEventTap is
  itself a userspace API tied to the running session. On Linux,
  evdev/uinput are kernel interfaces with no UI dependency whatsoever
  — they belong in the core crate for the same reason `Protocol/`
  does: platform-agnostic in the "doesn't need GTK" sense, even though
  it's still Linux-specific in the "doesn't need Wayland/X11" sense.
  See `STACK.md` for why that's the whole point.

## Message flow

```
                +-------------------+
   Local input  | InputCaptureSvc   |
   on Controller| (evdev)           |
                +---------+---------+
                          |
                          v
                  InputEventMessage
                          |
              EntangloMessage::encode_payload
                          |
                          v
                +-------------------+
                | NetworkTransport  |  ---heartbeat--->
                | (tokio TcpStream) |
                +---------+---------+
                          |
                  TCP / mDNS LAN
                          |
                          v
                +-------------------+
                | NetworkTransport  |  on Receiver
                +---------+---------+
                          |
              EntangloMessage::decode
                          |
                          v
            +---------------------------+
            | Safety gates (all must    |
            | pass before injection):   |
            |  - device is Trusted      |
            |  - Receiver role enabled  |
            |  - input group / uinput   |
            |    node accessible        |
            |  - heartbeat alive        |
            |  - emergency-stop off     |
            +-------------+-------------+
                          |
                          v
                +-------------------+
                | InputInjectionSvc |
                | (/dev/uinput)     |
                +-------------------+
```

Same shape as the Mac's flow diagram (`entanglo-macos/docs/
ARCHITECTURE.md`), with "Accessibility granted" replaced by "input
group / uinput node accessible" — the Linux equivalent permission
gate, checked once at `InputInjectionService::open()` time rather
than via a runtime OS prompt (see `entanglo-macos/docs/
PERMISSIONS.md` for the Mac's prompt-based model this substitutes
for).

## Controller / Receiver flow

Same state machine as the Mac (`entanglo-macos/docs/ARCHITECTURE.md`
§"Controller / Receiver flow"): hotkey or edge-push swaps capture
mode, `releaseControl` and lost-heartbeat both fall back to local, an
emergency-stop is always armed. The one open design question specific
to this platform — how "push past the edge" detects the edge at all
under Wayland's input-scoping model — is tracked as a Phase 1 risk in
`ROADMAP.md`, not resolved here yet.

## File transfer flow

Identical wire behavior to Mac/Windows/Android
(`PROTOCOL.md` §5.8); the only Linux-specific detail is the
destination directory resolution in
`entanglo_core::features::file_transfer::FileTransferService::download_dir()`
(`$XDG_DOWNLOAD_DIR`, falling back to `~/Downloads`).
