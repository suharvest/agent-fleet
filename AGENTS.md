# Agent Instructions

This repository is building a unified Fleet + PTY Router tool.

## Product Direction

The final product should be one Fleet-compatible CLI, not two separate tools.
Existing Fleet behavior is the device-runtime contract. The new PTY router is an
additional execution mode for local Coding Agents.

The binary supports both `fleet` and `rpty` invocation names. Prefer `fleet` in
docs and user-facing examples.

Agent-facing usage guidance lives in `skills/fleet-pty-router/SKILL.md`.
Keep it aligned with this file whenever command behavior changes.

Final command surface should support:

```bash
fleet shell
fleet use <device>
fleet run -- <cmd>
fleet doctor --fix
codex
claude
opencode
```

while preserving existing Fleet commands.

Cross-platform support is required:

- macOS/Linux install command shims as symlinks or shell scripts.
- Windows installs `.cmd` shims.
- `doctor --fix --write-shell-profile` writes the appropriate shell profile
  entry for zsh/bash/fish or PowerShell.
- Remote PTY mode still requires `tmux` on the target device.

## Fleet Rules

Fleet is mature and based on real device workflows. Do not replace it casually.

Preserve:

- `exec --sudo` for privileged commands;
- `exec --literal` for `python -c`, `bash -c`, awk/sed, heredoc-adjacent cases;
- `exec --stream` for live build output;
- `exec --detach` plus `jobs/log/kill-job` for non-interactive long jobs;
- `bootstrap` mirror/proxy behavior;
- direct `transfer` default and `--relay` fallback;
- WSL gateway recovery through `wsl`;
- `work-enter` and `work-sync`.

Never print device passwords.

## Development Workflow

Run before finishing changes:

```bash
cargo fmt -- --check
cargo test
```

For real-device validation, prefer:

```bash
cargo run -- doctor radxa
cargo run -- run --host radxa -- 'echo $(hostname)'
cargo run -- doctor wsl2-local
cargo run -- run --host wsl2-local -- 'echo $(hostname)'
cargo run -- install /tmp/rpty-shim-test
cargo run --bin fleet -- run --host radxa -- 'echo $(hostname)'
cargo run --bin fleet -- env radxa
```

Commands that access Fleet devices may need approval outside the sandbox.

## Routing Constraints

Do not send arbitrary Agent commands as quoted SSH strings.

The router should preserve command bytes by using the tmux buffer path:

```text
local temp payload -> fleet push -> tmux load-buffer -> tmux paste-buffer
```

Always keep raw output before parser cleanup. Parser failure is never success.

## State And Locking

Remote PTY sessions are named:

```text
rpty-<RPTY_SESSION>-<device>
```

Default behavior: installed Agent shims such as `~/.rpty/bin/codex`,
`~/.rpty/bin/claude`, and `~/.rpty/bin/opencode` generate a unique
`RPTY_SESSION` when one is not already set. Different Agent launches therefore
use different remote tmux sessions by default. `fleet doctor --fix` should
discover installed known Agents and create their shims automatically.

The preferred workflow is to launch Codex, Claude Code, or OpenCode directly,
then select devices from inside the Agent:

```bash
codex
claude
opencode
fleet hosts
fleet use <device>
fleet run -- 'pwd'
fleet run --host <other-device> -- 'hostname'
```

When a generated Agent session starts, it may inherit the current default host
only as a convenience. Subsequent `fleet use <device>` calls inside that Agent
write to the session's own state, not the global default host. Use
`fleet run --host` when a command should target a different device without
changing the current device for bash shim calls.

Use an explicit `RPTY_SESSION` only when you intentionally want to resume or
share a previous PTY session:

```bash
RPTY_SESSION=agent-task-1 codex
```

`fleet agent <cmd>` remains available as a generic lower-level wrapper for
commands that do not have an installed same-name shim.

The tool creates local lock files before PTY writes:

```text
~/.rpty/state/sessions/<RPTY_SESSION>/locks/<device>.lock
```

If the lock is held by a live process, choose another `RPTY_SESSION` or wait.

Use `fleet env [device]` to inspect the current remote environment. Use
`fleet cleanup [device]` to kill the current remote tmux session and remove temp
payloads.

## Agent Command Choice

Agents should choose commands using this rule:

- Use `fleet run` or bash shim when cwd/env/venv/shell state should persist.
- Use `fleet use <device>` inside the Agent to change the current bash-shim
  target.
- Use `fleet run --host <device> -- <cmd>` for one command on another device
  without changing the Agent's current target.
- Use `fleet env` before assuming where the current remote shell is.
- Use `fleet exec` for short stateless one-shot checks.
- Use `fleet exec --sudo` for apt/docker/systemd or privileged writes.
- Use `fleet exec --detach` for long non-interactive jobs.
- Use `fleet transfer`, `fleet push`, `fleet pull`, or `fleet work-sync` for files.
- Use `fleet cleanup` when the task's PTY session should be destroyed.

Do not intentionally share one `RPTY_SESSION + device` between multiple Agents
unless the user explicitly asks for a shared shell.

## Bash Shim

When invoked as `bash`, the binary intercepts common Agent calls:

```bash
bash -lc '<cmd>'
bash -c '<cmd>'
```

and routes them to the current host. Unsupported bash calls fall back to
`/bin/bash`. Use `RPTY_BASH_PASSTHROUGH=1` to force local bash.

## Productization Checks

Before calling the tool product-ready, verify:

```bash
cargo run --bin fleet -- --help
cargo run -- install /tmp/rpty-product-test
/tmp/rpty-product-test/fleet list --owner personal --json
RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet run --host radxa -- 'echo $(hostname)'
RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet cleanup radxa
```
