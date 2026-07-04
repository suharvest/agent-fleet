# Validation Notes

Date: 2026-07-04

## Community Scan

No exact open-source match was found for:

> local Coding Agent bash shim + multi-Fleet-host routing + persistent remote
> tmux/PTY state per host.

Related projects:

- `tmate`: terminal sharing based on tmux.
- `EternalTerminal`: reconnectable remote shell.
- `sshx`: web-based collaborative terminal with reconnection.
- `Teleport`: enterprise SSH access, audit, and recording.
- `Coder` / `DevPod`: remote development environments.
- `gptme`: terminal Agent runtime with persistent-agent concepts.

These projects are useful references but do not replace the proposed router.

## Local tmux Validation

Validated with Python driving local tmux:

- two sessions, `rpty_probe_a` and `rpty_probe_b`;
- independent cwd and env;
- heredoc containing `$`, backticks, quotes, and backslashes preserved;
- switching back to session A preserved `/tmp` and exported variables;
- `false` returned exit code `1` through an injected marker.

Result: core tmux session model is viable.

## Fleet Status Summary

Fleet status showed enough online diversity for validation:

- x86_64 WSL2 GPU host;
- ARM64 RK3588 board;
- ARM64 Jetson boards;
- ARM64 Raspberry Pi/Hailo;
- cloud server.

## Fleet tmux Availability

Passed tmux checks:

- `wsl2-local`: `tmux 3.4`
- `radxa`: `tmux 3.3a`

Missing tmux:

- `orin-nano`
- `orin-nx`
- `cloud-server`
- `harvest-pi`
- `cat-remote`

## Remote State Preservation

Validated on `wsl2-local`:

```text
cd /tmp
export RPTY_REMOTE_VAR=wsl2-local
pwd
printenv RPTY_REMOTE_VAR
```

Captured output showed:

```text
/tmp
wsl2-local
```

Validated on `radxa`:

```text
cd /tmp
export RPTY_REMOTE_VAR=radxa
pwd
printenv RPTY_REMOTE_VAR
```

Captured output showed:

```text
/tmp
radxa
```

## Long Task Survival

Validated on `radxa` with a clean tmux session:

```text
sleep 10
```

The local Fleet/SSH call returned after sending keys. A later pane capture
showed the command still running without a prompt. After completion, the same
session accepted:

```text
echo RPTY_AFTER_SLEEP
```

Captured output:

```text
RPTY_AFTER_SLEEP
```

## Rust Prototype Validation

Additional validation after implementing the Rust CLI:

- `cargo test`: 20 tests passed.
- `cargo check --target x86_64-pc-windows-gnu`: Windows target compile passed.
- `cargo run -- list --owner personal --json`: Fleet pass-through works and
  passwords remain masked.
- `cargo run -- doctor radxa`: online, tmux 3.3a.
- `cargo run -- doctor wsl2-local`: online, tmux 3.4, non-default SSH port
  works through Fleet.
- `rpty shell` with `RPTY_SESSION=verify-radxa`: heredoc content containing
  `$HOME`, backticks, single quotes, double quotes, and backslashes was written
  on `radxa` without local expansion.
- `radxa`: cwd `/tmp` and `RPTY_TEST_VAR=radxa` persisted across commands.
- `radxa`: `false` produced a non-zero command result.
- `radxa`: `sleep 8` with router timeout 3 returned an explicit timeout while
  the remote tmux session remained usable; follow-up `echo RPTY_AFTER_LONG`
  succeeded in the same session.
- `wsl2-local`: `cd /tmp`, env persistence, and `hostname` command succeeded.
- Bash shim validation: installed a temporary shim in `/tmp/rpty-shim-test`;
  `PATH=/tmp/rpty-shim-test:$PATH RPTY_SESSION=verify-shim bash -lc 'echo SHIM_HOST=$(hostname)'`
  routed to the current `radxa` host and printed `SHIM_HOST=rock-5t`.
- Product compatibility validation:
  - `cargo run --bin fleet -- --help` prints Fleet-compatible help.
  - `cargo run -- install /tmp/rpty-product-test` installs `rpty`, `fleet`,
    and `bash` symlinks.
  - `/tmp/rpty-product-test/fleet list --owner personal --json` passes through
    to existing Fleet and keeps passwords masked.
  - `RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet run --host radxa -- 'echo PRODUCT_HOST=$(hostname)'`
    prints `PRODUCT_HOST=rock-5t`.
  - `RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet cleanup radxa`
    removes the verification tmux session and temp payloads.
  - `PATH=/tmp/rpty-product-test:$PATH RPTY_SESSION=verify-product-shim bash -lc 'echo PRODUCT_SHIM=$(hostname)'`
    routes through the installed bash shim and prints `PRODUCT_SHIM=rock-5t`.
- State query validation:
  - `RPTY_SESSION=verify-env /tmp/rpty-product-test/fleet env radxa` prints a
    clean key/value summary including `hostname=rock-5t`, `pwd=/home/radxa`,
    `python=/usr/bin/python`, and `tmux_session=rpty-verify-env-radxa`.
- Lock validation:
  - a manually-created live lock at
    `~/.rpty/state/sessions/verify-lock/locks/radxa.lock` causes
    `RPTY_SESSION=verify-lock fleet run --host radxa -- ...` to fail before
    writing to tmux, with a clear "session is locked" message.
- Agent session behavior:
  - `fleet agent <cmd>` now generates a unique `RPTY_SESSION` if the caller did
    not set one, so different Agent launches use different PTY sessions by
    default. Explicit `RPTY_SESSION` is reserved for resume/share behavior.

## Windows Validation

Validated Windows-related behavior on Fleet devices:

- `home-win`: native Windows host, tags include `windows`, `desktop`,
  `tailscale`, `gpu`.
- `wsl2-local`: WSL2 Ubuntu device on the same Windows host, gateway
  `home-win`, SSH port `22222`.

Native Windows stateless execution works through Fleet:

```text
fleet exec --timeout 30 home-win -- powershell -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'
```

Result:

```text
5.1.26100.8655
```

```text
fleet exec --timeout 30 home-win -- cmd.exe /C echo fleet-windows-ok
```

Result:

```text
fleet-windows-ok
```

Persistent PTY mode works through the WSL2 device:

```text
RPTY_SESSION=verify-wsl-env fleet env wsl2-local
```

Result included:

```text
session=verify-wsl-env
device=wsl2-local
hostname=HarvestSu
shell=/bin/bash
python=/usr/bin/python3
tmux_session=rpty-verify-wsl-env-wsl2-local
```

Native Windows PTY mode is intentionally not supported because it requires a
Unix-like remote shell and `tmux`. The error now explains the correct fallback:
use `fleet exec home-win -- powershell ...` for stateless native Windows
commands, or use `wsl2-local` for persistent PTY sessions.

## Important Finding

Using `fleet exec` as a command transport can reinterpret shell metacharacters.
An attempted `tmux send-keys -l` command containing `;` was split by the remote
shell. This confirms that the router must write bytes to a real PTY instead of
passing arbitrary Agent commands through shell-quoted remote exec strings.
