# Recommended Linux (Debian) stack

The goal is **maximum visual + behavioural fidelity** with the macOS
app, on a platform that is far more fragmented than Windows (X11 vs
Wayland, GNOME vs KDE vs everything else). We resolve that
fragmentation by going as close to the kernel as possible for the
hard parts (input) and picking the desktop environment Debian ships
by default (GNOME) as the visual reference for the easy part (UI).

| Layer | macOS (today) | Linux (chosen) | Why this choice |
|---|---|---|---|
| Language | Swift 5.9 | **Rust** (2021 edition, stable toolchain) | Memory-safe systems language, excellent async story (`tokio`), no GC pauses that would jitter input forwarding, and directly builds the `entangl-core` Rust crate the macOS `ROADMAP.md` already names as the 0.3 target. |
| UI framework | SwiftUI + AppKit | **GTK4 + libadwaita** (`gtk4-rs`) | Debian's default desktop is GNOME; libadwaita gives rounded corners, adaptive light/dark, and system accent colour out of the box — the closest visual cousin to macOS Big Sur+ available natively on Linux. `AdwNavigationSplitView` maps ~1:1 to SwiftUI's `NavigationSplitView`. |
| App model | macOS .app bundle | **Single dynamically-linked binary** + `.desktop` file | Simplest packaging path; no runtime beyond GTK4/libadwaita, which ship in Debian's repos. |
| Discovery (mDNS / Bonjour) | `NetService` | **`mdns-sd`** crate (pure Rust) | No dependency on `avahi-daemon` running; works identically on a minimal Debian server install. Coexists fine with Avahi if present — both just speak standard mDNS. |
| TCP transport | `NWConnection` | `tokio::net::TcpStream` + `tokio_util::codec::Framed` | Idiomatic async Rust, and `LengthDelimitedCodec` gives the 4-byte-BE framing almost for free. |
| JSON | Swift `Codable` | **`serde` + `serde_json`** | The de-facto standard; mirror Swift's base64-for-`Data` behaviour explicitly (see `PROTOCOL.md` §3) with a custom `serde_with::base64::Base64` field. |
| Input capture | CGEvent tap | **raw `/dev/input/eventX` (evdev)** via the `evdev` (or `input-linux`) crate | Bypasses X11/Wayland entirely — works unmodified whether the session is GNOME/Wayland, GNOME/X11, KDE/Wayland, or a bare console. This is the same layer `entanglo-android`'s C `uinput.c` already operates at. |
| Input injection | `CGEventPost` | **`/dev/uinput`** via the same `evdev` crate's `uinput` module (`VirtualDeviceBuilder`) | Same reasoning as capture, and shares the exact `Key`/`RelativeAxisType` code space with the capture side — no separate `uinput` crate needed. No root needed on desktop Debian — see permissions note below. |
| Modifier flags | CGEventFlags | Track modifier `KEY_*` state manually from evdev; synthesize on injection | evdev has no bitmask concept — see `PROTOCOL.md` §5.5 for the translation rule. |
| Clipboard sync | `NSPasteboard` | **`arboard`** crate | Cross-platform clipboard crate with working Wayland support (shells out to `wl-clipboard` semantics internally) and X11 support (via `x11rb`). Text first, image after, matching the Mac's own rollout order. |
| Screen capture | `SCScreenshotManager` | **`ashpd`** crate → `org.freedesktop.portal.Screenshot` | Works on GNOME/Wayland and KDE/Wayland via the desktop portal, with a user consent dialog (unavoidable and correct — Wayland deliberately has no unprompted screen capture). X11 fallback via `x11rb` `GetImage` for portal-less sessions. |
| Tray icon | `NSStatusItem` | **`StatusNotifierItem`** via `ksni` crate, or libadwaita's own window controls if tray icons are unavailable (GNOME Shell needs an extension for them) | GNOME's tray situation is the roughest edge on this whole stack — see Risks in `ROADMAP.md`. |
| Trust store / Keychain | macOS Keychain | **Secret Service D-Bus API** (`secret-service` crate) → GNOME Keyring / KWallet | User-scoped, encrypted, no custom crypto needed on desktop. Falls back to a machine-id-derived encrypted file for headless boxes — see `PROTOCOL.md` §9. |
| Auto-update | Peer-push DMG over LAN | Same peer-push pattern + HTTPS poll of a `latest-linux.json` manifest; install via `pkexec dpkg -i` (polkit prompt) since a system-wide `.deb` install needs root | Keeps the "click once to install" UX; the polkit prompt is the Linux analogue of the macOS Gatekeeper/notarization click-through. |
| Packaging | `.dmg` (hdiutil) | **`.deb`** via `cargo-deb` | Debian is the explicit target distro; `cargo-deb` reads straight from `Cargo.toml` metadata, no separate packaging manifest to keep in sync. |
| Printer Bridge | `lpr` + `networksetup` | **CUPS** (`lp`/`lpr`, already the Linux print stack) + `nmcli`/`iwd` for the Wi-Fi-swap dance | Actually *simpler* than the Mac version — CUPS is native and scriptable, no macOS-style printer-driver quirks to fight. Ship in v2, same as Windows/Mac plan. |

## What this stack gets you visually

GTK4 + libadwaita on GNOME (Debian's default) supports:

- Rounded corners, adaptive light/dark theming picked up automatically
  from the desktop setting
- System accent colour (GNOME 42+) — same spirit as macOS's accent
  colour and Windows' Mica tint
- `AdwNavigationSplitView` / `AdwOverlaySplitView` — sidebar + content
  split, visually equivalent to the macOS `NavigationSplitView`
  Entanglo uses today
- HiDPI/fractional scaling out of the box (GTK4 handles this natively
  on Wayland; X11 fractional scaling is rougher but functional)
- `AdwToastOverlay` for the same kind of lightweight in-app
  notifications the Mac uses for transfer/pairing feedback

The dashboard, devices, input-sharing, files, print, network, news &
updates, settings, logs sections from the Mac app port to GTK4
`AdwNavigationPage`s under a single `AdwNavigationSplitView` with
very little visual translation — same story as the Windows
`NavigationView` mapping in `entanglo-windows/STACK.md`.

## Tools you need on the Debian dev machine

- **Debian 12 (bookworm) or 13 (trixie)** — GNOME 43+/46+ for full
  libadwaita fidelity
- **Rust stable** via `rustup` (Debian's packaged `rustc` lags; use
  rustup for a current toolchain)
- **GTK4 + libadwaita dev headers**: `sudo apt install libgtk-4-dev
  libadwaita-1-dev build-essential pkg-config`
- **`cargo-deb`**: `cargo install cargo-deb`
- Membership in the **`input` group** (`sudo usermod -aG input $USER`,
  then re-login) so the binary can open `/dev/input/eventX` and
  `/dev/uinput` without root — see the udev rule in `ROADMAP.md`
  Phase 1
- Git + the GitHub CLI (optional but nice, already set up in this
  environment)
- Optional: **GNOME Builder** or **VS Code + rust-analyzer** for dev

## What you don't need

- No Apple developer account, no Windows Store account.
- No code-signing certificate for local dev — Debian doesn't gate
  execution behind package signing the way macOS/Windows do; only
  matters if you later publish a `.deb` repo or ship via
  Flathub/Snap.
- No backend, no database, no cloud service — Entanglo is fully
  peer-to-peer, same as every other target.

## Cross-platform alternatives we considered & rejected

- **Electron / Tauri**: same fidelity loss discussed in
  `entanglo-windows/STACK.md` — web UI never quite matches native
  widget behaviour (focus rings, text selection, accessibility tree).
  Rejected for the same reason.
- **Qt/KDE Kirigami**: would give equally good native fidelity on KDE
  Plasma, and Qt has a genuinely nicer cross-DE story than GTK. But
  Debian's *default* desktop is GNOME, and Kirigami looks visibly
  foreign there (the reverse problem GTK has on KDE) — picking one
  requires picking a primary DE, and GNOME is the safer default for
  "Debian" specifically. Revisit if a KDE-first fork is ever wanted.
- **Avalonia UI / MAUI (.NET)**: same rationale as the Windows
  document's rejection — cross-platform .NET UI trades fidelity for
  code reuse, and there's no existing .NET Linux codebase here to
  reuse it *with* (unlike Windows, which shares nothing with Mac
  anyway).
- **egui / iced (pure Rust immediate/retained-mode GUI)**: tempting
  for being pure Rust with zero native-toolkit dependency, but neither
  renders as a GNOME-native app — no libadwaita theming, no
  accessibility tree integration via AT-SPI. Good fit for a
  throwaway debug tool, not for the flagship UI.
- **Flutter**: strong widget fidelity story on its own terms, but that
  means *Flutter's* look, not GNOME's — same objection as Electron,
  just compiled instead of web-rendered.

The chosen stack (Rust + GTK4/libadwaita + the `evdev` crate for both
raw evdev capture and its own `uinput` injection module) is the
**shortest path to a Debian app that doesn't feel like a port**, and
is the first client to actually stand up the shared `entanglo-core`
Rust crate the Mac roadmap has been planning toward since 0.1.
