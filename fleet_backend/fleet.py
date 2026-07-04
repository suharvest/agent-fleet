#!/usr/bin/env python3
"""fleet — Local cluster management CLI for edge devices."""
import json
import os
import sys
import stat
import shlex
import shutil
import argparse
import subprocess
import paramiko
from concurrent.futures import ThreadPoolExecutor, as_completed


DEFAULT_DEVICES_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "devices.json")
DEVICES_FILE = (
    os.environ.get("FLEET_DEVICES_FILE")
    or os.environ.get("RPTY_FLEET_DEVICES")
    or DEFAULT_DEVICES_FILE
)
SSH_TIMEOUT = 5
CMD_TIMEOUT = 10


def load_devices():
    """Load devices.json, warn if permissions are too open."""
    if not os.path.exists(DEVICES_FILE):
        example = os.path.join(os.path.dirname(os.path.abspath(__file__)), "devices.example.json")
        print(f"Error: {DEVICES_FILE} not found", file=sys.stderr)
        if os.path.exists(example):
            print(f"Copy {example} to {DEFAULT_DEVICES_FILE} and fill in your devices.", file=sys.stderr)
            print("Or set FLEET_DEVICES_FILE=/path/to/devices.json.", file=sys.stderr)
        sys.exit(1)

    if sys.platform != "win32":
        mode = os.stat(DEVICES_FILE).st_mode
        if mode & (stat.S_IRGRP | stat.S_IROTH):
            print(f"Warning: {DEVICES_FILE} permissions too open. Run: chmod 600 {DEVICES_FILE}", file=sys.stderr)

    with open(DEVICES_FILE) as f:
        data = json.load(f)
    return data.get("devices", {})


def mask_device(device):
    """Return a copy of device dict with password masked."""
    masked = dict(device)
    if masked.get("password"):
        masked["password"] = "***"
    return masked


def filter_by_tags(devices, tags):
    """Filter devices that have ALL specified tags."""
    result = {}
    for name, dev in devices.items():
        dev_tags = set(dev.get("tags", []))
        if all(t in dev_tags for t in tags):
            result[name] = dev
    return result


def filter_by_owner(devices, owner):
    """Filter devices by owner (personal/company)."""
    return {name: dev for name, dev in devices.items() if dev.get("owner") == owner}


def format_table(rows, headers):
    """Print a simple aligned table."""
    widths = [len(h) for h in headers]
    for row in rows:
        for i, val in enumerate(row):
            widths[i] = max(widths[i], len(str(val)))

    fmt = "  ".join(f"{{:<{w}}}" for w in widths)
    print(fmt.format(*headers))
    print(fmt.format(*("-" * w for w in widths)))
    for row in rows:
        print(fmt.format(*row))


def cmd_list(args):
    """List devices with masked passwords."""
    devices = load_devices()
    if args.tag:
        devices = filter_by_tags(devices, args.tag)
    if args.owner:
        devices = filter_by_owner(devices, args.owner)

    if args.json_output:
        output = {name: mask_device(dev) for name, dev in devices.items()}
        print(json.dumps(output, indent=2, ensure_ascii=False))
        return

    if not devices:
        print("No devices found.")
        return

    rows = []
    for name, dev in devices.items():
        rows.append([
            name,
            dev.get("host", ""),
            dev.get("owner", ""),
            ", ".join(dev.get("tags", [])),
            dev.get("description", ""),
        ])
    format_table(rows, ["NAME", "HOST", "OWNER", "TAGS", "DESCRIPTION"])


def ssh_connect(host, user, password, port=22):
    """Create an SSH client connection. Tries password first, falls back to key auth."""
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    try:
        client.connect(host, port=port, username=user, password=password or None,
                       timeout=SSH_TIMEOUT, look_for_keys=False,
                       allow_agent=False)
    except (paramiko.AuthenticationException, paramiko.SSHException):
        client.close()
        client = paramiko.SSHClient()
        client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
        key_path = os.path.expanduser("~/.ssh/id_ed25519")
        key_file = key_path if os.path.exists(key_path) else None
        client.connect(host, port=port, username=user, timeout=SSH_TIMEOUT,
                       look_for_keys=True, allow_agent=True,
                       key_filename=key_file)
    return client


def _decode(data: bytes) -> str:
    """Decode remote output: strict UTF-8 first, then GBK (Windows zh-CN OEM
    codepage — cmd.exe emits GBK bytes that mojibake under UTF-8 replace),
    finally UTF-8 with replacement as last resort."""
    for enc in ("utf-8", "gbk"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("utf-8", errors="replace")


def is_windows_device(dev) -> bool:
    """True if the device record marks a Windows host (tag or probed OS)."""
    if "windows" in {t.lower() for t in dev.get("tags", [])}:
        return True
    return "windows" in str(dev.get("specs", {}).get("os", "")).lower()


def ssh_exec(host, user, password, command, timeout=CMD_TIMEOUT, sudo=False, port=22, stream=False, raw=False, windows=False):
    """Execute command via SSH, return (success, stdout_text).

    When stream=True (non-sudo only), stdout/stderr chunks are written to the
    local terminal as they arrive, in addition to being collected for the
    return value. Useful for long builds where C stdio block-buffering
    otherwise withholds output until the command exits.
    """
    # Non-interactive SSH shells skip ~/.profile, so ~/.local/bin is often
    # missing from PATH. Export it unconditionally — harmless even if absent.
    if windows and sudo:
        return False, ("[fleet] --sudo is not supported on Windows devices (no sudo/PTY semantics). "
                       "Run an elevated command via an admin SSH account or Start-Process -Verb RunAs.")
    try:
        client = ssh_connect(host, user, password, port=port)
        if sudo:
            # Use PTY for sudo, send password via stdin.
            # Auto-inject DEBIAN_FRONTEND=noninteractive and PATH so
            # apt/dpkg doesn't drop into whiptail dialogs and user-installed
            # binaries (uv, pipx, etc.) are found.
            channel = client.get_transport().open_session()
            channel.get_pty()
            sudo_cmd = f'sudo -S -p \'\' env DEBIAN_FRONTEND=noninteractive PATH="$HOME/.local/bin:$PATH" {command}'
            channel.exec_command(sudo_cmd)
            channel.send(password + "\n")
            channel.shutdown_write()

            # Read output: poll until the remote command exits OR the caller's
            # timeout fires. Do NOT break on "silent for N seconds" — long
            # apt/dpkg stages (package download, postinst scripts) routinely
            # produce nothing for 10-30s and that's normal, not a hang.
            import time
            start = time.time()
            raw_output = b""

            while time.time() - start < timeout:
                if channel.recv_ready():
                    raw_output += channel.recv(65536)
                    continue
                if channel.exit_status_ready() and not channel.recv_ready():
                    break
                time.sleep(0.1)

            # Final drain (data can arrive right after exit_status_ready)
            for _ in range(20):
                if channel.recv_ready():
                    raw_output += channel.recv(65536)
                else:
                    break
                time.sleep(0.05)
            output = _decode(raw_output)

            timed_out = not channel.exit_status_ready()
            exit_code = channel.recv_exit_status() if channel.exit_status_ready() else -1
            client.close()
            # Filter out sudo prompt residue AND the PTY-echoed password line.
            # PTY has echo on by default, so the password we sent on stdin
            # appears verbatim in stdout — strip it before returning.
            lines = [
                l for l in output.split("\n")
                if not l.strip().startswith("[sudo]")
                and l.strip() != password
            ]
            cleaned = "\n".join(lines).strip()
            if timed_out:
                return False, f"[fleet] command timed out after {timeout}s — retry with --timeout <seconds>. Partial output:\n{cleaned}"
            if exit_code != 0:
                hint = ""
                if "permission denied" in cleaned.lower() or "not allowed" in cleaned.lower():
                    hint = "\n[fleet] Hint: verify the command has correct permissions or that --sudo was intended."
                return False, f"[fleet] command failed (exit {exit_code}): {cleaned}{hint}"
            return True, cleaned
        else:
            # raw/windows: pass the command verbatim (no bash wrapper).
            # Windows cmd.exe/PowerShell chokes on the `export ...;` prefix
            # below — auto-skipped for devices tagged 'windows'.
            if raw or windows:
                wrapped = command
            else:
                wrapped = f'export PATH="$HOME/.local/bin:$PATH"; [ -f "$HOME/.profile.d/mirrors.sh" ] && . "$HOME/.profile.d/mirrors.sh"; {command}'
            if stream:
                import time
                channel = client.get_transport().open_session()
                channel.exec_command(wrapped)
                start = time.time()
                parts = []
                while time.time() - start < timeout:
                    progressed = False
                    if channel.recv_ready():
                        data = channel.recv(65536)
                        sys.stdout.write(data.decode(errors="replace")); sys.stdout.flush()
                        parts.append(data)
                        progressed = True
                    if channel.recv_stderr_ready():
                        data = channel.recv_stderr(65536)
                        sys.stderr.write(data.decode(errors="replace")); sys.stderr.flush()
                        progressed = True
                    if not progressed:
                        if channel.exit_status_ready():
                            break
                        time.sleep(0.1)
                while channel.recv_ready():
                    data = channel.recv(65536)
                    sys.stdout.write(data.decode(errors="replace")); sys.stdout.flush()
                    parts.append(data)
                while channel.recv_stderr_ready():
                    sys.stderr.write(channel.recv_stderr(65536).decode(errors="replace"))
                    sys.stderr.flush()
                timed_out = not channel.exit_status_ready()
                exit_code = channel.recv_exit_status() if channel.exit_status_ready() else -1
                client.close()
                output = _decode(b"".join(parts)).strip()
                if timed_out:
                    return False, f"[fleet] command timed out after {timeout}s — retry with --timeout <seconds>. Partial output:\n{output}"
                if exit_code != 0:
                    hint = ""
                    if "permission denied" in output.lower() or "not allowed" in output.lower():
                        hint = "\n[fleet] Hint: this command likely needs --sudo. Retry with: fleet exec --sudo <device> -- <command>"
                    return False, f"[fleet] command failed (exit {exit_code}): {output}{hint}"
                return True, output
            _, stdout, stderr = client.exec_command(wrapped, timeout=timeout)
            output = _decode(stdout.read()).strip()
            err_output = _decode(stderr.read()).strip()
            exit_code = stdout.channel.recv_exit_status()
            client.close()
            if exit_code != 0:
                detail = f"{err_output}\n{output}".strip() or f"exit code {exit_code}"
                hint = ""
                combined = f"{err_output}\n{output}".lower()
                if "permission denied" in combined or "not allowed" in combined or "are you root" in combined:
                    hint = "\n[fleet] Hint: this command likely needs --sudo. Retry with: fleet exec --sudo <device> -- <command>"
                return False, f"[fleet] command failed (exit {exit_code}): {detail}{hint}"
            return True, output
    except Exception as e:
        return False, str(e)


def probe_device(name, dev):
    """Probe a single device for status info. Returns dict."""
    host = dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    result = {
        "name": name,
        "host": host,
        "online": False,
        "tags": dev.get("tags", []),
        "description": dev.get("description", ""),
        "gateway": dev.get("gateway"),
        "wsl_distro": dev.get("wsl_distro"),
    }

    cmd = (
        "echo '---DISK---' && df -h / | tail -1 && "
        "echo '---MEM---' && free -m | awk 'NR==2{print}' && "
        "echo '---CPU---' && uptime && "
        "echo '---GPU---' && (nvidia-smi --query-gpu=name,memory.used,memory.total,utilization.gpu --format=csv,noheader,nounits 2>/dev/null || echo 'N/A')"
    )

    ok, output = ssh_exec(host, user, password, cmd, port=port)
    if not ok:
        return result

    result["online"] = True

    sections = {}
    current = None
    for line in output.split("\n"):
        if line.startswith("---") and line.endswith("---"):
            current = line.strip("-")
            sections[current] = []
        elif current:
            sections[current].append(line)

    if "DISK" in sections and sections["DISK"]:
        parts = sections["DISK"][0].split()
        if len(parts) >= 4:
            result["disk"] = {"total": parts[1], "used": parts[2], "avail": parts[3], "use_pct": parts[4] if len(parts) > 4 else ""}

    if "MEM" in sections and sections["MEM"]:
        parts = sections["MEM"][0].split()
        if len(parts) >= 3:
            result["memory"] = {"total_mb": parts[1], "used_mb": parts[2], "free_mb": parts[3] if len(parts) > 3 else ""}

    if "CPU" in sections and sections["CPU"]:
        line = sections["CPU"][0]
        if "load average:" in line:
            load_str = line.split("load average:")[-1].strip()
            result["cpu_load"] = load_str

    if "GPU" in sections and sections["GPU"] and sections["GPU"][0] != "N/A":
        parts = sections["GPU"][0].split(", ")
        if len(parts) >= 4:
            result["gpu"] = {"name": parts[0], "mem_used_mb": parts[1], "mem_total_mb": parts[2], "util_pct": parts[3]}

    return result


def cmd_status(args):
    """Query device status via SSH probes."""
    devices = load_devices()

    if args.device:
        if args.device not in devices:
            print(f"Error: device '{args.device}' not found", file=sys.stderr)
            sys.exit(1)
        devices = {args.device: devices[args.device]}
    else:
        if args.tag:
            devices = filter_by_tags(devices, args.tag)
        if args.owner:
            devices = filter_by_owner(devices, args.owner)

    results = []
    with ThreadPoolExecutor(max_workers=len(devices) or 1) as pool:
        futures = {pool.submit(probe_device, name, dev): name for name, dev in devices.items()}
        for future in as_completed(futures):
            results.append(future.result())

    results.sort(key=lambda r: r["name"])

    if args.json_output:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return

    if not results:
        print("No devices found.")
        return

    rows = []
    for r in results:
        status = "ONLINE" if r["online"] else "OFFLINE"
        if not r["online"] and r.get("gateway"):
            status = f"OFFLINE [gw:{r['gateway']}]"
        disk = r.get("disk", {}).get("avail", "-") + " free" if r.get("disk") else "-"
        mem = r.get("memory", {})
        mem_str = f"{mem.get('used_mb', '?')}/{mem.get('total_mb', '?')} MB" if mem else "-"
        cpu = r.get("cpu_load", "-")
        gpu = r.get("gpu", {}).get("name", "-") if r.get("gpu") else "-"
        rows.append([r["name"], status, disk, mem_str, cpu, gpu])

    format_table(rows, ["NAME", "STATUS", "DISK", "MEMORY", "CPU LOAD", "GPU"])


def cmd_match(args):
    """Find online devices matching tags, optionally sorted by resource."""
    devices = load_devices()
    devices = filter_by_tags(devices, args.tag)
    if args.owner:
        devices = filter_by_owner(devices, args.owner)

    if not devices:
        if args.json_output:
            print("[]")
        else:
            print("No devices match the specified tags.")
        return

    results = []
    with ThreadPoolExecutor(max_workers=len(devices) or 1) as pool:
        futures = {pool.submit(probe_device, name, dev): name for name, dev in devices.items()}
        for future in as_completed(futures):
            results.append(future.result())

    results = [r for r in results if r["online"]]

    if not results:
        if args.json_output:
            print("[]")
        else:
            print("No online devices match the specified tags.")
        return

    if args.sort == "disk":
        def disk_avail(r):
            avail = r.get("disk", {}).get("avail", "0").upper()
            multiplier = {"K": 1, "M": 1024, "G": 1024**2, "T": 1024**3}
            for suffix, mult in multiplier.items():
                if avail.endswith(suffix):
                    try:
                        return float(avail[:-1]) * mult
                    except ValueError:
                        return 0
            try:
                return float(avail)
            except ValueError:
                return 0
        results.sort(key=disk_avail, reverse=True)
    elif args.sort == "memory":
        def mem_free(r):
            try:
                return int(r.get("memory", {}).get("free_mb", 0))
            except (ValueError, TypeError):
                return 0
        results.sort(key=mem_free, reverse=True)
    elif args.sort == "cpu":
        def cpu_load(r):
            load_str = r.get("cpu_load", "99, 99, 99")
            try:
                return float(load_str.split(",")[0].strip())
            except (ValueError, IndexError):
                return 99
        results.sort(key=cpu_load)
    else:
        results.sort(key=lambda r: r["name"])

    if args.json_output:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return

    rows = []
    for r in results:
        disk = r.get("disk", {}).get("avail", "-")
        mem = r.get("memory", {})
        mem_str = f"{mem.get('free_mb', '?')} MB free" if mem else "-"
        cpu = r.get("cpu_load", "-")
        ssh_cmd = f"ssh {r.get('host')}"
        rows.append([r["name"], disk, mem_str, cpu, ssh_cmd])

    format_table(rows, ["NAME", "DISK AVAIL", "MEM FREE", "CPU LOAD", "SSH"])


def save_devices(devices):
    """Save devices back to devices.json, preserving _meta."""
    with open(DEVICES_FILE) as f:
        data = json.load(f)
    data["devices"] = devices
    with open(DEVICES_FILE, "w") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")


def scan_device(name, dev):
    """SSH into device and auto-detect hardware specs and suggested tags."""
    host = dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    cmd = (
        "echo '---ARCH---' && uname -m && "
        "echo '---OS---' && (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\" || echo \"macOS $(sw_vers -productVersion 2>/dev/null || echo unknown)\") && "
        "echo '---MODEL---' && (cat /proc/device-tree/model 2>/dev/null || system_profiler SPHardwareDataType 2>/dev/null | grep 'Model Name\\|Chip' | head -2 || echo 'N/A') && "
        "echo '---CPU---' && (nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'N/A') && "
        "echo '---MEM---' && (if command -v free >/dev/null 2>&1; then free -b | awk 'NR==2{print $2}'; else sysctl -n hw.memsize 2>/dev/null || echo 'N/A'; fi) && "
        "echo '---DISK---' && (if df -B1 / >/dev/null 2>&1; then df -B1 / | tail -1 | awk '{print $2}'; else df -k / | tail -1 | awk '{print $2 * 1024}'; fi) && "
        "echo '---GPU---' && (nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits 2>/dev/null || echo 'N/A') && "
        "echo '---ACCEL---' && (ls /dev/hailo* 2>/dev/null && hailortcli fw-control identify 2>/dev/null | grep 'Board Name\\|Device Architecture' || echo 'N/A') && "
        "echo '---NET---' && (curl -s --connect-timeout 3 --max-time 5 -o /dev/null -w '%{http_code}' https://pypi.org 2>/dev/null || echo '0') && "
        "echo '---HOSTNAME---' && hostname"
    )

    ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, port=port)
    if not ok:
        return None, f"offline: {output}"

    sections = {}
    current = None
    for line in output.split("\n"):
        if line.startswith("---") and line.endswith("---"):
            current = line.strip("-")
            sections[current] = []
        elif current:
            sections[current].append(line)

    specs = {}
    suggested_tags = set()

    # Architecture
    if "ARCH" in sections and sections["ARCH"]:
        arch = sections["ARCH"][0].strip()
        specs["arch"] = arch
        if arch in ("aarch64", "arm64"):
            suggested_tags.add("arm64")
        elif arch in ("x86_64", "amd64"):
            suggested_tags.add("x86_64")

    # OS
    if "OS" in sections and sections["OS"]:
        os_str = sections["OS"][0].strip()
        specs["os"] = os_str
        if "macos" in os_str.lower() or os_str == "Darwin":
            suggested_tags.add("macos")

    # Device model (Jetson, Raspberry Pi, macOS, etc.)
    if "MODEL" in sections and sections["MODEL"]:
        raw_model = sections["MODEL"][0].split("\x00")[0].strip()
        # Parse system_profiler output (macOS): "      Model Name: Mac mini"
        if ":" in raw_model:
            parts = {}
            for line in sections["MODEL"]:
                if ":" in line:
                    k, v = line.split(":", 1)
                    parts[k.strip()] = v.strip()
            model = parts.get("Model Name", "")
            if parts.get("Chip"):
                specs["cpu"] = parts["Chip"]
                suggested_tags.add(parts["Chip"].lower().replace(" ", "-"))
        else:
            model = raw_model
        if model and model != "N/A":
            specs["model"] = model
            model_lower = model.lower()
            if "jetson" in model_lower:
                suggested_tags.add("jetson")
                if "orin" in model_lower:
                    suggested_tags.add("orin")
                    if "agx" in model_lower:
                        suggested_tags.add("agx-orin")
                    elif "nano" in model_lower:
                        suggested_tags.add("orin-nano")
                    elif "nx" in model_lower:
                        suggested_tags.add("orin-nx")
                if "xavier" in model_lower:
                    suggested_tags.add("xavier")
            elif "raspberry" in model_lower:
                suggested_tags.add("rpi")

    # CPU cores
    if "CPU" in sections and sections["CPU"]:
        try:
            specs["cpu_cores"] = int(sections["CPU"][0].strip())
        except ValueError:
            pass

    # Memory (bytes → human readable)
    if "MEM" in sections and sections["MEM"]:
        try:
            mem_bytes = int(sections["MEM"][0].strip())
            mem_gb = round(mem_bytes / (1024**3))
            specs["ram_gb"] = mem_gb
            specs["ram"] = f"{mem_gb}GB"
        except ValueError:
            pass

    # Disk (bytes → human readable)
    if "DISK" in sections and sections["DISK"]:
        try:
            disk_bytes = int(sections["DISK"][0].strip())
            disk_gb = round(disk_bytes / (1024**3))
            specs["storage_gb"] = disk_gb
            specs["storage"] = f"{disk_gb}GB"
        except ValueError:
            pass

    # GPU
    if "GPU" in sections and sections["GPU"] and sections["GPU"][0].strip() != "N/A":
        gpu_line = sections["GPU"][0].strip()
        parts = [p.strip() for p in gpu_line.split(",")]
        if parts:
            specs["gpu"] = parts[0]
            if len(parts) > 1:
                try:
                    specs["gpu_mem_mb"] = int(parts[1])
                    specs["gpu_mem"] = f"{round(int(parts[1]) / 1024)}GB"
                except ValueError:
                    pass
            suggested_tags.add("gpu")

    # Accelerators (Hailo, etc.)
    if "ACCEL" in sections and sections["ACCEL"] and sections["ACCEL"][0].strip() != "N/A":
        accel_lines = sections["ACCEL"]
        accels = []
        # Detect Hailo devices
        hailo_devs = [l.strip() for l in accel_lines if "/dev/hailo" in l]
        if hailo_devs:
            accel_info = {"type": "hailo", "devices": hailo_devs}
            for l in accel_lines:
                if "Board Name" in l:
                    accel_info["board"] = l.split(":")[-1].strip()
                if "Device Architecture" in l:
                    accel_info["arch"] = l.split(":")[-1].strip()
            accels.append(accel_info)
            suggested_tags.add("hailo")
            # Add specific model tag if detected
            for l in accel_lines:
                if "HAILO8" in l.upper():
                    suggested_tags.add("hailo-8")
                elif "HAILO8L" in l.upper() or "HAILO-8L" in l.upper():
                    suggested_tags.add("hailo-8l")
        if accels:
            specs["accelerators"] = accels

    # Hostname
    if "HOSTNAME" in sections and sections["HOSTNAME"]:
        specs["hostname"] = sections["HOSTNAME"][0].strip()

    # Network connectivity
    if "NET" in sections and sections["NET"]:
        try:
            http_code = int(sections["NET"][0].strip())
            if 200 <= http_code < 500:
                suggested_tags.add("direct-internet")
        except ValueError:
            pass

    return {"specs": specs, "suggested_tags": list(suggested_tags)}, None


def cmd_add(args):
    """Add a new device to the fleet."""
    devices = load_devices()
    if args.name in devices:
        print(f"Error: device '{args.name}' already exists", file=sys.stderr)
        sys.exit(1)

    devices[args.name] = {
        "host": args.host,
        "user": args.user,
        "password": args.password or "",
        "owner": args.owner,
        "tags": args.tag,
        "specs": {},
        "description": args.desc,
    }
    save_devices(devices)
    print(f"Added '{args.name}' ({args.host})")

    if args.scan:
        print(f"Scanning {args.name}...")
        result, err = scan_device(args.name, devices[args.name])
        if err:
            print(f"  scan failed: {err}")
        else:
            devices[args.name]["specs"] = result["specs"]
            # Merge suggested tags
            existing = set(devices[args.name]["tags"])
            for t in result.get("suggested_tags", []):
                if t not in existing:
                    devices[args.name]["tags"].append(t)
            save_devices(devices)
            print(f"  specs: {json.dumps(result['specs'], ensure_ascii=False)}")
            print(f"  tags: {devices[args.name]['tags']}")


def cmd_remove(args):
    """Remove a device from the fleet."""
    devices = load_devices()
    if args.name not in devices:
        print(f"Error: device '{args.name}' not found", file=sys.stderr)
        sys.exit(1)

    if not args.force:
        resp = input(f"Remove '{args.name}'? [y/N] ").strip().lower()
        if resp != "y":
            print("Cancelled.")
            return

    del devices[args.name]
    save_devices(devices)
    print(f"Removed '{args.name}'")


def cmd_scan(args):
    """Scan devices to auto-detect specs and update devices.json."""
    devices = load_devices()

    if args.device:
        if args.device not in devices:
            print(f"Error: device '{args.device}' not found", file=sys.stderr)
            sys.exit(1)
        targets = {args.device: devices[args.device]}
    elif args.tag:
        targets = filter_by_tags(devices, args.tag)
    else:
        targets = devices

    # Parallel scan
    results = {}
    with ThreadPoolExecutor(max_workers=len(targets) or 1) as pool:
        futures = {pool.submit(scan_device, name, dev): name for name, dev in targets.items()}
        for future in as_completed(futures):
            name = futures[future]
            info, err = future.result()
            results[name] = (info, err)

    updated = []
    for name in sorted(results):
        info, err = results[name]
        if err:
            print(f"  {name}: SKIP ({err})")
            continue

        dev = devices[name]
        changes = []

        # Update specs (only fill missing keys, never overwrite existing)
        if info["specs"]:
            old_specs = dev.get("specs", {})
            new_keys = {k: v for k, v in info["specs"].items() if k not in old_specs}
            if new_keys:
                dev["specs"] = {**old_specs, **new_keys}
                changes.append(f"specs: +{', '.join(new_keys.keys())}")

        # Add missing tags
        current_tags = set(dev.get("tags", []))
        new_tags = set(info["suggested_tags"]) - current_tags
        if new_tags:
            dev["tags"] = list(current_tags | new_tags)
            changes.append(f"tags: +{', '.join(sorted(new_tags))}")

        if changes:
            updated.append(name)
            print(f"  {name}: UPDATED ({'; '.join(changes)})")
        else:
            print(f"  {name}: OK (no changes)")

    if updated and not args.dry_run:
        save_devices(devices)
        print(f"\nSaved {len(updated)} device(s) to {DEVICES_FILE}")
    elif updated and args.dry_run:
        print(f"\nDry run: {len(updated)} device(s) would be updated")

    if args.json_output:
        output = {}
        for name in sorted(results):
            info, err = results[name]
            output[name] = {"error": err} if err else info
        print(json.dumps(output, indent=2, ensure_ascii=False))


def cmd_bootstrap(args):
    """Run bootstrap.sh on target device(s) to configure mirrors/proxy."""
    import base64

    devices = load_devices()

    # Determine targets
    if args.all_devices:
        targets = dict(devices)
    elif args.tag:
        targets = filter_by_tags(devices, args.tag)
        if not targets:
            print("No devices match the specified tags.", file=sys.stderr)
            sys.exit(1)
    elif args.device:
        if args.device not in devices:
            print(f"Error: device '{args.device}' not found", file=sys.stderr)
            sys.exit(1)
        targets = {args.device: devices[args.device]}
    else:
        print("Error: specify a device, --all, or --tag", file=sys.stderr)
        sys.exit(1)

    # Read bootstrap.sh from same directory as fleet.py
    script_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bootstrap.sh")
    if not os.path.exists(script_path):
        print(f"Error: bootstrap.sh not found at {script_path}", file=sys.stderr)
        sys.exit(1)

    with open(script_path) as f:
        script_content = f.read()

    # Build remote args
    remote_args = []
    if args.check:
        remote_args.append("--check")
    if args.force_bootstrap:
        remote_args.append("--force")
    if args.profile:
        remote_args.extend(["--profile", args.profile])

    # Base64-encode to safely transport script over SSH
    encoded = base64.b64encode(script_content.encode()).decode()
    remote_cmd = f"echo {shlex.quote(encoded)} | base64 -d | bash -s -- {' '.join(remote_args)}"

    results = {}

    def run_on(name, dev):
        host = dev.get("host", "")
        user = dev.get("user", "")
        password = dev.get("password", "")
        port = dev.get("port", 22)
        ok, output = ssh_exec(host, user, password, remote_cmd, timeout=60, port=port)
        return name, ok, output

    with ThreadPoolExecutor(max_workers=min(len(targets), 10)) as pool:
        futures = [pool.submit(run_on, name, dev) for name, dev in targets.items()]
        for future in as_completed(futures):
            name, ok, output = future.result()
            results[name] = {"success": ok, "output": output}

    if args.json_output:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return

    for name in sorted(results):
        r = results[name]
        if len(targets) > 1:
            print(f"=== {name} ===")
        print(r["output"])
        if len(targets) > 1:
            print()


def cmd_docker(args):
    """Show Docker container status on a device."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    cmd = 'docker ps -a --format \'{"name":"{{.Names}}","image":"{{.Image}}","status":"{{.Status}}","ports":"{{.Ports}}"}\''

    ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, port=port)
    if not ok:
        print(f"Error: cannot connect to {args.device} ({host}): {output}", file=sys.stderr)
        sys.exit(1)

    if not output.strip():
        print(f"No containers on {args.device}.")
        return

    containers = []
    for line in output.strip().split("\n"):
        try:
            containers.append(json.loads(line))
        except json.JSONDecodeError:
            continue

    if args.json_output:
        print(json.dumps({"device": args.device, "containers": containers}, indent=2, ensure_ascii=False))
        return

    if not containers:
        print(f"No containers on {args.device}.")
        return

    print(f"Containers on {args.device} ({host}):\n")
    rows = []
    for c in containers:
        rows.append([c.get("name", ""), c.get("image", ""), c.get("status", ""), c.get("ports", "")])
    format_table(rows, ["NAME", "IMAGE", "STATUS", "PORTS"])


def cmd_exec(args):
    """Execute a command on one or more devices."""
    devices = load_devices()

    # Determine targets
    if args.tag:
        targets = filter_by_tags(devices, args.tag)
        if not targets:
            print("No devices match the specified tags.", file=sys.stderr)
            sys.exit(1)
    else:
        if args.device not in devices:
            print(f"Error: device '{args.device}' not found", file=sys.stderr)
            sys.exit(1)
        targets = {args.device: devices[args.device]}

    # Command comes after -- in REMAINDER
    cmd_parts = args.cmd_args
    if cmd_parts and cmd_parts[0] == "--":
        cmd_parts = cmd_parts[1:]
    if not cmd_parts:
        print("Error: no command specified. Usage: fleet exec <device> -- <command>", file=sys.stderr)
        sys.exit(1)
    if cmd_parts[0] == "sudo" and not args.sudo:
        print(
            "Error: don't prefix the command with 'sudo' — non-interactive SSH can't prompt for a password.\n"
            "       Use the --sudo flag instead, which auto-injects the device password:\n"
            f"           fleet exec --sudo {args.device} -- {' '.join(cmd_parts[1:]) or '<cmd>'}",
            file=sys.stderr,
        )
        sys.exit(2)
    if not args.literal and any(p == "-c" for p in cmd_parts):
        print(
            f"Warning: command uses '-c' without --literal — quoting will be destroyed.\n"
            f"         Add --literal to preserve: fleet exec --literal {args.device} -- {' '.join(cmd_parts)}\n"
            f"         Rules: --literal needed for bash/python -c, awk/sed scripts, heredocs.\n"
            f"         No --literal needed for: simple cmds, pipes (|), redirects (>), globs (*).",
            file=sys.stderr,
        )
    if any("<<" in p for p in cmd_parts):
        print(
            f"Warning: command contains heredoc ('<<') — fragile over SSH due to multi-layer quoting.\n"
            f"         Heredoc terminator may not land on its own line after shlex.join + 'export PATH=...;' prefix,\n"
            f"         causing the command to silently produce no file. Recommended:\n"
            f"           1. Write the script/file locally\n"
            f"           2. fleet push <local> {args.device}:<remote>\n"
            f"           3. fleet exec {args.device} -- bash <remote>",
            file=sys.stderr,
        )
    command = shlex.join(cmd_parts) if args.literal else " ".join(cmd_parts)

    if args.stream:
        if args.sudo:
            print("Error: --stream is not compatible with --sudo (sudo path already streams via PTY).", file=sys.stderr)
            sys.exit(2)
        if args.json_output:
            print("Error: --stream is not compatible with --json.", file=sys.stderr)
            sys.exit(2)
        if len(targets) > 1:
            print("Error: --stream requires a single target (parallel streams would interleave).", file=sys.stderr)
            sys.exit(2)

    # ── Detach mode: start background job via nohup ─────────
    if args.detach:
        import secrets, datetime

        if args.stream:
            print("Error: --detach is not compatible with --stream.", file=sys.stderr)
            sys.exit(2)
        if len(targets) > 1:
            print("Error: --detach requires a single target.", file=sys.stderr)
            sys.exit(2)
        if any(is_windows_device(d) for d in targets.values()):
            print("Error: --detach uses nohup/sh and does not work on Windows devices.\n"
                  "       Use: fleet exec <device> -- powershell -Command \"Start-Process ... -WindowStyle Hidden\"", file=sys.stderr)
            sys.exit(2)

        name = list(targets.keys())[0]
        dev = targets[name]
        host = args.host if args.host else dev.get("host", "")
        user = dev.get("user", "")
        password = dev.get("password", "")
        port = dev.get("port", 22)
        job_id = secrets.token_hex(4)
        started_at = datetime.datetime.now().isoformat(timespec="seconds")
        jobs_dir = "/tmp/fleet-jobs"

        # Phase 1: create job directory and metadata (no sudo needed)
        setup_cmd = (
            f"mkdir -p {jobs_dir} && "
            f"cat > {jobs_dir}/{job_id}.json << 'FLEETEOF'\n"
            f'{{"id":"{job_id}","command":{json.dumps(command)},"started_at":"{started_at}","status":"running"}}\n'
            f"FLEETEOF"
        )
        ok, output = ssh_exec(host, user, password, setup_cmd, timeout=10, port=port)
        if not ok:
            print(f"Error: failed to create job metadata: {output}", file=sys.stderr)
            sys.exit(1)

        # Phase 2: launch background job via nohup.
        # For sudo: write a small launcher script first (avoids nested-quote hell),
        # then setsid+exec it inside sudo so it survives PTY close.
        # Password NEVER in command line — fleet's PTY injects the sudo password.
        if args.sudo:
            # Write launcher script (no quoting issues — heredoc-like via cat)
            script_path = f"{jobs_dir}/{job_id}.sh"
            write_script = (
                f"cat > {script_path} << 'JOBEOF'\n"
                f"#!/bin/sh\n"
                f"echo $$ > {jobs_dir}/{job_id}.pid\n"
                f"{{ {command}; }} >> {jobs_dir}/{job_id}.log 2>&1\n"
                f"JOBEOF\n"
                f"chmod +x {script_path}"
            )
            ok, out = ssh_exec(host, user, password, write_script, timeout=5, port=port)
            if not ok:
                print(f"Error: failed to write launcher script: {out}", file=sys.stderr)
                sys.exit(1)
            # Execute via setsid inside sudo: double-fork survives PTY close
            launch_cmd = (
                f"trap '' HUP; "
                f"setsid sh -c 'exec </dev/null; "
                f"nohup {script_path} >> {jobs_dir}/{job_id}.log 2>&1 &'"
            )
        else:
            launch_cmd = (
                f"nohup sh -c {shlex.quote(command)} "
                f">> {jobs_dir}/{job_id}.log 2>&1 & "
                f"echo $! > {jobs_dir}/{job_id}.pid"
            )
        ok, output = ssh_exec(host, user, password, launch_cmd, timeout=15,
                              port=port, sudo=args.sudo)

        if not ok:
            print(f"Error: failed to start detached job: {output}", file=sys.stderr)
            sys.exit(1)

        # Update JSON with PID
        pid_file = f"{jobs_dir}/{job_id}.pid"
        ok2, pid_val = ssh_exec(host, user, password, f"cat {pid_file} 2>/dev/null", timeout=5, port=port)
        if ok2 and pid_val.strip():
            update_cmd = (
                f"python3 -c \"import json; "
                f"d=json.load(open('{jobs_dir}/{job_id}.json')); "
                f"d['pid']={pid_val.strip()}; "
                f"json.dump(d, open('{jobs_dir}/{job_id}.json','w'))\""
            )
            ssh_exec(host, user, password, update_cmd, timeout=5, port=port)

        if args.json_output:
            print(json.dumps({"device": name, "job_id": job_id, "log": f"{jobs_dir}/{job_id}.log"}, indent=2))
        else:
            print(f"Job {job_id} started on {name}")
            print(f"  Log:    {jobs_dir}/{job_id}.log")
            print(f"  Status: fleet jobs {name}")
            print(f"  Tail:   fleet log {name} {job_id}")
            print(f"  Kill:   fleet kill {name} {job_id}")
        return

    # ── Normal (blocking) exec ─────────────────────────────
    results = {}

    def run_on(name, dev):
        host = args.host if args.host else dev.get("host", "")
        user = dev.get("user", "")
        password = dev.get("password", "")
        port = dev.get("port", 22)
        ok, output = ssh_exec(host, user, password, command, timeout=args.timeout, sudo=args.sudo, port=port, stream=args.stream, raw=getattr(args, "raw", False), windows=is_windows_device(dev))
        return name, ok, output

    with ThreadPoolExecutor(max_workers=len(targets) or 1) as pool:
        futures = [pool.submit(run_on, name, dev) for name, dev in targets.items()]
        for future in as_completed(futures):
            name, ok, output = future.result()
            results[name] = {"success": ok, "output": output}

    if args.json_output:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return

    for name in sorted(results):
        r = results[name]
        if len(targets) > 1:
            print(f"=== {name} ===")
        if r["success"]:
            if not args.stream:
                print(r["output"])
        else:
            print(f"Error: {r['output']}", file=sys.stderr)
            # If the device has a gateway, suggest recovery path
            dev = devices.get(name, {})
            gw = dev.get("gateway")
            if gw and gw in devices:
                distro = dev.get("wsl_distro", "")
                distro_flag = f" --distro {distro}" if distro else ""
                print(f"[fleet] Hint: {name} is unreachable but has a gateway → {gw}", file=sys.stderr)
                print(f"  Check WSL state : fleet wsl {name} status", file=sys.stderr)
                print(f"  Restart WSL     : fleet wsl {name} restart", file=sys.stderr)
                print(f"  Run via gateway : fleet exec --raw {gw} -- wsl{distro_flag} -e <cmd>", file=sys.stderr)
        if len(targets) > 1:
            print()
    if any(not r["success"] for r in results.values()):
        sys.exit(1)


def cmd_wsl(args):
    """Manage a WSL2 device via its Windows gateway host."""
    import time
    devices = load_devices()

    target = args.device
    if target not in devices:
        print(f"Error: device '{target}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[target]
    gw_name = dev.get("gateway")
    if not gw_name:
        print(f"Error: device '{target}' has no 'gateway' field in devices.json", file=sys.stderr)
        sys.exit(1)
    if gw_name not in devices:
        print(f"Error: gateway device '{gw_name}' not found in devices.json", file=sys.stderr)
        sys.exit(1)

    gw = devices[gw_name]
    distro = args.distro or dev.get("wsl_distro", "")
    distro_flag = f"-d {distro}" if distro else ""

    gw_host = gw.get("host", "")
    gw_user = gw.get("user", "")
    gw_pass = gw.get("password", "")
    gw_port = gw.get("port", 22)

    action = args.wsl_action

    if action == "status":
        print(f"[fleet] Querying WSL state via {gw_name} ({gw_host})...")
        ok, out = ssh_exec(gw_host, gw_user, gw_pass, "wsl -l -v", port=gw_port, raw=True)
        if not ok:
            print(f"Error connecting to gateway {gw_name}: {out}", file=sys.stderr)
            sys.exit(1)
        print(out)

    elif action == "restart":
        print(f"[fleet] Terminating WSL distro '{distro or 'all'}' via {gw_name}...")
        term_cmd = f"wsl --terminate {distro}" if distro else "wsl --shutdown"
        ok, out = ssh_exec(gw_host, gw_user, gw_pass, term_cmd, port=gw_port, raw=True)
        if not ok:
            print(f"Warning: terminate command returned error (may be normal if already stopped): {out}", file=sys.stderr)
        else:
            print(f"  Terminated. Waiting 3s for WSL to settle...")
            time.sleep(3)

        # Launch WSL so sshd starts
        print(f"[fleet] Starting WSL (launching sshd)...")
        start_cmd = f"wsl {distro_flag} -e bash -c 'sudo service ssh start; echo WSL_STARTED'"
        ok, out = ssh_exec(gw_host, gw_user, gw_pass, start_cmd, port=gw_port, raw=True, timeout=30)
        print(out)

        # Wait for WSL SSH port to accept connections
        wsl_host = dev.get("host", "")
        wsl_port = dev.get("port", 22222)
        print(f"[fleet] Waiting for {target} SSH ({wsl_host}:{wsl_port}) to come up...")
        for attempt in range(12):
            time.sleep(5)
            ok, _ = ssh_exec(wsl_host, dev.get("user", ""), dev.get("password", ""), "echo ok", port=wsl_port, timeout=5)
            if ok:
                print(f"[fleet] {target} is back online after {(attempt+1)*5}s")
                break
        else:
            print(f"[fleet] {target} did not come back online in 60s — check manually", file=sys.stderr)
            print(f"  Manual: fleet exec --raw {gw_name} -- wsl {distro_flag} -e bash -c 'sudo service ssh start'", file=sys.stderr)
            sys.exit(1)

    elif action == "exec":
        cmd_parts = args.cmd_args
        if cmd_parts and cmd_parts[0] == "--":
            cmd_parts = cmd_parts[1:]
        if not cmd_parts:
            print("Error: no command specified. Usage: fleet wsl <device> exec -- <cmd>", file=sys.stderr)
            sys.exit(1)
        inner = shlex.join(cmd_parts)
        wsl_cmd = f"wsl {distro_flag} -e bash -c {shlex.quote(inner)}"
        print(f"[fleet] Running via {gw_name} → WSL: {wsl_cmd}")
        ok, out = ssh_exec(gw_host, gw_user, gw_pass, wsl_cmd, port=gw_port, raw=True, timeout=args.timeout)
        print(out)
        if not ok:
            sys.exit(1)

    else:
        print(f"Error: unknown WSL action '{action}'. Use: status | restart | exec", file=sys.stderr)
        sys.exit(1)


def cmd_jobs(args):
    """List detached jobs on a device."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)
    jobs_dir = "/tmp/fleet-jobs"

    # List all job JSON files, extract status
    cmd = (
        f"python3 -c \""
        f"import json, os, glob; "
        f"jd='{jobs_dir}'; "
        f"[print(json.dumps({{**json.load(open(f)), '_pid_alive': "
        f"'yes' if (d:=json.load(open(f))).get('pid') and os.path.exists('/proc/'+str(d['pid'])) else 'no'}})) "
        f"for f in sorted(glob.glob(jd+'/*.json'))]\" 2>/dev/null"
    )

    ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, port=port)
    if not ok:
        print(f"Error: {output}", file=sys.stderr)
        sys.exit(1)

    jobs = []
    for line in output.strip().split("\n"):
        try:
            jobs.append(json.loads(line))
        except json.JSONDecodeError:
            continue

    if args.json_output:
        print(json.dumps({"device": args.device, "jobs": jobs}, indent=2, ensure_ascii=False))
        return

    if not jobs:
        print(f"No detached jobs on {args.device}.")
        return

    print(f"Jobs on {args.device}:\n")
    rows = []
    for j in jobs:
        pid_alive = j.get("_pid_alive", "no")
        if j.get("status") == "running" and pid_alive == "no":
            status = "stale"
        elif j.get("status") == "running":
            status = "running"
        else:
            status = j.get("status", "?")
        rows.append([
            j.get("id", ""),
            status,
            j.get("started_at", ""),
            j.get("command", "")[:60],
        ])
    format_table(rows, ["JOB ID", "STATUS", "STARTED", "COMMAND"])


def cmd_log(args):
    """Fetch log for a detached job."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    # Validate job_id format to prevent shell injection
    import re
    if not re.match(r'^[a-f0-9]{8}$', args.job_id):
        print(f"Error: invalid job ID format", file=sys.stderr)
        sys.exit(1)

    log_path = f"/tmp/fleet-jobs/{args.job_id}.log"

    if args.follow:
        # tail -f via ssh_exec won't work well (blocking). Print what we have and suggest ssh.
        print(f"Use 'fleet ssh {args.device}' then: tail -f {log_path}")
        return

    tail_n = args.tail or 50
    cmd = f"tail -n {tail_n} {log_path} 2>/dev/null || echo '[fleet] log file not found: {log_path}'"
    ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, port=port)
    print(output)


def cmd_kill(args):
    """Kill a detached job on a device."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    # Validate job_id format to prevent shell injection
    import re
    if not re.match(r'^[a-f0-9]{8}$', args.job_id):
        print(f"Error: invalid job ID format", file=sys.stderr)
        sys.exit(1)

    jobs_dir = "/tmp/fleet-jobs"
    job_id = args.job_id

    signal = "9" if args.force else "15"
    signal_name = "SIGKILL" if args.force else "SIGTERM"

    # Read PID, verify it still belongs to this job (no PID reuse), kill, update status
    cmd = (
        f"pid=$(cat {jobs_dir}/{job_id}.pid 2>/dev/null); "
        f"if [ -z \"$pid\" ]; then echo 'no-pid-file'; exit 0; fi; "
        f"python3 -c \""
        f"import json, os, sys; "
        f"d=json.load(open('{jobs_dir}/{job_id}.json')); "
        f"pid=int(open('{jobs_dir}/{job_id}.pid').read().strip()); "
        f"try: proc_ctime=os.stat('/proc/'+str(pid)).st_ctime; "
        f"except: proc_ctime=0; "
        f"job_start=__import__('datetime').datetime.fromisoformat(d['started_at']).timestamp(); "
        f"if proc_ctime > 0 and proc_ctime < job_start - 10: "
        f"  print('PID_REUSED'); sys.exit(2); "
        f"print('OK')\" 2>/dev/null; "
        f"case $? in "
        f"  2) echo 'pid-reused' ;; "
        f"  0) kill -{signal} $pid 2>/dev/null && echo 'killed' || echo 'not-found' ;; "
        f"  *) kill -{signal} $pid 2>/dev/null && echo 'killed' || echo 'not-found' ;; "
        f"esac; "
        f"python3 -c \"import json; d=json.load(open('{jobs_dir}/{job_id}.json')); "
        f"d['status']='killed'; json.dump(d, open('{jobs_dir}/{job_id}.json','w'))\" 2>/dev/null"
    )

    if args.sudo:
        ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, sudo=True, port=port)
    else:
        ok, output = ssh_exec(host, user, password, cmd, timeout=CMD_TIMEOUT, port=port)

    if not ok:
        print(f"Error: {output}", file=sys.stderr)
        sys.exit(1)

    if "killed" in output:
        print(f"Job {job_id} killed ({signal_name}) on {args.device}.")
    elif "pid-reused" in output:
        print(f"Job {job_id}: PID was reused by another process — refusing to kill. Marked as stale.")
    elif "not-found" in output:
        print(f"Job {job_id}: PID not alive. Marked as killed.")
    else:
        print(f"Job {job_id}: {output}")


def _tar_stream_push(host, user, password, local_path, remote_path, port=22):
    """Push a directory via tar+ssh streaming with inline MD5 verification."""
    import hashlib, subprocess
    local_path = os.path.abspath(local_path)
    print(f"  packing {local_path} ...", file=sys.stderr)

    # Local: tar cz and compute MD5 while streaming
    tar_proc = subprocess.Popen(
        ["tar", "cz", "-C", local_path, "."],
        stdout=subprocess.PIPE
    )

    client = ssh_connect(host, user, password, port=port)
    # Create remote dir and extract, computing MD5 of received stream
    remote_cmd = (
        f"mkdir -p {shlex.quote(remote_path)} && "
        f"tee >(md5sum | cut -d' ' -f1 > /tmp/_fleet_tar_md5) | "
        f"tar xz -C {shlex.quote(remote_path)}"
    )
    stdin, stdout, stderr = client.exec_command(f"bash -c {shlex.quote(remote_cmd)}")

    local_md5 = hashlib.md5()
    total_bytes = 0
    while True:
        chunk = tar_proc.stdout.read(64 * 1024)
        if not chunk:
            break
        local_md5.update(chunk)
        stdin.write(chunk)
        total_bytes += len(chunk)
    stdin.close()

    tar_proc.wait()
    exit_status = stdout.channel.recv_exit_status()
    client_err = stderr.read().decode(errors="replace").strip()

    if tar_proc.returncode != 0:
        client.close()
        print(f"Error: local tar failed (exit {tar_proc.returncode})", file=sys.stderr)
        sys.exit(1)
    if exit_status != 0:
        client.close()
        print(f"Error: remote extract failed: {client_err}", file=sys.stderr)
        sys.exit(1)

    # Get remote MD5
    ok, remote_hash = ssh_exec(host, user, password, "cat /tmp/_fleet_tar_md5 2>/dev/null", port=port)
    remote_hash = remote_hash.strip() if ok and remote_hash else None
    ssh_exec(host, user, password, "rm -f /tmp/_fleet_tar_md5", port=port)
    client.close()

    local_hash = local_md5.hexdigest()
    print(f"  stream: {_human_size(total_bytes)} compressed", file=sys.stderr)
    if not _verify_transfer("push-dir", local_hash, remote_hash):
        sys.exit(1)


def _tar_stream_pull(host, user, password, remote_path, local_path, port=22):
    """Pull a directory via tar+ssh streaming with inline MD5 verification."""
    import hashlib
    os.makedirs(local_path, exist_ok=True)

    client = ssh_connect(host, user, password, port=port)
    # Remote: tar cz and tee to md5sum
    remote_cmd = (
        f"tar cz -C {shlex.quote(remote_path)} . | "
        f"tee >(md5sum | cut -d' ' -f1 > /tmp/_fleet_tar_md5)"
    )
    stdin, stdout, stderr = client.exec_command(f"bash -c {shlex.quote(remote_cmd)}")

    # Local: receive stream, compute MD5, extract
    import subprocess
    tar_proc = subprocess.Popen(
        ["tar", "xz", "-C", local_path],
        stdin=subprocess.PIPE
    )

    local_md5 = hashlib.md5()
    total_bytes = 0
    while True:
        chunk = stdout.read(64 * 1024)
        if not chunk:
            break
        local_md5.update(chunk)
        tar_proc.stdin.write(chunk)
        total_bytes += len(chunk)

    tar_proc.stdin.close()
    tar_proc.wait()
    stdout.channel.recv_exit_status()
    client.close()

    # Get remote MD5
    ok, remote_hash = ssh_exec(host, user, password, "cat /tmp/_fleet_tar_md5 2>/dev/null", port=port)
    remote_hash = remote_hash.strip() if ok and remote_hash else None
    ssh_exec(host, user, password, "rm -f /tmp/_fleet_tar_md5", port=port)

    local_hash = local_md5.hexdigest()
    print(f"  stream: {_human_size(total_bytes)} compressed", file=sys.stderr)
    if not _verify_transfer("pull-dir", local_hash, remote_hash):
        sys.exit(1)


def _sftp_transfer(device_name, local_path, remote_path, direction, host_override=None):
    """Transfer file or directory. Uses tar+ssh streaming for directories, SFTP for single files."""
    devices = load_devices()
    if device_name not in devices:
        print(f"Error: device '{device_name}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[device_name]
    host = host_override if host_override else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    try:
        if direction == "push":
            if os.path.isdir(local_path):
                print(f"{local_path}/ -> {device_name}:{remote_path}/", file=sys.stderr)
                _tar_stream_push(host, user, password, local_path, remote_path, port=port)
                print(f"{local_path}/ -> {device_name}:{remote_path}/")
            else:
                client = ssh_connect(host, user, password, port=port)
                sftp = client.open_sftp()
                sftp.put(local_path, remote_path)
                size = os.path.getsize(local_path)
                print(f"{local_path} -> {device_name}:{remote_path} ({_human_size(size)})")
                local_hash = _local_md5(local_path)
                remote_hash = _remote_md5(host, user, password, remote_path, port=port)
                if not _verify_transfer("push", local_hash, remote_hash):
                    sftp.close()
                    client.close()
                    sys.exit(1)
                sftp.close()
                client.close()
        else:
            # Check if remote path is a directory
            ok, result = ssh_exec(host, user, password,
                                  f"test -d {shlex.quote(remote_path)} && echo DIR || echo FILE", port=port)
            is_dir = result and result.strip() == "DIR"

            if is_dir:
                print(f"{device_name}:{remote_path}/ -> {local_path}/", file=sys.stderr)
                _tar_stream_pull(host, user, password, remote_path, local_path, port=port)
                print(f"{device_name}:{remote_path}/ -> {local_path}/")
            else:
                client = ssh_connect(host, user, password, port=port)
                sftp = client.open_sftp()
                sftp.get(remote_path, local_path)
                size = os.path.getsize(local_path)
                print(f"{device_name}:{remote_path} -> {local_path} ({_human_size(size)})")
                remote_hash = _remote_md5(host, user, password, remote_path, port=port)
                local_hash = _local_md5(local_path)
                if not _verify_transfer("pull", remote_hash, local_hash):
                    sftp.close()
                    client.close()
                    sys.exit(1)
                sftp.close()
                client.close()
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


def _local_md5(filepath):
    """Calculate MD5 of a local file."""
    import hashlib
    h = hashlib.md5()
    with open(filepath, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _remote_md5(host, user, password, filepath, port=22):
    """Calculate MD5 of a remote file via SSH."""
    # Try md5sum (Linux) then md5 (macOS)
    cmd = f"md5sum {shlex.quote(filepath)} 2>/dev/null | awk '{{print $1}}' || md5 -q {shlex.quote(filepath)} 2>/dev/null"
    ok, output = ssh_exec(host, user, password, cmd, port=port)
    if ok and output:
        return output.strip().split()[0]
    return None


def _remote_dir_fingerprint(host, user, password, dirpath, port=22):
    """Return a stable content fingerprint for a remote directory."""
    script = r'''
import hashlib
import os
import sys

root = sys.argv[-1]
if not os.path.isdir(root):
    print("ERROR:not-a-directory", file=sys.stderr)
    sys.exit(2)

digest = hashlib.sha256()
file_count = 0
dir_count = 0
link_count = 0

for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
    for name in list(dirs):
        path = os.path.join(current, name)
        if os.path.islink(path):
            rel = os.path.relpath(path, root).replace(os.sep, "/")
            target = os.readlink(path)
            digest.update(f"L\0{rel}\0{target}\n".encode())
            link_count += 1
            dirs.remove(name)
    dirs.sort()
    files.sort()
    rel_dir = os.path.relpath(current, root)
    rel_dir = "." if rel_dir == "." else rel_dir.replace(os.sep, "/")
    digest.update(f"D\0{rel_dir}\n".encode())
    dir_count += 1

    for name in files:
        path = os.path.join(current, name)
        rel = os.path.relpath(path, root).replace(os.sep, "/")
        st = os.lstat(path)

        if os.path.islink(path):
            target = os.readlink(path)
            digest.update(f"L\0{rel}\0{target}\n".encode())
            link_count += 1
            continue

        if not os.path.isfile(path):
            digest.update(f"O\0{rel}\0{st.st_size}\n".encode())
            continue

        file_hash = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1024 * 1024), b""):
                file_hash.update(chunk)
        digest.update(f"F\0{rel}\0{st.st_size}\0{file_hash.hexdigest()}\n".encode())
        file_count += 1

print(f"{digest.hexdigest()} files={file_count} dirs={dir_count} links={link_count}")
'''
    cmd = f"python3 -c {shlex.quote(script)} -- {shlex.quote(dirpath)}"
    ok, output = ssh_exec(host, user, password, cmd, timeout=3600, port=port)
    if not ok or not output:
        return None
    return output.strip().splitlines()[-1]


def _verify_remote_dirs(label, src_conn, src_path, dst_conn, dst_path):
    """Compare source and destination directory fingerprints."""
    src_host, src_user, src_pwd, src_port = src_conn
    dst_host, dst_user, dst_pwd, dst_port = dst_conn
    src_fp = _remote_dir_fingerprint(src_host, src_user, src_pwd, src_path, port=src_port)
    dst_fp = _remote_dir_fingerprint(dst_host, dst_user, dst_pwd, dst_path, port=dst_port)
    if src_fp is None or dst_fp is None:
        print(f"  verify: FAILED! could not fingerprint directories", file=sys.stderr)
        return False
    if src_fp == dst_fp:
        print(f"  verify: OK ({src_fp})", file=sys.stderr)
        return True
    print(f"  verify: FAILED! src={src_fp} dst={dst_fp}", file=sys.stderr)
    return False


def _verify_transfer(label, expected_md5, actual_md5):
    """Compare MD5 hashes and report result."""
    if actual_md5 is None:
        print(f"  verify: SKIP (md5 unavailable on remote)", file=sys.stderr)
        return True
    if expected_md5 == actual_md5:
        print(f"  verify: OK (md5: {expected_md5})", file=sys.stderr)
        return True
    else:
        print(f"  verify: FAILED! src={expected_md5} dst={actual_md5}", file=sys.stderr)
        return False


def _human_size(nbytes):
    """Convert bytes to human-readable size."""
    for unit in ("B", "KB", "MB", "GB"):
        if nbytes < 1024:
            return f"{nbytes:.1f}{unit}" if unit != "B" else f"{nbytes}{unit}"
        nbytes /= 1024
    return f"{nbytes:.1f}TB"


# --- work-* commands for remote development workflow ---

EXCLUDE_ALWAYS = [
    ".git/",
    "node_modules/", ".venv/", "venv/", "__pycache__/",
    "*.pyc", ".cache/", "dist/", "build/", ".tox/", ".nox/",
    ".pytest_cache/", "*.egg-info/", ".mypy_cache/", ".ruff_cache/",
    "target/", ".gradle/", ".DS_Store", "*.log", "*.tmp",
]


def cmd_work_sync(args):
    """Sync project directory with remote device using rsync."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    # Build rsync exclude arguments
    exclude_args = []
    for item in EXCLUDE_ALWAYS:
        exclude_args.extend(["--exclude", item])

    # Read .gitignore if exists and add to excludes
    gitignore_path = os.path.join(args.local, ".gitignore")
    if os.path.exists(gitignore_path):
        with open(gitignore_path) as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    exclude_args.extend(["--exclude", line])

    # Determine direction
    if args.push:
        src = args.local
        dst = f"{user}@{host}:{args.remote}"
        direction = "push"
    elif args.pull:
        src = f"{user}@{host}:{args.remote}"
        dst = args.local
        direction = "pull"
    else:
        print("Error: must specify --push or --pull", file=sys.stderr)
        sys.exit(1)

    # Build rsync command
    rsync_cmd = [
        "rsync",
        "-az",  # archive + compress
        "--info=progress2",
        "--stats",
    ]
    if port != 22:
        rsync_cmd.extend(["-e", f"ssh -p {port}"])
    rsync_cmd.extend(exclude_args)

    if args.dry_run:
        rsync_cmd.append("--dry-run")
        rsync_cmd.append("--itemize-changes")

    rsync_cmd.append(src.rstrip("/") + "/")
    rsync_cmd.append(dst.rstrip("/") + "/")

    # Use sshpass for password-based auth if available
    if password and shutil.which("sshpass"):
        rsync_cmd = ["sshpass", "-p", password] + rsync_cmd

    # Run rsync
    try:
        result = subprocess.run(rsync_cmd, capture_output=False, text=True)
        if result.returncode != 0:
            print(f"Error: rsync failed with code {result.returncode}", file=sys.stderr)
            sys.exit(result.returncode)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


def cmd_push(args):
    _sftp_transfer(args.device, args.local, args.remote, "push", args.host)


def cmd_pull(args):
    _sftp_transfer(args.device, args.local, args.remote, "pull", args.host)


def _transfer_direct(src_name, src, src_path, src_port,
                     dst_name, dst, dst_path, dst_port,
                     dest_host_override=None):
    """Direct A->B transfer: source SSH's to dest using sshpass, data flows over their network.

    Control machine only orchestrates — file bytes do NOT traverse control's connection.
    Requires `sshpass` on source and dest reachable from source.
    """
    src_host = src["host"]
    src_user = src["user"]
    src_pwd = src.get("password", "")
    dst_host = dest_host_override or dst["host"]
    dst_user = dst["user"]
    dst_pwd = dst.get("password", "")

    # Preflight: sshpass installed on source?
    ok, out = ssh_exec(src_host, src_user, src_pwd,
                       "command -v sshpass >/dev/null && echo OK || echo MISSING",
                       port=src_port)
    if not ok or "OK" not in out:
        print(f"Error: sshpass not found on {src_name}. Direct transfer requires it.\n"
              f"       Quick fix: add --relay to route through control machine instead.\n"
              f"       Or install sshpass: fleet exec --sudo {src_name} -- apt install -y sshpass",
              file=sys.stderr)
        sys.exit(1)

    # Detect dir vs file on source
    ok, out = ssh_exec(src_host, src_user, src_pwd,
                       f"test -d {shlex.quote(src_path)} && echo DIR || echo FILE",
                       port=src_port)
    is_dir = ok and out.strip() == "DIR"

    # Build the inner ssh command run on source. Password fed via SSHPASS env
    # (transient — visible in /proc/<pid>/environ on source only while running).
    ssh_to_dst = (
        f"sshpass -e ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "
        f"-o LogLevel=ERROR -p {dst_port} "
        f"{shlex.quote(f'{dst_user}@{dst_host}')}"
    )

    if is_dir:
        print(f"Transferring {src_name}:{src_path}/ -> {dst_name}:{dst_path}/ (direct, tar stream over their network)",
              file=sys.stderr)
        remote_dst_cmd = f"mkdir -p {shlex.quote(dst_path)} && tar xz -C {shlex.quote(dst_path)}"
        pipeline = (
            f"set -o pipefail; "
            f"export SSHPASS={shlex.quote(dst_pwd)}; "
            f"tar cz -C {shlex.quote(src_path)} . | "
            f"tee >(md5sum | cut -d' ' -f1 > /tmp/_fleet_tar_md5_src) | "
            f"{ssh_to_dst} {shlex.quote(remote_dst_cmd)}"
        )
    else:
        print(f"Transferring {src_name}:{src_path} -> {dst_name}:{dst_path} (direct)",
              file=sys.stderr)
        # Use cat → ssh pipe; verify via md5 on each side after.
        remote_dst_cmd = f"cat > {shlex.quote(dst_path)}"
        pipeline = (
            f"export SSHPASS={shlex.quote(dst_pwd)}; "
            f"cat {shlex.quote(src_path)} | {ssh_to_dst} {shlex.quote(remote_dst_cmd)}"
        )

    ok, out = ssh_exec(src_host, src_user, src_pwd,
                       f"bash -c {shlex.quote(pipeline)}",
                       timeout=3600, port=src_port)
    if not ok:
        print(f"Error: direct transfer failed: {out}", file=sys.stderr)
        sys.exit(1)

    # Verify
    print("  verifying...", file=sys.stderr)
    if is_dir:
        ok, src_hash = ssh_exec(src_host, src_user, src_pwd,
                                "cat /tmp/_fleet_tar_md5_src 2>/dev/null", port=src_port)
        ssh_exec(src_host, src_user, src_pwd, "rm -f /tmp/_fleet_tar_md5_src", port=src_port)
        src_hash = (src_hash or "").strip() if ok else ""
        if src_hash:
            print(f"  source tar md5: {src_hash}", file=sys.stderr)
        if not _verify_remote_dirs(
            "transfer-direct-dir",
            (src_host, src_user, src_pwd, src_port),
            src_path,
            (dst_host, dst_user, dst_pwd, dst_port),
            dst_path,
        ):
            sys.exit(1)
        print(f"{src_name}:{src_path}/ -> {dst_name}:{dst_path}/ (direct)")
    else:
        src_hash = _remote_md5(src_host, src_user, src_pwd, src_path, port=src_port)
        dst_hash = _remote_md5(dst_host, dst_user, dst_pwd, dst_path, port=dst_port)
        if not _verify_transfer("transfer-direct", src_hash, dst_hash):
            sys.exit(1)
        print(f"{src_name}:{src_path} -> {dst_name}:{dst_path} (direct)")


def cmd_transfer(args):
    """Transfer file or directory between two remote devices. Defaults to direct LAN transfer."""
    import hashlib
    devices = load_devices()

    # Parse source and dest: "device:/path"
    if ":" not in args.source or ":" not in args.dest:
        print("Error: use format device:/path for both source and dest", file=sys.stderr)
        sys.exit(1)

    src_dev, src_path = args.source.split(":", 1)
    dst_dev, dst_path = args.dest.split(":", 1)

    for name in (src_dev, dst_dev):
        if name not in devices:
            print(f"Error: device '{name}' not found", file=sys.stderr)
            sys.exit(1)

    src = devices[src_dev]
    dst = devices[dst_dev]
    src_port = src.get("port", 22)
    dst_port = dst.get("port", 22)

    if not getattr(args, "relay", False):
        _transfer_direct(src_dev, src, src_path, src_port,
                         dst_dev, dst, dst_path, dst_port,
                         dest_host_override=getattr(args, "dest_host", None))
        return

    try:
        # Check if source is a directory
        ok, result = ssh_exec(src["host"], src["user"], src.get("password", ""),
                              f"test -d {shlex.quote(src_path)} && echo DIR || echo FILE", port=src_port)
        is_dir = result and result.strip() == "DIR"

        if is_dir:
            # Directory transfer via tar+ssh streaming through local
            print(f"Transferring {src_dev}:{src_path}/ -> {dst_dev}:{dst_path}/ (tar stream)",
                  file=sys.stderr)

            src_client = ssh_connect(src["host"], src["user"], src.get("password", ""), port=src_port)
            dst_client = ssh_connect(dst["host"], dst["user"], dst.get("password", ""), port=dst_port)

            # Source: tar cz, tee to md5sum
            src_cmd = (
                "set -o pipefail; "
                f"tar cz -C {shlex.quote(src_path)} . | "
                f"tee >(md5sum | cut -d' ' -f1 > /tmp/_fleet_tar_md5)"
            )
            src_stdin, src_stdout, src_stderr = src_client.exec_command(f"bash -c {shlex.quote(src_cmd)}")

            # Dest: mkdir + extract
            dst_cmd = (
                f"mkdir -p {shlex.quote(dst_path)} && "
                f"tar xz -C {shlex.quote(dst_path)}"
            )
            dst_stdin, dst_stdout, dst_stderr = dst_client.exec_command(f"bash -c {shlex.quote(dst_cmd)}")

            # Stream src -> local (md5) -> dst
            local_md5 = hashlib.md5()
            total_bytes = 0
            while True:
                chunk = src_stdout.read(64 * 1024)
                if not chunk:
                    break
                local_md5.update(chunk)
                dst_stdin.write(chunk)
                total_bytes += len(chunk)
            dst_stdin.close()

            src_exit = src_stdout.channel.recv_exit_status()
            dst_exit = dst_stdout.channel.recv_exit_status()

            if src_exit != 0:
                err = src_stderr.read().decode(errors="replace").strip()
                print(f"Error: source archive failed: {err}", file=sys.stderr)
                src_client.close()
                dst_client.close()
                sys.exit(1)
            if dst_exit != 0:
                err = dst_stderr.read().decode(errors="replace").strip()
                print(f"Error: remote extract failed: {err}", file=sys.stderr)
                src_client.close()
                dst_client.close()
                sys.exit(1)

            # Verify: compare src tar MD5 with local MD5
            ok, src_hash = ssh_exec(src["host"], src["user"], src.get("password", ""),
                                    "cat /tmp/_fleet_tar_md5 2>/dev/null", port=src_port)
            src_hash = src_hash.strip() if ok and src_hash else None
            ssh_exec(src["host"], src["user"], src.get("password", ""), "rm -f /tmp/_fleet_tar_md5", port=src_port)

            local_hash = local_md5.hexdigest()
            print(f"  stream: {_human_size(total_bytes)} compressed", file=sys.stderr)
            if not _verify_transfer("transfer-dir", local_hash, src_hash):
                src_client.close()
                dst_client.close()
                sys.exit(1)
            if not _verify_remote_dirs(
                "transfer-dir",
                (src["host"], src["user"], src.get("password", ""), src_port),
                src_path,
                (dst["host"], dst["user"], dst.get("password", ""), dst_port),
                dst_path,
            ):
                src_client.close()
                dst_client.close()
                sys.exit(1)

            print(f"{src_dev}:{src_path}/ -> {dst_dev}:{dst_path}/ ({_human_size(total_bytes)} compressed)")
            src_client.close()
            dst_client.close()
        else:
            # Single file transfer via temp file (existing logic)
            print(f"Connecting to {src_dev}...", file=sys.stderr)
            src_client = ssh_connect(src["host"], src["user"], src.get("password", ""), port=src_port)
            src_sftp = src_client.open_sftp()

            print(f"Connecting to {dst_dev}...", file=sys.stderr)
            dst_client = ssh_connect(dst["host"], dst["user"], dst.get("password", ""), port=dst_port)
            dst_sftp = dst_client.open_sftp()

            file_stat = src_sftp.stat(src_path)
            total_size = file_stat.st_size
            print(f"Transferring {src_dev}:{src_path} -> {dst_dev}:{dst_path} ({_human_size(total_size)})",
                  file=sys.stderr)

            import tempfile
            with tempfile.NamedTemporaryFile(delete=True, suffix=os.path.basename(src_path)) as tmp:
                tmp_path = tmp.name

            try:
                def pull_progress(transferred, total):
                    pct = transferred * 100 // total if total else 100
                    print(f"\r  pull: {_human_size(transferred)} / {_human_size(total)} ({pct}%)",
                          end="", file=sys.stderr)
                src_sftp.get(src_path, tmp_path, callback=pull_progress)
                print(file=sys.stderr)

                def push_progress(transferred, total):
                    pct = transferred * 100 // total if total else 100
                    print(f"\r  push: {_human_size(transferred)} / {_human_size(total)} ({pct}%)",
                          end="", file=sys.stderr)
                try:
                    dst_sftp.put(tmp_path, dst_path, callback=push_progress, confirm=True)
                except OSError:
                    dst_sftp.put(tmp_path, dst_path, callback=push_progress, confirm=False)
                print(file=sys.stderr)
            finally:
                if os.path.exists(tmp_path):
                    os.unlink(tmp_path)

            print(f"  verifying...", file=sys.stderr)
            src_hash = _remote_md5(src["host"], src["user"], src.get("password", ""), src_path, port=src_port)
            dst_hash = _remote_md5(dst["host"], dst["user"], dst.get("password", ""), dst_path, port=dst_port)
            if not _verify_transfer("transfer", src_hash, dst_hash):
                src_sftp.close()
                src_client.close()
                dst_sftp.close()
                dst_client.close()
                sys.exit(1)

            print(f"{src_dev}:{src_path} -> {dst_dev}:{dst_path} ({_human_size(total_size)})")
            src_sftp.close()
            src_client.close()
            dst_sftp.close()
            dst_client.close()

    except Exception as e:
        print(f"\nError: {e}", file=sys.stderr)
        sys.exit(1)


def cmd_work_enter(args):
    """SSH into device, cd to directory, start tmux session with claude CLI."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    session = args.session or f"claude-{os.path.basename(args.remote_dir)}"

    # Build the remote command: cd + tmux new/attach + claude
    # First check if session exists
    check_session = f"tmux has-session -t {session} 2>/dev/null && echo EXISTS || echo NEW"
    ok, output = ssh_exec(host, user, password, check_session, port=port)

    if "EXISTS" in output:
        # Attach to existing session
        remote_cmd = f"tmux attach -t {session}"
    else:
        # Create new session with claude
        remote_cmd = f"cd {args.remote_dir} && tmux new-session -s {session} claude"

    # Build SSH command
    port_args = ["-p", str(port)] if port != 22 else []
    if shutil.which("sshpass") and password:
        cmd = ["sshpass", "-p", password, "ssh",
               "-o", "StrictHostKeyChecking=no",
               "-o", "UserKnownHostsFile=/dev/null",
               *port_args,
               "-t",  # Force PTY allocation
               f"{user}@{host}",
               remote_cmd]
    else:
        if not shutil.which("sshpass") and password:
            print("Tip: install sshpass for auto-login; password is configured but hidden.", file=sys.stderr)
        cmd = ["ssh",
               "-o", "StrictHostKeyChecking=no",
               "-o", "UserKnownHostsFile=/dev/null",
               *port_args,
               "-t",
               f"{user}@{host}",
               remote_cmd]

    print(f"Entering {session} on {args.device}...")
    os.execvp(cmd[0], cmd)


def cmd_work_monitor(args):
    """Monitor tmux session and execute command on detach (runs on remote device)."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    # Create the monitor script to run on remote
    monitor_script = f'''#!/bin/bash
SESSION="{args.session}"
ON_EXIT="{args.on_exit}"
while tmux has-session -t "$SESSION" 2>/dev/null; do sleep 2; done
echo "Session $SESSION detached, executing: $ON_EXIT"
eval "$ON_EXIT"
'''

    # Run the monitor script in background on the remote
    # Use a temp file on the remote
    script_path = f"/tmp/monitor-{args.session}.sh"
    ok, output = ssh_exec(host, user, password, f"cat > {script_path} << 'MONITOR_EOF'\n{monitor_script}\nMONITOR_EOF && chmod +x {script_path} && nohup {script_path} > /tmp/monitor-{args.session}.log 2>&1 &", port=port)

    if ok:
        print(f"Monitor started for session '{args.session}' on {args.device}")
        print(f"On exit will run: {args.on_exit}")
    else:
        print(f"Error: {output}", file=sys.stderr)
        sys.exit(1)


def cmd_ssh(args):
    """SSH into a device, using sshpass if available."""
    devices = load_devices()
    if args.device not in devices:
        print(f"Error: device '{args.device}' not found", file=sys.stderr)
        sys.exit(1)

    dev = devices[args.device]
    host = args.host if args.host else dev.get("host", "")
    user = dev.get("user", "")
    password = dev.get("password", "")
    port = dev.get("port", 22)

    port_args = ["-p", str(port)] if port != 22 else []
    if shutil.which("sshpass") and password:
        cmd = ["sshpass", "-p", password, "ssh",
               "-o", "StrictHostKeyChecking=no",
               "-o", "UserKnownHostsFile=/dev/null",
               *port_args,
               f"{user}@{host}"]
    else:
        if not shutil.which("sshpass") and password:
            print(f"Tip: install sshpass for auto-login (brew install esolitos/ipa/sshpass)", file=sys.stderr)
            print("Password is configured but hidden.", file=sys.stderr)
            print(file=sys.stderr)
        cmd = ["ssh",
               "-o", "StrictHostKeyChecking=no",
               "-o", "UserKnownHostsFile=/dev/null",
               *port_args,
               f"{user}@{host}"]

    os.execvp(cmd[0], cmd)


def main():
    parser = argparse.ArgumentParser(prog="fleet", description="Local cluster management CLI")
    sub = parser.add_subparsers(dest="command")

    # list
    p_list = sub.add_parser("list", help="List devices")
    p_list.add_argument("--tag", action="append", default=[], help="Filter by tag (can repeat)")
    p_list.add_argument("--owner", choices=["personal", "company"], help="Filter by owner")
    p_list.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # status
    p_status = sub.add_parser("status", help="Query device status")
    p_status.add_argument("device", nargs="?", help="Specific device name")
    p_status.add_argument("--tag", action="append", default=[], help="Filter by tag")
    p_status.add_argument("--owner", choices=["personal", "company"], help="Filter by owner")
    p_status.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # match
    p_match = sub.add_parser("match", help="Find matching online devices")
    p_match.add_argument("--tag", action="append", default=[], required=True, help="Filter by tag")
    p_match.add_argument("--owner", choices=["personal", "company"], help="Filter by owner")
    p_match.add_argument("--sort", choices=["disk", "memory", "cpu"], help="Sort by resource")
    p_match.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # exec: fleet exec <device> -- <command...>
    # All flags must come before device name
    p_exec = sub.add_parser("exec", help="Execute command: fleet exec [flags] <device> -- <cmd>")
    p_exec.add_argument("--tag", action="append", default=[], help="Run on all devices with tag")
    p_exec.add_argument("--sudo", action="store_true", help="Run with sudo (auto-injects password)")
    p_exec.add_argument("--timeout", type=int, default=60, help="Command timeout seconds (default: 60). Bump for long tasks: 300s for apt install, 1800s for docker pull, 3600s for builds.")
    p_exec.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_exec.add_argument("--json", dest="json_output", action="store_true", help="JSON output")
    p_exec.add_argument("--literal", action="store_true", help="Preserve quoting via shlex.join (use for python -c, heredoc, complex strings). Disables remote shell interpretation of pipes/vars/globs — wrap with bash -c '...' if you need those.")
    p_exec.add_argument("--stream", action="store_true", help="Stream stdout/stderr live as they arrive (for long builds). Single target only; not compatible with --sudo or --json.")
    p_exec.add_argument("--detach", action="store_true", help="Run in background via nohup, return job ID immediately. Use 'fleet jobs/log/kill' to manage.")
    p_exec.add_argument("--raw", action="store_true", help="Send the command verbatim with no bash wrapper. Required for Windows (cmd.exe/PowerShell) devices, which choke on the POSIX 'export ...;' prefix.")
    p_exec.add_argument("device", help="Device name")
    p_exec.add_argument("cmd_args", nargs=argparse.REMAINDER, help="Command (after --)")

    # add
    p_add = sub.add_parser("add", help="Add a new device")
    p_add.add_argument("name", help="Device name (e.g. seeed-j40)")
    p_add.add_argument("host", help="IP or hostname")
    p_add.add_argument("--user", "-u", default="root", help="SSH user (default: root)")
    p_add.add_argument("--password", "-p", help="SSH password")
    p_add.add_argument("--owner", choices=["personal", "company"], default="company", help="Owner (default: company)")
    p_add.add_argument("--tag", action="append", default=[], help="Tags (can repeat)")
    p_add.add_argument("--desc", default="", help="Description")
    p_add.add_argument("--scan", action="store_true", help="Auto-detect specs after adding")

    # remove
    p_remove = sub.add_parser("remove", help="Remove a device")
    p_remove.add_argument("name", help="Device name to remove")
    p_remove.add_argument("--force", action="store_true", help="Skip confirmation")

    # scan
    p_scan = sub.add_parser("scan", help="Auto-detect device specs and update devices.json")
    p_scan.add_argument("device", nargs="?", help="Specific device name")
    p_scan.add_argument("--tag", action="append", default=[], help="Filter by tag")
    p_scan.add_argument("--dry-run", action="store_true", help="Show changes without saving")
    p_scan.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # push
    p_push = sub.add_parser("push", help="Upload file to device")
    p_push.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_push.add_argument("device", help="Device name")
    p_push.add_argument("local", help="Local file/directory path")
    p_push.add_argument("remote", help="Remote destination path")

    # pull
    p_pull = sub.add_parser("pull", help="Download file from device")
    p_pull.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_pull.add_argument("device", help="Device name")
    p_pull.add_argument("remote", help="Remote file path")
    p_pull.add_argument("local", help="Local destination path")

    # transfer
    p_transfer = sub.add_parser("transfer", help="Transfer file between two remote devices")
    p_transfer.add_argument("source", help="Source: device:/path")
    p_transfer.add_argument("dest", help="Destination: device:/path")
    p_transfer.add_argument("--relay", action="store_true",
                            help="Route data through control machine instead of direct A->B LAN transfer (default: direct).")
    p_transfer.add_argument("--dest-host", help="Override dest host as seen from source (e.g. LAN IP). Use when devices.json stores public/Tailscale addr but source can reach dest via LAN.")

    # ssh
    p_ssh = sub.add_parser("ssh", help="SSH into a device")
    p_ssh.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_ssh.add_argument("device", help="Device name")

    # docker
    p_docker = sub.add_parser("docker", help="Docker container status")
    p_docker.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_docker.add_argument("device", help="Device name")
    p_docker.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # bootstrap: fleet bootstrap [--all | <device>] [--profile <name>] [--check]
    p_bootstrap = sub.add_parser("bootstrap", help="Configure mirrors/proxy on device")
    p_bootstrap.add_argument("device", nargs="?", help="Device name (use with --all for all devices)")
    p_bootstrap.add_argument("--all", dest="all_devices", action="store_true", help="Run on all devices")
    p_bootstrap.add_argument("--tag", action="append", default=[], help="Filter by tag")
    p_bootstrap.add_argument("--profile", choices=["wsl2-proxy", "edge-mirror", "isolated"], help="Force profile (default: auto-detect)")
    p_bootstrap.add_argument("--check", action="store_true", help="Check-only mode (dry-run, report status)")
    p_bootstrap.add_argument("--force", dest="force_bootstrap", action="store_true", help="Apply bootstrap even on devices with direct internet access")
    p_bootstrap.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # jobs: fleet jobs <device>
    p_jobs = sub.add_parser("jobs", help="List detached background jobs on device")
    p_jobs.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_jobs.add_argument("device", help="Device name")
    p_jobs.add_argument("--json", dest="json_output", action="store_true", help="JSON output")

    # log: fleet log <device> <job-id> [--tail N] [--follow]
    p_log = sub.add_parser("log", help="Fetch log for a detached job")
    p_log.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_log.add_argument("device", help="Device name")
    p_log.add_argument("job_id", help="Job ID")
    p_log.add_argument("--tail", type=int, default=50, help="Number of lines (default: 50)")
    p_log.add_argument("--follow", "-f", action="store_true", help="Follow mode (prints current tail and suggests SSH)")

    # kill: fleet kill <device> <job-id> [--force]
    p_kill = sub.add_parser("kill-job", help="Kill a detached background job on device")
    p_kill.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_kill.add_argument("--sudo", action="store_true", help="Kill with sudo (if job was started with --sudo)")
    p_kill.add_argument("device", help="Device name")
    p_kill.add_argument("job_id", help="Job ID")
    p_kill.add_argument("--force", "-9", action="store_true", help="SIGKILL instead of SIGTERM")

    # work-sync: fleet work-sync <device> <local> <remote> [--push|--pull] [--dry-run]
    p_work_sync = sub.add_parser("work-sync", help="Sync project directory with remote (rsync)")
    p_work_sync.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_work_sync.add_argument("device", help="Device name")
    p_work_sync.add_argument("local", help="Local directory path")
    p_work_sync.add_argument("remote", help="Remote directory path")
    p_work_sync.add_argument("--push", action="store_true", help="Push local to remote")
    p_work_sync.add_argument("--pull", action="store_true", help="Pull remote to local")
    p_work_sync.add_argument("--dry-run", action="store_true", help="Show what would be transferred")

    # work-enter: fleet work-enter <device> <remote-dir> [--session <name>]
    p_work_enter = sub.add_parser("work-enter", help="SSH into device and start tmux+claude session")
    p_work_enter.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_work_enter.add_argument("device", help="Device name")
    p_work_enter.add_argument("remote_dir", help="Remote directory to work in")
    p_work_enter.add_argument("--session", help="Tmux session name (default: claude-<basename>)")

    # work-monitor: fleet work-monitor <device> <session> --on-exit "<command>"
    p_work_monitor = sub.add_parser("work-monitor", help="Monitor tmux session and run command on detach")
    p_work_monitor.add_argument("--host", help="Override device IP/hostname (temporary)")
    p_work_monitor.add_argument("device", help="Device name")
    p_work_monitor.add_argument("session", help="Tmux session name to monitor")
    p_work_monitor.add_argument("--on-exit", required=True, help="Command to run when session detaches")

    # wsl: fleet wsl <device> status|restart|exec [-- <cmd>]
    p_wsl = sub.add_parser("wsl", help="Manage a WSL2 device via its Windows gateway")
    p_wsl.add_argument("device", help="WSL2 device name (must have 'gateway' set in devices.json)")
    p_wsl.add_argument("wsl_action", choices=["status", "restart", "exec"],
                       help="status: show WSL state; restart: terminate+relaunch WSL and wait for SSH; exec: run command inside WSL via gateway")
    p_wsl.add_argument("--distro", help="WSL distro name (overrides wsl_distro from devices.json)")
    p_wsl.add_argument("--timeout", type=int, default=60, help="Command timeout for exec action (seconds)")
    p_wsl.add_argument("cmd_args", nargs=argparse.REMAINDER, help="Command for exec action (after --)")

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        sys.exit(1)

    if args.command == "list":
        cmd_list(args)
    elif args.command == "add":
        cmd_add(args)
    elif args.command == "remove":
        cmd_remove(args)
    elif args.command == "status":
        cmd_status(args)
    elif args.command == "match":
        cmd_match(args)
    elif args.command == "exec":
        cmd_exec(args)
    elif args.command == "scan":
        cmd_scan(args)
    elif args.command == "push":
        cmd_push(args)
    elif args.command == "pull":
        cmd_pull(args)
    elif args.command == "transfer":
        cmd_transfer(args)
    elif args.command == "ssh":
        cmd_ssh(args)
    elif args.command == "docker":
        cmd_docker(args)
    elif args.command == "bootstrap":
        cmd_bootstrap(args)
    elif args.command == "jobs":
        cmd_jobs(args)
    elif args.command == "log":
        cmd_log(args)
    elif args.command == "kill-job":
        cmd_kill(args)
    elif args.command == "work-sync":
        cmd_work_sync(args)
    elif args.command == "work-enter":
        cmd_work_enter(args)
    elif args.command == "work-monitor":
        cmd_work_monitor(args)
    elif args.command == "wsl":
        cmd_wsl(args)


if __name__ == "__main__":
    main()
