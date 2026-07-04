#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ -n "${RPTY_FLEET_BIN:-}" ]; then
  fleet_bin=$RPTY_FLEET_BIN
elif [ -x "$root/fleet" ]; then
  fleet_bin="$root/fleet"
elif [ -x "$root/target/release/fleet" ]; then
  fleet_bin="$root/target/release/fleet"
else
  cargo build --release
  fleet_bin="$root/target/release/fleet"
fi

"$fleet_bin" doctor --fix --write-shell-profile

cat <<'EOF'

AgentFleet installed.
Open a new shell, then run:

  fleet doctor
  codex
  claude
  opencode

Fleet backend is bundled. For a fresh install, create your private device inventory:

  cp ~/.rpty/bin/fleet_backend/devices.example.json ~/.rpty/bin/fleet_backend/devices.json
  chmod 600 ~/.rpty/bin/fleet_backend/devices.json

If you already have an external Fleet backend, you can override the bundled one:

  fleet config --fleet-py /path/to/fleet.py

EOF
