use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths;

#[derive(Debug)]
pub struct SessionLock {
    path: PathBuf,
}

impl SessionLock {
    pub fn acquire(session_id: &str, host: &str) -> Result<Self, String> {
        let path = paths::lock_path(session_id, host);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create lock dir: {err}"))?;
        }

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                writeln!(file, "pid={}", std::process::id())
                    .map_err(|err| format!("failed to write lock: {err}"))?;
                writeln!(file, "started_at={now}")
                    .map_err(|err| format!("failed to write lock: {err}"))?;
                Ok(Self { path })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(pid) = read_lock_pid(&path) {
                    if process_alive(pid) {
                        return Err(format!(
                            "session is locked by pid {pid}: {}\n\
                             Use a different RPTY_SESSION for another Agent, or remove the stale lock if that process is gone.",
                            path.display()
                        ));
                    }
                }
                fs::remove_file(&path)
                    .map_err(|err| format!("failed to remove stale lock: {err}"))?;
                Self::acquire(session_id, host)
            }
            Err(err) => Err(format!("failed to create lock {}: {err}", path.display())),
        }
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn read_lock_pid(path: &PathBuf) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(pid) = line.strip_prefix("pid=") {
            return pid.parse::<u32>().ok();
        }
    }
    None
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
        || std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

#[cfg(not(unix))]
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let pid_text = pid.to_string();
    std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
        .map(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            output.status.success()
                && text.lines().any(|line| {
                    line.split(',').nth(1).map(|value| value.trim_matches('"'))
                        == Some(pid_text.as_str())
                })
        })
        .unwrap_or(false)
}

#[cfg(all(not(unix), not(windows)))]
fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::SessionLock;

    #[test]
    fn lock_excludes_same_session_and_host() {
        let home = std::env::temp_dir().join(format!("rpty-lock-test-{}", std::process::id()));
        std::env::set_var("RPTY_HOME", &home);
        let session = format!("test-{}", std::process::id());
        let first = SessionLock::acquire(&session, "radxa").unwrap();
        let second = SessionLock::acquire(&session, "radxa").unwrap_err();
        assert!(second.contains("locked"));
        drop(first);
        SessionLock::acquire(&session, "radxa").unwrap();
        let _ = std::fs::remove_dir_all(home);
    }
}
