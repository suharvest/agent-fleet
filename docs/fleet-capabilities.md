# Existing Fleet Capabilities

Fleet is an established local cluster management tool, not a thin SSH wrapper.
The final product should integrate Fleet and the PTY router into one CLI. rpty
is the development name for the new PTY execution mode, not a separate long-term
product line.

## Inventory And Device Model

Fleet owns the device inventory. Callers must use `fleet.py` commands instead
of parsing `devices.json` directly.

Important device fields observed through Fleet behavior:

- `host`
- `port`
- `user`
- `password`
- `owner`: `personal` or `company`
- `tags`
- `gateway`
- `wsl_distro`
- `specs`

Passwords are masked in `fleet list --json`. rpty must not print credentials.

## Auth Model

Fleet's Paramiko path tries password auth first, then falls back to SSH key and
agent auth. It uses `AutoAddPolicy` for Paramiko host keys.

Interactive paths such as `fleet ssh` and `fleet work-enter` use external
`ssh`, with `sshpass` when available for password devices. These paths currently
prefer automation over strict host key checks.

Implication for the unified tool:

- Do not assume every Fleet device has an OpenSSH alias.
- Do not require all Fleet devices to be converted to key-only auth.
- Expose the host-key policy explicitly if rpty launches external SSH.

## Exec Modes

`fleet exec` supports several distinct modes:

- normal blocking command with a 60s CLI default timeout;
- `--sudo`, using PTY + password injection + `DEBIAN_FRONTEND=noninteractive`;
- `--literal`, using `shlex.join` for `python -c`, `bash -c`, awk/sed, and complex quoting;
- `--stream`, live stdout/stderr streaming for long non-sudo commands;
- `--detach`, background jobs via `/tmp/fleet-jobs`;
- `--raw`, no POSIX shell wrapper, required for Windows/PowerShell devices;
- `--tag`, fan-out to all matching devices;
- `--host`, temporary host override;
- `--json`, structured result output.

Normal non-raw exec prepends:

```bash
export PATH="$HOME/.local/bin:$PATH";
[ -f "$HOME/.profile.d/mirrors.sh" ] && . "$HOME/.profile.d/mirrors.sh";
<command>
```

This is intentional: Fleet bootstrapping makes mirrors/proxy settings available
without the caller remembering them.

## Detached Jobs

`fleet exec --detach` is the existing answer for long-running non-interactive
jobs. It creates:

```text
/tmp/fleet-jobs/<job-id>.json
/tmp/fleet-jobs/<job-id>.log
/tmp/fleet-jobs/<job-id>.pid
```

Companion commands:

- `fleet jobs <device>`
- `fleet log <device> <job-id>`
- `fleet kill-job <device> <job-id>`

The PTY router should not replace this for batch jobs. Its long-task value is
keeping an interactive shell foreground process alive inside tmux for Agent
workflows.

## Bootstrap And Network Reality

Fleet bootstrap is critical on domestic/edge networks. It configures:

- shell env mirrors/proxy;
- Hugging Face endpoint;
- uv/pip index;
- git `insteadOf`;
- Docker registry mirrors.

Profiles include:

- `wsl2-proxy`
- `edge-mirror`
- `isolated`

The unified `doctor` should run Fleet bootstrap checks before diagnosing Python,
Docker, or download failures.

## File And Workspace Movement

Fleet provides:

- `push` / `pull` with SFTP or tar streaming for directories;
- MD5 verification after transfer;
- `transfer` between two remote devices;
- direct device-to-device transfer by default;
- `--relay` fallback through the control machine;
- `--dest-host` when source sees destination through a different LAN IP;
- `work-sync` using rsync, `.gitignore`, and standard excludes.

The PTY router should delegate file movement to these commands instead of
implementing its own file transfer in v0.1.

## Human Remote Workflows

Fleet already supports:

- `fleet ssh <device>` for interactive login;
- `fleet work-enter <device> <remote-dir> --session <name>` for entering a
  remote tmux session and launching Claude;
- `fleet work-monitor` for running a command after a tmux session disappears;
- `fleet wsl <device> status|restart|exec` for WSL2 recovery via gateway hosts.

The PTY router should not duplicate these human workflows. Its target is
different:

```text
local Agent shell -> rpty router -> remote tmux/bash
```

instead of:

```text
human terminal -> fleet work-enter -> remote Claude inside tmux
```

## Output And Parsing Lessons

Fleet already contains defenses:

- warnings for `-c` without `--literal`;
- heredoc warnings;
- `--stream` for live output;
- timeout messages with partial output;
- JSON output for structured commands;
- password stripping from sudo PTY output;
- GBK fallback decoding for Windows.

The unified implementation should preserve these lessons. The missing piece is
a command transport that writes exact bytes to a persistent PTY without
reconstructing shell text.
