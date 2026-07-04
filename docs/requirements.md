# AgentFleet Requirements

## Goal

AgentFleet is a unified Fleet + Remote PTY routing tool. It keeps Fleet's
existing device-management behavior and adds a local terminal routing layer for
Coding Agents such as Codex, Claude Code, Aider, and Gemini CLI.

The Agent continues to use ordinary shell commands. The router sends those
commands to a selected remote host, while preserving a persistent shell state
per host.

Expected user flow:

```bash
use radxa
cd /tmp
export TARGET=radxa
pwd

use wsl2-local
cd /workspace/project
pytest -q

use radxa
echo $TARGET
```

## Core Requirements

- Preserve existing Fleet workflows and semantics.
- Provide a migration path where `fleet` remains the stable user-facing command.
- Manage multiple configured hosts.
- Switch current host with `use <host>`.
- Keep one persistent remote tmux session per local session and host.
- Preserve cwd, environment variables, foreground/background processes, and shell history per host.
- Support long-running remote tasks surviving local Agent restarts or SSH disconnects.
- Return command output without silently dropping stdout, stderr, exit code, or parser errors.
- Use normal shell syntax from the Agent perspective.
- Avoid wrapping arbitrary Agent commands inside quoted `ssh host "..."` strings.
- Support attach mode for interactive TUI programs.
- Support session recovery by local session id.

## Existing Fleet Compatibility

The unified tool must preserve these Fleet capabilities:

- `list`, `status`, `match`, `scan`;
- `exec` with `--sudo`, `--literal`, `--stream`, `--detach`, `--raw`,
  `--tag`, `--host`, and `--json`;
- `jobs`, `log`, `kill-job`;
- `bootstrap` with profiles and check mode;
- `push`, `pull`, `transfer`, `work-sync`;
- `ssh`, `work-enter`, `work-monitor`;
- `wsl status|restart|exec`.

The new PTY router is a new execution mode, not a replacement for these modes.

## Non-Goals For v0.1

- Remote filesystem mount.
- LSP, debugger, IDE extension host, or workspace index.
- Remote daemon installation.
- MCP protocol.
- Web UI or TUI dashboard.
- Multi-Agent concurrent locking beyond basic session isolation.

## Meta Commands

These commands are intercepted locally:

```bash
hosts
use <host>
where
sessions
attach
detach
logs
status
exit-router
```

All other input is sent to the current host's remote PTY.

## Acceptance Criteria

### Host Switching

```bash
fleet shell
hosts
use wsl2-local
pwd
use radxa
pwd
```

Both hosts execute commands successfully, and switching hosts does not destroy
previous host sessions.

### State Preservation

```bash
use radxa
cd /tmp
export RPTY_TEST=hello

use wsl2-local
pwd

use radxa
pwd
echo $RPTY_TEST
```

Expected output includes:

```text
/tmp
hello
```

### Long Task Survival

```bash
use radxa
sleep 300
```

After local disconnect or router restart, the remote tmux session still exists.
The user can reattach or inspect output.

### Exit Code Integrity

```bash
use radxa
false
```

The router reports non-zero exit status and does not convert failures into
successful empty output.

### No Silent Output Loss

For every command, the router must preserve:

- raw PTY bytes in a local log;
- parsed user-visible output;
- exit status or explicit "unknown exit status";
- parser errors, if parsing failed;
- transport errors, if SSH or tmux failed.

If output cannot be parsed, the command is failed-open with raw output attached,
not treated as success.
