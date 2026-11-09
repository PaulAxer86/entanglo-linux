# Ethernet path selection — 0.1.14

Task: LINUX-ETHERNET-001. Owner: Codex. Date: 2026-11-09.

## Problem and scope

Linux 0.1.13 refused all discovery connections after any session with a UUID
existed. Its first-contact path also discarded the IPv6 interface scope and
ignored all later announcements for that service. The desktop selected the
oldest connection for input and files. A TCP read error could leave a failed
session running instead of releasing it for reconnect.

## Behavior

- Cache each Bonjour interface/address separately. A removal on Wi-Fi does
  not erase Ethernet or the other address family.
- Resolve link-local IPv6 with its interface scope and bind discovered
  connections to a local address of the advertised interface.
- Reconcile trusted candidates every three seconds, preferring Ethernet
  framing (including a detected Thunderbolt network interface), then Wi-Fi.
- Establish the better path before releasing control on the old path.
  Existing sockets remain available for in-progress transfers; new input,
  files and clipboard traffic prefer the better path. A failed wired attempt
  leaves Wi-Fi usable. Same-priority paths do not trigger repeated dialing.
- Show the actual selected socket interface, including addresses assigned
  after app startup. Discovery metadata does not grant trust.
- End a session on TCP read errors so its state can be cleaned up.

## Verified local network

The local Ethernet link negotiated 1000 Mbit/s. Direct peer ICMP replies were
approximately 0.3 ms and TCP port 55555 was reachable on IPv4 and scoped IPv6.
No Thunderbolt controller/network interface was exposed by this Linux host;
this release does not claim to enable missing Thunderbolt hardware support.

## Validation and release

Core tests cover interface binding, duplicate suppression while hello is
pending, path upgrade/downgrade policy, scoped IPv6, and removal of one
interface without removing the remaining paths. Full workspace tests and
Debian 12 release packaging run in GitHub Actions before publication.

Acceptance: both Linux peers run 0.1.14, the selected session uses the direct
Ethernet addresses, and keyboard/mouse/file transfer work. Cable removal
must leave or restore Wi-Fi communication. Live acceptance is separate from
unit tests and packaging; record the observed result in HANDOVER.md.

## Rollback

Reinstall the previously verified 0.1.13 package from GitHub Releases and
restart Entanglo. Device identity, saved trust and edge assignments are
unchanged. Restore the previous website manifest if the release is withdrawn.
The dedicated NetworkManager profile is independent of the app; the original
wired profile remains available.

## Additional input fixes in this release

- LINUX-MEDIA-001: capture and inject the protocol's volume, mute, brightness,
  playback, seek and eject media keys. The virtual keyboard advertises those
  capabilities. Press, release and held-key repeat are preserved. Physical
  brightness still requires support from the receiving desktop/display driver.
- LINUX-RETURN-001: when the virtual remote cursor crosses its entry edge,
  return the local X11 cursor just inside the adjoining edge at the normalized
  position along it. Do not reveal its unrelated hidden position. Refresh
  root dimensions before restoring the cursor.
- LINUX-POINTER-SPEED-001: Input Sharing offers a persisted per-controller
  remote pointer multiplier (0.25–4.0), applied only to forwarded relative
  mouse motion before virtual-cursor tracking. Local movement and scrolling
  are unaffected. The default is 1.0; iMac1's Mighty Mouse will be tested at
  2.0 because its local libinput acceleration setting is 1.0 (maximum), which
  raw evdev capture bypasses. iMac2's Magic Mouse keeps its own setting.

The final local workspace tests and website build must pass after these
changes. Physical media keys, cursor return and mouse speed need live review.

## Local validation, 2026-11-09

- Rust 1.85.1, Debian 13: `cargo test --locked --workspace -- --test-threads=1`
  passed: 79 core tests, 4 protocol tests, 33 desktop tests; 116 total.
- Development libraries were extracted into the user's cache. Local tests used
  `PKG_CONFIG_SYSROOT_DIR`, `PKG_CONFIG_PATH` and `LIBRARY_PATH` pointing at
  `~/.cache/entanglo-build-deps/sysroot`; no system development packages were
  installed. `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=3`.
- `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update --workspace`
  changed no dependency versions; only workspace version fields changed.
- Website: `npm ci --no-audit --no-fund`, then `npm run build` passed (six pages).
- `git diff --check` and Python release-script syntax validation passed.
