#!/usr/bin/env bash
set -euo pipefail
# The executable owns permissions, logging and the default receiver role.
exec "${ENTANGLO_BINARY:-$HOME/.local/bin/entanglo-linux}" "$@"
