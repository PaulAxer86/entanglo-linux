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
+ `libadwaita-1-dev` installed via `apt`). 12 tests pass in
`entanglo-core`:

- keymap table integrity + one roundtrip (`protocol::keymap`)
- modifier bitmask ↔ evdev-key translation (`input::modifiers`)
- protocol envelope roundtrip + version rejection + optional-field
  omission (`tests/protocol_roundtrip.rs`)
- trust-store fallback-file encrypt/decrypt roundtrip + tamper
  rejection + a real save-then-load through a temp directory
  (`net::trust_store::file_backend`)
- **the connection lifecycle end to end**: two `net::session::run_session`
  instances connected over a real loopback TCP socket exchange
  `hello`, pair with each other (auto-approved, exactly like a UI
  callback would after a user click), and forward one `inputEvent`
  from one side to the other only after trust is established
  (`net::session::tests::two_peers_pair_and_forward_input`) — this is
  `PROTOCOL.md` §6's full lifecycle running for real, not mocked.

The GTK4 app shell builds and its sidebar navigation is wired up
(`window.rs` swaps the content page via `pages::build_by_id` on row
selection).

## What's real vs. what's still a skeleton

`net::session` (hello/pairing/heartbeat/RTT/input-forwarding) and
`net::trust_store` (Secret Service + encrypted-file-fallback
persistence) are genuine, tested implementations — not placeholders.
Everything below is still a gap, tracked with ✅/🚧/⬜ status per item
in `ROADMAP.md`'s Phase 1 deliverables list:

- `net/coordinator.rs` is still a thin skeleton — it needs to actually
  spawn `session::run_session` per accepted/dialed connection and
  merge multiple peers' event streams for the UI. This is app-shell
  wiring, not protocol logic, so it's deferred until there's a
  display to exercise it against.
- `input/capture.rs`, `input/inject.rs` compile against real `evdev`
  0.12.2 APIs and their pure-logic pieces (keymap, modifier state) are
  unit-tested, but neither has run against an actual `/dev/input`/
  `/dev/uinput` device yet — this scaffolding machine has neither a
  display nor guaranteed input-device access. First real-device smoke
  test is next Phase 1 work.
- `features/*` are Phase 2 stubs that return "not yet implemented"
  errors or no-ops.
- Page widgets in `crates/entanglo-linux/src/pages/*.rs` are
  placeholder `Label`s — sidebar navigation works, but no page is
  wired to `net::session`'s `SessionEvent` stream yet (Pairing page
  approving/rejecting a real `PairingRequested` event is the most
  load-bearing one to build next).
- No CI workflow yet — add one so `cargo check --workspace` and
  `cargo test --workspace` run on every push (needs a runner image
  with `libgtk-4-dev`/`libadwaita-1-dev`, not just Rust).
