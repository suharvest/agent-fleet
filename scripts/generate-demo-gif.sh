#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out="$root/media/demo.gif"

if ! command -v magick >/dev/null 2>&1; then
  echo "ImageMagick 'magick' is required to generate media/demo.gif" >&2
  exit 127
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/agentfleet-demo.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

escape_xml() {
  printf '%s' "$1" \
    | sed -e 's/&/\&amp;/g' \
          -e 's/</\&lt;/g' \
          -e 's/>/\&gt;/g' \
          -e 's/"/\&quot;/g'
}

terminal_line() {
  color=$1
  y=$2
  text=$3
  printf '<text x="86" y="%s" class="mono" fill="%s">%s</text>\n' \
    "$y" "$color" "$(escape_xml "$text")"
}

status_item() {
  y=$1
  title=$2
  body=$3
  accent=$4
  printf '<circle cx="792" cy="%s" r="5" fill="%s"/>\n' "$((y - 5))" "$accent"
  printf '<text x="808" y="%s" class="label" fill="#111315">%s</text>\n' "$y" "$(escape_xml "$title")"
  printf '<text x="808" y="%s" class="small" fill="#5a5f66">%s</text>\n' "$((y + 25))" "$(escape_xml "$body")"
}

frame_svg() {
  step=$1
  file=$2

  case "$step" in
    1)
      eyebrow="Install once"
      headline="AgentFleet"
      subhead="One login. Work across every device with your coding agent."
      host="local"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet doctor --fix --write-shell-profile
ok installed fleet, rpty, and bash shims
ok discovered codex, claude, and opencode
ok bundled Fleet backend ready

$ codex
-> Agent session: agent-project-1042
EOF
)
      ;;
    2)
      eyebrow="Discover devices"
      headline="Fleet inventory stays the source of truth"
      subhead="Existing device config, auth, transfer, WSL, jobs, and bootstrap still work."
      host="unset"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet hosts
NAME          STATUS   PLATFORM       MODE
radxa         online   linux/aarch64  tmux
wsl2-local    online   linux/x86_64   tmux
home-win      online   windows        exec

$ fleet status --json
ok passwords masked, raw errors preserved
EOF
)
      ;;
    3)
      eyebrow="Use a device"
      headline="Switch the agent to a real board"
      subhead="The selected host is scoped to this agent session."
      host="radxa"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet use radxa
Current host: radxa

$ fleet run -- 'cd /tmp && export TARGET=radxa && pwd'
/tmp

ok remote tmux: rpty-agent-project-1042-radxa
EOF
)
      ;;
    4)
      eyebrow="Keep shell state"
      headline="cwd, env, and virtualenv state survive"
      subhead="Follow-up commands run in the same remote shell."
      host="radxa"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet env
host:    radxa
cwd:     /tmp
shell:   bash
TARGET:  radxa

$ fleet run -- 'echo "$PWD $TARGET"'
/tmp radxa
EOF
)
      ;;
    5)
      eyebrow="Move across devices"
      headline="Switch to WSL without losing Radxa state"
      subhead="Every device gets its own tmux shell under the same agent session."
      host="wsl2-local"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet use wsl2-local
Current host: wsl2-local

$ fleet run -- 'hostname && pwd'
wsl2-local
/home/harvest/project

ok radxa session remains alive
EOF
)
      ;;
    6)
      eyebrow="Target another device"
      headline="Run one command elsewhere"
      subhead="Use --host for a one-shot command without changing the current target."
      host="wsl2-local"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet run --host radxa -- 'echo "$PWD $TARGET"'
/tmp radxa

$ fleet where
Current host: wsl2-local

ok no ssh quoting gymnastics
EOF
)
      ;;
    7)
      eyebrow="Avoid shell fights"
      headline="Session locks protect each remote pane"
      subhead="Different agents get different sessions by default."
      host="wsl2-local"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet sessions
SESSION              DEVICE       LOCK
agent-project-1042   radxa        clear
agent-project-1042   wsl2-local   clear

$ claude
-> Agent session: agent-project-2097
EOF
)
      ;;
    8)
      eyebrow="Use the right mode"
      headline="Stateful PTY plus classic Fleet commands"
      subhead="Long jobs, sudo, file transfer, and stateless checks keep their Fleet behavior."
      host="wsl2-local"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ fleet exec --sudo radxa -- apt-get update
ok stateless privileged command

$ fleet push radxa ./model.onnx /tmp/model.onnx
ok direct transfer

$ fleet exec --detach radxa -- ./train.sh
job: fleet-20260704-1319
EOF
)
      ;;
    9)
      eyebrow="Release ready"
      headline="Prebuilt packages for macOS, Linux, and Windows"
      subhead="No local compile needed for normal users."
      host="any device"
      session="agent-project-1042"
      terminal=$(cat <<'EOF'
$ gh release download v0.1.0
ok repo: suharvest/agent-fleet
ok agent-fleet-macos-aarch64.tar.gz
ok agent-fleet-windows-x86_64.zip

$ ./scripts/install.sh
AgentFleet installed.
EOF
)
      ;;
    *)
      eyebrow="AgentFleet"
      headline="One login. Work across every device."
      subhead="Codex, Claude Code, and OpenCode can move through your Fleet without losing shell state."
      host="radxa + wsl2-local + home-win"
      session="isolated per agent"
      terminal=$(cat <<'EOF'
$ codex
$ fleet use radxa
$ fleet run -- 'source .venv/bin/activate'
$ fleet run -- 'pytest -q'
$ fleet use wsl2-local
$ fleet run -- 'cargo test'

ok one agent workflow across every device
EOF
)
      ;;
  esac

  svg="$tmp/frame-$step.svg"
  {
    cat <<EOF
<svg xmlns="http://www.w3.org/2000/svg" width="1120" height="650" viewBox="0 0 1120 650">
  <defs>
    <linearGradient id="page" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#fbfaf7"/>
      <stop offset="0.55" stop-color="#f2f5f2"/>
      <stop offset="1" stop-color="#e9eef0"/>
    </linearGradient>
    <filter id="softShadow" x="-10%" y="-10%" width="120%" height="130%">
      <feDropShadow dx="0" dy="18" stdDeviation="18" flood-color="#101418" flood-opacity="0.18"/>
    </filter>
    <style>
      .title { font-family: Arial, Helvetica, sans-serif; font-weight: 800; font-style: normal; letter-spacing: 0; }
      .body { font-family: Arial, Helvetica, sans-serif; font-weight: 500; font-style: normal; letter-spacing: 0; }
      .mono { font-family: Menlo, Monaco, monospace; font-size: 22px; font-style: normal; letter-spacing: 0; }
      .smallmono { font-family: Menlo, Monaco, monospace; font-size: 16px; font-style: normal; letter-spacing: 0; }
      .label { font-family: Arial, Helvetica, sans-serif; font-size: 21px; font-weight: 750; font-style: normal; letter-spacing: 0; }
      .small { font-family: Arial, Helvetica, sans-serif; font-size: 16px; font-weight: 500; font-style: normal; letter-spacing: 0; }
    </style>
  </defs>
  <rect width="1120" height="650" fill="url(#page)"/>
  <rect x="34" y="34" width="1052" height="582" rx="18" fill="#ffffff" filter="url(#softShadow)"/>
  <rect x="34" y="34" width="1052" height="72" rx="18" fill="#111315"/>
  <rect x="34" y="88" width="1052" height="18" fill="#111315"/>
  <circle cx="70" cy="70" r="8" fill="#ff6b57"/>
  <circle cx="96" cy="70" r="8" fill="#f6c64f"/>
  <circle cx="122" cy="70" r="8" fill="#38c172"/>
  <text x="158" y="77" class="smallmono" fill="#d8dde1">agentfleet demo</text>
  <text x="862" y="77" class="smallmono" fill="#8fd19e">v0.1.0</text>

  <text x="64" y="148" class="small" fill="#0f9f6e">$(escape_xml "$eyebrow")</text>
  <text x="64" y="184" class="title" font-size="34" fill="#111315">$(escape_xml "$headline")</text>
  <text x="64" y="216" class="body" font-size="18" fill="#5f6670">$(escape_xml "$subhead")</text>

  <rect x="64" y="250" width="690" height="324" rx="10" fill="#151819"/>
  <rect x="64" y="250" width="690" height="38" rx="10" fill="#20252a"/>
  <rect x="64" y="276" width="690" height="12" fill="#20252a"/>
  <text x="86" y="276" class="smallmono" fill="#b7c0c8">fleet shell</text>
EOF

    y=326
    printf '%s\n' "$terminal" | while IFS= read -r line; do
      case "$line" in
        '$ '*)
          terminal_line "#75e0a7" "$y" "$line"
          ;;
        'ok '*|'-> '*)
          terminal_line "#8cc8ff" "$y" "$line"
          ;;
        '')
          y=$((y + 20))
          continue
          ;;
        *)
          terminal_line "#e8ecef" "$y" "$line"
          ;;
      esac
      y=$((y + 32))
    done

    cat <<EOF
  <rect x="786" y="250" width="270" height="324" rx="10" fill="#f4efe7"/>
  <text x="812" y="294" class="label" fill="#111315">Agent state</text>
  <text x="812" y="324" class="smallmono" fill="#4b5563">RPTY_SESSION</text>
  <text x="812" y="350" class="smallmono" fill="#111315">$(escape_xml "$session")</text>
EOF
    status_item 400 "Current host" "$host" "#22c55e"
    status_item 466 "Transport" "tmux-backed remote shell" "#3b82f6"
    status_item 532 "Locking" "one writer per session + device" "#f59e0b"
    cat <<EOF
  <rect x="64" y="584" width="992" height="1" fill="#d9dee4"/>
  <text x="64" y="612" class="small" fill="#626b75">Codex / Claude Code / OpenCode</text>
  <text x="806" y="612" class="small" fill="#626b75">macOS / Linux / Windows controller</text>
</svg>
EOF
  } > "$svg"

  magick -background none "$svg" "$file"
}

frames=""
for step in 1 2 3 4 5 6 7 8 9 10; do
  png="$tmp/frame-$(printf '%02d' "$step").png"
  frame_svg "$step" "$png"
  frames="$frames $png"
done

magick -delay 110 -loop 0 $frames -layers Optimize "$out"
echo "Generated $out"
