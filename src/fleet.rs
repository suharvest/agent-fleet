use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use crate::config::Config;
use crate::fleet_native::NativeFleet;

#[derive(Debug, Clone)]
pub struct FleetCommand {
    program: String,
    prefix_args: Vec<String>,
    native: Option<NativeFleet>,
}

#[derive(Debug, Clone)]
pub struct Captured {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl FleetCommand {
    pub fn discover() -> Self {
        if let Ok(path) = std::env::var("RPTY_FLEET_PY") {
            return Self::from_fleet_py(PathBuf::from(path), hub_dir(), native_from_env());
        }

        let config = Config::load();
        if let Some(path) = config.fleet_py {
            return Self::from_fleet_py(
                path,
                config.fleet_hub.unwrap_or_else(hub_dir),
                native_from_env(),
            );
        }

        if let Some(hub) = config.fleet_hub {
            return Self::from_fleet_py(hub.join("fleet.py"), hub.clone(), native_from_hub(&hub));
        }

        if let Some(hub) = bundled_hub_dir() {
            return Self::from_fleet_py(hub.join("fleet.py"), hub.clone(), native_from_hub(&hub));
        }

        let hub = hub_dir();
        Self::from_fleet_py(hub.join("fleet.py"), hub.clone(), native_from_hub(&hub))
    }

    pub fn describe(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.prefix_args.clone());
        if let Some(native) = &self.native {
            native.describe()
        } else {
            parts.join(" ")
        }
    }

    fn from_fleet_py(fleet_py: PathBuf, hub: PathBuf, native: Option<NativeFleet>) -> Self {
        Self {
            program: "uv".to_string(),
            prefix_args: vec![
                "run".to_string(),
                "--project".to_string(),
                hub.display().to_string(),
                "python".to_string(),
                fleet_py.display().to_string(),
            ],
            native,
        }
    }

    pub fn passthrough<I, S>(&self, args: I) -> Result<ExitCode, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if let Some(native) = &self.native {
            if let Some(code) = native.passthrough(&args)? {
                return Ok(code);
            }
        }
        let status = self
            .command()
            .args(args.iter())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|err| format!("failed to run fleet: {err}"))?;

        Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
    }

    pub fn capture<I, S>(&self, args: I) -> Result<Captured, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if let Some(native) = &self.native {
            if let Some(captured) = native.capture(&args)? {
                return Ok(captured);
            }
        }
        let output = self
            .command()
            .args(args.iter())
            .output()
            .map_err(|err| format!("failed to run fleet: {err}"))?;

        Ok(Captured {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            code: output.status.code().unwrap_or(1),
        })
    }

    pub fn exec_capture(
        &self,
        device: &str,
        timeout: u64,
        literal: bool,
        remote_args: &[String],
    ) -> Result<Captured, String> {
        let mut args = vec![
            "exec".to_string(),
            "--timeout".to_string(),
            timeout.to_string(),
        ];
        if literal {
            args.push("--literal".to_string());
        }
        args.push(device.to_string());
        args.push("--".to_string());
        args.extend(remote_args.iter().cloned());
        self.capture(args)
    }

    pub fn push(&self, device: &str, local: &str, remote: &str) -> Result<Captured, String> {
        self.capture([
            "push".to_string(),
            device.to_string(),
            local.to_string(),
            remote.to_string(),
        ])
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix_args);
        command
    }
}

fn native_from_env() -> Option<NativeFleet> {
    if native_disabled() {
        return None;
    }
    std::env::var("FLEET_DEVICES_FILE")
        .or_else(|_| std::env::var("RPTY_FLEET_DEVICES"))
        .ok()
        .map(PathBuf::from)
        .map(NativeFleet::new)
}

fn native_from_hub(hub: &std::path::Path) -> Option<NativeFleet> {
    if native_disabled() {
        return None;
    }
    native_from_env().or_else(|| Some(NativeFleet::new(hub.join("devices.json"))))
}

fn native_disabled() -> bool {
    std::env::var("RPTY_FLEET_NATIVE")
        .map(|value| matches!(value.as_str(), "0" | "false" | "off"))
        .unwrap_or(false)
}

pub fn is_passthrough_command(command: &str) -> bool {
    matches!(
        command,
        "list"
            | "status"
            | "match"
            | "exec"
            | "push"
            | "pull"
            | "transfer"
            | "ssh"
            | "docker"
            | "bootstrap"
            | "jobs"
            | "log"
            | "kill-job"
            | "work-sync"
            | "work-enter"
            | "work-monitor"
            | "wsl"
            | "scan"
            | "add"
            | "remove"
    )
}

fn hub_dir() -> PathBuf {
    if let Ok(path) = std::env::var("RPTY_FLEET_HUB") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("project").join("_hub")
}

fn bundled_hub_dir() -> Option<PathBuf> {
    let installed = crate::paths::rpty_home().join("bin").join("fleet_backend");
    if installed.join("fleet.py").exists() || installed.join("devices.example.json").exists() {
        return Some(installed);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sibling = parent.join("fleet_backend");
            if sibling.join("fleet.py").exists() {
                return Some(sibling);
            }
        }
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fleet_backend");
    if manifest.join("fleet.py").exists() {
        return Some(manifest);
    }

    None
}
