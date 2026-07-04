# Product Direction: Unified Fleet + PTY Router

## Final Shape

The final product should be one local tool, not two parallel tools.

It should combine:

- Fleet's existing device inventory, auth, status, bootstrap, transfer, WSL,
  detached jobs, and human remote workflows;
- the new Remote PTY Router capability for local Coding Agents.

Working name during development can remain `rpty`, but the long-term CLI should
either become the next `fleet` or expose a compatibility alias so existing
workflows keep working.

## Why Unified

Fleet already encodes years of real device operations:

- password and key auth;
- non-default SSH ports;
- WSL gateway recovery;
- domestic mirror/proxy bootstrap;
- uv/Python deployment rules;
- direct device-to-device transfer;
- sudo behavior on Jetson/RPi/edge devices;
- detached jobs and logs;
- human `ssh` and `work-enter` sessions.

Keeping rpty separate would force users and Agents to choose between two mental
models for the same device cluster. The PTY router should be a new execution
mode inside the Fleet runtime model.

## Compatibility Principle

Existing Fleet commands are product contracts. The unified tool must preserve
their behavior unless explicitly migrated:

```bash
fleet list --json
fleet status --json
fleet exec --sudo --timeout 300 <device> -- apt-get update
fleet exec --literal <device> -- python -c 'print("x")'
fleet exec --detach <device> -- <long-command>
fleet jobs <device>
fleet log <device> <job-id>
fleet transfer <src> <dst>
fleet work-sync <device> <local> <remote> --push
fleet work-enter <device> <remote-dir>
fleet wsl <device> restart
```

The new mode adds:

```bash
fleet shell
fleet use <device>
fleet agent codex
fleet agent claude
fleet attach
```

or, during development:

```bash
rpty shell
rpty use <device>
rpty agent codex
```

## Execution Modes

The unified tool should expose distinct execution modes instead of forcing one
transport to handle everything:

| Mode | Existing Fleet Capability | Best For |
|---|---|---|
| `exec` | blocking Paramiko SSH command | short deterministic commands |
| `exec --stream` | live streaming SSH command | builds with live output |
| `exec --detach` | `/tmp/fleet-jobs` nohup jobs | non-interactive long jobs |
| `ssh` / `work-enter` | human interactive SSH/tmux | human remote sessions |
| `shell` / `agent` | new persistent PTY router | local Agent terminal routing |
| `transfer` / `work-sync` | SFTP/tar/rsync | file and workspace movement |

This preserves Fleet's tested behavior while adding the one missing mode:
exact-byte persistent terminal routing.

## Migration Strategy

1. Build and validate PTY router as a Rust module.
2. Add a Fleet-compatible command surface in Rust.
3. Call the existing Python Fleet CLI for commands not yet ported.
4. Gradually port stable Fleet capabilities to Rust only when tests cover their
   real behavior.
5. Keep `fleet` command compatibility throughout.

The first useful integrated version can be a hybrid:

```text
fleet-rs
  shell/agent/use/attach  -> native Rust PTY router
  list/status/bootstrap   -> call existing fleet.py, parse JSON safely
  exec/transfer/work-*    -> call existing fleet.py unchanged
```

This avoids breaking operational workflows while making the new PTY mode
available early.

## Non-Negotiables

- Never print device passwords.
- Preserve Fleet's bootstrap/mirror behavior.
- Preserve Fleet's sudo rules.
- Preserve WSL gateway recovery behavior.
- Preserve direct transfer as default and `--relay` fallback.
- Preserve detached job metadata and log conventions.
- Preserve raw command evidence when parser behavior is ambiguous.
