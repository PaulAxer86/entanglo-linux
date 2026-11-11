# Handover — Entanglo for Linux

Last updated 2026-11-11. Read this first; `docs/DEV.md` holds the older history.

## 0.1.17 Bluetooth input and escape keys — 2026-11-11

Reported on iMac 1: the local mouse sometimes locked at the screen edge while
nothing moved on the other iMac, and only quitting Entanglo recovered it.
Cause: evdev devices were opened only at startup. iMac 1's Bluetooth mouse and
keyboard reconnect under new `/dev/input/event*` nodes (keyboard appeared
two minutes after login, mouse after a sleep), so X11 reached the edge and was
grabbed but no input was forwarded and the return could never be detected.

- Input devices are rescanned every second and reconnected ones are captured.
- While controlling a peer, Left Shift+Escape, Escape three times and
  Ctrl+Shift+Escape return control (`input/escape.rs`). They are also read
  from the X11 keyboard grab, so they work even when evdev lost the keyboard.
  Held Shift/Control are released on the peer. Ctrl+Shift+Escape with no
  peer under control still toggles Emergency Stop.
- If X11 delivers a click or key press during the grab that evdev did not
  see within 1.5 s, the grab is released automatically.
- Verified live by the user on iMac 1 with the test build before release.

Not Entanglo: keypad Enter acted as AltGr because the `gb(mac)` XKB variant
includes `level3(enter_switch)`; fixed on both iMacs with the XKB option
`keypad:legacy` in `~/.config/kxkbrc`.

## 0.1.16 pointer-capture fix deployed — 2026-11-09

Task LINUX-POINTER-CAPTURE-001: X11 grab status was ignored and the local
pointer was not confined. The fix requires successful mouse/keyboard grabs
before forwarding raw input and confines the local pointer to an invisible
InputOnly window. A real Xvfb test verifies denial/retry, immobility, return,
emergency release and X11 connection loss. See `docs/POINTER-CAPTURE-0.1.16.md`.

## 0.1.15 update release — 2026-11-09

Published the requested 0.1.15 update: Linux source `5df7df1`, website
`4600199`. The public manifest was verified as 0.1.15. While the build was
finishing, the user reported that the local pointer also moved during
remote control. The immutable 0.1.15 release is retained; 0.1.16 fixes this
newly reported issue.

## 0.1.14 deployed — 2026-11-09

Direct Ethernet selection, media keys, cursor return and adjustable remote
pointer speed are implemented;
see [the task, validation and rollback](docs/ETHERNET-0.1.14.md). Release
publication and both-machine installation are verified. The user still needs
to assess physical media keys, pointer return and Mighty Mouse sensitivity. No Thunderbolt interface is currently detected on
iMac1. Both iMacs now run Debian 13.

## Current state

- Published: **0.1.16**, Linux source commit `221e922`, website commit `e397291`.
  The requested 0.1.15 update was also published (`5df7df1`, website `4600199`)
  and is retained unchanged in release history.
- Both iMacs run the official 0.1.16 executable as a personal installation
  (`~/.local/bin/entanglo-linux`), selected by their user desktop launchers.
  iMac2's existing autostart was updated too. System 0.1.13 remains available
  for rollback. Device identity, trust and edge assignments were retained.
- Live: both peers report 0.1.16; the TCP session uses direct Ethernet.
- The local pointer is now confined and hidden during remote control. X11
  grab denial cannot enable forwarding. A real iMac1 control session yielded
  26 captured samples at the same local coordinates; iMac2 received trusted
  input and subsequent natural return released the local pointer.
- Follow-up acceptance: disabling Ethernet for 12 seconds produced a trusted
  Wi-Fi session; restoring the profile automatically established scoped IPv6
  Ethernet again. The existing Wi-Fi socket remains available as fallback.
  iMac1 remote pointer speed is 2.0; iMac2 keeps the default 1.0.
- Release/website CI passed: 117 regular workspace tests plus the separate
  real X11 confinement regression. The public 0.1.16 manifest exactly matches
  the verified package and the download page serves the new release.
- Brightness: iMac2 exposes `nv_backlight`; iMac1 currently has no backlight
  device or KDE brightness display. Media forwarding cannot supply missing
  display-driver support on iMac1.
- One `.deb`, built on Debian 12, runs on Debian 12 and 13 (amd64, X11).
- Working live: mouse, keyboard and clipboard in both directions, Linux as
  controller on all four screen edges, keypad, tray, start at login, verified
  updates, file reception from Mac, file sending to Mac.
- Website is at <https://entanglo.pages.dev>; release steps are in
  [docs/RELEASING.md](docs/RELEASING.md).

## Machines

| Name | System | Role |
| --- | --- | --- |
| iMac 2 | Debian 13, KDE, X11, small keyboard | Development machine |
| iMac 1 | Debian 13, KDE, X11, Apple A1016 and Mighty Mouse | Linux workstation |
| lenovo | Debian 12 laptop | Debian 12 test machine |
| SERVERP | Windows 11, monitor above the iMacs | Windows peer |

The two iMacs also have a direct cable, using link-local addressing.

## Next, in priority order

1. **Large files stall when sending from Linux.** Reported by the user on
   0.1.13; not diagnosed and the size threshold is unknown. Reproduce with a
   file of a few GB, then read `~/.local/state/entanglo/last.log` and the Mac
   log. Unverified places to look: heartbeats sharing the session writer with
   512 KiB chunks, and the receiver's heartbeat timeout during slow disk writes.
2. **Confirm live:** Linux to SERVERP, SERVERP to Linux (reception still not
   confirmed), and drag and drop into the Files page.
3. **Persist file history.** Rows are rebuilt from `Downloads/Entanglo` at
   startup, so sender names and sent files are lost on restart.
4. **Live input review:** confirm media keys, adjoining-edge cursor return and
   the per-controller pointer speed on both iMacs (implemented in 0.1.14).
5. **Resume interrupted transfers.** A dropped connection means sending again.
6. Measure Linux-to-Linux file throughput. Ethernet interruption/fallback and
   automatic wired recovery were verified live on 2026-11-09. Thunderbolt
   hardware is not currently detected on iMac1.

## Waiting on other apps

Prompts were given to the user for the Claude sessions on those machines. Their
status is unknown here, so check before relying on them.

- **Mac, fileAck:** `sendRawToPeer` looks up `outbound` by `senderDeviceId`, but
  `outbound` is keyed by Bonjour name, so the ack is dropped with "no outbound
  transport". Found by reading the Mac source. Until fixed, a Linux send shows
  `Sending` at 100% for 15 seconds, then `Sent` with an unconfirmed tooltip. A
  rejection from the Mac is lost the same way, so it also appears as `Sent`.
- **Mac, Devices page:** one row per device with capability and connection
  icons, from the same name versus UUID identity problem.
- **Windows:** relative moves, screen size and the Bonjour id.

## Pitfalls

- **No portal file dialogs.** The app gains the `input` group by re-executing
  through `sg`. `xdg-desktop-portal` then cannot read `/proc/PID` and shows
  nothing, with no error. That is why `pages/files.rs` uses
  `gtk::FileChooserDialog`. Reproduce with `sg input -c '<app>'`.
- **Minimums:** Rust 1.85, GTK 4.8, libadwaita 1.2. Newer APIs need a check
  against Debian 12. After a version bump, run
  `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update --workspace`
  so dependencies stay compatible with Rust 1.85.
- **Sent and received rows are separate concerns:** `recent_transfers` on the
  Dashboard deliberately shows incoming files only.
- Wire key codes are macOS virtual key codes. Keypad Enter is injected as the
  main Enter key because it acts as AltGr on the X11 `gb(mac)` layout.
- A second app instance panics while port 55555 is in use, which also matters
  when testing a build next to the installed app. Use a sandbox with its own
  network namespace.

## Known small gaps

The ISO key is unmapped. The Clear key (71) logs harmless warnings. Printing and
Wayland input reception are not implemented.
