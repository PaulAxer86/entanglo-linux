# Entanglo for Linux (Debian) — handoff package

This folder is the **entry point** for porting Entanglo (a native
macOS app for sharing input / clipboard / files / printer across
machines on the same Wi-Fi) to Linux, with **Debian** as the primary
target distro.

Everything you need to start is here:

- [`PROTOCOL.md`](PROTOCOL.md) — the wire protocol spec. Authoritative.
  Build the Linux client to match this and it will pair with the Mac,
  Windows, and Android peers out of the box.
- [`STACK.md`](STACK.md) — recommended Linux stack, with rationale for
  each choice.
- [`ROADMAP.md`](ROADMAP.md) — phased plan, from MVP to feature parity.
- [`SKELETON.md`](SKELETON.md) — concrete Cargo workspace layout +
  file-by-file starter to scaffold a new `entanglo-linux` project.

**Picking this up mid-development?** Start with the "Handover status"
section at the top of [`docs/DEV.md`](docs/DEV.md) instead of this
file — it says exactly what's done, what's just been fixed but not
yet re-verified live, and what to check first.

---

## TL;DR if you have 60 seconds

Goal: a Debian machine that **talks the same wire protocol** as the
Mac, Windows and Android peers, so any of them can share a single
mouse + keyboard + clipboard with it on the same LAN.

Stack chosen: **Rust** for everything (core + UI), **GTK4 +
libadwaita** for the app shell, **raw `/dev/input` (evdev) capture +
`/dev/uinput` injection** for input — bypassing X11/Wayland entirely
so the same binary works unmodified on both display servers.

Why: this repo is the first target to actually build the Rust
`entanglo-core` crate the macOS roadmap has been pointing at since
0.1 ("0.3 — extract Protocol/ into a Rust crate; Linux receiver").
Kernel-level evdev/uinput is also exactly the technique
`entanglo-android`'s `cpp/uinput.c` already proved works end-to-end
against a live Mac controller — Linux desktop just gets it without
needing root, via the `input` group.

First milestone (3–4 weeks solo): MVP that pairs with a Mac (or
Windows/Android) peer and forwards mouse + keyboard, running on
stock Debian 12/13 with GNOME (Wayland) as the reference desktop.
Everything else layers on top.

---

## Reference implementations

| Repo | Platform | Role |
|---|---|---|
| [`PaulAxer86/entanglo-macos`](https://github.com/PaulAxer86/entanglo-macos) | macOS, Swift/SwiftUI | Reference implementation, protocol source of truth |
| [`PaulAxer86/entanglo-windows`](https://github.com/PaulAxer86/entanglo-windows) | Windows, C#/WinUI 3 | Handoff package (this repo's sibling / template) |
| [`PaulAxer86/entanglo-android`](https://github.com/PaulAxer86/entanglo-android) | Android TV (DQ08 box), Kotlin | Receiver-only; proves evdev/uinput injection against a real Mac controller |
| `entanglo-linux` (this repo) | Debian, Rust + GTK4 | New target |

They only share the protocol, not the code — each platform repo is
independent.

Current versions at time of writing: Mac **0.1.58** (2026-08-08),
Windows **0.1.55**, Android **v0.1** (2026-08-21, end-to-end working).

Website + downloads: <https://entanglo.pages.dev>.
Update manifests: `/updates/latest.json` (Mac), `/updates/latest-win.json`
(Win) — this repo should add `/updates/latest-linux.json` when it ships
auto-update (Phase 2).

If the Mac protocol bumps, `PROTOCOL.md` here gets updated. Watch its
first line for the document date.

---

## License & ownership

All Entanglo apps are by Paolo Asara. Copy this folder freely to any
machine you'll develop on.
