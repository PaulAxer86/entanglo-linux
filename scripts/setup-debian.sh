#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if (( EUID == 0 )); then
    echo "Run as your normal user: the script uses sudo where needed." >&2
    exit 1
fi
sudo apt-get update
sudo apt-get install -y cargo rustc build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev avahi-daemon avahi-utils
if ! dpkg --compare-versions "$(rustc --version | cut -d' ' -f2)" ge 1.85; then
    echo 'Entanglo needs Rust 1.85 or newer. On Debian 12, install it with rustup and run this script again.' >&2
    exit 1
fi
sudo install -m 0644 "$repo_dir/packaging/60-entanglo-uinput.rules" /etc/udev/rules.d/60-entanglo-uinput.rules
sudo install -m 0644 "$repo_dir/packaging/entanglo-uinput.conf" /etc/modules-load.d/entanglo-uinput.conf
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --action=change --sysname-match=uinput
sudo usermod -aG input "$(id -un)"
cd "$repo_dir"
cargo test --locked --workspace
cargo build --locked --release -p entanglo-linux
install -d "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons"
cp -a crates/entanglo-linux/data/icons/hicolor "$HOME/.local/share/icons/"
install -m 0755 target/release/entanglo-linux "$HOME/.local/bin/entanglo-linux"
install -m 0755 scripts/start-receiver.sh "$HOME/.local/bin/entanglo-receiver"
python3 - "$HOME/.local/share/applications/com.paoloasara.Entanglo.desktop" "$HOME/.local/bin/entanglo-receiver" <<'PYDESKTOP'
import sys
from pathlib import Path
binary = sys.argv[2].replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
Path(sys.argv[1]).write_text('[Desktop Entry]\nType=Application\nName=Entanglo\nComment=Share mouse, keyboard and clipboard on the LAN\nExec="' + binary + '"\nIcon=com.paoloasara.Entanglo\nTerminal=false\nCategories=Network;Utility;\n')
PYDESKTOP
printf '%s\n' 'Installation complete. Open Entanglo from the application menu.'
