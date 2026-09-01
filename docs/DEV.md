# Dev setup — entanglo-linux

## Handover status (read this first)

As of 2026-09-01: Phase 1 (see `ROADMAP.md`) is functionally complete
and has been exercised against **real, live peers on the LAN** — a
real Mac (`entanglo-macos` 0.1.58) and a real Android box
(`entanglo-android`), not just loopback tests. Trust, pairing,
heartbeats, edge-detection, and pointer grab/hide have all been
observed working live at some point in this session. That said, two
things need attention before calling this solid:

1. **A real safety bug was just fixed but not yet re-verified live.**
   `edge.rs`'s pointer-grab loop used to ignore emergency-stop state,
   so hitting Ctrl+Shift+Escape while controlling a peer left the
   local mouse grabbed/invisible with no in-app recovery — the
   developer had to `pkill -9 -f entanglo-linux` to get their own
   mouse back. The fix (`should_grab_pointer`, `edge.rs`) is
   committed and unit-tested (`cargo test --workspace` — 39 tests
   green), but **has not been re-tested against a real running app and
   a real peer since the fix landed**. Do that first: `cargo build -p
   entanglo-linux`, launch it, grab control of any trusted peer
   (edge-push or the Devices page "Control this device" button), hit
   Ctrl+Shift+Escape, and confirm the local cursor is immediately
   visible and movable again. See `ROADMAP.md`'s pointer-grab bullet
   for the full incident writeup.
2. **Real Mac control was never confirmed end-to-end with the pointer
   fix in place** — only Android control was confirmed live, and that
   was with the *buggy* grab code still active. Re-verify against the
   Mac once (1) above passes.

Everything else below this point (setup, build, test commands,
"verified state") is still accurate and was true at the time it was
written; it's kept as-is rather than restated.

## First-time setup on the Debian dev machine

```bash
# Rust toolchain (Debian's packaged rustc lags; use rustup for current stable)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# GTK4 + libadwaita dev headers
sudo apt install libgtk-4-dev libadwaita-1-dev build-essential pkg-config

# input group, so /dev/input and /dev/uinput are usable without root
sudo usermod -aG input "$USER"
# then log out and back in — group membership is read at login time
```

## Build & run

```bash
cargo run -p entanglo-linux
```

## Test

`entanglo-core` has zero GTK dependency and runs on any machine with
a Rust toolchain, no display server or `input`-group membership
needed for the pure protocol/codec tests:

```bash
cargo test -p entanglo-core
```

`crates/entanglo-core/src/protocol/keymap.rs` and
`crates/entanglo-core/src/input/modifiers.rs` have inline `#[cfg(test)]`
unit tests; `crates/entanglo-core/tests/protocol_roundtrip.rs` covers
the envelope/payload wire format end-to-end. Add captured Mac frames
to `tests/interop/mac_fixtures/` per `SKELETON.md` as Phase 1 interop
work.

## Package a `.deb`

```bash
cargo install cargo-deb   # once
cargo build --release -p entanglo-linux
cargo deb -p entanglo-linux
# -> target/debian/entanglo-linux_0.1.0_amd64.deb
```

## Verified state of this scaffold

`cargo check --workspace` and `cargo test --workspace` both pass
(Rust stable via rustup, `pkg-config` + `libudev-dev` + `libgtk-4-dev`
+ `libadwaita-1-dev` installed via `apt`). 14 tests pass in
`entanglo-core`:

- keymap table integrity + one roundtrip (`protocol::keymap`)
- modifier bitmask ↔ evdev-key translation (`input::modifiers`)
- protocol envelope roundtrip + version rejection + optional-field
  omission (`tests/protocol_roundtrip.rs`)
- trust-store fallback-file encrypt/decrypt roundtrip + tamper
  rejection + a real save-then-load through a temp directory
  (`net::trust_store::file_backend`)
- the connection lifecycle end to end over loopback TCP — hello,
  pairing, trust, one `inputEvent` forwarded only after trust
  (`net::session::tests::two_peers_pair_and_forward_input`)
- the same, through two full `Coordinator`s (listener + dial, not just
  a bare session) — `net::coordinator::tests::
  two_coordinators_connect_and_forward_input`
- a regression test for a real self-connection bug found live (below)
  — `net::session::tests::self_connection_is_rejected_before_pairing`

**Beyond the unit tests**, this scaffold's dev machine turned out to
have a real desktop session (Xorg via lightdm) and real network access
— so it was actually *run*, not just compiled, against real peers:

- The GTK window opened for real on the X11 display (confirmed with
  `xwininfo -root -tree`, not just "the binary didn't crash").
- mDNS discovery found two live Entanglo installs already on the
  LAN — an Android TV box (`entanglo-android`) and a Mac
  (`entanglo-macos` `0.1.58`) — and both `hello` exchanges completed
  and were logged with their real reported names/platforms/versions.
- The Android box **auto-accepted** the `pairRequest` this app sent
  it; the resulting trust was written to the real GNOME Keyring via
  Secret Service (not the file fallback) and correctly reloaded,
  still trusted, on a second run of the app.
- The Mac's `pairRequest` sat unanswered — turned out this is not a
  pending-approval state at all: reading `entanglo-macos`'s actual
  source shows v0.1.58 **never implements the wire pairRequest/
  pairResponse handshake** (confirmed by `grep`, zero hits outside the
  `MessageType` enum declaration). Its real trust model is local-only
  per side, no network negotiation. See `ROADMAP.md`'s Trust store
  entry for the full writeup and the fix
  (`Coordinator::trust_manually`, a "Trust" button on the Devices
  page). If you're picking this up fresh: there is no Mac-side
  approval to go find — use the Trust button (or
  `examples/manual_trust_smoke_test.rs` for a GTK-free check) instead.
- Found and fixed a real bug this way: mDNS's periodic
  re-announcements were causing the discovery loop to re-dial every
  peer (including this device's own advertisement) repeatedly, which
  meant re-sending `pairRequest` to the Mac over and over — a real
  annoyance for whoever's sitting at that Mac. Fixed with a
  per-service dial dedup (`app_state::spawn_discovery`) and a
  self-connection guard (`session::run_session`, regression-tested
  above). **If you add new discovery/dial logic, watch for this
  class of bug again** — anything that reacts to mDNS events needs to
  assume they repeat.
- `/dev/uinput` access was correctly refused (no `input` group
  membership on this run) and the app degraded gracefully — logged a
  clear warning, kept running with receiver capability simply off, no
  crash. Actual input injection/capture against real hardware is
  still unverified — that needs `input` group membership granted in
  this environment, an environment change rather than a code one.

## What's real vs. what's still a gap

`net::session`, `net::coordinator`, and `net::trust_store` are
genuine, tested implementations exercised both in isolation and live
against real peers — not placeholders. `Coordinator::enable_receiver`/
`enable_controller` wire `input::inject`/`input::capture` into the
connection lifecycle for real, though the actual hardware path is
unverified per above. Dashboard/Devices/Pairing pages are live GTK
widgets bound to `state::AppShared`, updated from real
`CoordinatorEvent`s. Everything below is still a gap, tracked with
✅/🚧/⬜ status per item in `ROADMAP.md`'s Phase 1 deliverables list:

- **This section is stale as of the first pass — updated 2026-09-01,
  see below.** Files, Print, and News & Updates remain placeholder
  pages (deliberately, per `ROADMAP.md` — real Phase 2 protocol/backend
  work, not just UI). Input Sharing, Network, Logs, and Settings are
  now real, live-data pages, not placeholders.
- Edge-detection + pointer grab/hide now exist for real
  (`crates/entanglo-linux/src/edge.rs`, X11 only — see `ROADMAP.md`'s
  Wayland risk note) and are reachable both via true edge-push and the
  Devices page's manual "Control this device" button. **Just fixed a
  real safety bug here** (emergency-stop wasn't releasing the grab) —
  see "Handover status" at the top of this file before trusting this
  fully.
- `features/network_quality.rs` and `logging.rs` are real, not stubs,
  and back the Network/Logs pages with live data. `features/clipboard`,
  `file_transfer`, `screenshot`, `url_push` are still Phase 2 stubs.
- No CI workflow yet — add one so `cargo check --workspace` and
  `cargo test --workspace` run on every push (needs a runner image
  with `libgtk-4-dev`/`libadwaita-1-dev`, not just Rust).
- Pairing page Accept/Reject buttons are wired but unexercised in this
  pass — no *incoming* pairRequest happened to arrive live (only
  outgoing ones, to Android and the Mac). Worth a deliberate test:
  have another Entanglo peer dial this device and confirm the Accept
  button actually calls back through to `session.rs`'s oneshot. Note
  also that the real Mac (0.1.58) never sends `pairRequest` at all —
  see the Trust store section of `ROADMAP.md` — so this path can only
  be exercised against Android or a future Mac build that implements
  the wire ceremony.
