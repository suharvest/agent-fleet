---
name: fleet
description: Query and manage local edge device cluster through AgentFleet. MUST use when user mentions devices, deployment targets, Jetson, remote machines, device status, device credentials, SSH login, persistent remote shell state, fleet use, or long-running remote work. NEVER parse devices.json directly; use fleet CLI.
---

# Fleet / AgentFleet

Use this skill when working with local Fleet devices through this repository's
unified `fleet` CLI, especially when an Agent needs persistent remote shell
state across commands.

## Core Rule

Use `fleet` as the single entry point. Existing Fleet commands remain valid, and
PTY routing is an additional mode for stateful Agent work.

Do not read or parse Fleet device inventory files directly. Use `fleet list`,
`fleet status`, `fleet match`, and the pass-through Fleet commands.

Never print device passwords or secrets.

Implementation note: normal Fleet commands are served by the Rust native
backend. The bundled Python backend is legacy fallback only; agents should not
select it unless explicitly debugging with `RPTY_FLEET_NATIVE=0`.

## Pick The Right Mode

Use persistent PTY mode when command state matters:

```bash
fleet use <device>
cd /path
source .venv/bin/activate
python script.py
python another_step.py
fleet env
```

After `fleet use <device>`, the installed bash shim routes shell calls made as
`bash -lc` / `bash -c` to the current device. Use `fleet run -- <cmd>` when an
explicit wrapper is clearer, and `fleet run --host <device> -- <cmd>` for one
command on another device.

> ⚠️ **Claude Code 不走 bash shim。** 它的 Bash 工具在 `/bin/zsh` 里执行命令，
> 从不调用 `bash`，所以 shim 看不到这些调用。**即使 `fleet where` 显示已路由到
> 某设备，Claude Code 敲的普通命令仍然跑在本机**——不会报错，只是跑错机器。
>
> 实测（2026-09-05，`fleet use wsl2-local` 之后）：
>
> ```
> hostname            → HedeMacBook-Air.local   ← 本机
> bash -c hostname    → HarvestSu               ← 远程设备
> ```
>
> **所以在 Claude Code 里不要依赖「use 之后直接敲普通命令」这条路径**，
> 用 `fleet exec <device> -- <cmd>`（无状态）或 `fleet run --host <device> -- <cmd>`。
> 换 harness 时先自测一次上面两条命令：输出相同才说明普通命令真的被路由。

Use regular Fleet execution when the command is stateless:

```bash
fleet exec <device> -- hostname
fleet exec --literal <device> -- 'python -c "print(1)"'
fleet exec --sudo <device> -- apt-get update
fleet exec --detach <device> -- ./long-job.sh
fleet jobs <device>
fleet log <device> <job-id>
```

### `fleet exec` 引号规则（2026-08-23 修复后）

`--` 后面的 token 数量决定拼接方式：

| 形式 | 语义 | 例子 |
|---|---|---|
| 恰好 1 个 token | 当 shell 片段：管道、重定向、`&&`、glob 都生效 | `fleet exec dev -- "ls /tmp \| wc -l"` |
| ≥ 2 个 token | 当 argv，逐个 shell-quote；单个参数里的 `#` `;` `\|` 空格 引号 全部保留 | `fleet exec dev -- grep -E "a\|b" file` |
| `--shell` | 全部 token 用空格拼成 shell 片段（旧行为，引号会丢） | `fleet exec --shell dev -- echo a \| wc -c` |
| `--literal` | 每个 token 都 quote，包括单 token | `fleet exec --literal dev -- python3 -c "print(1)"` |
| `--raw` | 原样下发，不加包装（Windows cmd/PowerShell） | `fleet exec --raw win -- dir` |

优先级：`--literal` > `--shell` > 默认混合规则。

命令整串以一个参数交给远端 `bash -c`，登录 shell 不会二次解析。`--sudo` 下整串
都在 root：`fleet exec --sudo dev -- bash -c 'id -u; id -u'` 输出两行 `0`。

历史坑（已修）：旧版把 token 用空格裸拼进远端登录 shell，`--sudo` 只有第一个 `;`
之前在 root 下跑、`#` 被当注释、单参数里的空格被再次切分——2026-07-30 记录的
「`--sudo` 与引号两种形式各有一个坑」已由该修复消除，不再需要绕道文件。

后台任务用 `--detach`，不要在命令末尾写 `&`：SSH 通道关闭会杀掉它（fleet 现在
会对结尾的 `&` 打一行 warning）。`--detach` 走 `setsid` + stdin 分离，能活过
SSH 会话。

`modinfo` / `lsmod` 之类的 `/sbin` 工具**不在默认 PATH** 里（尤其 sudo /
非登录 shell）→ 用全路径，或在命令里先 `PATH=/sbin:/usr/sbin:$PATH`。

Do not choose PTY mode only because a command is long-running. For long
non-interactive jobs such as builds, installs, training, or batch scripts,
prefer `fleet exec --detach` and monitor with `fleet jobs` / `fleet log`.
Use PTY mode for long work only when an interactive shell state must survive
across multiple Agent steps.

Use Fleet transfer commands for files:

```bash
fleet push <device> local/path remote/path
fleet pull <device> remote/path local/path
fleet transfer src:/path dst:/path
fleet work-sync <device> <local-dir> <remote-dir>
```

### ⚠️ 大文件设备间传输：Go 版 `fleet transfer` 会卡死（2026-07-30 实测）

`~/.rpty/bin/fleet transfer`（Go 二进制）在几百 MB 以上的设备间传输上**实质卡住**：
一个 777MB 文件 **18 分钟零字节落地**（30MB 花了约 2 分钟）。原因是它没走上真正的
局域网直连（目标设备缺 `sshpass`/出站条件时，字节爬在控制机那条链路上；而控制机
Mac 当时不在局域网内）。

**改用 live 的 Python backend，并显式给目标设备的局域网 IP：**

```bash
~/.rpty/bin/fleet_backend/fleet.py transfer \
    radxa:/path/big.bin spark:/path/big.bin --dest-host <dest-lan-ip>
```

同一批文件：**3.2GB 约 90 秒**跑完。

注意三点：
1. `fleet_backend/fleet.py` 是**live 后端**（和 Go 版共用同一份
   `~/.rpty/bin/fleet_backend/devices.json`），**不是**那个过时的
   `~/project/_hub/fleet.py`（后者带自己一份陈旧 devices.json，禁用）。
2. 它会打印 `verify: SKIP (md5 unavailable on remote)` → **必须自己两边跑 md5 核对**，
   不能信工具。
3. 仍然**不要加 `--relay`**（那是强制经控制机中转）。目标 IP 用 `fleet status --json` 查。

小文件（几十 MB 以内）用 Go 版的 `push`/`pull` 没问题，而且它们**会自动 md5 校验**。

### ⚠️ 从 spark 传 HF：默认的 xet 上传会失败

`hf upload` 默认走 xet，在 **直连**（非镜像）下依然会在 commit 阶段炸：
`ConnectionError: ... 400 Bad Request, domain: https://cas-server.xethub.hf.co/v1/shards`
—— 传了 1.83GB 之后 **0 commit**，等于全白费。这跟以前记的 hf-mirror multipart LFS
问题是**两回事**（那个是镜像站的，这个是直连的）。

**解法**：`HF_HUB_DISABLE_XET=1`（退回经典 LFS multipart）+ **逐文件提交**。一次过。

```bash
HF_HUB_DISABLE_XET=1 hf upload <repo> <local-file> <path-in-repo>
```

顺带：**spark 能直连 hf.co，radxa 不能**（`curl https://huggingface.co/` 超时 exit 28）。
所以 radxa 上拉 HF 必须走 `HF_ENDPOINT=https://hf-mirror.com`（镜像同步有延迟，
大文件先 HEAD 一下确认 `Content-Length` 再开始拉）。**上传只能走直连，镜像站不收上传。**

## Agent Sessions

Preferred workflow: run local doctor once, launch Codex, Claude Code, or
OpenCode directly, then select or switch devices from inside the Agent.

```bash
fleet doctor --fix --write-shell-profile
codex
claude
opencode
fleet hosts
fleet use radxa
cd /tmp
pwd
fleet use wsl2-local
fleet env
fleet run --host radxa -- 'hostname'
```

If `RPTY_SESSION` is not already set, the installed Agent shim creates a unique
session. Different Agent launches therefore use different remote tmux sessions
by default. `fleet doctor --fix --write-shell-profile` discovers known local
Agents, currently `codex`, `claude`, and `opencode`, installs matching shims for
the ones it finds, and writes the local shim directory to the user's shell
profile.

Fleet backend is bundled. If device inventory is missing on a fresh install,
create the private file from the example and fill in real hosts/credentials:

```bash
cp ~/.rpty/bin/fleet_backend/devices.example.json ~/.rpty/bin/fleet_backend/devices.json
chmod 600 ~/.rpty/bin/fleet_backend/devices.json
```

Use an external Fleet backend only when intentionally overriding the bundled
one:

```bash
fleet config --fleet-py /path/to/fleet.py
```

Add another Agent command to doctor discovery:

```bash
fleet config --agent gemini
fleet doctor --fix
```

On macOS/Linux, shims are symlinks or shell scripts. On Windows, shims are
`.cmd` files and `--write-shell-profile` updates the PowerShell profile.

Set `RPTY_SESSION` only when intentionally resuming or sharing a PTY session:

```bash
RPTY_SESSION=task-debug codex
```

The generated Agent session may inherit the current default host as a
convenience. Later `fleet use <device>` calls inside that Agent affect only that
session. Use `fleet run --host <device> -- <cmd>` for one command on another
device without changing the current bash-shim target.

`fleet agent <cmd>` remains available as a generic lower-level wrapper when a
same-name shim is not installed.

## Status And Safety

Before assuming current remote state, run:

```bash
fleet where
fleet env
```

`fleet env` reports the current session, device, hostname, cwd, shell, user,
virtualenv, Python path, and tmux session.

The router locks writes per `RPTY_SESSION + device`. If a lock is held by a live
process, wait or choose another `RPTY_SESSION`.

Raw captures are kept under:

```text
~/.rpty/state/sessions/<RPTY_SESSION>/logs/<device>.raw.log
```

Parser failure is not success. Inspect raw logs when output looks truncated or
the exit marker is missing.

## Cleanup

Destroy the current PTY session on a device with:

```bash
fleet cleanup [device]
```

This kills the remote tmux session for the current `RPTY_SESSION + device` and
removes temporary payload files. It does not remove regular Fleet state or local
logs.

AgentFleet does not auto-destroy PTY sessions after each command; persistence is
the feature. At the end of a task, run `fleet cleanup [device]` unless the user
explicitly wants to resume that shell later. If many old AgentFleet tmux
sessions exist on a device, use the explicit bulk cleanup:

```bash
fleet cleanup --all <device>
```

`--all` only targets remote tmux sessions whose names start with `rpty-`.

## Bash Shim

After installation, the `bash` shim intercepts common Agent shell calls:

```bash
bash -lc '<cmd>'
bash -c '<cmd>'
```

路由是**按会话显式开启**的：没跑过 `fleet use <device>` 时所有调用都在本机执行；
shim 不路由的形态（脚本路径、交互式 shell）同样回落 `/bin/bash`。这道闸门是有意的
——shim 挂在每个进程的 PATH 上，若拦截无关的 `bash` 调用，会打断
`#!/usr/bin/env bash` 脚本：git hook、pre-commit、凭据 helper 都靠它。

设 `RPTY_BASH_PASSTHROUGH=1` 可在已路由状态下强制走本机 bash。

## Dispatch Guardrail Block（派 subagent 操作 Fleet 设备时必贴）

凡是 subagent 会用到 `fleet exec` / `fleet ssh` / `fleet push` 等命令操作远程设备的，派发 prompt 必须包含以下规则块（从全局 CLAUDE.md 移入，此处为唯一权威版本）：

```
FLEET RULES (MUST FOLLOW):
- Fleet path: ~/.rpty/bin/fleet
  (PATH may not include ~/.rpty/bin in subshells — use the full path above.
  Do NOT use ~/project/_hub/fleet.py — stale copy with its own outdated
  devices.json; the live registry is ~/.rpty/bin/fleet_backend/devices.json.)
- CRITICAL — --sudo is MANDATORY for: apt/apt-get, dpkg, systemctl, docker,
  docker compose, writes outside $HOME (/etc/, /opt/, /usr/, /var/), mount,
  modprobe, nvpmodel, iptables, ports <1024. The remote user is NOT root.
  Pattern: `$FLEET exec --sudo <device> -- <command>`
  Flags (--sudo, --timeout, --json) MUST come BEFORE the device name.
- Default timeout is 60s. Bump --timeout for anything beyond a status check:
  apt install → 300s, docker pull → 1800s, build → 3600s.
- Python: always use uv (uv run / uv add / uv sync), NEVER pip install to
  system Python. No --sudo needed for uv.
- Mirrors: most devices have no cross-wall access. Bootstrap auto-configures
  HF_ENDPOINT + UV_INDEX_URL + git insteadOf. Check with:
  `$FLEET bootstrap <device> --check`
  If mirrors are missing: `$FLEET bootstrap <device> --profile edge-mirror`
  WARNING: plain `bootstrap <device>` AUTO-DETECTS and will pick profile
  `direct` (= install NO mirrors) on any device running a transparent proxy,
  because github.com/pypi.org probe as reachable. Always pass
  `--profile edge-mirror` on such devices, and verify the var is visible from a
  NON-login shell: `$FLEET exec <device> -- 'bash -c "echo \$HF_ENDPOINT"'`
  (dropin lives in ~/.profile.d/ = login shells only; /etc/environment and
  ~/.config/environment.d/ cover PAM sessions and systemd --user).
- Transfer: device-to-device uses direct LAN (fast). Add --relay to route
  through control machine instead.
- Check device status: $FLEET status --json
- NEVER parse devices.json directly — always use fleet CLI.
```
