#!/bin/bash
# bootstrap.sh — Device mirror/proxy auto-configuration
# Idempotent. Safe to run multiple times — existing config is backed up before overwriting.
#
# Usage:
#   bootstrap.sh                          # auto-detect profile and apply
#   bootstrap.sh --check                  # dry-run: probe and report, no changes
#   bootstrap.sh --profile wsl2-proxy     # force a specific profile
#   bootstrap.sh --profile edge-mirror
#   bootstrap.sh --profile isolated
#   bootstrap.sh --force                # skip direct-internet check, apply mirrors anyway
#   bootstrap.sh --rollback             # restore from latest backup
#
# Profiles:
#   wsl2-proxy   — has proxy (127.0.0.1:7890), proxy for international, mirrors for PyPI/HF
#   direct       — can reach pypi.org + github.com directly, no config needed
#   edge-mirror  — can reach Chinese CDN but not international, mirrors only, no proxy
#   isolated     — no network at all, skip config, print warning

set -euo pipefail

# ── Config ──────────────────────────────────────────────────
MANAGED_DIR="${HOME}/.config/mirrors"
MANAGED_FILE="${MANAGED_DIR}/managed.json"
BACKUP_DIR="${MANAGED_DIR}/backups"
SHELL_DROPIN="${HOME}/.profile.d/mirrors.sh"
GIT_BACKUP="${MANAGED_DIR}/git-config.backup"
DOCKER_BACKUP="${MANAGED_DIR}/daemon.json.backup"
TIMESTAMP=$(date -Iseconds)

# Mirror endpoints
HF_ENDPOINT_URL="https://hf-mirror.com"
PYPI_INDEX_URL="https://pypi.tuna.tsinghua.edu.cn/simple"
GITHUB_MIRROR="https://ghproxy.com/https://github.com"

# Proxy (WSL2 — Clash on Windows host)
PROXY_HOST="127.0.0.1"
PROXY_PORT="7890"

# ── Helpers ─────────────────────────────────────────────────
check_mode=false
force_profile=""
force_mode=false
rollback_mode=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --check) check_mode=true; shift ;;
        --profile)
            shift
            if [[ $# -eq 0 || "$1" == -* ]]; then
                echo "Error: --profile requires a value (wsl2-proxy, edge-mirror, or isolated)" >&2
                exit 1
            fi
            force_profile="$1"; shift
            ;;
        --profile=*) force_profile="${1#*=}"; shift ;;
        --force) force_mode=true; shift ;;
        --rollback) rollback_mode=true; shift ;;
        -h|--help)
            sed -n '2,13p' "$0"
            exit 0
            ;;
        *) echo "Error: unknown option $1" >&2; exit 1 ;;
    esac
done

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERR]${NC} $*"; }
info() { echo -e "      $*"; }

probe() {
    # Probe URL with timeout. Returns: reachable (0) or not (1)
    local url="$1"
    local timeout="${2:-3}"
    curl -s --connect-timeout "$timeout" --max-time "$timeout" -o /dev/null -w "%{http_code}" "$url" 2>/dev/null | grep -qE '^(2|3|4)'
}

# ── Rollback ────────────────────────────────────────────────
do_rollback() {
    if [[ ! -f "$MANAGED_FILE" ]]; then
        err "No managed config found at $MANAGED_FILE — nothing to rollback."
        exit 1
    fi

    local last_backup
    last_backup=$(python3 -c "
import json
with open('$MANAGED_FILE') as f:
    data = json.load(f)
print(data.get('backup', ''))
" 2>/dev/null || true)

    if [[ -z "$last_backup" || ! -d "$last_backup" ]]; then
        err "No backup found to rollback to."
        exit 1
    fi

    echo "Rolling back to: $last_backup"

    # Restore shell dropin
    if [[ -f "${last_backup}/mirrors.sh" ]]; then
        cp "${last_backup}/mirrors.sh" "$SHELL_DROPIN"
        ok "Restored $SHELL_DROPIN"
    else
        rm -f "$SHELL_DROPIN"
        ok "Removed $SHELL_DROPIN (was absent in backup)"
    fi

    # Restore git config
    if [[ -f "${last_backup}/git-config.backup" ]]; then
        git config --global --replace-all url."https://github.com".insteadOf "https://ghproxy.com/https://github.com" 2>/dev/null || true
        ok "Restored git config"
    fi

    # Restore Docker daemon.json
    if [[ -f "${last_backup}/daemon.json" ]]; then
        if [[ -w /etc/docker/daemon.json ]] || { [[ ! -f /etc/docker/daemon.json ]] && [[ -w /etc/docker ]]; }; then
            cp "${last_backup}/daemon.json" /etc/docker/daemon.json
        elif command -v sudo &>/dev/null && sudo -n true 2>/dev/null; then
            sudo cp "${last_backup}/daemon.json" /etc/docker/daemon.json
        else
            warn "Cannot restore /etc/docker/daemon.json (sudo requires password) — manual restore from ${last_backup}/daemon.json"
        fi
    fi

    echo ""
    echo "Rollback complete. Run 'source $SHELL_DROPIN' or open a new shell."
    exit 0
}

$rollback_mode && do_rollback

# ── Network probing ─────────────────────────────────────────
echo "=== Network Probe ==="
echo ""

REACH_HF_MIRROR=false
REACH_PYPI_MIRROR=false
REACH_PYPI_DIRECT=false
REACH_GITHUB=false
REACH_GITHUB_MIRROR=false
REACH_PROXY=false

info "Testing pypi.org (direct) ..."
probe "https://pypi.org" && REACH_PYPI_DIRECT=true && ok "reachable" || warn "unreachable"

info "Testing github.com (direct) ..."
probe "https://github.com" && REACH_GITHUB=true && ok "reachable" || warn "unreachable"

info "Testing hf-mirror.com ..."
probe "$HF_ENDPOINT_URL" && REACH_HF_MIRROR=true && ok "reachable" || warn "unreachable"

info "Testing pypi.tuna.tsinghua.edu.cn ..."
probe "https://pypi.tuna.tsinghua.edu.cn/simple/" && REACH_PYPI_MIRROR=true && ok "reachable" || warn "unreachable"

info "Testing ghproxy.com ..."
probe "https://ghproxy.com" && REACH_GITHUB_MIRROR=true && ok "reachable" || warn "unreachable"

info "Testing proxy (${PROXY_HOST}:${PROXY_PORT}) ..."
if [[ "$(uname -s)" == "Linux" ]]; then
    timeout 2 bash -c "echo >/dev/tcp/${PROXY_HOST}/${PROXY_PORT}" 2>/dev/null && REACH_PROXY=true && ok "reachable" || warn "unreachable"
else
    nc -z -w 2 "$PROXY_HOST" "$PROXY_PORT" 2>/dev/null && REACH_PROXY=true && ok "reachable" || warn "unreachable"
fi

echo ""

# ── Determine profile ───────────────────────────────────────
determine_profile() {
    if [[ -n "$force_profile" ]]; then
        echo "$force_profile"
        return
    fi

    if $REACH_PROXY; then
        echo "wsl2-proxy"
    elif $REACH_GITHUB && $REACH_PYPI_DIRECT; then
        echo "direct"
    elif $REACH_PYPI_MIRROR || $REACH_HF_MIRROR; then
        echo "edge-mirror"
    else
        echo "isolated"
    fi
}

PROFILE=$(determine_profile)
echo "Profile: $PROFILE"

# ── Check mode ──────────────────────────────────────────────
if $check_mode; then
    echo ""
    echo "=== Current State ==="
    echo ""

    if [[ -f "$SHELL_DROPIN" ]]; then
        ok "Shell dropin: $SHELL_DROPIN"
        info "$(cat "$SHELL_DROPIN" | grep -v '^#' | grep -v '^$' || echo '  (empty)')"
    else
        warn "Shell dropin: not configured"
    fi

    if git config --global --get-regexp 'url\.https://ghproxy\.com' &>/dev/null; then
        ok "Git mirror: configured"
    else
        warn "Git mirror: not configured"
    fi

    if [[ -f /etc/docker/daemon.json ]]; then
        if python3 -c "
import json
c = json.load(open('/etc/docker/daemon.json'))
mirrors = c.get('registry-mirrors', [])
exit(0 if mirrors and any(m for m in mirrors) else 1)
" 2>/dev/null; then
            ok "Docker registry mirrors: configured"
        else
            warn "Docker registry mirrors: not configured (or empty)"
        fi
    else
        info "Docker: not installed (skip)"
    fi

    echo ""
    echo "Probe results:"
    echo "  pypi.org:      $REACH_PYPI_DIRECT"
    echo "  github.com:    $REACH_GITHUB"
    echo "  hf-mirror:     $REACH_HF_MIRROR"
    echo "  pypi-mirror:   $REACH_PYPI_MIRROR"
    echo "  ghproxy.com:   $REACH_GITHUB_MIRROR"
    echo "  proxy:7890:    $REACH_PROXY"
    echo "  → profile:     $PROFILE"

    if [[ "$PROFILE" == "isolated" ]]; then
        echo ""
        warn "This device appears to have no network connectivity."
        echo "      Bootstrap cannot configure mirrors — manual intervention required."
    fi

    exit 0
fi

# ── Direct: no config needed (unless --force) ────────────────
if [[ "$PROFILE" == "direct" ]]; then
    if $force_mode; then
        warn "Device has direct internet, but --force was passed — applying edge-mirror profile."
        PROFILE="edge-mirror"
    else
        echo ""
        ok "Device has direct internet access (pypi.org + github.com reachable)."
        echo "      No mirror/proxy configuration needed."
        echo "      To force bootstrap anyway: bootstrap.sh --force"
        exit 0
    fi
fi

# ── Isolated: warn and exit ─────────────────────────────────
if [[ "$PROFILE" == "isolated" ]]; then
    err "Device is isolated (no network detected)."
    echo "      Bootstrap cannot configure mirrors — no reachable endpoints."
    echo "      Set up a local PyPI mirror or proxy and re-run with --profile <name>."
    exit 1
fi

# ── Backup current state ────────────────────────────────────
backup_dir="${BACKUP_DIR}/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$MANAGED_DIR" "$BACKUP_DIR" "$backup_dir" "$(dirname "$SHELL_DROPIN")"

# Shell dropin
if [[ -f "$SHELL_DROPIN" ]]; then
    cp "$SHELL_DROPIN" "${backup_dir}/mirrors.sh"
fi

# Git config
git config --global --list 2>/dev/null > "${backup_dir}/git-config.backup" || true

# Docker daemon.json
if [[ -f /etc/docker/daemon.json ]]; then
    if [[ -r /etc/docker/daemon.json ]]; then
        cp /etc/docker/daemon.json "${backup_dir}/daemon.json" 2>/dev/null || true
    elif command -v sudo &>/dev/null && sudo -n true 2>/dev/null; then
        sudo cp /etc/docker/daemon.json "${backup_dir}/daemon.json"
    fi
fi

ok "Backed up to $backup_dir"

# ── Layer 1: Shell environment ──────────────────────────────
echo ""
echo "=== Layer 1: Shell Environment ==="

case "$PROFILE" in
    wsl2-proxy)
        cat > "$SHELL_DROPIN" <<MIRRORS_PROXY
# Managed by bootstrap.sh ($TIMESTAMP)
# Profile: wsl2-proxy
# Do not edit manually — re-run bootstrap.sh to update.

# Proxy
export https_proxy=http://${PROXY_HOST}:${PROXY_PORT}
export http_proxy=http://${PROXY_HOST}:${PROXY_PORT}

# Bypass proxy for: localhost, LAN, Chinese mirrors, internal registries
export no_proxy="localhost,127.0.0.1,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,.tsinghua.edu.cn,.aliyun.com,hf-mirror.com,ghproxy.com,.mirrors.ustc.edu.cn,.local,.internal,.docker.com"
export NO_PROXY="\$no_proxy"

# HuggingFace mirror
export HF_ENDPOINT=${HF_ENDPOINT_URL}

# PyPI mirror (uv + pip)
export UV_INDEX_URL=${PYPI_INDEX_URL}
export PIP_INDEX_URL=${PYPI_INDEX_URL}
MIRRORS_PROXY
        ok "Wrote $SHELL_DROPIN (proxy + no_proxy + mirrors)"
        ;;
    edge-mirror)
        cat > "$SHELL_DROPIN" <<MIRRORS_EDGE
# Managed by bootstrap.sh ($TIMESTAMP)
# Profile: edge-mirror
# Do not edit manually — re-run bootstrap.sh to update.

# HuggingFace mirror
export HF_ENDPOINT=${HF_ENDPOINT_URL}

# PyPI mirror (uv + pip)
export UV_INDEX_URL=${PYPI_INDEX_URL}
export PIP_INDEX_URL=${PYPI_INDEX_URL}
MIRRORS_EDGE
        ok "Wrote $SHELL_DROPIN (mirrors only, no proxy)"
        ;;
esac

# Source into this session too
source "$SHELL_DROPIN"

# Ensure shell profiles source our dropin
DOT_SOURCE='[ -f "$HOME/.profile.d/mirrors.sh" ] && source "$HOME/.profile.d/mirrors.sh"'

# Bash
if [[ -f "$HOME/.bashrc" ]]; then
    if ! grep -q "mirrors.sh" "$HOME/.bashrc" 2>/dev/null; then
        echo "$DOT_SOURCE" >> "$HOME/.bashrc"
        ok "Added source line to ~/.bashrc"
    else
        ok "~/.bashrc already sources mirrors.sh"
    fi
fi
# Zsh
if [[ -f "$HOME/.zshrc" ]]; then
    if ! grep -q "mirrors.sh" "$HOME/.zshrc" 2>/dev/null; then
        echo "$DOT_SOURCE" >> "$HOME/.zshrc"
        ok "Added source line to ~/.zshrc"
    else
        ok "~/.zshrc already sources mirrors.sh"
    fi
fi
# .profile — needed for login shells and as a fallback for BASH_ENV-aware non-interactive shells
if [[ -f "$HOME/.profile" ]]; then
    if ! grep -q "mirrors.sh" "$HOME/.profile" 2>/dev/null; then
        echo "$DOT_SOURCE" >> "$HOME/.profile"
        ok "Added source line to ~/.profile"
    fi
fi
# non-interactive Bash won't read .bashrc; set BASH_ENV so `ssh host cmd` still picks up mirrors
if ! grep -q "BASH_ENV" "$HOME/.bashrc" 2>/dev/null; then
    echo 'export BASH_ENV="$HOME/.profile.d/mirrors.sh"' >> "$HOME/.bashrc"
    ok "Set BASH_ENV in ~/.bashrc (non-interactive shells will auto-source mirrors)"
fi

# ── Layer 2: Git global config ──────────────────────────────
echo ""
echo "=== Layer 2: Git Global Config ==="

# Add insteadOf for GitHub→ghproxy (HTTPS only, SSH unaffected)
if ! git config --global --get-regexp 'url\.https://ghproxy\.com' &>/dev/null; then
    git config --global url."https://ghproxy.com/https://github.com".insteadOf "https://github.com"
    ok "git: https://github.com → https://ghproxy.com/https://github.com"
else
    ok "git mirror already configured"
fi

# If WSL2 profile & proxy reachable, also configure proxy bypass for mirrors
if [[ "$PROFILE" == "wsl2-proxy" ]]; then
    if ! git config --global --get-regexp 'http\.https://hf-mirror\.com' &>/dev/null; then
        git config --global "http.https://hf-mirror.com.proxy" ""
        ok "git: hf-mirror.com bypasses proxy"
    fi
fi

# ── Layer 3: Docker daemon ──────────────────────────────────
echo ""
echo "=== Layer 3: Docker Daemon ==="

if command -v docker &>/dev/null; then
    DOCKER_CONFIG="/etc/docker/daemon.json"
    DOCKER_MIRROR="https://docker.1ms.run"
    docker_modified=false

    # Detect whether we can write to /etc/docker (root, writable dir, or passwordless sudo)
    can_write_docker=false
    use_sudo=false
    if [[ -w "$DOCKER_CONFIG" ]] || { [[ ! -f "$DOCKER_CONFIG" ]] && [[ -w /etc/docker ]]; }; then
        can_write_docker=true
    elif command -v sudo &>/dev/null && sudo -n true 2>/dev/null; then
        can_write_docker=true
        use_sudo=true
    fi

    if ! $can_write_docker; then
        warn "Docker: no write access to $DOCKER_CONFIG (sudo requires password)"
        warn "       Run manually: sudo bash bootstrap.sh --profile $PROFILE"
    elif [[ -f "$DOCKER_CONFIG" ]]; then
        # Merge: preserve existing config, add registry-mirrors if missing
        tmpfile=$(mktemp /tmp/bootstrap-daemon.XXXXXX)
        trap "rm -f '$tmpfile'" EXIT
        merge_rc=0
        python3 -c "
import json, sys

with open('$DOCKER_CONFIG') as f:
    try:
        config = json.load(f)
    except json.JSONDecodeError as e:
        print('JSON_PARSE_ERROR', file=sys.stderr)
        sys.exit(2)

existing = config.get('registry-mirrors', [])
mirror = '$DOCKER_MIRROR'

if mirror not in existing:
    config['registry-mirrors'] = existing + [mirror]
    print(json.dumps(config, indent=2, ensure_ascii=False))
    sys.exit(0)
else:
    sys.exit(1)
" > "$tmpfile" 2>/dev/null && merge_rc=$? || merge_rc=$?

        if [[ $merge_rc -eq 0 ]]; then
            if $use_sudo; then
                sudo cp "$tmpfile" "$DOCKER_CONFIG"
            else
                cp "$tmpfile" "$DOCKER_CONFIG"
            fi
            ok "Docker: added registry-mirror $DOCKER_MIRROR"
            docker_modified=true
        elif [[ $merge_rc -eq 2 ]]; then
            warn "Docker: failed to parse $DOCKER_CONFIG (invalid JSON) — skipping"
        else
            ok "Docker: registry-mirror already present"
        fi
        rm -f "$tmpfile"
    else
        # Create new daemon.json
        tmpfile=$(mktemp /tmp/bootstrap-daemon.XXXXXX)
        trap "rm -f '$tmpfile'" EXIT
        cat > "$tmpfile" <<DOCKEREOF
{
  "registry-mirrors": ["${DOCKER_MIRROR}"]
}
DOCKEREOF
        if $use_sudo; then
            sudo mkdir -p /etc/docker
            sudo cp "$tmpfile" "$DOCKER_CONFIG"
        else
            mkdir -p /etc/docker
            cp "$tmpfile" "$DOCKER_CONFIG"
        fi
        rm -f "$tmpfile"
        ok "Docker: created $DOCKER_CONFIG with registry-mirror"
        docker_modified=true
    fi

    # Restart Docker if config was modified and we have root access
    if $docker_modified; then
        _restart_docker() {
            # systemctl → kill -HUP (bypasses polkit when sudo works but systemctl doesn't)
            if systemctl restart docker 2>/dev/null; then
                return 0
            fi
            local pidfile=""
            for f in /run/docker.pid /var/run/docker.pid; do
                [[ -f "$f" ]] && pidfile="$f" && break
            done
            if [[ -n "$pidfile" ]]; then
                kill -HUP "$(cat "$pidfile")" 2>/dev/null && return 0
            fi
            return 1
        }

        if [[ $EUID -eq 0 ]]; then
            if _restart_docker; then
                ok "Docker: restarted — registry mirrors now active"
            else
                warn "Docker: restart failed — run 'systemctl restart docker' manually"
            fi
        elif $use_sudo; then
            if sudo bash -c "$(declare -f _restart_docker); _restart_docker" 2>/dev/null; then
                ok "Docker: restarted — registry mirrors now active"
            else
                warn "Docker: restart failed (polkit timeout?) — run 'sudo systemctl restart docker' manually"
            fi
        else
            warn "Docker: daemon.json updated but cannot restart — run 'sudo systemctl restart docker' manually"
        fi
    fi

    trap - EXIT
else
    info "Docker not installed — skipping"
fi

# ── Save managed state ──────────────────────────────────────
python3 -c "
import json

state = {
    'version': 1,
    'profile': '$PROFILE',
    'configured_at': '$TIMESTAMP',
    'shell_dropin': '$SHELL_DROPIN',
    'config': {
        'hf_endpoint': '$HF_ENDPOINT_URL' if '$PROFILE' != 'isolated' else None,
        'pypi_index': '$PYPI_INDEX_URL' if '$PROFILE' != 'isolated' else None,
        'proxy': 'http://${PROXY_HOST}:${PROXY_PORT}' if '$PROFILE' == 'wsl2-proxy' else None,
    },
    'backup': '$backup_dir'
}

with open('$MANAGED_FILE', 'w') as f:
    json.dump(state, f, indent=2, ensure_ascii=False)
print('ok')
" && ok "Saved managed state to $MANAGED_FILE"

# ── Summary ─────────────────────────────────────────────────
echo ""
echo "=== Bootstrap Complete ==="
echo ""
echo "Profile:   $PROFILE"
echo "Shell:     $SHELL_DROPIN"
echo "Backup:    $backup_dir"
echo ""
echo "To apply immediately in this shell:"
echo "  source $SHELL_DROPIN"
echo ""
echo "New shells will pick up the config automatically."
echo ""
echo "Verify with: bootstrap.sh --check"

