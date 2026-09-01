# Roadmap — Entanglo for Linux (Debian)

Three phases, same shape as `entanglo-windows/ROADMAP.md`. Solo-dev
effort estimates assume someone comfortable in Rust but new to this
protocol; double if learning Rust at the same time. Evdev/uinput
plumbing is somewhat easier here than the Windows hook story, since
`entanglo-android/app/src/main/cpp/uinput.c` already proves the
approach against a live Mac controller — this is mostly "do that
again, without root, with a GTK front end."

---

## Phase 1 — MVP (3–4 weeks)

**Goal**: a Debian machine and a Mac (or Windows/Android) machine on
the same Wi-Fi can pair and share **one mouse + keyboard**. Nothing
else.

### Deliverables

Status legend: ✅ done + tested, 🚧 written but not fully exercised,
⬜ not started. See `docs/DEV.md` for what "tested" means per item —
this scaffold turned out to be built on an actual Debian desktop
(real X11 session, real LAN with a real Mac and Android running
Entanglo already), so several items below are verified against those
live reference implementations, not just loopback-TCP unit tests.

- ✅ Cargo workspace with `entanglo-core` (protocol + transport + input
  translation, no UI) and `entanglo-linux` (GTK4 app) crates — see
  `SKELETON.md`.
- ✅ GTK4/libadwaita app shell: `AdwNavigationSplitView` with all ten
  pages, sidebar navigation, and a **real window that opens on a real
  X11 display** — confirmed via `xwininfo`, not just `cargo build`.
  Dashboard/Devices/Pairing are live, wired to `net::session` events
  through `Coordinator`/`state::AppShared`; the other seven pages are
  still placeholders.
- ✅ `udev` rule shipped in `packaging/60-entanglo-uinput.rules`.
  ⬜ first-run onboarding that checks `$USER` is in the `input`
  group and shows the exact `usermod` command — not written yet,
  needs the Settings page. Today the app just logs a warning and
  keeps running without receiver/controller capability, confirmed
  live: `could not open /dev/uinput ... Permission denied`.
- ✅ mDNS advertise + browse for `_entanglo._tcp` via `mdns-sd` —
  **live-tested on the real LAN**: advertised successfully, and
  discovered two real peers already running Entanglo — an Android TV
  box and a Mac (`iMac Pro (2)`, app `0.1.58`). Found and fixed a real
  bug this way: mDNS re-announces trigger repeat discovery of the same
  service (including our own advertisement), which was dialing peers
  — and re-sending `pairRequest` to a human-gated peer like the Mac —
  over and over. Fixed with a per-service dial dedup in
  `app_state::spawn_discovery` and a self-connection guard in
  `session::run_session` (see `net::session::tests::
  self_connection_is_rejected_before_pairing`).
- ✅ TCP listener on an OS-assigned port + outbound `TcpStream`, via
  `tokio` — exercised by the session/coordinator integration tests
  *and* by real inbound/outbound connections to the Android/Mac peers.
- ✅ Wire framing (4-byte big-endian length prefix + JSON envelope) via
  `tokio_util::codec::Framed` + a custom codec.
- ✅ All payload structs (`#[derive(Serialize, Deserialize)]`) per
  `PROTOCOL.md` §5.
- ✅ `hello` handshake, both directions — verified against the real
  Mac and Android apps live, not just the loopback test: logs show
  `hello received device_name="iMac Pro (2)" platform=Some("macOS")
  app_version=0.1.58` and the equivalent for the Android box.
- ✅ Trust store (Secret Service, fallback encrypted file) + pairing —
  **verified live against the real Secret Service** (GNOME Keyring,
  not just the file-fallback unit tests): a `pairRequest` sent to the
  Android box came back accepted, trust was written to the real
  keyring, and a second run of the app loaded that same trust from
  disk-backed Secret Service storage and skipped pairing entirely.
  The Mac correctly did **not** auto-trust — its `pairRequest` is
  still pending real human approval on that machine, matching the
  Mac's own human-approval pairing UI. ✅ GTK **Pairing page** exists
  and is wired to `SessionEvent::PairingRequested` with real
  Accept/Reject buttons (`state.rs`), though clicking them hasn't
  been exercised in this pass (would need a *local* incoming
  pairRequest, which didn't happen to occur live).
- ✅ Heartbeats (1 Hz) with RTT echo per §5.4 — integration-tested;
  live behavior not separately confirmed beyond the successful hello
  exchanges above (heartbeats would need a longer live run than this
  pass did, to avoid leaving connections open against real peers
  longer than necessary).
- 🚧 `inputEvent` send (evdev capture) and receive (`/dev/uinput`
  injection) — `Coordinator::enable_controller`/`enable_receiver` are
  wired end to end into the connection lifecycle (an incoming trusted
  peer's `InputEvent` is injected automatically; local evdev devices
  are captured and forwarded to whichever peer
  `set_active_receiver` targets, with a manual "Control this device"
  button on the Devices page as the stand-in for real edge-detection).
  Confirmed live that this **fails safely**: no `input` group
  membership on this run, so `enable_receiver` logged a permission
  error and the app kept running with receiver capability simply
  off — no crash, no partial/unsafe state. Not yet confirmed with
  `input` group membership actually granted (that's an environment
  change, not a code change, for whoever runs this next).
- ⬜ Cursor edge-detection on the controller — still needs the
  Wayland pointer-position spike from `ROADMAP.md` Risks. The manual
  "Control this device" button (see above) is the interim substitute.
- 🚧 `releaseControl` on the receiver when local input is touched —
  the session side (receiving/emitting `SessionEvent::ReleaseControl`)
  is done; the sending side (detecting local input via evdev and
  actually calling it) isn't wired up.
- ⬜ Emergency stop button in the toolbar (matches the Mac's
  triple-Escape + explicit button). Not started — needs the Input
  Sharing page.

### Test of done

Plug a Debian laptop next to your Mac (or Windows box), open Entanglo
on both, approve the pairing, push the mouse off the shared edge —
cursor appears on the Debian machine, typing works, push back — control
returns. Repeat once on GNOME/Wayland and once on GNOME/X11 (or a
Debian VM with X11 forced via `GDK_BACKEND=x11`) to confirm the
evdev/uinput path really is display-server-agnostic as designed.

### Things deliberately deferred from phase 1

- Clipboard sync
- File transfer
- Screenshot peer
- URL push
- Printer Bridge
- Auto-update (manual `.deb` install during dev)
- Network Quality dashboard
- Tray icon (GNOME Shell tray support needs an extension; punt to
  Phase 2 and rely on the main window in Phase 1)

---

## Phase 2 — Feature parity v1 (4–6 weeks)

**Goal**: everything the Mac app does *except* Printer Bridge.

### Deliverables

- **Clipboard sync** (`arboard` crate, text first, image after).
- **File drop** (offer/chunk/ack). Files land in `$XDG_DOWNLOAD_DIR`
  (fallback `~/Downloads`).
- **URL push** (`xdg-open` with a scheme allow-list, spawned via
  `std::process::Command` — never via a shell string).
- **Screenshot peer** (`ashpd` → portal `Screenshot` request, scale
  long side to 2048 px, encode PNG via the `image` crate, return
  inline). Includes the 8 s sender-side timeout from `PROTOCOL.md`
  §5.10 — critical here since the portal call blocks on a user
  consent dialog that may never be answered.
- **Network Quality** dashboard tile (RTT trend, packet loss estimate
  via heartbeat sequence gaps) — same computation as Mac/Windows,
  ported straight from the algorithm description in
  `entanglo-macos/docs/ARCHITECTURE.md`.
- **Recent Transfers** tile + log.
- **News & Updates** page (parse
  `https://entanglo.pages.dev/updates/latest-linux.json` — new
  manifest, mirroring the existing `latest.json`/`latest-win.json`
  pattern in the website repo).
- **Auto-update** integration: app polls the manifest above and
  downloads the next `entanglo_X.Y.Z_amd64.deb` from GitHub Releases.
  Verify SHA-256 from the manifest, then install via
  `pkexec dpkg -i <path>` so the user gets exactly one polkit prompt
  (no silent root escalation).
- **Tray icon** via `ksni` (`StatusNotifierItem`), with a documented
  fallback ("pin the window" tip) for GNOME Shell installs without
  the AppIndicator extension.

### Test of done

Open Mac/Windows/Android and Debian clients together. Every feature
in the Mac Dashboard works in both directions with the Debian peer,
except Print.

---

## Phase 3 — Printer Bridge + polish (4–8 weeks)

**Goal**: print from any peer (Mac, Windows, Android, Linux) to a USB
printer attached to any Linux peer in the link, and vice versa.

### Deliverables

- Receiver-side: incoming `fileOffer` with `kind: "printJob"` spools
  straight to CUPS via `lp` (shell out, or the `cups` crate if it's
  in good enough shape at implementation time).
- Sender-side: capture the user's print job. Linux makes this easier
  than Mac/Windows — install a CUPS "virtual PDF printer" backend
  (or just let the user pick "Print to File (PDF)", already built
  into every Linux print dialog) and watch a drop folder, then
  forward as `fileOffer kind=printJob` to the peer attached to the
  real printer.
- Wi-Fi profile management via `nmcli` (NetworkManager, GNOME
  default) if mirroring the Mac Printer Bridge's network-switch
  behaviour; skip entirely if the printer is reachable over the same
  LAN (increasingly the common case — Wi-Fi-Direct printers are less
  common than they were when the Mac feature was designed).
- **Packaging hardening**: sign the `.deb` repo if distributing
  publicly (a GPG-signed APT repo, or ship via Flathub for wider
  reach beyond Debian). Self-signed/unsigned is fine for direct
  `.deb` download + `dpkg -i`, same trust model as the Mac's
  self-signed cert story.
- **Website listing**: add a "Download for Linux (.deb)" button next
  to the macOS/Windows ones. Reuse the `scripts/publish-release.sh`
  pattern from the website repo, extended for the `.deb` artifact and
  `latest-linux.json`.

### Test of done

You print a PDF from your Debian laptop and it comes out of the USB
printer plugged into the Mac in the next room — and the reverse.

---

## Stretch (post-parity)

- **TLS** on the wire (protocol v2). Per-device self-signed cert,
  pinned at pair time. Same `pinHash` field becomes meaningful, per
  `entanglo-macos/README.md`'s 0.2 plan.
- **Flatpak / Flathub** distribution for non-Debian distros, once the
  `.deb` path is solid — the GTK4/libadwaita choice was made partly
  because it's also Flatpak's best-supported toolkit.
- **KDE-native fork or Kirigami front end**, if there's ever demand —
  `entanglo-core` is UI-agnostic by construction, so this is "write a
  new front-end crate," not "port the app."
- **iOS / iPadOS client** and any other target — tracked in the
  sibling repos, not here.

---

## Risks & open questions

- **Wayland pointer position readback for edge-detection**: unlike
  X11's `XQueryPointer`, there's no universal Wayland protocol for
  "give me the global cursor position" (by design — Wayland scopes
  input to focused surfaces). Options: (a) use the app's own window
  as a resizable "hot edge" strip instead of true screen-edge
  detection, which is a UX compromise but portal-free; (b) use a
  compositor-specific protocol extension where available (e.g.
  `wlr-layer-shell` + pointer constraints on wlroots-based
  compositors); (c) fall back to a global hotkey toggle only (already
  planned as the secondary control method) when true edge detection
  isn't available. Decide in Phase 1 after a spike — don't let this
  block the whole milestone.
- **GNOME Shell tray icon support**: removed from core GNOME years
  ago, needs the community "AppIndicator and KStatusNotifierItem
  Support" extension. Document this clearly rather than silently
  degrading; KDE Plasma needs no such extension.
- **`/dev/uinput` permissions on first run**: the udev rule only takes
  effect after a re-login (group membership is read at login time).
  First-run UX must detect "group added but not yet active in this
  session" and say so plainly, not just fail silently on `open()`.
- **X11 vs Wayland fractional/HiDPI scaling**: GTK4 handles this well
  on Wayland; X11 sessions with fractional scaling can report
  inconsistent logical-pixel coordinates to different apps. Worth an
  explicit test pass on a mixed-DPI setup before declaring Phase 1
  done, same caution the Windows roadmap calls out for its own DPI
  story.
- **Different keyboard layouts** (ANSI vs ISO, AZERTY, etc.): the
  protocol passes raw key codes; evdev codes are physical-position,
  layout-independent, same property as macOS virtual key codes — so
  this should "just work" as long as both sides report the same
  physical key. Worth a conscious test pass, same note as the Windows
  roadmap.
