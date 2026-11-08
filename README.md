# Entanglo for Linux

Native GTK4/libadwaita receiver for Entanglo peers on the same LAN.
Tested on Debian 13 amd64 with X11 and macOS Entanglo 0.1.58/0.1.59.

## Download and install

Download the Linux `.deb` from <https://entanglo.pages.dev/download>:

```sh
sudo apt install ./entanglo-linux_0.1.1_amd64.deb
sudo usermod -aG input "$USER"
```

Open **Entanglo** from the application menu. Existing group membership is
activated for the app without requiring a desktop logout. In **Devices**,
trust the controller. On macOS enable sharing and the Controller role, then
select this Linux device. Use an X11 desktop for Mac absolute pointer input.

## Included

- Mouse, keyboard and plain-text clipboard reception from trusted peers.
- Streaming file reception into `Downloads/Entanglo`, with progress, collision
  handling, partial-file cleanup and completion acknowledgements.
- One row per device, with capability icons and clear authorization state.
- A live Dashboard with actual TCP RTT, interface, heartbeat and recent files.
- Stable device identity, persisted trust, port 55555 and Bonjour reconnect
  announcements when a controller has not returned after an update.
- Standard minimize/maximize/close controls, Ctrl+Q and the original rabbit icon.
- Automatic hourly update checks. **News & Updates** downloads and verifies
  releases before installing. Personal installs update in the user account;
  system installs use the standard administrator authentication dialog.

Receiving is the default. Local input capture is opt-in via
`ENTANGLO_ENABLE_CONTROLLER=1`; Linux-to-Linux control needs separate testing.
File sending, printing and Wayland input reception are not yet implemented.

## Build and test

```sh
sudo apt install cargo rustc build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev
cargo test --locked --workspace -- --test-threads=1
cargo build --locked -p entanglo-linux
python3 scripts/package-debian.py
```

The project targets Debian 13's Rust 1.85 and GTK 4.18. Packages are built on
Debian 13 to avoid introducing a newer glibc requirement. Run
`scripts/setup-debian.sh` for a development install.

## Releases and automatic publishing

The Linux repository is <https://github.com/PaulAxer86/entanglo-linux>.
The website repository publishes the Linux build after a new version is
pushed to the Linux repository: tests, build, `.deb`, checksum manifest,
website commit and deployment. See [docs/RELEASING.md](docs/RELEASING.md).

Logs are saved for both desktop and terminal launches in
`~/.local/state/entanglo/last.log`. The **Reconnect** Dashboard action refreshes
Bonjour without changing device identity. Closing the window stops input,
withdraws the network advertisement and quits.

Protocol details: [PROTOCOL.md](PROTOCOL.md). Development history:
[docs/DEV.md](docs/DEV.md). No keystroke content is logged.

Copyright Paolo Asara. See Cargo workspace metadata for licensing.
