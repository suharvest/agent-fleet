# Agent Instructions

This repository builds AgentFleet: one Fleet-compatible CLI for local edge
devices and persistent remote PTY sessions.

## Product Direction

AgentFleet should be one Fleet-compatible CLI, not two separate tools. Existing
Fleet behavior is the device-runtime contract. The PTY router is an additional
execution mode for local Coding Agents.

The binary supports both `fleet` and `rpty` invocation names. Prefer `fleet` in
docs and user-facing examples.

Agent-facing usage guidance lives in `skills/agentfleet/SKILL.md`.
Keep it aligned with this file whenever command behavior changes.

Final command surface should support:

```bash
fleet shell
fleet use <device>
<ordinary shell commands>
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
- `exec` default quoting: one token after `--` is a shell snippet, two or more
  are argv and get shell-quoted; `--shell` opts back into the old
  join-with-spaces behaviour;
- `exec` always runs the remote command through `bash -c '<one argument>'`, and
  `--sudo` puts the whole command under sudo rather than only its first
  statement;
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

### Transfer Throughput

`push`, `pull` and `transfer` all stream through `sftp_put`/`sftp_get` in
`src/fleet_native.rs`. Both copy through a 4 MiB buffer on purpose: libssh2
pipelines SFTP packets only as far as one call's buffer reaches, so a small
buffer costs one round trip per write and pins transfers near 1 MB/s. Do not
replace the copy loop with `std::io::copy`, whose buffer is 8 KiB.

After touching the transfer path, run the device-gated benchmark:

```bash
RPTY_BENCH_DEVICE=wsl2-local cargo test --release --test transfer_throughput -- --nocapture
```

It is skipped when `RPTY_BENCH_DEVICE` is unset. Reference numbers for
Mac -> wsl2-local over a 3.8 ms LAN: 200 MB push in ~6.7 s, pull in ~4.6 s.
Raw HTTP over the same link reaches 93 MB/s while SSH tops out near 30 MB/s,
which the paramiko backend also measures, so ~30 MB/s is the transport
ceiling and not a defect to chase.

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
pwd
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
payloads. Use `fleet cleanup --all <device>` only when intentionally clearing
all remote AgentFleet tmux sessions named `rpty-*` on that device.

## Agent Command Choice

Agents should choose commands using this rule:

- Use `fleet use <device>` inside the Agent to change the current bash-shim
  target.
- Use ordinary shell commands after `fleet use <device>` when
  cwd/env/venv/shell state should persist.
- Use `fleet run -- <cmd>` only when an explicit, scriptable wrapper is clearer
  than relying on the bash shim.
- Use `fleet run --host <device> -- <cmd>` for one command on another device
  without changing the Agent's current target.
- Use `fleet env` before assuming where the current remote shell is.
- Use `fleet exec` for short stateless one-shot checks.
- Use `fleet exec --sudo` for apt/docker/systemd or privileged writes.
- Use `fleet exec --detach` for long non-interactive jobs.
- Use `fleet transfer`, `fleet push`, `fleet pull`, or `fleet work-sync` for files.
- Use `fleet cleanup` when the task's PTY session should be destroyed. Do this
  at task end unless the user wants to resume the remote shell later.

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

Interception is opt-in per session. The shim routes only when `RPTY_SESSION`
is set and that session ran `use <device>`; it never falls back to the global
`current_host`. The shim sits on `PATH` for every process, so a global default
would capture `bash -c` calls nobody aimed at Fleet — git hooks, pre-commit,
build scripts, `child_process` — and run them on a remote host in the wrong
working directory. `where` reports the shim's routing state; `unuse` clears
the current host and returns the session to local bash.

## Productization Checks

Before calling the tool product-ready, verify:

```bash
cargo run --bin fleet -- --help
cargo run -- install /tmp/rpty-product-test
/tmp/rpty-product-test/fleet list --owner personal --json
RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet run --host radxa -- 'echo $(hostname)'
RPTY_SESSION=verify-product /tmp/rpty-product-test/fleet cleanup radxa
```
