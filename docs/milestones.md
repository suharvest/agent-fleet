# Development Milestones

## M0: Repository Setup

- Create Rust crate.
- Document final AgentFleet direction.
- Add CLI skeleton.
- Add structured logging.
- Add config/state path helpers.
- Add tests for config parsing.

Exit criteria:

- `cargo test` passes.
- `rpty --help` works.

## M1: Local tmux Prototype

Implement command routing against local tmux sessions only.

Commands:

- `rpty shell`
- `hosts`
- `use local-a`
- `where`
- normal command execution

Exit criteria:

- two local sessions preserve independent cwd/env;
- multiline commands and heredocs survive;
- exit code marker works;
- raw logs are written;
- parser failure cannot become silent success.

## M2: SSH + Remote tmux

Use system OpenSSH and remote tmux:

```bash
ssh -tt <target> 'tmux new-session -A -s <session>'
```

Exit criteria:

- `wsl2-local` and `radxa` pass state preservation;
- long task survives local SSH process restart;
- `attach` works for the current host;
- transport errors are explicit.

## M3: Fleet Discovery And Doctor

Integrate Fleet as the inventory/status source for the unified tool.

Commands:

- `rpty fleet list`
- `rpty fleet import`
- `rpty doctor`

Exit criteria:

- online/offline status is displayed;
- missing tmux is detected;
- no device password is printed;
- Fleet JSON parse errors surface with raw stderr/stdout references;
- Fleet metadata is preserved: port, user, tags, owner, gateway, WSL distro;
- doctor understands Fleet bootstrap status and mirror/proxy setup;
- docs explicitly map rpty behavior against `fleet exec --literal`, `--raw`,
  `--stream`, `--detach`, `work-enter`, `work-sync`, `transfer`, and `wsl`.

## M4: Agent Launcher

Add:

```bash
rpty agent codex
rpty agent claude
fleet agent codex   # final compatible surface
fleet agent claude
```

The launcher sets session environment and starts the Agent in a router-aware
shell.

Exit criteria:

- current host persists across Agent shell calls;
- logs are grouped by local session id;
- Agent does not need to emit SSH commands for normal remote terminal work.

## M5: Bash Shim

Add optional PATH shim for common Agent shell calls such as:

```bash
bash -lc "pytest -q"
```

Exit criteria:

- common `bash -lc` commands route to the selected host;
- unsupported bash flags fail explicitly;
- local passthrough mode exists for commands that must run locally.

## M6: Hardening

- Ctrl-C handling.
- timeout handling.
- prompt/marker collision tests.
- log rotation.
- session recovery tests.
- unsafe command audit-only policy.

Exit criteria:

- no known silent output loss path remains;
- every ambiguous command result links to raw logs.

## M7: Unified Fleet Compatibility

Make the Rust tool capable of acting as the user-facing Fleet-compatible CLI.

Transition strategy:

- keep existing `fleet.py` as the source of truth for commands not yet ported;
- add pass-through subcommands for existing Fleet operations;
- add native Rust implementation for PTY-router commands;
- preserve output shape and exit codes for existing Fleet commands;
- keep a compatibility alias or wrapper for `fleet`.

Exit criteria:

- existing common Fleet commands behave the same through the unified binary;
- PTY-router commands are available under the same CLI;
- no duplicated device inventory format is required;
- no password is printed or copied into logs.
