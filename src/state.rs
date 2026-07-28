use std::fs;
use std::io;

use crate::paths;

pub fn current_host() -> io::Result<Option<String>> {
    let session_id = current_session_id();
    current_host_for_session(&session_id)
}

pub fn current_host_for_session(session_id: &str) -> io::Result<Option<String>> {
    if session_id != "default" {
        if let Some(host) = read_host(paths::session_current_host_path(session_id))? {
            return Ok(Some(host));
        }
    }
    read_host(paths::current_host_path())
}

/// Host used by the transparent `bash -c` shim.
///
/// Deliberately narrower than [`current_host`]: it never falls back to the
/// global `current_host` file. The shim intercepts every `bash -c` that runs
/// with `~/.rpty/bin` on PATH, including invocations nobody aimed at Fleet
/// (git hooks, pre-commit, npm scripts, `child_process`). Routing those to a
/// remote host silently breaks them, so interception has to be armed
/// explicitly by the agent session that wants it, via `rpty use` inside a
/// session that has `RPTY_SESSION` set.
pub fn shim_host() -> io::Result<Option<String>> {
    shim_host_for_session(&current_session_id())
}

pub fn shim_host_for_session(session_id: &str) -> io::Result<Option<String>> {
    if session_id == "default" {
        return Ok(None);
    }
    read_host(paths::session_current_host_path(session_id))
}

pub fn clear_current_host() -> io::Result<bool> {
    let session_id = current_session_id();
    let path = if session_id == "default" {
        paths::current_host_path()
    } else {
        paths::session_current_host_path(&session_id)
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

pub fn set_current_host(host: &str) -> io::Result<()> {
    let session_id = current_session_id();
    set_current_host_for_session(&session_id, host)
}

pub fn set_current_host_for_session(session_id: &str, host: &str) -> io::Result<()> {
    let path = if session_id == "default" && std::env::var_os("RPTY_SESSION").is_none() {
        paths::current_host_path()
    } else {
        paths::session_current_host_path(session_id)
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{host}\n"))
}

fn current_session_id() -> String {
    std::env::var("RPTY_SESSION").unwrap_or_else(|_| "default".to_string())
}

fn read_host(path: std::path::PathBuf) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bash shim must never inherit the global current_host: it sits on
    /// PATH for every process, so an unrelated `bash -c` (git hook,
    /// pre-commit, build script) would be silently shipped to a remote host.
    #[test]
    fn shim_host_ignores_global_current_host() {
        let home = std::env::temp_dir().join(format!("rpty-shim-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("state")).expect("create state dir");
        fs::write(home.join("state").join("current_host"), "spark\n").expect("write global host");

        // A session that never ran `use` stays local, even with a global host.
        assert_eq!(
            read_host(home.join("state/sessions/agent-x/current_host")).unwrap(),
            None
        );
        assert_eq!(shim_host_for_session("default").unwrap(), None);

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn shim_host_reads_session_scoped_host() {
        let home = std::env::temp_dir().join(format!("rpty-shim-scoped-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let session = home.join("state/sessions/agent-x");
        fs::create_dir_all(&session).expect("create session dir");
        fs::write(session.join("current_host"), "spark\n").expect("write session host");

        assert_eq!(
            read_host(session.join("current_host")).unwrap(),
            Some("spark".to_string())
        );

        let _ = fs::remove_dir_all(&home);
    }
}
