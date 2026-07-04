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
