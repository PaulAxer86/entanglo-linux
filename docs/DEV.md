# Dev setup — entanglo-linux

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
- The Mac's `pairRequest` is (as of this writing) still pending real
  human approval on that machine — it did not auto-trust, which is
  the *correct* behavior matching the Mac's own pairing UI. If you're
  picking this scaffold back up: check whether that Mac still has a
  pending Entanglo pairing prompt, and either approve or dismiss it.
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

- The seven other pages (Input Sharing, Files, Print, Network, News &
  Updates, Settings, Logs) are still placeholder `Label`s.
- No edge-detection or emergency-stop UI yet — `Coordinator::
  set_active_receiver` exists and is reachable today only via a manual
  "Control this device" button on the Devices page.
- `features/*` are Phase 2 stubs that return "not yet implemented"
  errors or no-ops.
- No CI workflow yet — add one so `cargo check --workspace` and
  `cargo test --workspace` run on every push (needs a runner image
  with `libgtk-4-dev`/`libadwaita-1-dev`, not just Rust).
- Pairing page Accept/Reject buttons are wired but unexercised in this
  pass — no *incoming* pairRequest happened to arrive live (only
  outgoing ones, to Android and the Mac). Worth a deliberate test:
  have another Entanglo peer dial this device and confirm the Accept
  button actually calls back through to `session.rs`'s oneshot.
