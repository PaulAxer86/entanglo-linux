#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if (( EUID == 0 )); then
    echo 'Run as your normal user, without sudo.' >&2
    exit 1
fi
sudo install -m 0644 "$repo_dir/packaging/60-entanglo-uinput.rules" /etc/udev/rules.d/60-entanglo-uinput.rules
sudo install -m 0644 "$repo_dir/packaging/entanglo-uinput.conf" /etc/modules-load.d/entanglo-uinput.conf
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --action=change --sysname-match=uinput
sudo udevadm settle
sudo usermod -aG input "$(id -un)"
if [[ ! -c /dev/uinput ]]; then
    echo 'ERROR: /dev/uinput is missing after modprobe. Check the kernel configuration.' >&2
    exit 1
fi
stat -c 'Device: %n; owner/group: %U:%G; permissions: %a' /dev/uinput
if [[ ! -w /dev/uinput ]]; then
    echo 'Log out and back in to activate the input group, then reopen Entanglo.'
else
    echo 'uinput is accessible. Close Entanglo completely and reopen it from the menu.'
fi
printf '%s\n' 'In Devices, trust iMac 1. On iMac 1, trust iMac 2 and enable control of iMac 2.' 'If input still fails, copy the receiver/uinput/connection lines from Logs.'
