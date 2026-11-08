#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if (( EUID == 0 )); then
    echo 'Run this script as your desktop user, without sudo.' >&2
    exit 1
fi
if [[ "$(dpkg --print-architecture)" != amd64 ]]; then
    echo 'This package requires amd64 Linux (Intel iMac).' >&2
    exit 1
fi
. /etc/os-release
if [[ "$ID" != debian || "$VERSION_ID" != 13 ]]; then
    echo 'This installer targets Debian 13.' >&2
    exit 1
fi
sha256sum --check SHA256SUMS
sudo apt-get install ./@PACKAGE@
sudo usermod -aG input "$(id -un)"
if [[ -e "$HOME/.local/bin/entanglo-linux" ]]; then
    echo 'A personal Entanglo executable also exists in ~/.local/bin. The system menu launcher uses /usr/bin.'
fi
printf '\n%s\n' 'Installed. Open Entanglo (Mac receiver) from the application menu.' 'Use an X11 session for Mac input reception.' 'Do not run Entanglo with sudo.'
