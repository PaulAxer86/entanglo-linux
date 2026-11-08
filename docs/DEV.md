# Dev setup — entanglo-linux

## Handover status (read this first)

### 2026-11-08 — portable Linux 0.1.1 release and publication

All fixes are included in the versioned application/package, not a host patch.
The listener uses stable port 55555. When a trusted controller is missing,
Bonjour is withdrawn/re-announced under the same identity to prompt a retry.
The last controller that actually sent trusted input is persisted as a
preference, independently from trust, so a connected receiver (e.g. Windows)
does not mask the missing controller. Reconnect is also available on Dashboard.
Receiver mode now browses without auto-dialing: Devices shows discovered peers
waiting for a protocol connection separately from connected/identified peers.
Opening another copy presents the existing window before creating a backend.

News & Updates checks the HTTPS Linux manifest at launch and hourly, verifies
size/SHA-256 and Debian metadata, then installs in the user account or through
polkit for a system install. Version 0.1.1 is built/tested on Debian 13 in CI.
GitHub publishes the artifact; the website sync workflow mirrors verified
releases into its existing Cloudflare deployment. See docs/RELEASING.md.
No machine addresses or controller identities are hard-coded.

### 2026-11-08 — live macOS-style Dashboard and rabbit icon

Replaced the identity-only Dashboard with a live status banner, This Device,
Network Quality, Pairing, Recent Transfers and Quick Actions. Status uses
actual backend permissions, emergency stop, controller ownership and heartbeat
health. Per-connection local/remote socket addresses identify the active
interface against the local interface inventory, rather than assuming Ethernet
from cable presence. Refresh interval is 500 ms. Quick Actions open received
files, toggle plain-text clipboard and pause/resume input. Latest received
files are scanned from the download folder on startup for persistent history.

The displayed latency now uses Linux TCP_INFO smoothed RTT rather than
heartbeat echo timing, which included the peer’s 1-second scheduling delay
and falsely labelled a healthy local connection unstable. A loopback test
checks RTT availability without waiting for a heartbeat. Default window size
is 1120x860 so the overview fits with quick actions visible.

The original Mac rabbit PNG assets (16 through 1024 pixels) are installed in
hicolor for the app/window/sidebar/header/desktop entry and Debian package.
No new icon was generated. 71 tests pass including TCP RTT sampling. All 11 symbolic icon names and the
rabbit icon were checked against the active GTK theme; missing icons replaced.
New build and package revision 8 prepared; previous
personal binary preserved for rollback.

### 2026-11-08 — unified startup, reconnect and file receiver

Direct executable launches previously bypassed the wrapper: they lost input
permissions and enabled unwanted controller mode. startup::prepare now handles
receiver-only defaults, input-group re-exec and persistent logs in the binary.
Controller mode requires explicit ENTANGLO_ENABLE_CONTROLLER=1. Normal launches
no longer dial discovered controllers; they advertise and await inbound peers.
Discovery is retained for the app lifetime and explicitly unregisters on quit.
A real close/reopen test kept the same device ID, reloaded trust, accepted a
single Mac connection and logged received input. This reduces the Mac-side
service-name vs hello-UUID duplicate-device behavior without modifying the Mac.

Implemented streaming fileOffer/fileChunk reception and fileAck replies inside
the trust-gated session. Files save to XDG Downloads/Entanglo via unique partial
files, exact sequence/size checks, durable flush and no-overwrite publication.
Interrupted or invalid transfers delete partials. Existing names get numbered
suffixes. Printer jobs are explicitly rejected. Files page shows incoming
progress, failures, destinations and a folder button; Devices has a file icon.
70 full workspace tests pass, including real TCP framing of binary chunks and
rejection of untrusted offers. Live Mac file-send check passed: screenshot PNG (158254 bytes) received at 10:25:49 and saved under Downloads/Entanglo; success ack sent after publication.

Listener now accepts IPv6 (IPv4 fallback if unavailable) to support the new
Ethernet link. On this Debian, bindv6only=0 permits mapped IPv4 as well.
The cable interface has only link-local IPv6; the observed Mac session was
still on Wi-Fi global IPv6. No interface, DHCP or routing settings changed.
Package revision 7 is reproducible with scripts/package-debian.py.

### 2026-11-08 — English UI and normal window lifecycle

All application UI strings, tooltips, launch entries and installer messages
are now in English. User preference for English project content is recorded
in /home/pasura/AGENTS.md. Added AdwHeaderBar in AdwToolbarView with explicit
minimize/maximize/close controls. Close quits the app; Ctrl+Q invokes app.quit.
Shutdown activates emergency stop before exit. Activation presents an existing
window instead of creating another one. Dashboard roles reflect advertised
roles instead of claiming controller mode in a receiver-only run.

64 workspace tests pass. Actual X11 checks verified minimize to Iconic state,
restore to Normal state, and WM_DELETE_WINDOW -> process exit with shutdown
log. Personal binary and package revision 6 updated, previous binary retained
for rollback. The app is reopened in receiver-only mode after verification.

### 2026-11-08 — one row per physical device

Devices now groups live sessions by device ID, preserving separate peers
with the same name and unidentified sessions. Two regression tests cover
this behavior. Device identity, platform and authorization appear once;
icons show local authorization, mouse/keyboard receive availability and
connectivity. Availability uses peer-advertised roles and real local input
backend status. Raw inbound/outbound connection details live in tooltips.
Controls are offered only when the local backend and peer roles support
them. Existing edge-assigned connection is preserved during grouping.
Personal executable and package revision 5 updated; receiver-only launcher
restarted and old binary preserved. Build and diff checks pass.

### 2026-11-08 — compact Devices rows

Devices now uses horizontal rows with separators rather than stacked cards.
Each row preserves PeerConnected.direction and labels the connection Ingresso
(opened by the peer) or Uscita (opened locally). Matching device IDs are
explicitly labelled as the same device with multiple connections. Direction
is the TCP initiation direction, not the direction of input sharing.
Receiver-only mode omits outbound control/edge actions. Build and whitespace
checks passed; no protocol or input behavior changed. Personal executable
and distributable package updated; previous binary preserved for rollback.

### 2026-11-08 — Mac -> Debian confirmed live

User confirmed control now works after restarting the actual desktop app
with `sg input` and receiver-only mode, then trusting the Mac in Devices.
The earlier process had TCP connections but lacked the input group; uinput
was inaccessible. Verified `receiver ready`, macOS 0.1.58 Hello, explicit
untrusted-input rejection, manual trust on both sessions and finally
`trusted peer input received; receiver control started` in the live log.
The launcher now reapplies the existing input membership per-process using
sg when the desktop session has not yet inherited it. No root permission
or logout is required when the user is already a group member.

Full workspace tests ran on the host: 61 passed, including socket tests.
The live session also exposed failed Secret Service writes falling back to
the encrypted file. Startup now treats an existing fallback as authoritative
so a readable but stale/empty keyring does not discard the saved trust.
This persistence change is installed on disk for the next start; the working
process is intentionally left running. Group/udev configuration was not
changed in this turn. Stop command remains ~/.local/bin/entanglo-stop.
Rollback binaries are preserved under ~/.local/bin/entanglo-linux.before-*.


### 2026-11-08 — shared trust across inbound/outbound sessions

Mac and Linux may simultaneously dial each other. Each Linux session cached
its own trusted boolean, so granting trust via one Devices row left the
other connection untrusted until reconnect. Incoming frames now refresh the
trust gate from the shared device store. A socket-free regression test covers
external trust, no duplicate Trusted events and revocation. Input rejects
for absent trust and decode failure now emit a one-time diagnostic (no key
content). The receiver launcher persists info logs in
~/.local/state/entanglo/ultimo.log. Live cause still unconfirmed until logs
from the actual desktop run are available.

### 2026-11-08 — Mac absolute mouse input / receiver-only diagnosis

The recovered executable was not a validated Linux release. Source inspection
found that Swift InputCaptureService sends absolute x/y without deltas during
edge control, while Linux injection silently ignored these events. Linux now
warps the X11 root pointer for absolute frames, scaling the legacy Mac
2560x1440 logical canvas to the Linux root dimensions. uinput still handles
keys, buttons and relative motion. This absolute path targets macOS 0.1.58
on an X11 Linux receiver; Wayland and other coordinate conventions remain
outside this fix. It does not establish that trust or LAN transport works.

ENTANGLO_RECEIVER_ONLY disables local capture and edge watcher and advertises
only the receiver role. scripts/test-receiver.sh uses this mode for a
120-second run, captures info logs, and refuses to start without writable
uinput. A personal-menu Ferma Entanglo command closes the process without
requiring Escape. No desktop process was launched automatically.

Compilation and 52 workspace tests pass; 8 socket tests excluded. Live device/network checks cannot
run in the sandbox (no uinput visible, socket bind denied). Still required:
real trusted Mac -> Linux movement/click/keyboard and exit/stop checks.


### 2026-09-17 — display-size negotiation and repeated edge reset fixed

`HelloPayload` now carries optional `screenWidth` / `screenHeight`. Linux
advertises the X11 root coordinate space and stores a peer's advertised size;
edge-activated `RemoteCursorState` uses it for seeding, clamping, and return.
Peers that predate the fields remain compatible and use the 2560x1440 logical
fallback. The installed iMac 2 app is still macOS 0.1.58 and therefore uses
that fallback. A live attempt with a guessed physical 5120x2880 size pinned the
cursor to the edge and proved that this peer expects logical 2560x1440
coordinates; the guess was removed.

The same live run exposed the immediate cause of the remaining vertical-edge
behaviour. `XGrabPointer` hides and redirects the X11 pointer but does not
freeze its root position. While remote control was already active, the hidden
pointer could leave and re-enter the assigned local edge. The poller treated
each crossing as a new session and called `set_active_receiver_from_edge`
again, reseeding the remote virtual cursor at x=0. `edge.rs` now activates from
an edge only when `active_receiver` is `None`; a regression test pins this
state rule.

Verification: `cargo fmt --all`, `cargo test --workspace` (58/58), and `git
diff --check` pass. The corrected GUI build was relaunched. Final physical
confirmation after the new activation guard is still pending. The Swift source
has matching optional negotiation and codec coverage, but cannot be compiled
on this Debian machine; the deployed Mac binary has not yet been rebuilt.

### 2026-09-16 — natural edge return fixed and verified live

The reported defect “mouse reaches iMac 2 but never comes back from its
edge” was reproduced by source inspection: Linux set `active_receiver`
when crossing the local edge but, unlike Swift's `InputCaptureService`,
kept no sender-side virtual cursor and therefore had no condition capable
of clearing the target on a remote-edge return. The receiver cannot infer
this reliably; the controller must track its own raw deltas.

`Coordinator` now has a per-edge `RemoteCursorState`: local-left maps to
remote-right entry and local-right maps to remote-left entry, seeded in the
same 2560×1440 notional space as the current Swift app. Only the entry edge
can return control; the opposite edge clamps as a wall to prevent bounce
loops, and exit is suppressed while any mouse button is held. Natural exit
clears `active_receiver`; the existing 50 ms X11 loop then ungrabs and
reveals the local pointer.

Verification: full workspace suite is 50/50 green. Against the real trusted
iMac 2 (Entanglo 0.1.58), three consecutive round trips logged pointer grab,
`remote cursor exited via entry edge; control returned locally`, then pointer
release. A fourth active grab was also released safely. The app remained
open after the test with the pointer local and visible.

### 2026-09-16 — clipboard path implemented; uinput startup fixed

Plain-text clipboard sync is now wired end to end, following the real
Swift implementation rather than the old stub: it is opt-in/off by
default, polls GTK/GDK every 500 ms, establishes a baseline when enabled,
broadcasts changes through every trusted session, applies only trusted
inbound `clipboardText` frames, suppresses remote-write echo, and caps
outgoing UTF-8 at 1 MiB. The Settings page contains the toggle. The
setting is currently per-run (off again after restart), matching the safe
default but not yet the Mac's persisted preference.

The full workspace suite passed at this checkpoint: 47 tests total (30 core,
3 protocol integration, 14 Linux/UI logic). New coverage includes
default-off, UTF-8 boundary truncation, baseline/dedup behavior, trusted
coordinator-to-coordinator clipboard transfer, and explicit rejection of
clipboard text from an untrusted raw peer.

Live verification on this Debian/X11 machine: the GTK window opened, the
real iMac 2 peer connected and reported macOS 15 / Entanglo 0.1.58, and
the kernel registered `Entanglo Virtual Input`. A packaging bug was found:
the udev rule existed, but the `uinput` module was not loaded after boot,
leaving a stale root-only `/dev/uinput`. Packaging now ships a
`modules-load.d` entry and `postinst` loads the module before retriggering
udev. Runtime verification after `modprobe uinput`: `/dev/uinput` is
`root:input` mode `0660`, and Entanglo starts with both controller and
receiver available. Remaining manual checks: enable clipboard on Linux
and Mac and copy/paste both ways; then repeat the documented real
Ctrl+Shift+Escape pointer-ungrab test.

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
- `features/network_quality.rs`, `logging.rs`, and the plain-text
  clipboard path are real, not stubs, and back live UI/protocol flows.
  `file_transfer`, `screenshot`, and `url_push` are still Phase 2 stubs.
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
