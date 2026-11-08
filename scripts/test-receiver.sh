#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
binary="$HOME/.local/bin/entanglo-linux"
if [[ ! -x "$binary" ]]; then
    echo 'The corrected build is not available yet.' >&2
    exit 1
fi
if pgrep -x entanglo-linux >/dev/null; then
    echo 'Close Entanglo first using Stop Entanglo in the application menu.' >&2
    exit 1
fi
if [[ "${XDG_SESSION_TYPE:-}" != x11 ]]; then
    echo 'This test with macOS 0.1.58 requires an X11 session.' >&2
    exit 1
fi
log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/entanglo"
mkdir -p "$log_dir"
log_file="$log_dir/receiver-test-$(date +%Y%m%d-%H%M%S).log"
printf '%s\n' 'RECEIVER-ONLY test, limited to 120 seconds.' 'On iMac 1, enable control of iMac 2. On Linux, click Trust for iMac 1 if needed.' 'To stop: press Ctrl+C in this terminal or use Stop Entanglo in the menu.' "Log: $log_file"
set +e
ENTANGLO_RECEIVER_ONLY=1 RUST_LOG=info timeout --signal=TERM --kill-after=3s 120s "$binary" 2>&1 | tee "$log_file"
result=${PIPESTATUS[0]}
if [[ "$result" == 124 ]]; then
    echo 'Test stopped automatically after 120 seconds.'
    exit 0
fi
exit "$result"
