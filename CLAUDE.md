# Claude Code Fleet Usage

This workspace provides a unified `fleet` CLI for local edge devices and
persistent remote PTY sessions.

## Main Workflow

Run local doctor once. It installs Fleet router shims and auto-configures known
local Agents such as Codex, Claude Code, and OpenCode when their real commands
exist:

```bash
fleet doctor --fix
export PATH="$HOME/.rpty/bin:$PATH"
```

Then start Claude Code directly:

```bash
claude
```

Inside Claude Code, choose devices with Fleet commands:

```bash
fleet hosts
fleet use <device>
fleet where
fleet env
fleet run -- 'pwd && hostname'
```

Switch devices when needed:

```bash
fleet use radxa
fleet run -- 'cd /tmp && export X=1'
fleet use wsl2-local
fleet run -- 'hostname'
fleet run --host radxa -- 'echo "$X"'
```

`fleet use <device>` changes the current target for this Claude session only.
`fleet run --host <device> -- <cmd>` targets another device for one command
without changing the current target.

## Command Choice

- Use `fleet run` when cwd, env vars, virtualenvs, or shell state should persist.
- Use `fleet env` before assuming current remote cwd or environment.
- Use `fleet exec` for short stateless checks.
- Use `fleet exec --sudo` for privileged package, Docker, or systemd operations.
- Use `fleet exec --detach`, `fleet jobs`, and `fleet log` for long
  non-interactive jobs.
- Use `fleet push`, `fleet pull`, `fleet transfer`, or `fleet work-sync` for
  files.
- Use `fleet cleanup [device]` when the current remote PTY session should be
  destroyed.

Do not parse Fleet inventory files directly and do not print device passwords.
