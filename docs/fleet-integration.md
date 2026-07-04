# Fleet Integration In The Unified Tool

## Role Of Fleet

Fleet should be used for:

- host discovery;
- online/offline status;
- tags, owner, hardware specs;
- bootstrap checks;
- one-time setup commands such as installing tmux when approved;
- WSL2 status/restart/exec via gateway hosts;
- file movement through `push`, `pull`, `transfer`, and `work-sync`;
- existing background jobs through `exec --detach`, `jobs`, `log`, and `kill-job`;
- existing human workflows through `ssh` and `work-enter`.

The PTY router should not be framed as a separate Fleet replacement. It should
become one missing Fleet surface: a local Agent terminal router with exact-byte
PTY input and per-host persistent shell state.

## Fleet Exec Boundary

`fleet exec` is mature and has several modes that must be preserved:

- `--sudo` for privileged work with PTY/password injection;
- `--literal` for `python -c`, `bash -c`, awk/sed, and complex quoting;
- `--stream` for live output;
- `--detach` for long background jobs;
- `--raw` for Windows/PowerShell and no POSIX wrapper;
- `--tag`, `--host`, and `--json`.

For short deterministic remote commands, the unified tool should keep using
Fleet's existing exec semantics. For arbitrary Agent shell input, it needs a
different transport: a persistent PTY attached to remote tmux/bash.

This is not because Fleet is poorly designed. Fleet deliberately optimizes for
operational commands, bootstrap, sudo, transfer, WSL recovery, and deterministic
job management. Agent terminal routing has a different requirement: preserve
the exact command bytes and shell state across commands.

Observed issue:

```bash
fleet exec radxa -- tmux send-keys -l -t rpty_fleet_probe 'sleep 3; echo RPTY_LONG_DONE'
```

The semicolon was interpreted by a shell layer rather than preserved as PTY
input. Fleet already has `--literal` and heredoc warnings for this class of
problem; the PTY router should avoid the class entirely by writing to a real
PTY.

- shell quoting breaks;
- heredocs become fragile;
- JSON/regex/script text can be reinterpreted;
- output and parser state can be conflated if the caller treats transport
  success as command success.

## Compatibility With Fleet Defaults

Fleet's normal exec path sources mirror/proxy config:

```bash
[ -f "$HOME/.profile.d/mirrors.sh" ] && . "$HOME/.profile.d/mirrors.sh"
```

remote shell initialization should preserve this behavior. When creating a
remote tmux shell, the unified tool should source the same bootstrap config if
present, so Fleet-configured devices behave the same inside PTY-router sessions.

Fleet's auth model includes password devices and `sshpass`-based interactive
paths. The unified tool must either:

- launch through a Fleet-provided SSH command builder in the future; or
- support Fleet device fields directly, including port, user, password, gateway,
  and WSL metadata.

Do not assume every device has a clean OpenSSH alias.

## Preventing Swallowed Results

Previous Fleet usage sometimes lost or hid parsed results. AgentFleet
must treat this as a design constraint, not an incidental bug.

Required defenses:

1. Always keep raw output.

   Raw PTY bytes are appended to a per-session raw log before any parsing.

2. Separate transport, parser, and command result.

   A successful SSH/tmux read does not mean command success. A parser failure
   does not mean command success. Missing marker does not mean exit code 0.

3. Use structured execution envelopes.

   Each command returns visible output plus metadata:

   ```json
   {
     "host": "radxa",
     "command_id": "...",
     "exit_code": 0,
     "transport_ok": true,
     "parser_ok": true,
     "raw_log_path": "...",
     "visible_output": "..."
   }
   ```

4. Fail open with raw evidence.

   If parsing fails, return raw output and an explicit parser error. Never drop
   the output and never report a silent success.

5. Keep stderr-like diagnostics out of user-visible command output, but not out
   of logs.

6. Add regression tests for tricky output:

   - empty output with exit 0;
   - empty output with exit non-zero;
   - binary-ish bytes;
   - output containing fake exit markers;
   - output without a prompt;
   - long output;
   - interleaved command echo and command output;
   - parser timeout.

## Fleet Device Reality

Validated online devices:

| Device | Status | tmux | Result |
|---|---:|---:|---|
| `wsl2-local` | online | `3.4` | passed state preservation |
| `radxa` | online | `3.3a` | passed state preservation and long-task survival |

Online devices missing tmux during validation:

- `orin-nano`
- `orin-nx`
- `cloud-server`
- `harvest-pi`
- `cat-remote`

Implication: `doctor` must detect missing tmux and present an explicit
bootstrap step. v0.1 should not assume every Fleet device is ready. It should
prefer existing Fleet bootstrap and package-management rules.

## Inventory Strategy

In the final unified CLI, no separate import should be necessary: Fleet's
inventory is the source of truth. During transition, a Rust prototype may still
use:

```bash
rpty fleet import --owner personal
rpty doctor --host radxa
rpty doctor --host orin-nano
```

Transition imports should produce host config, not credentials:

```toml
[hosts.radxa]
ssh = "radxa"
tmux_session = "rpty-default-radxa"
source = "fleet"
enabled = true
```

Credentials remain in SSH config, ssh-agent, or the existing Fleet system. If
rpty needs password-based external SSH for a Fleet device, that behavior must be
explicitly designed rather than accidentally bypassing Fleet.
