# AgentFleet

Use this skill when working with local Fleet devices through this repository's
unified `fleet` CLI, especially when an Agent needs persistent remote shell
state across commands.

## Core Rule

Use `fleet` as the single entry point. Existing Fleet commands remain valid, and
PTY routing is an additional mode for stateful Agent work.

Do not read or parse Fleet device inventory files directly. Use `fleet list`,
`fleet status`, `fleet match`, and the pass-through Fleet commands.

Never print device passwords or secrets.

Implementation note: normal Fleet commands are served by the Rust native
backend. The bundled Python backend is legacy fallback only; agents should not
select it unless explicitly debugging with `RPTY_FLEET_NATIVE=0`.

## Pick The Right Mode

Use persistent PTY mode when command state matters:

```bash
fleet use <device>
cd /path
source .venv/bin/activate
python script.py
python another_step.py
fleet env
```

After `fleet use <device>`, prefer ordinary shell commands. The installed bash
shim routes common Agent shell calls (`bash -lc` / `bash -c`) to the current
device. Use `fleet run -- <cmd>` only when an explicit wrapper is clearer, and
use `fleet run --host <device> -- <cmd>` for one command on another device.

Use regular Fleet execution when the command is stateless:

```bash
fleet exec <device> -- hostname
fleet exec --literal <device> -- 'python -c "print(1)"'
fleet exec --sudo <device> -- apt-get update
fleet exec --detach <device> -- ./long-job.sh
fleet jobs <device>
fleet log <device> <job-id>
```

Do not choose PTY mode only because a command is long-running. For long
non-interactive jobs such as builds, installs, training, or batch scripts,
prefer `fleet exec --detach` and monitor with `fleet jobs` / `fleet log`.
Use PTY mode for long work only when an interactive shell state must survive
across multiple Agent steps.

Use Fleet transfer commands for files:

```bash
fleet push <device> local/path remote/path
fleet pull <device> remote/path local/path
fleet transfer src:/path dst:/path
fleet work-sync <device> <local-dir> <remote-dir>
```

## Agent Sessions

Preferred workflow: run local doctor once, launch Codex, Claude Code, or
OpenCode directly, then select or switch devices from inside the Agent.

```bash
fleet doctor --fix --write-shell-profile
codex
claude
opencode
fleet hosts
fleet use radxa
cd /tmp
pwd
fleet use wsl2-local
fleet env
fleet run --host radxa -- 'hostname'
```

If `RPTY_SESSION` is not already set, the installed Agent shim creates a unique
session. Different Agent launches therefore use different remote tmux sessions
by default. `fleet doctor --fix --write-shell-profile` discovers known local
Agents, currently `codex`, `claude`, and `opencode`, installs matching shims for
the ones it finds, and writes the local shim directory to the user's shell
profile.

Fleet backend is bundled. If device inventory is missing on a fresh install,
create the private file from the example and fill in real hosts/credentials:

```bash
cp ~/.rpty/bin/fleet_backend/devices.example.json ~/.rpty/bin/fleet_backend/devices.json
chmod 600 ~/.rpty/bin/fleet_backend/devices.json
```

Use an external Fleet backend only when intentionally overriding the bundled
one:

```bash
fleet config --fleet-py /path/to/fleet.py
```

Add another Agent command to doctor discovery:

```bash
fleet config --agent gemini
fleet doctor --fix
```

On macOS/Linux, shims are symlinks or shell scripts. On Windows, shims are
`.cmd` files and `--write-shell-profile` updates the PowerShell profile.

Set `RPTY_SESSION` only when intentionally resuming or sharing a PTY session:

```bash
RPTY_SESSION=task-debug codex
```

The generated Agent session may inherit the current default host as a
convenience. Later `fleet use <device>` calls inside that Agent affect only that
session. Use `fleet run --host <device> -- <cmd>` for one command on another
device without changing the current bash-shim target.

`fleet agent <cmd>` remains available as a generic lower-level wrapper when a
same-name shim is not installed.

## Status And Safety

Before assuming current remote state, run:

```bash
fleet where
fleet env
```

`fleet env` reports the current session, device, hostname, cwd, shell, user,
virtualenv, Python path, and tmux session.

The router locks writes per `RPTY_SESSION + device`. If a lock is held by a live
process, wait or choose another `RPTY_SESSION`.

Raw captures are kept under:

```text
~/.rpty/state/sessions/<RPTY_SESSION>/logs/<device>.raw.log
```

Parser failure is not success. Inspect raw logs when output looks truncated or
the exit marker is missing.

## Cleanup

Destroy the current PTY session on a device with:

```bash
fleet cleanup [device]
```

This kills the remote tmux session for the current `RPTY_SESSION + device` and
removes temporary payload files. It does not remove regular Fleet state or local
logs.

AgentFleet does not auto-destroy PTY sessions after each command; persistence is
the feature. At the end of a task, run `fleet cleanup [device]` unless the user
explicitly wants to resume that shell later. If many old AgentFleet tmux
sessions exist on a device, use the explicit bulk cleanup:

```bash
fleet cleanup --all <device>
```

`--all` only targets remote tmux sessions whose names start with `rpty-`.

## Bash Shim

After installation, the `bash` shim intercepts common Agent shell calls:

```bash
bash -lc '<cmd>'
bash -c '<cmd>'
```

Unsupported bash calls fall back to `/bin/bash`. Set
`RPTY_BASH_PASSTHROUGH=1` to force local bash.
