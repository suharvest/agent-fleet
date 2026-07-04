# Getting Started

This guide is for a fresh local setup after cloning or downloading AgentFleet.

## Prerequisites

- Rust toolchain with `cargo`
- remote devices reachable by SSH
- `tmux` on each remote device used by PTY mode
- the real Agent command you want to wrap, such as `codex`, `claude`, or
  `opencode`

Fleet backend is bundled. The only private file you need is a device inventory:

```text
~/.rpty/bin/fleet_backend/devices.json
```

Create it from the example after install:

```bash
cp ~/.rpty/bin/fleet_backend/devices.example.json ~/.rpty/bin/fleet_backend/devices.json
chmod 600 ~/.rpty/bin/fleet_backend/devices.json
```

The repo ships the Rust-native Fleet backend, `bootstrap.sh`, and the example
inventory. Your real `devices.json` is the only private Fleet data expected
during normal setup; do not commit it.

Python is not required for normal Fleet use. The bundled Python backend remains
only as a legacy compatibility fallback for old `work-enter`/`work-monitor`
commands and for behavior comparisons. Use `RPTY_FLEET_NATIVE=0` only when
debugging fallback behavior.

If you want to use an external Fleet backend instead, configure one of:

```bash
fleet config --fleet-hub "$HOME/project/_hub"
fleet config --fleet-py "$HOME/project/_hub/fleet.py"
```

## Install

From this repository on macOS/Linux:

```bash
./scripts/install.sh
```

From this repository on Windows PowerShell:

```powershell
.\scripts\install.ps1
```

Manual install on any platform:

```bash
cargo test
cargo build
cargo run --bin fleet -- doctor --fix --write-shell-profile
```

Open a new shell after install. On macOS/Linux, to use the shims in the current
shell immediately, run:

```bash
export PATH="$HOME/.rpty/bin:$PATH"
```

On Windows PowerShell, use:

```powershell
$env:Path = "$HOME\.rpty\bin;$env:Path"
```

Open a new shell, then verify:

```bash
command -v fleet
fleet version
fleet doctor
```

`fleet doctor --fix` installs the stable `fleet-router` runtime plus `fleet`,
`rpty`, and `bash` shims. It also scans for known local Agent commands and
installs same-name shims for the ones it finds, currently `codex`, `claude`,
and `opencode`.

After that, launch installed Agents directly:

```bash
codex
claude
opencode
```

The shim creates a unique `RPTY_SESSION`, keeps `~/.rpty/bin` at the front of
`PATH`, and then execs the real Agent command found later in `PATH`. On Windows
the installed command shims are `.cmd` files in the same directory.

To configure one Agent manually, or add a custom Agent:

```bash
fleet install-agent-shim codex ~/.rpty/bin
fleet install-agent-shim claude ~/.rpty/bin
fleet install-agent-shim opencode ~/.rpty/bin
fleet config --agent gemini
fleet doctor --fix
```

## Verify A Device

PTY mode requires `tmux` on the remote device. macOS/Linux controllers can use
the bash shim directly. Windows controllers use `.cmd` command wrappers; remote
commands still execute through Fleet on the target device.

Native Windows remote targets should use stateless Fleet execution:

```bash
fleet exec home-win -- powershell -NoProfile -Command '$PSVersionTable.PSVersion'
fleet exec home-win -- cmd.exe /C echo ok
```

For persistent shell state on a Windows machine, use its WSL Fleet device, for
example:

```bash
fleet use wsl2-local
cd /tmp
pwd
```

Check that Fleet can see devices:

```bash
fleet hosts
fleet status --json
```

Check tmux on a device:

```bash
fleet doctor <device>
```

If tmux is missing and the device supports package installation:

```bash
fleet doctor --fix <device>
```

Verify persistent PTY state:

```bash
fleet use <device>
cd /tmp
export RPTY_TEST=ok
pwd
printf "%s\n" "$PWD $RPTY_TEST"
fleet env
```

## Agent Workflow

Inside `codex`, `claude`, or `opencode`, choose and switch devices with Fleet
commands:

```bash
fleet hosts
fleet use radxa
cd /tmp
pwd

fleet use wsl2-local
fleet env
hostname

fleet run --host radxa -- 'hostname'
```

Use `fleet use <device>` to change the current target for that Agent session.
After that, use ordinary shell commands for stateful work on the current target.
Use `fleet run --host <device> -- <cmd>` for one command on another device
without changing the current target.

## Troubleshooting

If `codex`, `claude`, or `opencode` says the real command is not found, install
the real Agent command first or put its directory after `~/.rpty/bin` in `PATH`.

If `fleet list --json` fails, verify the device inventory:

```bash
ls ~/.rpty/bin/fleet_backend/devices.json
```

If an Agent appears to share a remote shell unexpectedly, check:

```bash
echo "$RPTY_SESSION"
fleet env
```

Set a new session explicitly when needed:

```bash
RPTY_SESSION=my-task codex
```

Destroy the current remote PTY session when finished:

```bash
fleet cleanup <device>
```
