use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::fleet::FleetCommand;
use crate::paths;
use crate::protocol::{parse_marked_output, ExitCodeState, ParsedOutput, ParserState};

#[derive(Debug, Clone)]
pub struct Router {
    fleet: FleetCommand,
    session_id: String,
    timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct RouterRun {
    pub parsed: ParsedOutput,
    pub raw: String,
    pub raw_log: PathBuf,
}

impl Router {
    pub fn new() -> Self {
        Self {
            fleet: FleetCommand::discover(),
            session_id: std::env::var("RPTY_SESSION").unwrap_or_else(|_| "default".to_string()),
            timeout: Duration::from_secs(600),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn tmux_session(&self, device: &str) -> String {
        format!(
            "rpty-{}-{}",
            sanitize_name(&self.session_id),
            sanitize_name(device)
        )
    }

    pub fn ensure_session(&self, device: &str) -> Result<String, String> {
        let session = self.tmux_session(device);
        let script = format!(
            "mkdir -p /tmp/rpty-router; \
             if ! tmux has-session -t {session} 2>/dev/null; then \
               tmux new-session -d -s {session} \"bash -lc '[ -f \\\"$HOME/.profile.d/mirrors.sh\\\" ] && . \\\"$HOME/.profile.d/mirrors.sh\\\"; exec bash -i'\"; \
             fi"
        );
        self.exec_shell(device, &script, 60)
            .map_err(|err| pty_prerequisite_error(device, &err))?;
        Ok(session)
    }

    pub fn run_command(&self, device: &str, command: &str) -> Result<RouterRun, String> {
        let session = self.ensure_session(device)?;
        let nonce = nonce();
        let payload = command_payload(command, &nonce);
        let local_path = write_temp_payload(&nonce, &payload)?;
        let remote_path = format!("/tmp/rpty-router/{session}-{nonce}.cmd");

        let push = self
            .fleet
            .push(device, path_str(&local_path)?, &remote_path)?;
        let _ = fs::remove_file(&local_path);
        if !push.success {
            return Err(format!(
                "failed to push command payload: {}{}",
                push.stderr, push.stdout
            ));
        }

        self.exec_tmux(device, &["send-keys", "-t", &session, "C-l"], 60)?;
        self.exec_tmux(device, &["clear-history", "-t", &session], 60)?;
        self.exec_tmux(device, &["load-buffer", "-b", &nonce, &remote_path], 60)?;
        self.exec_tmux(
            device,
            &["paste-buffer", "-d", "-b", &nonce, "-t", &session],
            60,
        )?;

        let raw = self.wait_for_marker(device, &session, &nonce)?;
        let raw_log = append_raw_log(&self.session_id, device, &raw)?;
        let parsed = parse_marked_output(&raw, &nonce);
        Ok(RouterRun {
            parsed,
            raw,
            raw_log,
        })
    }

    pub fn capture(&self, device: &str) -> Result<String, String> {
        let session = self.ensure_session(device)?;
        self.capture_session(device, &session)
    }

    pub fn environment_summary(&self, device: &str) -> Result<String, String> {
        let session = self.tmux_session(device);
        let command = r#"printf 'session=%s\n' "__RPTY_SESSION_ID__"
printf 'device=%s\n' "__RPTY_DEVICE__"
printf 'hostname=%s\n' "$(hostname 2>/dev/null || uname -n)"
printf 'pwd=%s\n' "$PWD"
printf 'shell=%s\n' "$SHELL"
printf 'user=%s\n' "$(id -un 2>/dev/null || whoami)"
printf 'venv=%s\n' "${VIRTUAL_ENV:-}"
printf 'python=%s\n' "$(command -v python 2>/dev/null || command -v python3 2>/dev/null || true)"
printf 'tmux=%s\n' "$TMUX"
printf 'tmux_session=%s\n' "__RPTY_TMUX_SESSION__"
"#;
        let replaced = command
            .replace("__RPTY_SESSION_ID__", &self.session_id)
            .replace("__RPTY_DEVICE__", device)
            .replace("__RPTY_TMUX_SESSION__", &session);
        let run = self.run_command(device, &format!("(\n{replaced})"))?;
        Ok(filter_env_summary(&run.parsed.visible_output))
    }

    pub fn attach(&self, device: &str) -> Result<std::process::ExitCode, String> {
        let session = self.ensure_session(device)?;
        self.fleet.passthrough([
            "work-enter".to_string(),
            device.to_string(),
            "~".to_string(),
            "--session".to_string(),
            session,
        ])
    }

    pub fn doctor_device(&self, device: &str, fix: bool) -> Result<(), String> {
        println!("Checking {device}...");
        let status = self.fleet.capture([
            "status".to_string(),
            device.to_string(),
            "--json".to_string(),
        ])?;
        print!("{}", status.stdout);
        if !status.stderr.trim().is_empty() {
            eprint!("{}", status.stderr);
        }
        let tmux = self.fleet.exec_capture(
            device,
            60,
            true,
            &[
                "sh".to_string(),
                "-lc".to_string(),
                "command -v tmux && tmux -V".to_string(),
            ],
        )?;
        if tmux.success {
            print!("{}", tmux.stdout);
        } else {
            eprint!("{}", tmux.stderr);
            print!("{}", tmux.stdout);
            if fix {
                self.install_tmux(device)?;
                let verify = self.fleet.exec_capture(
                    device,
                    60,
                    true,
                    &[
                        "sh".to_string(),
                        "-lc".to_string(),
                        "command -v tmux && tmux -V".to_string(),
                    ],
                )?;
                if verify.success {
                    print!("{}", verify.stdout);
                    return Ok(());
                }
                return Err(format!(
                    "{device}: tmux install was attempted but verification failed: {}{}",
                    verify.stderr, verify.stdout
                ));
            }
            return Err(format!(
                "{device}: tmux is missing or unavailable; run `rpty doctor --fix {device}` or install tmux with Fleet"
            ));
        }
        Ok(())
    }

    pub fn cleanup(&self, device: &str) -> Result<(), String> {
        let session = self.tmux_session(device);
        let script = format!(
            "tmux kill-session -t {session} 2>/dev/null || true; \
             rm -f /tmp/rpty-router/{session}-*.cmd"
        );
        self.exec_shell(device, &script, 60)?;
        Ok(())
    }

    fn exec_shell(&self, device: &str, script: &str, timeout: u64) -> Result<String, String> {
        let captured = self.fleet.exec_capture(
            device,
            timeout,
            true,
            &["sh".to_string(), "-lc".to_string(), script.to_string()],
        )?;
        if captured.success {
            Ok(captured.stdout)
        } else {
            Err(format!(
                "fleet exec failed ({}): {}{}",
                captured.code, captured.stderr, captured.stdout
            ))
        }
    }

    fn install_tmux(&self, device: &str) -> Result<(), String> {
        println!("Installing tmux on {device} with Fleet sudo exec...");
        let captured = self.fleet.capture([
            "exec".to_string(),
            "--sudo".to_string(),
            "--timeout".to_string(),
            "600".to_string(),
            "--literal".to_string(),
            device.to_string(),
            "--".to_string(),
            "sh".to_string(),
            "-lc".to_string(),
            "if command -v apt-get >/dev/null 2>&1; then apt-get update && apt-get install -y tmux; elif command -v yum >/dev/null 2>&1; then yum install -y tmux; elif command -v dnf >/dev/null 2>&1; then dnf install -y tmux; else echo 'no supported package manager found' >&2; exit 127; fi".to_string(),
        ])?;
        print!("{}", captured.stdout);
        if !captured.stderr.trim().is_empty() {
            eprint!("{}", captured.stderr);
        }
        if captured.success {
            Ok(())
        } else {
            Err(format!(
                "failed to install tmux on {device}: {}{}",
                captured.stderr, captured.stdout
            ))
        }
    }

    fn exec_tmux(&self, device: &str, args: &[&str], timeout: u64) -> Result<String, String> {
        let mut remote = vec!["tmux".to_string()];
        remote.extend(args.iter().map(|arg| (*arg).to_string()));
        let captured = self.fleet.exec_capture(device, timeout, false, &remote)?;
        if captured.success {
            Ok(captured.stdout)
        } else {
            Err(format!(
                "tmux command failed ({}): {}{}",
                captured.code, captured.stderr, captured.stdout
            ))
        }
    }

    fn capture_session(&self, device: &str, session: &str) -> Result<String, String> {
        self.exec_tmux(
            device,
            &["capture-pane", "-p", "-S", "-2000", "-t", session],
            60,
        )
    }

    fn wait_for_marker(&self, device: &str, session: &str, nonce: &str) -> Result<String, String> {
        let deadline = SystemTime::now()
            .checked_add(self.timeout)
            .unwrap_or_else(SystemTime::now);
        let marker = crate::protocol::exit_marker(nonce);

        loop {
            let last = self.capture_session(device, session)?;
            if last.contains(&marker) {
                return Ok(last);
            }
            if SystemTime::now() >= deadline {
                let raw_log = append_raw_log(&self.session_id, device, &last)?;
                return Err(format!(
                    "command timed out after {}s; remote tmux session is still alive; raw log: {}",
                    self.timeout.as_secs(),
                    raw_log.display()
                ));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
}

pub fn command_payload(command: &str, nonce: &str) -> String {
    let mut payload = String::new();
    payload.push_str(command.trim_end());
    payload.push('\n');
    payload.push_str(&format!("printf '\\n__RPTY_EXIT__:{nonce}:%s\\n' \"$?\"\n"));
    payload
}

pub fn sanitize_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

pub fn print_run(run: &RouterRun) -> i32 {
    print!("{}", run.parsed.visible_output);
    if run.parsed.parser != ParserState::Ok {
        eprintln!(
            "rpty: parser status {:?}; raw log: {}",
            run.parsed.parser,
            run.raw_log.display()
        );
    }
    match run.parsed.exit_code {
        ExitCodeState::Code(code) => code,
        ExitCodeState::TimedOut => 124,
        ExitCodeState::Interrupted => 130,
        ExitCodeState::Unknown => 1,
    }
}

fn write_temp_payload(nonce: &str, payload: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("rpty-{nonce}.cmd"));
    fs::write(&path, payload).map_err(|err| format!("failed to write temp payload: {err}"))?;
    Ok(path)
}

fn append_raw_log(session_id: &str, device: &str, raw: &str) -> Result<PathBuf, String> {
    let path = paths::raw_log_path(session_id, device);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("failed to create log dir: {err}"))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open raw log: {err}"))?;
    writeln!(file, "\n===== rpty capture =====\n{raw}")
        .map_err(|err| format!("failed to append raw log: {err}"))?;
    Ok(path)
}

fn path_str(path: &PathBuf) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "temp path is not valid UTF-8".to_string())
}

fn nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn filter_env_summary(output: &str) -> String {
    const KEYS: &[&str] = &[
        "session=",
        "device=",
        "hostname=",
        "pwd=",
        "shell=",
        "user=",
        "venv=",
        "python=",
        "tmux=",
        "tmux_session=",
    ];
    let mut clean = String::new();
    for line in output.lines() {
        if KEYS.iter().any(|key| line.starts_with(key)) {
            clean.push_str(line);
            clean.push('\n');
        }
    }
    clean
}

fn pty_prerequisite_error(device: &str, err: &str) -> String {
    format!(
        "{device}: PTY mode requires a Unix-like remote shell with `sh`, `bash`, and `tmux`.\n\
         Native Windows SSH targets should use `fleet exec {device} -- powershell ...` for stateless commands, \
         or a WSL Fleet device such as `wsl2-local` for persistent PTY sessions.\n\
         Bootstrap error: {err}"
    )
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{command_payload, sanitize_name};

    #[test]
    fn sanitizes_tmux_session_names() {
        assert_eq!(sanitize_name("wsl2-local"), "wsl2-local");
        assert_eq!(sanitize_name("bad/name:x"), "bad_name_x");
        assert_eq!(sanitize_name(""), "default");
    }

    #[test]
    fn payload_preserves_command_and_adds_marker() {
        let payload = command_payload("echo '$HOME'; false", "abc");
        assert!(payload.starts_with("echo '$HOME'; false\n"));
        assert!(payload.contains("__RPTY_EXIT__:abc"));
        assert!(payload.ends_with("\"$?\"\n"));
    }

    #[test]
    fn filters_environment_summary() {
        let output = "prompt$ printf\nhostname=rock-5t\nprompt$ pwd\npwd=/tmp\n";
        assert_eq!(
            super::filter_env_summary(output),
            "hostname=rock-5t\npwd=/tmp\n"
        );
    }
}
