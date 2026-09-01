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

`cargo check --workspace` and `cargo test --workspace` both pass as
of the initial commit (Rust stable via rustup, `pkg-config` +
`libudev-dev` + `libgtk-4-dev` + `libadwaita-1-dev` installed via
`apt`). 8 tests pass in `entanglo-core` (keymap table integrity,
modifier bitmask translation, protocol envelope roundtrip). The GTK4
app shell builds and its sidebar navigation is wired up
(`window.rs` swaps the content page via `pages::build_by_id` on row
selection).

## Known gaps in this scaffold (see ROADMAP.md for the phased plan)

- `net/coordinator.rs`, `net/trust_store.rs` are structural only — no
  real Secret Service wiring or safety-gate enforcement yet.
- `input/capture.rs`, `input/inject.rs` compile against real `evdev`
  0.12.2 APIs but are untested against actual hardware — no device
  has run this on a real `/dev/input`/`/dev/uinput` pair yet, only
  `cargo check`/`cargo test` (which don't touch either node). First
  real-device smoke test is Phase 1 work per `ROADMAP.md`.
- `features/*` are Phase 2 stubs that return "not yet implemented"
  errors or no-ops.
- Page widgets in `crates/entanglo-linux/src/pages/*.rs` are
  placeholder `Label`s — sidebar navigation works, but no page has
  its real UI yet.
- No CI workflow yet — add one so `cargo check --workspace` and
  `cargo test --workspace` run on every push (needs a runner image
  with `libgtk-4-dev`/`libadwaita-1-dev`, not just Rust).
