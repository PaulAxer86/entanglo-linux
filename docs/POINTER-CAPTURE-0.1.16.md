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
