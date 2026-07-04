use std::env;
use std::path::PathBuf;

pub fn rpty_home() -> PathBuf {
    if let Some(path) = env::var_os("RPTY_HOME") {
        return PathBuf::from(path);
    }

    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".rpty")
}

pub fn state_dir() -> PathBuf {
    rpty_home().join("state")
}

pub fn config_path() -> PathBuf {
    rpty_home().join("config.toml")
}

pub fn current_host_path() -> PathBuf {
    state_dir().join("current_host")
}

pub fn session_dir(session_id: &str) -> PathBuf {
    state_dir().join("sessions").join(session_id)
}

pub fn session_current_host_path(session_id: &str) -> PathBuf {
    session_dir(session_id).join("current_host")
}

pub fn raw_log_path(session_id: &str, host: &str) -> PathBuf {
    session_dir(session_id)
        .join("logs")
        .join(format!("{host}.raw.log"))
}

pub fn lock_path(session_id: &str, host: &str) -> PathBuf {
    session_dir(session_id)
        .join("locks")
        .join(format!("{host}.lock"))
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::raw_log_path;

    #[test]
    fn raw_log_path_is_host_scoped() {
        let path = raw_log_path("default", "radxa");
        assert!(path.ends_with("state/sessions/default/logs/radxa.raw.log"));
    }
}
