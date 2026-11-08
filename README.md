# Entanglo for Linux

Native GTK4/libadwaita receiver for Entanglo peers on the same LAN.
Runs on Debian 12 and 13 amd64 with X11. Tested live on Debian 13 with macOS
Entanglo 0.1.58/0.1.59.

## Download and install

Download the Linux `.deb` from <https://entanglo.pages.dev/download>:

```sh
sudo apt install ./entanglo-linux_0.1.8_amd64.deb
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
- Notification sounds for trusted device connection and disconnection, file
  errors and received files.
- Automatic hourly update checks. **News & Updates** downloads and verifies
  releases before installing. Personal installs update in the user account;
  system installs use the standard administrator authentication dialog.

Receiving is the default. To control other devices from this computer, enable
**Share my keyboard & mouse** in **Input Sharing** and restart the app; assign
devices to any screen edge there (X11 only).
File sending, printing and Wayland input reception are not yet implemented.

## Build and test

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev
# Rust 1.85 or newer: `apt install cargo rustc` on Debian 13, rustup on Debian 12
cargo test --locked --workspace -- --test-threads=1
cargo build --locked -p entanglo-linux
python3 scripts/package-debian.py
```

The project needs Rust 1.85 and supports GTK 4.8 with libadwaita 1.2 or newer.
Release packages are built on Debian 12 so one package runs on Debian 12 and 13;
a package built on Debian 13 only runs on Debian 13. Run
`scripts/setup-debian.sh` for a development install.

## Releases and automatic publishing

The Linux repository is <https://github.com/PaulAxer86/entanglo-linux>.
The website repository publishes the Linux build after a new version is
pushed to the Linux repository: tests, build, `.deb`, checksum manifest,
website commit and deployment. See [docs/RELEASING.md](docs/RELEASING.md).

Logs are saved for both desktop and terminal launches in
`~/.local/state/entanglo/last.log`. The **Reconnect** Dashboard action refreshes
Bonjour without changing device identity. On desktops with a system tray,
closing the window keeps Entanglo running there; choose **Quit Entanglo** from
the tray, or press Ctrl+Q, to stop input, withdraw the network advertisement and
quit. Without a tray, closing the window quits. **Settings** can start Entanglo
at login.

Protocol details: [PROTOCOL.md](PROTOCOL.md). Development history:
[docs/DEV.md](docs/DEV.md). No keystroke content is logged.

Copyright Paolo Asara. See Cargo workspace metadata for licensing.
