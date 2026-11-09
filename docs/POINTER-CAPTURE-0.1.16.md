# Local pointer confinement — 0.1.16

Task: LINUX-POINTER-CAPTURE-001. Owner: Codex. Date: 2026-11-09.

## Problem

The user observed both local and remote pointers moving while controlling
a peer. The X11 pointer-grab reply was treated as success regardless of its
status. Successful grabs also hid the pointer without constraining its
local position. The 0.1.15 version bump finished publishing during diagnosis;
it remains immutable.

## Change

Check GrabStatus, retry a rejected grab, and forward physical input only
after both pointer and keyboard capture succeed. Confine the core pointer
to a mapped 1x1 InputOnly window while raw evdev movement drives the remote
cursor. Unconfine before restoring the adjoining-edge return position.
On emergency stop or other local release, restore the pre-grab local
position. A closed X11 connection releases the grab and window automatically.

## Validation

The isolated X11 regression test creates a competing grab and verifies a
failed capture is never reported ready. It then verifies retry, local
immobility during attempted movement, natural return, emergency ungrab and
connection-loss release. Run it on an isolated server, not a user's desktop:

```sh
xvfb-run -a -s '-screen 0 1024x768x24' env ENTANGLO_X11_TEST=1 \
  cargo test --locked -p entanglo-linux isolated_x11_capture -- --ignored --test-threads=1
cargo test --locked --workspace -- --test-threads=1
```

The Debian 12 release workflow runs both commands. Physical testing on the
two iMacs follows installation; it is distinct from the isolated X11 test.

## Rollback

Restore the prior personal executable or use `/usr/bin/entanglo-linux`.
Saved identity, trust, edge assignments and speed settings are unchanged.

## Local evidence before publication

Rust 1.85.1 / Debian 13: the full workspace passed (79 core, 4 protocol,
34 desktop tests; 117 regular tests). The isolated X11 test passed separately
on Xvfb display :95. The cursor mask is explicitly cleared before cursor
creation; X11 pixmap allocation does not guarantee initialized pixels.
The confinement window is centered using current root geometry, away from
desktop hot corners. Website build and `git diff --check` passed.

## Release and deployment receipt

- Source release commit: `221e922d5bb640909e60acebb82378bacce6685f`.
- Release: https://github.com/PaulAxer86/entanglo-linux/releases/tag/v0.1.16
- Main workflow `42700103456` passed, including the dedicated Xvfb step.
- Website commit `e397291`: Cloudflare Pages reports success. The live
  download page links to 0.1.16 and the manifest exactly matches the release.
- Package: 2,753,876 bytes; SHA-256
  `caba4434790376dea2515f4162d2f3d98ec9d386261812fad30d12dc2071aba5`.
- Official executable on both iMacs: SHA-256
  `244ec856249a23cc4b47af9b3be22bf7b754a20eebd4037a60c9fa608c249f61`.
- Both programs were restarted with the official binary and report 0.1.16
  over their direct Ethernet connection. Trust, edge assignments and pointer
  speed preferences were preserved. The previous executable is saved as
  `~/Documents/entanglo-0.1.14-rollback/entanglo-before-pointer-fix`.
- Before final official-binary replacement, an actual iMac1 control session
  produced 26 captured X11 pointer samples, all at `(1280, 720)`. iMac2 logged
  trusted input, and iMac1 later logged natural return and pointer release.
  Evidence: `~/Documents/entanglo-0.1.14-rollback/pointer-confinement-live.json`.
- The 0.1.15 release remains immutable and recorded on the website; the newly
  reported pointer bug was corrected in 0.1.16 rather than replacing it.

This receipt changes documentation only; the tested release source is intact.
