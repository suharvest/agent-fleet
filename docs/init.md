下面是一版可以直接保存为 `requirements.md` 的需求文档草案。

~~~markdown
# Remote PTY Router 需求文档

版本：v0.1  
状态：Draft  
目标用户：本地使用 Codex / Claude Code / Aider / Gemini CLI 等 Coding Agent，并希望通过 SSH 在多台远程机器上开发的工程师。

---

## 1. 背景

当前 Coding Agent 通常假设自己运行在本地开发环境中，或者直接安装在远程机器上。

但在实际开发中，经常存在以下需求：

- Agent 只安装在本地机器，例如 Mac。
- 代码、GPU、Docker、编译环境在远程机器上。
- 一次任务可能需要同时操作多台远程设备，例如：
  - DGX / GPU Server
  - Jetson
  - RK3588
  - 云服务器
- 不希望 Agent 每次生成复杂的 SSH 命令，例如：

```bash
sshpass -p xxx ssh user@host "cd /repo && pytest"
~~~

- 不希望使用类似：

```bash
rbash dgx "pytest -q"
```

这种“命令包在参数里”的模式，因为容易遇到 shell quoting、转义、多行脚本、特殊字符解析问题。

期望的体验更接近：

```bash
use dgx
cd /workspace/project
pytest -q

use jetson
cd /home/ubuntu/project
python benchmark.py

use dgx
cat results/log.txt
```

也就是说：

本地 Agent 仍然只使用 bash / terminal，但实际 bash 运行在远程机器上，并且可以在一次任务中切换不同远程主机。

------

## **2. 产品目标**

Remote PTY Router 的目标是提供一个本地运行的远程终端路由层。

它负责：

- 管理多个远程 SSH Host。
- 为每个 Host 维护持久化 PTY / bash / tmux session。
- 允许 Agent 在一个任务中切换不同 Host。
- 让 Agent 使用普通 bash 命令，而不是自己拼 SSH 命令。
- 保证远程 shell 的状态可以持久化，例如：
  - `cd`
  - `source venv/bin/activate`
  - 环境变量
  - 长时间运行任务
  - 后台进程
- 本地断线或 Agent 重启后，可以重新 attach 到远程 session。
- 远程端默认不需要安装常驻 server。

------

## **3. 非目标**

v0.1 不追求完整 VS Code Remote / Cursor Remote / Zed Remote 级别体验。

以下能力不属于 v0.1 核心目标：

- 远程文件系统透明挂载。
- LSP / 代码补全 / Rename Symbol。
- 远程 Debug Adapter。
- 远程 Extension Host。
- 完整 Workspace Index。
- 图形化 IDE。
- MCP 协议封装。
- 在远程安装完整 Agent。

v0.1 的目标很明确：

只把 Agent 的 bash / terminal 能力稳定地远程化。

如果某些 Agent 强依赖本地文件系统 API 直接读写文件，则 v0.1 不保证完全兼容。可以后续通过 sshfs、rsync、SFTP adapter 或 remote file API 补足。

------

## **4. 核心用户体验**

### **4.1 本地启动 Agent**

用户在本地启动 Agent：

```bash
rpty agent claude
```

或者：

```bash
rpty agent codex
```

也可以通过 PATH shim 方式：

```bash
export PATH="$HOME/.rpty/bin:$PATH"
claude
```

Agent 仍然认为自己在使用本地 bash。

------

### **4.2 查看可用设备**

在 Agent 的 terminal 中：

```bash
hosts
```

输出示例：

```text
Available hosts:

  dgx       ubuntu@192.168.1.10      /workspace/project
  jetson    ubuntu@192.168.1.20      /home/ubuntu/project
  rk3588    linaro@192.168.1.30      /home/linaro/project

Current host: dgx
```

------

### **4.3 切换远程设备**

```bash
use dgx
```

之后所有普通 bash 命令都发送到 `dgx` 的远程 PTY。

```bash
pwd
nvidia-smi
cd /workspace/project
pytest -q
```

切换到 Jetson：

```bash
use jetson
pwd
tegrastats
python benchmark.py
```

再切回 DGX：

```bash
use dgx
pwd
git status
```

要求：

- 切换 Host 不会销毁之前 Host 的 session。
- 每台 Host 的 shell 状态独立保存。
- 切回某台 Host 后，之前的 cwd、env、tmux session 仍然存在。

------

### **4.4 持久化 shell 状态**

示例：

```bash
use dgx
cd /workspace/project
source .venv/bin/activate
export CUDA_VISIBLE_DEVICES=0
python -V
```

切换到其他设备：

```bash
use jetson
cd /home/ubuntu/project
```

再切回：

```bash
use dgx
pwd
python -V
echo $CUDA_VISIBLE_DEVICES
```

应保持之前状态。

------

### **4.5 长任务不中断**

示例：

```bash
use dgx
python train.py
```

如果本地网络断开、Agent 退出或本地 terminal 关闭，远程任务应继续在 tmux 中运行。

用户重新连接后：

```bash
use dgx
attach
```

或者：

```bash
logs
```

可以继续查看任务输出。

------

## **5. 架构设计**

### **5.1 总体架构**

```text
Local Machine
├── Codex / Claude Code / Aider
│
├── bash shim / shell adapter
│
├── rpty local broker
│   ├── host router
│   ├── session manager
│   ├── PTY manager
│   ├── SSH process manager
│   ├── tmux attach manager
│   ├── log manager
│   └── policy / audit layer
│
└── OpenSSH client
        │
        ▼
Remote Host A
├── sshd
├── bash
└── tmux session

Remote Host B
├── sshd
├── bash
└── tmux session

Remote Host C
├── sshd
├── bash
└── tmux session
```

------

### **5.2 本地组件**

#### **5.2.1** **`rpty`**

主 CLI。

负责：

```bash
rpty hosts
rpty add
rpty use
rpty status
rpty attach
rpty logs
rpty agent claude
rpty agent codex
```

------

#### **5.2.2** **`rptyd`**

本地 broker daemon，可选。

负责长期维护：

- SSH 连接。
- Host 状态。
- 当前 Agent session。
- 每个 Host 的 PTY。
- 每个 Host 的 tmux session 映射。
- 命令日志。
- 当前 host 指针。
- 命令执行状态。

v0.1 可以不做常驻 daemon，先用单进程模式实现。

------

#### **5.2.3 bash shim**

本地放置一个可选的 `bash` shim。

路径示例：

```bash
~/.rpty/bin/bash
```

当 Agent 调用：

```bash
bash -lc "pytest -q"
```

实际由 shim 接管，然后转发给当前 remote host 的持久化 PTY。

要求：

- Agent 不需要调用 `rbash "command"`。
- Agent 不需要知道 SSH。
- Agent 不需要知道 sshpass。
- Agent 不需要知道远程 IP、端口、密钥路径。
- shim 必须尽可能兼容普通 bash 的常见调用方式。

------

### **5.3 远程组件**

v0.1 默认不安装远程 server。

远程仅依赖：

- `sshd`
- `bash`
- `tmux`

推荐依赖：

- `git`
- `rsync`
- `rg`
- `python3`

远程允许创建工作目录：

```text
~/.rpty/
├── sessions/
├── logs/
├── tmp/
└── scripts/
```

这不是常驻 server，只是状态和日志目录。

------

## **6. 配置设计**

### **6.1 配置文件路径**

```bash
~/.rpty/config.toml
```

------

### **6.2 配置示例**

```toml
default_session = "default"

[hosts.dgx]
ssh = "dgx"
default_cwd = "/workspace/project"
tmux_session = "rpty-dgx"
enabled = true

[hosts.jetson]
ssh = "jetson-orin"
default_cwd = "/home/ubuntu/project"
tmux_session = "rpty-jetson"
enabled = true

[hosts.rk3588]
ssh = "rk3588"
default_cwd = "/home/linaro/project"
tmux_session = "rpty-rk3588"
enabled = true

[policy]
allow_unknown_hosts = false
require_host_key_checking = true
log_commands = true
log_output = true
confirm_dangerous_commands = false

[ssh]
control_master = true
server_alive_interval = 30
connect_timeout = 10
```

------

### **6.3 SSH 配置**

用户仍然通过标准 OpenSSH 配置 Host。

示例：

```sshconfig
Host dgx
  HostName 192.168.1.10
  User ubuntu
  IdentityFile ~/.ssh/id_ed25519
  ControlMaster auto
  ControlPath ~/.ssh/cm-%r@%h:%p
  ControlPersist 1h
  ServerAliveInterval 30
  ServerAliveCountMax 3

Host jetson-orin
  HostName 192.168.1.20
  User ubuntu
  IdentityFile ~/.ssh/id_ed25519
  ControlMaster auto
  ControlPath ~/.ssh/cm-%r@%h:%p
  ControlPersist 1h
  ServerAliveInterval 30
  ServerAliveCountMax 3
```

要求：

- 不保存明文密码。
- 不使用 sshpass。
- 不自动关闭 HostKeyChecking。
- 优先复用用户现有 SSH config、ssh-agent、ProxyJump、FIDO key、ControlMaster。

------

## **7. 命令设计**

### **7.1 元命令**

以下命令由本地 rpty 拦截，不发送到远程 bash：

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

------

### **7.2** **`hosts`**

显示所有可用 Host。

```bash
hosts
```

输出：

```text
Available hosts:
  dgx
  jetson
  rk3588

Current host: dgx
```

------

### **7.3** **`use <host>`**

切换当前 Host。

```bash
use jetson
```

要求：

- 如果目标 Host 还没有连接，则自动建立 SSH 连接。
- 如果远程 tmux session 不存在，则自动创建。
- 如果远程 tmux session 存在，则自动 attach 或复用。
- 切换 Host 不影响其他 Host 的 session。

------

### **7.4** **`where`**

显示当前路由状态。

```bash
where
```

输出示例：

```text
Session: default
Current host: jetson
SSH alias: jetson-orin
Remote cwd: /home/ubuntu/project
Remote tmux: rpty-jetson
Connected: yes
```

------

### **7.5** **`attach`**

进入当前 Host 的真实交互式终端。

```bash
attach
```

用于：

- vim
- top
- htop
- python REPL
- git add -p
- docker logs -f
- npm run dev
- long-running command

退出 attach 不应杀死远程 session。

------

### **7.6** **`logs`**

查看当前 Host 最近输出。

```bash
logs
```

或者：

```bash
logs dgx
```

------

### **7.7 普通 bash 命令**

除元命令之外，其余所有输入都原样发送给当前 Host 的远程 PTY。

例如：

```bash
pwd
ls -la
cd build
cmake ..
make -j8
pytest -q
docker ps
git status
```

这些命令不应被包装成：

```bash
ssh host "command"
```

也不应要求用户或 Agent 写：

```bash
rbash host "command"
```

------

## **8. 命令传输要求**

### **8.1 禁止复杂 shell quoting**

系统内部不得依赖这种方式传递复杂命令：

```bash
ssh host "cd /repo && bash -lc \"$CMD\""
```

原因：

- 多行脚本容易坏。
- 引号容易坏。
- `$`、反引号、 heredoc、JSON、正则表达式容易坏。
- Agent 生成代码时经常包含复杂字符。

------

### **8.2 推荐传输方式**

命令应通过以下方式之一发送：

1. 直接写入远程 PTY。
2. 通过 stdin 传输脚本内容。
3. 使用临时脚本文件。
4. 必要时使用 base64 传输精确字节内容。

示例内部流程：

```text
Agent command
  ↓
local shim
  ↓
broker
  ↓
remote PTY stdin
  ↓
remote bash
```

而不是：

```text
Agent command
  ↓
shell escaped string
  ↓
ssh "bash -lc \"...\""
```

------

### **8.3 Exit Code 捕获**

对于非交互命令，系统应能返回真实 exit code。

内部可以在命令后注入 marker：

```bash
printf '\n__RPTY_EXIT_CODE__:%s\n' "$?"
```

要求：

- marker 不应暴露给 Agent 的最终输出，除非 debug 模式。
- stdout / stderr 应尽量保持原始顺序。
- 命令超时、断线、Ctrl-C 应有明确状态。

------

## **9. Session 模型**

### **9.1 Local Session**

每次 Agent 任务对应一个 local session。

session id 可以来自：

- 用户显式指定：

```bash
rpty agent --session build-test claude
```

- 当前 repo 路径 hash。
- 当前 terminal id。
- 当前进程树。

------

### **9.2 Host Session**

每个 local session 下，每台 Host 有独立远程 session。

```text
local session: build-test
├── host: dgx
│   └── tmux: rpty-build-test-dgx
├── host: jetson
│   └── tmux: rpty-build-test-jetson
└── host: rk3588
    └── tmux: rpty-build-test-rk3588
```

要求：

- Host A 的 cwd 不影响 Host B。
- Host A 的 env 不影响 Host B。
- Host A 的 foreground process 不影响 Host B。
- 切换 Host 时，之前 Host 的进程继续存在。

------

## **10. 持久化与恢复**

### **10.1 SSH 连接复用**

应使用 OpenSSH ControlMaster 或等价机制。

目标：

- 避免每条命令重新认证。
- 降低 latency。
- 支持 ProxyJump / bastion / ssh-agent / FIDO key。

------

### **10.2 tmux 持久化**

每个 Host 应使用 tmux session 承载远程 bash。

要求：

- SSH 断开后，远程 bash 不退出。
- 本地 broker 崩溃后，重新启动可以 attach。
- 长任务继续运行。
- 支持查看历史输出。

------

### **10.3 本地状态文件**

本地保存：

```text
~/.rpty/state/
├── sessions/
│   └── default.json
├── hosts/
│   ├── dgx.json
│   └── jetson.json
└── logs/
```

状态内容包括：

- 当前 local session。
- 当前 host。
- 每个 host 的连接状态。
- 每个 host 的 tmux session 名。
- 最近命令。
- 最近 exit code。
- 日志路径。

------

## **11. 多设备任务示例**

### **11.1 构建 + 部署 + Benchmark**

Agent 可以执行：

```bash
use dgx
cd /workspace/project
git pull
python build_engine.py --target jetson

use jetson
cd /home/ubuntu/project
rsync -av dgx:/workspace/project/output/ ./output/
python benchmark.py --engine output/model.engine

use dgx
cd /workspace/project
mkdir -p results/jetson
rsync -av jetson:/home/ubuntu/project/benchmark.json results/jetson/
python analyze.py results/jetson/benchmark.json
```

要求：

- Agent 可以在一次任务中自然切换 Host。
- 不需要显式 SSH wrapper。
- 每个 Host 的 shell 状态持续存在。

------

### **11.2 同时观察多设备**

```bash
use jetson
tegrastats

use dgx
nvidia-smi
```

v0.1 可以串行查看。

v0.2 可支持：

```bash
watch jetson tegrastats
watch dgx nvidia-smi
```

------

## **12. 安全要求**

### **12.1 凭据管理**

必须：

- 使用 SSH key / ssh-agent。
- 使用用户已有 `~/.ssh/config`。
- 不保存明文密码。
- 不使用 sshpass。
- 不在日志中记录私钥、密码、token。

------

### **12.2 Host 白名单**

只有配置文件中的 Host 可以被使用。

如果 Agent 输入：

```bash
use unknown-host
```

应拒绝。

------

### **12.3 Host Key Checking**

默认必须尊重 OpenSSH Host Key Checking。

不得默认使用：

```bash
-o StrictHostKeyChecking=no
```

------

### **12.4 审计日志**

应记录：

- session id
- host
- command
- start time
- end time
- exit code
- 是否超时
- 是否被 Ctrl-C 中断

可选记录 stdout / stderr。

------

### **12.5 危险命令策略**

v0.1 可以只记录，不拦截。

v0.2 可支持策略：

```toml
[policy]
confirm_dangerous_commands = true
dangerous_patterns = [
  "rm -rf /",
  "mkfs",
  "dd if=",
  "shutdown",
  "reboot"
]
```

------

## **13. 稳定性要求**

### **13.1 Ctrl-C**

当用户或 Agent 发送 Ctrl-C 时，应中断当前 Host 的 foreground process。

要求：

- 不杀死整个 tmux session。
- 不断开 SSH。
- 不影响其他 Host。

------

### **13.2 超时**

非交互命令应支持超时。

```toml
[execution]
default_timeout_seconds = 600
```

超时后：

- 返回明确错误。
- 尝试发送 Ctrl-C。
- 不销毁 Host session。

------

### **13.3 网络断开**

网络断开后：

- 本地返回明确错误。
- 远程 tmux session 保持。
- 重新连接后可以 attach。
- 不重复执行已经发送成功的命令，除非用户明确重试。

------

### **13.4 Broker 崩溃**

如果本地 broker 崩溃：

- 远程 tmux session 继续存在。
- 重启 broker 后，可以根据 session id 找回。
- 不要求远程安装 server。

------

## **14. Agent 集成方式**

### **14.1 方式一：Agent Launcher**

推荐方式：

```bash
rpty agent claude
```

内部做：

```bash
export PATH="$HOME/.rpty/bin:$PATH"
export RPTY_SESSION="..."
claude
```

------

### **14.2 方式二：手动 PATH shim**

```bash
export PATH="$HOME/.rpty/bin:$PATH"
export RPTY_SESSION="my-task"
codex
```

------

### **14.3 方式三：交互 Shell**

```bash
rpty shell
```

进入：

```text
rpty [dgx] > pwd
rpty [dgx] > use jetson
rpty [jetson] > pwd
```

------

## **15. MVP 范围**

### **15.1 v0.1 必须支持**

- 多 Host 配置。
- `hosts`
- `use <host>`
- `where`
- 普通 bash 命令转发到当前 Host。
- 每个 Host 一个持久化 tmux session。
- SSH 使用用户已有 config。
- 不使用 sshpass。
- 不要求远程安装 server。
- 支持 Ctrl-C。
- 支持 stdout / stderr streaming。
- 支持 exit code 捕获。
- 支持本地 Agent launcher。
- 支持 session 恢复。

------

### **15.2 v0.1 可以不支持**

- 图形 UI。
- MCP。
- LSP。
- 文件树。
- 远程文件 watcher。
- Debugger。
- Port forward 管理。
- 多路并发 dashboard。
- 远程 daemon。
- 复杂权限系统。

------

## **16. 后续版本规划**

### **16.1 v0.2**

增加：

- `run --background`
- `jobs`
- `logs <job-id>`
- `stop <job-id>`
- `attach <host>`
- `copy <host>:<path> <host>:<path>`
- port forward helper
- 更好的命令审计
- 更好的 reconnect 逻辑

------

### **16.2 v0.3**

增加：

- 可选 remote helper。
- 更低延迟的远程 PTY 管理。
- 远程文件读写 helper。
- Agent-friendly diff / patch。
- Web UI 或 TUI 管理界面。

------

### **16.3 v1.0**

目标：

- 稳定支持多 Agent。
- 稳定支持多 Host。
- 稳定支持长期任务。
- 远程零 server 安装仍然可用。
- 可选 remote helper 只作为增强功能存在。
- 形成可复用的 Agent Remote Terminal Runtime。

------

## **17. 验收标准**

### **17.1 基础连接**

给定配置：

```toml
[hosts.dgx]
ssh = "dgx"

[hosts.jetson]
ssh = "jetson"
```

执行：

```bash
rpty shell
hosts
use dgx
pwd
use jetson
pwd
```

应能成功切换并执行远程命令。

------

### **17.2 状态保持**

执行：

```bash
use dgx
cd /tmp
export TEST_VAR=hello

use jetson
pwd

use dgx
pwd
echo $TEST_VAR
```

期望：

```text
/tmp
hello
```

------

### **17.3 长任务恢复**

执行：

```bash
use dgx
python long_task.py
```

关闭本地 terminal。

重新打开：

```bash
rpty shell --session default
use dgx
attach
```

应能看到任务仍在运行或已完成输出。

------

### **17.4 Agent 不生成 SSH 命令**

在 Agent 使用过程中，正常任务不应要求 Agent 生成：

```bash
ssh host "command"
```

也不应要求 Agent 生成：

```bash
sshpass ...
```

Agent 只需要使用：

```bash
use dgx
pytest -q
use jetson
python benchmark.py
```

------

### **17.5 Exit Code 正确**

执行：

```bash
use dgx
false
echo $?
```

或通过 Agent shell tool 获取 exit code。

应返回非零状态。

------

### **17.6 Ctrl-C 正确**

执行长任务：

```bash
use dgx
sleep 1000
```

发送 Ctrl-C。

期望：

- `sleep` 被中断。
- tmux session 仍然存在。
- SSH 连接不被永久破坏。
- 可以继续执行：

```bash
echo ok
```

------

## **18. 主要风险**

### **18.1 Agent 可能依赖本地文件系统**

有些 Coding Agent 不只用 bash，还会直接读写本地文件。

v0.1 只保证 terminal 远程化。

解决方向：

- 配合 sshfs。
- 配合 rsync。
- 后续实现 remote file adapter。
- 提示 Agent 通过 bash 操作远程文件。

------

### **18.2 PTY 输出边界难判断**

交互式 PTY 不像非交互命令一样天然有结束边界。

解决方向：

- 对非交互命令注入 exit marker。
- 对交互命令使用 attach 模式。
- 对长任务使用 jobs / logs 模式。

------

### **18.3 TUI 程序输出复杂**

例如：

- vim
- htop
- less
- git add -p

这些程序更适合 attach 模式，而不是普通命令模式。

------

### **18.4 多 Agent 并发**

多个 Agent 同时操作同一个 Host / session 可能互相干扰。

v0.1 默认一个 local session 对应一组 remote tmux session。

v0.2 再支持更完整的锁和并发隔离。

------

## **19. 推荐技术选型**

### **19.1 语言**

优先考虑：

- Go
- Rust
- Python

建议：

- MVP 用 Python 或 Go 更快。
- 长期产品化用 Rust 或 Go 更稳。

------

### **19.2 SSH 实现**

优先使用系统 OpenSSH client，而不是自己重写 SSH。

原因：

- 兼容用户已有 `~/.ssh/config`。
- 兼容 ProxyJump。
- 兼容 ssh-agent。
- 兼容 FIDO / YubiKey。
- 兼容 ControlMaster。
- 兼容企业内网配置。

------

### **19.3 PTY 实现**

可选：

- Python: `pty`, `pexpect`, `asyncio`
- Go: `creack/pty`
- Rust: `portable-pty`

本地 broker 可以 spawn：

```bash
ssh -tt dgx 'tmux new-session -A -s rpty-default-dgx'
```

然后通过本地 PTY 读写。

------

## **20. 一句话总结**

Remote PTY Router 不是 MCP，不是完整 Remote IDE，也不是远程 Agent。

它是一个：

面向本地 Coding Agent 的多主机远程 bash / PTY 路由器。

它让 Agent 仍然使用普通 bash，但 bash 实际运行在远程机器上；并且可以在同一个任务里稳定切换多台远程设备，保持 session、cwd、env 和长任务状态。