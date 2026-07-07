use std::ffi::OsString;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::fleet::{is_passthrough_command, FleetCommand};
use crate::lock::SessionLock;
use crate::router::{print_run, Router};
use crate::state;
use crate::{config::Config, paths};

const KNOWN_AGENT_COMMANDS: &[&str] = &["codex", "claude", "opencode"];

pub fn run<I>(args: I) -> Result<ExitCode, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    match args.as_slice() {
        [] => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        [flag] if flag == "-h" || flag == "--help" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        [cmd] if cmd == "version" => {
            println!("rpty {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        [cmd, rest @ ..] if cmd == "doctor" => doctor(rest),
        [cmd, rest @ ..] if cmd == "config" => config_command(rest),
        [cmd] if cmd == "shell" => shell(),
        [cmd] if cmd == "hosts" => FleetCommand::discover()
            .passthrough(["list", "--json"])
            .map_err(|err| err.to_string()),
        [cmd] if cmd == "where" => where_current(),
        [cmd, host] if cmd == "use" => use_host(host),
        [cmd, rest @ ..] if cmd == "run" => run_command(rest),
        [cmd, rest @ ..] if cmd == "env" => env_summary(rest),
        [cmd, rest @ ..] if cmd == "capture" || cmd == "logs" => capture(rest),
        [cmd, rest @ ..] if cmd == "attach" => attach(rest),
        [cmd, rest @ ..] if cmd == "agent" => agent(rest),
        [cmd, rest @ ..] if cmd == "cleanup" => cleanup(rest),
        [cmd, rest @ ..] if cmd == "install" => install(rest),
        [cmd, rest @ ..] if cmd == "install-shim" => install_shim(rest),
        [cmd, rest @ ..] if cmd == "install-agent-shim" => install_agent_shim(rest),
        [cmd, rest @ ..] if cmd == "fleet" => FleetCommand::discover()
            .passthrough(rest.iter())
            .map_err(|err| err.to_string()),
        [cmd, rest @ ..] if is_passthrough_command(cmd) => {
            let mut pass_args = vec![cmd.clone()];
            pass_args.extend(rest.iter().cloned());
            FleetCommand::discover()
                .passthrough(pass_args)
                .map_err(|err| err.to_string())
        }
        [cmd, ..] => Err(format!("unknown or unimplemented command: {cmd}")),
    }
}

pub fn run_os(args: impl IntoIterator<Item = OsString>) -> Result<ExitCode, String> {
    let converted = args
        .into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    run(converted)
}

fn print_help() {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_owned()))
        .and_then(|name| name.to_str().map(|value| value.to_string()))
        .unwrap_or_else(|| "rpty".to_string());
    println!(
        "\
{bin} - unified Fleet + remote PTY router

Usage:
  {bin} list --json                 # Fleet-compatible passthrough
  {bin} status --json               # Fleet-compatible passthrough
  {bin} run --host <device> -- <cmd>
  {bin} env [device]
  {bin} use <device>
  {bin} where
  {bin} shell
  {bin} agent <cmd> [args...]
  {bin} install [dir]
  {bin} install-shim [dir]
  {bin} install-agent-shim <agent> [dir]
  {bin} cleanup [--all] [device]
  {bin} attach [device]
  {bin} doctor [--fix] [--write-shell-profile] [device]
  {bin} config [--fleet-py <path>] [--fleet-hub <path>] [--agent <name>]
  {bin} version

Existing Fleet commands are passed through. New shell/agent/run modes use the
persistent PTY router."
    );
}

pub fn run_bash_shim<I>(args: I) -> Result<ExitCode, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let command = match args.as_slice() {
        [flag, command, ..] if flag == "-c" || flag == "-lc" => Some(command.clone()),
        [login, flag, command, ..] if login == "-l" && flag == "-c" => Some(command.clone()),
        _ => None,
    };

    if std::env::var_os("RPTY_BASH_PASSTHROUGH").is_some() {
        return run_system_bash(&args);
    }

    if let Some(command) = command {
        let router = Router::new();
        let host = state::current_host()
            .map_err(|err| format!("failed to read current host: {err}"))?
            .ok_or_else(|| "bash shim has no current host; run `rpty use <device>`".to_string())?;
        let _lock = SessionLock::acquire(router.session_id(), &host)?;
        let run = router.run_command(&host, &command)?;
        let code = print_run(&run);
        return Ok(ExitCode::from(code.clamp(0, 255) as u8));
    }

    run_system_bash(&args)
}

fn run_system_bash(args: &[String]) -> Result<ExitCode, String> {
    let status = std::process::Command::new("/bin/bash")
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to run /bin/bash: {err}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn use_host(host: &str) -> Result<ExitCode, String> {
    state::set_current_host(host).map_err(|err| format!("failed to save current host: {err}"))?;
    println!("Current host: {host}");
    Ok(ExitCode::SUCCESS)
}

fn where_current() -> Result<ExitCode, String> {
    match state::current_host().map_err(|err| format!("failed to read current host: {err}"))? {
        Some(host) => println!("Current host: {host}"),
        None => println!("Current host: <unset>"),
    }
    Ok(ExitCode::SUCCESS)
}

fn config_command(args: &[String]) -> Result<ExitCode, String> {
    let mut config = Config::load();
    if args.is_empty() {
        print_config(&config);
        return Ok(ExitCode::SUCCESS);
    }

    let mut changed = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--fleet-py" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--fleet-py requires a path".to_string())?;
                config.fleet_py = Some(PathBuf::from(path));
                changed = true;
            }
            "--fleet-hub" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--fleet-hub requires a path".to_string())?;
                config.fleet_hub = Some(PathBuf::from(path));
                changed = true;
            }
            "--agent" => {
                index += 1;
                let agent = args
                    .get(index)
                    .ok_or_else(|| "--agent requires a command name".to_string())?;
                validate_agent_name(agent)?;
                if !config.agents.iter().any(|existing| existing == agent) {
                    config.agents.push(agent.clone());
                }
                changed = true;
            }
            "--clear-fleet-py" => {
                config.fleet_py = None;
                changed = true;
            }
            "--clear-fleet-hub" => {
                config.fleet_hub = None;
                changed = true;
            }
            other => return Err(format!("unknown config option: {other}")),
        }
        index += 1;
    }

    if changed {
        config
            .save()
            .map_err(|err| format!("failed to write {}: {err}", paths::config_path().display()))?;
    }
    print_config(&config);
    Ok(ExitCode::SUCCESS)
}

fn print_config(config: &Config) {
    println!("Config: {}", paths::config_path().display());
    println!(
        "fleet_py: {}",
        config
            .fleet_py
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<default>".to_string())
    );
    println!(
        "fleet_hub: {}",
        config
            .fleet_hub
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<default>".to_string())
    );
    println!("agents: {}", known_agent_commands().join(", "));
}

fn run_command(args: &[String]) -> Result<ExitCode, String> {
    let (host, timeout, command) = parse_run_args(args)?;
    let router = Router::new().with_timeout(Duration::from_secs(timeout));
    let _lock = SessionLock::acquire(router.session_id(), &host)?;
    let run = router.run_command(&host, &command)?;
    let code = print_run(&run);
    Ok(ExitCode::from(code.clamp(0, 255) as u8))
}

fn env_summary(args: &[String]) -> Result<ExitCode, String> {
    let host = host_from_optional_arg(args)?;
    let router = Router::new();
    let _lock = SessionLock::acquire(router.session_id(), &host)?;
    let output = router.environment_summary(&host)?;
    print!("{output}");
    Ok(ExitCode::SUCCESS)
}

fn parse_run_args(args: &[String]) -> Result<(String, u64, String), String> {
    let mut host = None;
    let mut timeout = 600;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" | "-H" => {
                index += 1;
                host = args.get(index).cloned();
                if host.is_none() {
                    return Err("--host requires a device".to_string());
                }
            }
            "--timeout" | "-t" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--timeout requires seconds".to_string())?;
                timeout = value
                    .parse::<u64>()
                    .map_err(|_| "--timeout must be an integer".to_string())?;
            }
            "--" => {
                index += 1;
                break;
            }
            _ => break,
        }
        index += 1;
    }

    let host = match host {
        Some(host) => host,
        None => state::current_host()
            .map_err(|err| format!("failed to read current host: {err}"))?
            .ok_or_else(|| "no host set; use --host <device> or rpty use <device>".to_string())?,
    };
    let command = args[index..].join(" ");
    if command.trim().is_empty() {
        return Err("no command specified; use: rpty run --host <device> -- <cmd>".to_string());
    }
    Ok((host, timeout, command))
}

fn capture(args: &[String]) -> Result<ExitCode, String> {
    let host = host_from_optional_arg(args)?;
    let router = Router::new();
    let output = router.capture(&host)?;
    print!("{output}");
    Ok(ExitCode::SUCCESS)
}

fn attach(args: &[String]) -> Result<ExitCode, String> {
    let host = host_from_optional_arg(args)?;
    Router::new().attach(&host)
}

fn agent(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return Err("usage: rpty agent <cmd> [args...]".to_string());
    }
    let session = match std::env::var("RPTY_SESSION") {
        Ok(session) => session,
        Err(_) => {
            let inherited_host = state::current_host()
                .map_err(|err| format!("failed to read current host: {err}"))?;
            let session = generated_agent_session();
            if let Some(host) = inherited_host {
                state::set_current_host_for_session(&session, &host)
                    .map_err(|err| format!("failed to initialize agent host: {err}"))?;
            }
            session
        }
    };
    eprintln!("rpty: RPTY_SESSION={session}");
    let status = std::process::Command::new(&args[0])
        .args(&args[1..])
        .env("RPTY_SESSION", &session)
        .env("RPTY_BIN", current_exe_string()?)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to launch agent command '{}': {err}", args[0]))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn generated_agent_session() -> String {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_owned()))
        .and_then(|name| name.to_str().map(|value| value.to_string()))
        .unwrap_or_else(|| "workspace".to_string());
    format!(
        "agent-{}-{}-{}",
        sanitize_session_fragment(&cwd),
        std::process::id(),
        unix_seconds()
    )
}

fn sanitize_session_fragment(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn install_shim(args: &[String]) -> Result<ExitCode, String> {
    let dir = if let Some(dir) = args.first() {
        std::path::PathBuf::from(dir)
    } else {
        crate::paths::rpty_home().join("bin")
    };
    let runtime = install_runtime_binary(&dir)?;
    install_link(&dir, "bash", &runtime)?;
    println!(
        "Installed bash shim: {}",
        command_shim_path(&dir, "bash").display()
    );
    println!("Add this to PATH before launching an Agent:");
    println!("  export PATH=\"{}:$PATH\"", dir.display());
    Ok(ExitCode::SUCCESS)
}

fn install(args: &[String]) -> Result<ExitCode, String> {
    let dir = if let Some(dir) = args.first() {
        std::path::PathBuf::from(dir)
    } else {
        crate::paths::rpty_home().join("bin")
    };
    let runtime = install_runtime_binary(&dir)?;
    install_link(&dir, "rpty", &runtime)?;
    install_link(&dir, "fleet", &runtime)?;
    install_link(&dir, "bash", &runtime)?;

    println!("Installed:");
    println!("  {}", runtime.display());
    println!("  {}", command_shim_path(&dir, "rpty").display());
    println!("  {}", command_shim_path(&dir, "fleet").display());
    println!("  {}", command_shim_path(&dir, "bash").display());
    println!("Add this to PATH before launching an Agent:");
    println!("  export PATH=\"{}:$PATH\"", dir.display());
    Ok(ExitCode::SUCCESS)
}

fn install_agent_shim(args: &[String]) -> Result<ExitCode, String> {
    let agent = args
        .first()
        .ok_or_else(|| "usage: rpty install-agent-shim <agent> [dir]".to_string())?;
    validate_agent_name(agent)?;
    let dir = if let Some(dir) = args.get(1) {
        std::path::PathBuf::from(dir)
    } else {
        crate::paths::rpty_home().join("bin")
    };
    install_runtime_binary(&dir)?;
    install_agent_wrapper(&dir, agent)?;
    println!(
        "Installed Agent shim: {}",
        command_shim_path(&dir, agent).display()
    );
    println!("Put this directory before the real Agent command in PATH:");
    println!("  export PATH=\"{}:$PATH\"", dir.display());
    println!("Then launch directly:");
    println!("  {agent}");
    Ok(ExitCode::SUCCESS)
}

fn install_runtime_binary(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("failed to create install dir {}: {err}", dir.display()))?;
    let exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable: {err}"))?;
    let runtime = dir.join(runtime_binary_name());
    let same_file = std::fs::canonicalize(&exe)
        .ok()
        .zip(std::fs::canonicalize(&runtime).ok())
        .is_some_and(|(current, installed)| current == installed);
    if !same_file {
        std::fs::copy(&exe, &runtime).map_err(|err| {
            format!(
                "failed to install runtime binary {}: {err}",
                runtime.display()
            )
        })?;
    }
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&runtime)
            .map_err(|err| format!("failed to inspect {}: {err}", runtime.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&runtime, perms)
            .map_err(|err| format!("failed to chmod {}: {err}", runtime.display()))?;
    }
    Ok(runtime)
}

fn install_agent_wrapper(dir: &std::path::Path, agent: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("failed to create shim dir {}: {err}", dir.display()))?;
    let shim = command_shim_path(dir, agent);
    if let Ok(existing) = std::fs::read_to_string(&shim) {
        if !is_agentfleet_agent_shim(&existing) {
            return Err(format!(
                "{} already exists and is not a Fleet agent shim; refusing to overwrite",
                shim.display()
            ));
        }
    } else if let Ok(meta) = std::fs::symlink_metadata(&shim) {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&shim)
                .map_err(|err| format!("failed to replace existing shim: {err}"))?;
        } else {
            return Err(format!(
                "{} already exists and is not readable; refusing to overwrite",
                shim.display()
            ));
        }
    }

    let script = agent_wrapper_script(agent);
    std::fs::write(&shim, script)
        .map_err(|err| format!("failed to write {} shim: {err}", shim.display()))?;
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&shim)
            .map_err(|err| format!("failed to inspect {}: {err}", shim.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&shim, perms)
            .map_err(|err| format!("failed to chmod {}: {err}", shim.display()))?;
    }
    Ok(())
}

fn runtime_binary_name() -> &'static str {
    if cfg!(windows) {
        "fleet-router.exe"
    } else {
        "fleet-router"
    }
}

fn command_shim_path(dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        dir.join(format!("{name}.cmd"))
    } else {
        dir.join(name)
    }
}

#[cfg(unix)]
fn agent_wrapper_script(agent: &str) -> String {
    format!(
        r#"#!/bin/sh
# AgentFleet agent shim
set -eu

self_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
old_ifs=$IFS
IFS=:
path_without_self=
for entry in $PATH; do
  if [ "$entry" = "$self_dir" ]; then
    continue
  fi
  if [ -z "$path_without_self" ]; then
    path_without_self=$entry
  else
    path_without_self=$path_without_self:$entry
  fi
done
IFS=$old_ifs

real_agent=$(PATH="$path_without_self" command -v {agent} || true)
if [ -z "$real_agent" ]; then
  echo "fleet router: real {agent} command not found after removing $self_dir from PATH" >&2
  exit 127
fi

if [ -z "${{RPTY_SESSION:-}}" ]; then
  workspace=$(basename "${{PWD:-workspace}}" | tr -c 'A-Za-z0-9_-' '_')
  export RPTY_SESSION=agent-${{workspace}}-$$-$(date +%s)
fi

export RPTY_BIN=$self_dir/fleet
export PATH=$self_dir:$path_without_self
exec "$real_agent" "$@"
"#
    )
}

#[cfg(windows)]
fn agent_wrapper_script(agent: &str) -> String {
    format!(
        r#"@echo off
rem AgentFleet agent shim
setlocal EnableExtensions
set "SELF_DIR=%~dp0"
set "SELF_DIR=%SELF_DIR:~0,-1%"
set "PATH_WITHOUT_SELF="

for %%I in ("%PATH:;=" "%") do (
  if /I not "%%~I"=="%SELF_DIR%" (
    if defined PATH_WITHOUT_SELF (
      set "PATH_WITHOUT_SELF=%PATH_WITHOUT_SELF%;%%~I"
    ) else (
      set "PATH_WITHOUT_SELF=%%~I"
    )
  )
)

for %%I in ({agent}.exe {agent}.cmd {agent}.bat {agent}) do (
  for /f "usebackq delims=" %%P in (`cmd /d /c "set PATH=%PATH_WITHOUT_SELF%&& where %%I 2>nul"`) do (
    set "REAL_AGENT=%%P"
    goto :found
  )
)

echo fleet router: real {agent} command not found after removing %SELF_DIR% from PATH 1>&2
exit /b 127

:found
if not defined RPTY_SESSION (
  set "RPTY_SESSION=agent-%RANDOM%-%RANDOM%"
)
set "RPTY_BIN=%SELF_DIR%\fleet.cmd"
set "PATH=%SELF_DIR%;%PATH_WITHOUT_SELF%"
endlocal & set "RPTY_SESSION=%RPTY_SESSION%" & set "RPTY_BIN=%RPTY_BIN%" & set "PATH=%PATH%" & call "%REAL_AGENT%" %*
"#
    )
}

fn validate_agent_name(agent: &str) -> Result<(), String> {
    if agent.is_empty()
        || !agent
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("agent name must contain only ASCII letters, digits, '-' or '_'".to_string());
    }
    Ok(())
}

fn install_link(dir: &std::path::Path, name: &str, exe: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("failed to create shim dir {}: {err}", dir.display()))?;
    let shim = command_shim_path(dir, name);

    if cfg!(windows) {
        if let Ok(existing) = std::fs::read_to_string(&shim) {
            if !is_agentfleet_command_shim(&existing) {
                return Err(format!(
                    "{} already exists and is not a Fleet command shim; refusing to overwrite",
                    shim.display()
                ));
            }
        } else if shim.exists() {
            return Err(format!(
                "{} already exists and is not readable; refusing to overwrite",
                shim.display()
            ));
        }
    } else if let Ok(meta) = std::fs::symlink_metadata(&shim) {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&shim)
                .map_err(|err| format!("failed to replace existing shim: {err}"))?;
        } else {
            return Err(format!(
                "{} already exists and is not a symlink; refusing to overwrite",
                shim.display()
            ));
        }
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(exe, &shim)
        .map_err(|err| format!("failed to create {} shim: {err}", shim.display()))?;

    #[cfg(windows)]
    {
        let script = windows_command_wrapper_script(name, exe);
        std::fs::write(&shim, script)
            .map_err(|err| format!("failed to write {} shim: {err}", shim.display()))?;
    }

    Ok(())
}

#[cfg(windows)]
fn windows_command_wrapper_script(name: &str, exe: &Path) -> String {
    let exe = exe.display();
    if name == "bash" {
        format!(
            r#"@echo off
rem AgentFleet command shim
set "RPTY_ARGV0=bash"
"{exe}" %*
"#
        )
    } else {
        format!(
            r#"@echo off
rem AgentFleet command shim
"{exe}" %*
"#
        )
    }
}

fn current_exe_string() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable: {err}"))?
        .to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| "current executable path is not valid UTF-8".to_string())
}

fn host_from_optional_arg(args: &[String]) -> Result<String, String> {
    if let Some(host) = args.first() {
        return Ok(host.clone());
    }
    state::current_host()
        .map_err(|err| format!("failed to read current host: {err}"))?
        .ok_or_else(|| "no host set; pass a device or run rpty use <device>".to_string())
}

fn cleanup(args: &[String]) -> Result<ExitCode, String> {
    let mut all = false;
    let mut host = None;

    for arg in args {
        match arg.as_str() {
            "--all" => all = true,
            value if value.starts_with('-') => {
                return Err(format!(
                    "unknown cleanup option: {value}. Usage: fleet cleanup [--all] [device]"
                ));
            }
            value => {
                if host.replace(value.to_string()).is_some() {
                    return Err("cleanup accepts at most one device".to_string());
                }
            }
        }
    }

    let host = match host {
        Some(host) => host,
        None => host_from_optional_arg(&[])?,
    };

    let router = Router::new();
    if all {
        router.cleanup_all(&host)?;
        println!("Cleaned all rpty sessions for {host}");
    } else {
        router.cleanup(&host)?;
        println!("Cleaned rpty session for {host}");
    }
    Ok(ExitCode::SUCCESS)
}

fn doctor(args: &[String]) -> Result<ExitCode, String> {
    let mut fix = false;
    let mut write_shell_profile = false;
    let mut device = None;
    for arg in args {
        match arg.as_str() {
            "--fix" => fix = true,
            "--write-shell-profile" => write_shell_profile = true,
            value if device.is_none() => device = Some(value),
            _ => {
                return Err(
                    "usage: rpty doctor [--fix] [--write-shell-profile] [device]".to_string(),
                );
            }
        }
    }

    println!("rpty doctor: Rust CLI is installed");
    println!("Fleet command: {}", FleetCommand::discover().describe());
    match device {
        Some(device) => {
            Router::new().doctor_device(device, fix)?;
        }
        None => {
            doctor_local(fix, write_shell_profile)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn doctor_local(fix: bool, write_shell_profile: bool) -> Result<(), String> {
    let dir = crate::paths::rpty_home().join("bin");
    println!("Install dir: {}", dir.display());
    println!("PATH contains install dir: {}", path_contains_dir(&dir));

    if fix {
        install_base_shims(&dir)?;
        if write_shell_profile {
            let profile = write_shell_profile_path(&dir)?;
            println!("Updated shell profile: {}", profile.display());
        }
    }

    print_device_inventory_hint(&dir);

    println!("Fleet inventory:");
    match FleetCommand::discover().capture(["list", "--json"]) {
        Ok(captured) => {
            if captured.success {
                println!("  ok");
            } else {
                eprintln!("  fleet list exited with {}", captured.code);
                print_first_nonempty_line("  stdout", &captured.stdout);
                print_first_nonempty_line("  stderr", &captured.stderr);
            }
        }
        Err(err) => {
            eprintln!("  {err}");
        }
    }

    println!("Agent commands:");
    for agent in known_agent_commands() {
        let real = find_command_excluding_dir(&agent, &dir);
        let shim = agent_shim_installed(&command_shim_path(&dir, &agent));
        match (&real, shim) {
            (Some(path), true) => println!("  {agent}: real={} shim=installed", path.display()),
            (Some(path), false) => {
                println!("  {agent}: real={} shim=missing", path.display());
                if fix {
                    install_agent_wrapper(&dir, &agent)?;
                    println!("  {agent}: shim installed");
                }
            }
            (None, true) => println!("  {agent}: real=<not found> shim=installed"),
            (None, false) => println!("  {agent}: real=<not found> shim=missing"),
        }
    }

    if fix {
        println!("Local Fleet router environment fixed.");
        if write_shell_profile {
            println!("Open a new shell or source your shell profile to use the shims.");
        } else {
            println!("Ensure this is in your shell startup file:");
            println!("  export PATH=\"{}:$PATH\"", dir.display());
            println!("Or rerun: fleet doctor --fix --write-shell-profile");
        }
    }
    Ok(())
}

fn print_device_inventory_hint(dir: &Path) {
    let devices = dir.join("fleet_backend").join("devices.json");
    let example = dir.join("fleet_backend").join("devices.example.json");
    println!("Device inventory:");
    if devices.exists() {
        println!("  {}", devices.display());
    } else {
        println!("  missing: {}", devices.display());
        if example.exists() {
            println!("  create with:");
            println!("    cp {} {}", example.display(), devices.display());
            if !cfg!(windows) {
                println!("    chmod 600 {}", devices.display());
            }
        }
    }
}

fn install_base_shims(dir: &Path) -> Result<(), String> {
    let runtime = install_runtime_binary(dir)?;
    install_bundled_backend(dir)?;
    install_link(dir, "rpty", &runtime)?;
    install_link(dir, "fleet", &runtime)?;
    install_link(dir, "bash", &runtime)?;
    println!("Installed base shims:");
    println!("  {}", runtime.display());
    println!("  {}", dir.join("fleet").display());
    println!("  {}", dir.join("rpty").display());
    println!("  {}", dir.join("bash").display());
    println!("  {}", dir.join("fleet_backend").display());
    Ok(())
}

fn install_bundled_backend(dir: &Path) -> Result<(), String> {
    let Some(source) = bundled_backend_source() else {
        return Ok(());
    };
    let dest = dir.join("fleet_backend");
    std::fs::create_dir_all(&dest)
        .map_err(|err| format!("failed to create {}: {err}", dest.display()))?;
    for name in [
        "fleet.py",
        "bootstrap.sh",
        "pyproject.toml",
        "devices.example.json",
    ] {
        let src = source.join(name);
        if src.exists() {
            std::fs::copy(&src, dest.join(name))
                .map_err(|err| format!("failed to install {}: {err}", src.display()))?;
        }
    }
    Ok(())
}

fn bundled_backend_source() -> Option<PathBuf> {
    let exe_source = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("fleet_backend")));
    if let Some(source) = exe_source {
        if source.join("devices.example.json").exists()
            || source.join("bootstrap.sh").exists()
            || source.join("fleet.py").exists()
        {
            return Some(source);
        }
    }

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fleet_backend");
    if source.join("devices.example.json").exists()
        || source.join("bootstrap.sh").exists()
        || source.join("fleet.py").exists()
    {
        Some(source)
    } else {
        None
    }
}

fn known_agent_commands() -> Vec<String> {
    let mut agents = KNOWN_AGENT_COMMANDS
        .iter()
        .map(|agent| (*agent).to_string())
        .collect::<Vec<_>>();
    for agent in Config::load().agents {
        if validate_agent_name(&agent).is_ok() && !agents.iter().any(|existing| existing == &agent)
        {
            agents.push(agent);
        }
    }
    agents
}

fn write_shell_profile_path(dir: &Path) -> Result<PathBuf, String> {
    let profile = shell_profile_path();
    let export_line = shell_profile_export_line(&profile, dir);
    ensure_line_in_file(&profile, &export_line)?;
    Ok(profile)
}

fn shell_profile_path() -> PathBuf {
    if let Some(profile) = std::env::var_os("RPTY_SHELL_PROFILE") {
        return PathBuf::from(profile);
    }
    if cfg!(windows) {
        return home_dir()
            .join("Documents")
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1");
    }
    if let Some(shell) = std::env::var_os("SHELL").and_then(|shell| shell.into_string().ok()) {
        let home = home_dir();
        if shell.ends_with("zsh") {
            return home.join(".zshrc");
        }
        if shell.ends_with("bash") {
            return home.join(".bashrc");
        }
        if shell.ends_with("fish") {
            return home.join(".config").join("fish").join("config.fish");
        }
    }
    home_dir().join(".profile")
}

fn shell_profile_export_line(profile: &Path, dir: &Path) -> String {
    if cfg!(windows) {
        return format!("$env:Path = \"{};$env:Path\"", dir.display());
    }
    if profile
        .to_str()
        .map(|value| value.ends_with("config.fish"))
        .unwrap_or(false)
    {
        return format!("set -gx PATH {} $PATH", dir.display());
    }
    format!("export PATH=\"{}:$PATH\"", dir.display())
}

fn ensure_line_in_file(path: &Path, line: &str) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing
        .lines()
        .any(|existing_line| existing_line.trim() == line)
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file).map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    writeln!(file, "\n# AgentFleet\n{line}")
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn path_contains_dir(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|entry| same_path(&entry, dir)))
        .unwrap_or(false)
}

fn find_command_excluding_dir(command: &str, excluded_dir: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        if same_path(&entry, excluded_dir) {
            continue;
        }
        let candidate = entry.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn agent_shim_installed(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| content.contains("AgentFleet agent shim"))
        .unwrap_or(false)
}

fn is_agentfleet_agent_shim(content: &str) -> bool {
    content.contains("AgentFleet agent shim") || content.contains("fleet-pty-router agent shim")
}

fn is_agentfleet_command_shim(content: &str) -> bool {
    content.contains("AgentFleet command shim") || content.contains("fleet-pty-router command shim")
}

fn print_first_nonempty_line(label: &str, value: &str) {
    if let Some(line) = value.lines().find(|line| !line.trim().is_empty()) {
        eprintln!("{label}: {line}");
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn shell() -> Result<ExitCode, String> {
    let router = Router::new();
    let stdin = io::stdin();
    let mut host = state::current_host().map_err(|err| format!("failed to read host: {err}"))?;

    loop {
        print!("rpty[{}]> ", host.as_deref().unwrap_or("-"));
        io::stdout()
            .flush()
            .map_err(|err| format!("failed to flush stdout: {err}"))?;

        let mut line = String::new();
        let bytes = stdin
            .read_line(&mut line)
            .map_err(|err| format!("failed to read input: {err}"))?;
        if bytes == 0 {
            println!();
            break;
        }
        let line = line.trim_end().to_string();
        if line.trim().is_empty() {
            continue;
        }

        if matches!(line.as_str(), "exit" | "quit" | "exit-router") {
            break;
        }
        if line == "hosts" {
            FleetCommand::discover().passthrough(["list", "--json"])?;
            continue;
        }
        if line == "where" {
            match &host {
                Some(value) => println!("Current host: {value}"),
                None => println!("Current host: <unset>"),
            }
            continue;
        }
        if line == "env" {
            let Some(current) = &host else {
                eprintln!("no host set; use <device>");
                continue;
            };
            match SessionLock::acquire(router.session_id(), current)
                .and_then(|_lock| router.environment_summary(current))
            {
                Ok(output) => print!("{output}"),
                Err(err) => eprintln!("{err}"),
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            let next = rest.trim();
            if next.is_empty() {
                eprintln!("usage: use <device>");
                continue;
            }
            state::set_current_host(next)
                .map_err(|err| format!("failed to save current host: {err}"))?;
            host = Some(next.to_string());
            println!("Current host: {next}");
            continue;
        }
        if line == "attach" {
            let Some(current) = &host else {
                eprintln!("no host set; use <device>");
                continue;
            };
            return router.attach(current);
        }
        if line == "logs" || line == "capture" {
            let Some(current) = &host else {
                eprintln!("no host set; use <device>");
                continue;
            };
            match router.capture(current) {
                Ok(output) => print!("{output}"),
                Err(err) => eprintln!("{err}"),
            }
            continue;
        }

        let Some(current) = &host else {
            eprintln!("no host set; use <device>");
            continue;
        };
        let command = read_multiline_if_needed(line)?;
        match SessionLock::acquire(router.session_id(), current)
            .and_then(|_lock| router.run_command(current, &command))
        {
            Ok(run) => {
                let code = print_run(&run);
                if code != 0 {
                    eprintln!("rpty: exit {code}");
                }
            }
            Err(err) => eprintln!("{err}"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn read_multiline_if_needed(first_line: String) -> Result<String, String> {
    let Some(delimiter) = heredoc_delimiter(&first_line) else {
        return Ok(first_line);
    };

    let stdin = io::stdin();
    let mut command = first_line;
    loop {
        print!("... ");
        io::stdout()
            .flush()
            .map_err(|err| format!("failed to flush stdout: {err}"))?;
        let mut line = String::new();
        let bytes = stdin
            .read_line(&mut line)
            .map_err(|err| format!("failed to read heredoc: {err}"))?;
        if bytes == 0 {
            return Err(format!("EOF before heredoc terminator {delimiter}"));
        }
        let trimmed = line.trim_end().to_string();
        command.push('\n');
        command.push_str(&trimmed);
        if trimmed == delimiter {
            break;
        }
    }
    Ok(command)
}

fn heredoc_delimiter(line: &str) -> Option<String> {
    let marker = line.find("<<")?;
    let mut rest = &line[marker + 2..];
    if let Some(stripped) = rest.strip_prefix('-') {
        rest = stripped;
    }
    rest = rest.trim_start();
    let quote = rest.chars().next().filter(|ch| *ch == '\'' || *ch == '"');
    if quote.is_some() {
        rest = &rest[1..];
    }
    let mut delimiter = String::new();
    for ch in rest.chars() {
        if Some(ch) == quote {
            break;
        }
        if quote.is_none() && ch.is_whitespace() {
            break;
        }
        if quote.is_none() && matches!(ch, ';' | '&' | '|' | '<' | '>') {
            break;
        }
        delimiter.push(ch);
    }
    if delimiter.is_empty() {
        None
    } else {
        Some(delimiter)
    }
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn help_succeeds() {
        let code = run(Vec::<String>::new()).unwrap();
        assert_eq!(code, std::process::ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_fails() {
        let err = run(["missing".to_string()]).unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[test]
    fn parses_run_args_with_host_and_timeout() {
        let args = vec![
            "--host".to_string(),
            "radxa".to_string(),
            "--timeout".to_string(),
            "5".to_string(),
            "--".to_string(),
            "echo".to_string(),
            "ok".to_string(),
        ];
        let (host, timeout, command) = super::parse_run_args(&args).unwrap();
        assert_eq!(host, "radxa");
        assert_eq!(timeout, 5);
        assert_eq!(command, "echo ok");
    }

    #[test]
    fn detects_heredoc_delimiter() {
        assert_eq!(super::heredoc_delimiter("cat <<'EOF' > x").unwrap(), "EOF");
        assert_eq!(super::heredoc_delimiter("cat <<EOF").unwrap(), "EOF");
    }

    #[test]
    fn sanitizes_generated_session_fragment() {
        assert_eq!(super::sanitize_session_fragment("a/b:c"), "a_b_c");
    }

    #[test]
    fn rejects_invalid_agent_names() {
        assert!(super::validate_agent_name("opencode").is_ok());
        assert!(super::validate_agent_name("bad/name").is_err());
        assert!(super::validate_agent_name("").is_err());
    }

    #[test]
    fn old_agent_shim_marker_is_upgradable() {
        assert!(super::is_agentfleet_agent_shim(
            "# fleet-pty-router agent shim"
        ));
        assert!(super::is_agentfleet_agent_shim("# AgentFleet agent shim"));
        assert!(super::is_agentfleet_command_shim(
            "rem fleet-pty-router command shim"
        ));
        assert!(super::is_agentfleet_command_shim(
            "rem AgentFleet command shim"
        ));
    }

    #[test]
    fn shell_profile_line_is_idempotent() {
        let path = std::env::temp_dir().join(format!("rpty-profile-test-{}", std::process::id()));
        let line = "export PATH=\"/tmp/rpty:$PATH\"";
        super::ensure_line_in_file(&path, line).unwrap();
        super::ensure_line_in_file(&path, line).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches(line).count(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn command_shim_path_is_platform_specific() {
        let path = super::command_shim_path(std::path::Path::new("/tmp/rpty"), "codex");
        if cfg!(windows) {
            assert!(path.ends_with("codex.cmd"));
        } else {
            assert!(path.ends_with("codex"));
        }
    }
}
