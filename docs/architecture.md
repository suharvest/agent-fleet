# Unified Fleet + PTY Router Architecture

## Core Decision

The router should own a real PTY connected to system OpenSSH:

```text
Agent shell
  -> rpty local process
  -> local PTY
  -> ssh -tt <host> 'tmux new-session -A -s <session>'
  -> remote tmux
  -> remote bash
```

Fleet remains the established device runtime: inventory, status, bootstrap,
transfer, WSL recovery, detached jobs, and human SSH/tmux entry. The PTY router
adds a different transport for local Agent shell routing: exact-byte writes to a
persistent PTY. Final form: one Fleet-compatible CLI containing all modes.

## Rust Components

### CLI

Commands:

```bash
fleet shell
fleet hosts
fleet use <host>
fleet where
fleet attach
fleet logs
fleet doctor
fleet agent <cmd>
```

During development, the Rust binary can expose the same surface as `rpty` until
the compatibility story for the existing `fleet` command is ready.

Suggested crates:

- `clap` for CLI parsing.
- `serde`, `serde_json`, `toml` for config/state.
- `portable-pty` for local PTY management.
- `tokio` for async process and IO orchestration.
- `anyhow`/`thiserror` for error reporting.
- `tracing` for structured logs.

### Host Registry

Sources:

- `~/.rpty/config.toml`
- existing Fleet inventory
- existing OpenSSH config aliases

The runtime host model should be Fleet-compatible, but the PTY execution layer
should use a normalized host structure internally:

```rust
struct HostConfig {
    name: String,
    ssh_target: String,
    default_cwd: Option<String>,
    tmux_session: String,
    enabled: bool,
    source: HostSource,
}

enum HostSource {
    Manual,
    Fleet,
}
```

Fleet-derived hosts must keep enough metadata for real devices:

- SSH port;
- user;
- auth mode;
- gateway and WSL distro;
- tags and owner;
- bootstrap status;
- whether tmux is installed.

Do not require users to rewrite their long-lived Fleet inventory into OpenSSH
aliases before the PTY router can use it.

### Session Manager

One local session maps to one remote tmux session per host:

```text
local session: default
  host: wsl2-local -> tmux: rpty-default-wsl2-local
  host: radxa      -> tmux: rpty-default-radxa
```

State files:

```text
~/.rpty/state/
  sessions/default.json
  logs/default/<host>.raw.log
  logs/default/<host>.events.jsonl
```

### Command Execution

For non-interactive commands, the router writes bytes to the remote PTY and
injects an exit marker:

```bash
<user command>
printf '\n__RPTY_EXIT__:<nonce>:%s\n' "$?"
```

The marker is parsed from PTY output and removed from user-visible output. Raw
output remains available in logs.

Remote shell startup should source Fleet bootstrap config if present:

```bash
[ -f "$HOME/.profile.d/mirrors.sh" ] && . "$HOME/.profile.d/mirrors.sh"
```

This preserves Fleet's existing network/mirror behavior inside rpty sessions.

### Relationship To Fleet Jobs

For batch jobs over roughly five minutes, Fleet already has:

```bash
fleet exec --detach <device> -- <command>
fleet jobs <device>
fleet log <device> <job-id>
fleet kill-job <device> <job-id>
```

The unified tool should use or recommend that path for non-interactive jobs.
The PTY router's own long-running behavior is for interactive foreground work
inside tmux, where the Agent needs shell state, prompts, and follow-up commands.

### Attach Mode

`attach` temporarily gives the user the real interactive PTY stream for the
current host. It is required for:

- editors such as vim/nano;
- `top`, `htop`, `less`;
- REPLs;
- `git add -p`;
- long-running processes with live TUI output.

Leaving attach mode must not kill the remote tmux session.

## Error Model

Every command produces an execution envelope:

```rust
struct ExecResult {
    host: String,
    command_id: String,
    raw_log_path: PathBuf,
    visible_output: String,
    exit_code: ExitCodeState,
    transport: TransportState,
    parser: ParserState,
}

enum ExitCodeState {
    Code(i32),
    TimedOut,
    Interrupted,
    Unknown,
}
```

The key rule: parser failure is never success. If parsing fails, surface raw
output and mark the command as ambiguous.
