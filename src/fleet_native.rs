use std::io::Read;
#[cfg(not(windows))]
use std::io::Write;
#[cfg(not(windows))]
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::fleet::Captured;

const NATIVE_FALLBACK: &str = "__RPTY_NATIVE_FALLBACK__";
const EXCLUDE_ALWAYS: &[&str] = &[
    ".git/",
    "node_modules/",
    ".venv/",
    "venv/",
    "__pycache__/",
    "*.pyc",
    ".cache/",
    "dist/",
    "build/",
    ".tox/",
    ".nox/",
    ".pytest_cache/",
    "*.egg-info/",
    ".mypy_cache/",
    ".ruff_cache/",
    "target/",
    ".gradle/",
    ".DS_Store",
    "*.log",
    "*.tmp",
];

#[derive(Debug, Clone)]
pub struct NativeFleet {
    inventory_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListArgs {
    tags: Vec<String>,
    owner: Option<String>,
    json: bool,
}

#[cfg_attr(windows, allow(dead_code))]
#[derive(Debug, Clone)]
struct Device {
    name: String,
    value: Value,
    host: String,
    user: String,
    password: String,
    port: u16,
    tags: Vec<String>,
    owner: String,
    description: String,
    gateway: Option<String>,
    wsl_distro: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteRun {
    success: bool,
    output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusArgs {
    device: Option<String>,
    tags: Vec<String>,
    owner: Option<String>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchArgs {
    tags: Vec<String>,
    owner: Option<String>,
    sort: Option<String>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecArgs {
    tags: Vec<String>,
    sudo: bool,
    timeout: u64,
    host: Option<String>,
    json: bool,
    literal: bool,
    stream: bool,
    detach: bool,
    raw: bool,
    device: String,
    command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceJsonArgs {
    host: Option<String>,
    device: String,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogArgs {
    host: Option<String>,
    device: String,
    job_id: String,
    tail: u64,
    follow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KillArgs {
    host: Option<String>,
    sudo: bool,
    device: String,
    job_id: String,
    force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransferArgs {
    host: Option<String>,
    device: String,
    first_path: String,
    second_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AddArgs {
    name: String,
    host: String,
    user: String,
    password: Option<String>,
    owner: String,
    tags: Vec<String>,
    description: String,
    scan: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoveArgs {
    name: String,
    force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BootstrapArgs {
    device: Option<String>,
    all: bool,
    tags: Vec<String>,
    profile: Option<String>,
    check: bool,
    force: bool,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanArgs {
    device: Option<String>,
    tags: Vec<String>,
    dry_run: bool,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteTransferArgs {
    source: String,
    dest: String,
    relay: bool,
    dest_host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WslArgs {
    device: String,
    action: String,
    distro: Option<String>,
    timeout: u64,
    command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkSyncArgs {
    host: Option<String>,
    device: String,
    local: String,
    remote: String,
    push: bool,
    pull: bool,
    dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SshArgs {
    host: Option<String>,
    device: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkEnterArgs {
    host: Option<String>,
    device: String,
    remote_dir: String,
    session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkMonitorArgs {
    host: Option<String>,
    device: String,
    session: String,
    on_exit: String,
}

impl BootstrapArgs {
    fn remote_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.check {
            args.push("--check".to_string());
        }
        if self.force {
            args.push("--force".to_string());
        }
        if let Some(profile) = &self.profile {
            args.push("--profile".to_string());
            args.push(sh_quote(profile));
        }
        args
    }
}

impl Device {
    fn is_windows(&self) -> bool {
        self.tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case("windows"))
            || self
                .value
                .get("specs")
                .and_then(|specs| specs.get("os"))
                .and_then(Value::as_str)
                .map(|os| os.to_lowercase().contains("windows"))
                .unwrap_or(false)
    }
}

impl NativeFleet {
    pub fn new(inventory_path: PathBuf) -> Self {
        Self { inventory_path }
    }

    pub fn describe(&self) -> String {
        format!("rust-native devices={}", self.inventory_path.display())
    }

    pub fn capture(&self, args: &[String]) -> Result<Option<Captured>, String> {
        let output = match self.run(args) {
            Ok(Some(output)) => output,
            Ok(None) => return Ok(None),
            Err(err) if err == NATIVE_FALLBACK => return Ok(None),
            Err(err) => return Err(err),
        };
        Ok(Some(Captured {
            success: output.code == 0,
            stdout: output.stdout,
            stderr: output.stderr,
            code: output.code,
        }))
    }

    pub fn passthrough(&self, args: &[String]) -> Result<Option<ExitCode>, String> {
        let output = match self.run(args) {
            Ok(Some(output)) => output,
            Ok(None) => return Ok(None),
            Err(err) if err == NATIVE_FALLBACK => return Ok(None),
            Err(err) => return Err(err),
        };
        print!("{}", output.stdout);
        eprint!("{}", output.stderr);
        Ok(Some(ExitCode::from(output.code.clamp(0, 255) as u8)))
    }

    fn run(&self, args: &[String]) -> Result<Option<NativeOutput>, String> {
        match args.first().map(String::as_str) {
            Some("list") => self.list(&args[1..]).map(Some),
            Some("status") => self.status(&args[1..]).map(Some),
            Some("match") => self.match_devices(&args[1..]).map(Some),
            Some("exec") => self.exec(&args[1..]).map(Some),
            Some("docker") => self.docker(&args[1..]).map(Some),
            Some("jobs") => self.jobs(&args[1..]).map(Some),
            Some("log") => self.log(&args[1..]).map(Some),
            Some("kill-job") => self.kill_job(&args[1..]).map(Some),
            Some("push") => {
                if args.iter().any(|arg| arg == "--host") {
                    self.push(&args[1..]).map(Some)
                } else if args.len() >= 4
                    && std::fs::metadata(&args[2])
                        .map(|m| m.is_dir())
                        .unwrap_or(false)
                {
                    Ok(None)
                } else {
                    self.push(&args[1..]).map(Some)
                }
            }
            Some("pull") => self.pull(&args[1..]).map(Some),
            Some("add") => self.add(&args[1..]).map(Some),
            Some("remove") => self.remove(&args[1..]).map(Some),
            Some("bootstrap") => self.bootstrap(&args[1..]).map(Some),
            Some("scan") => self.scan(&args[1..]).map(Some),
            Some("transfer") => self.transfer(&args[1..]).map(Some),
            Some("wsl") => self.wsl(&args[1..]).map(Some),
            Some("work-sync") => self.work_sync(&args[1..]).map(Some),
            Some("ssh") => self.ssh(&args[1..]).map(Some),
            Some("work-enter") => self.work_enter(&args[1..]).map(Some),
            Some("work-monitor") => self.work_monitor(&args[1..]).map(Some),
            _ => Ok(None),
        }
    }

    fn list(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_list_args(args) {
            Ok(args) => args,
            Err(err) => {
                return Ok(NativeOutput {
                    stdout: String::new(),
                    stderr: format!("{err}\n"),
                    code: 2,
                })
            }
        };
        let mut devices = match load_devices(&self.inventory_path) {
            Ok(devices) => devices,
            Err(err) => {
                return Ok(NativeOutput {
                    stdout: String::new(),
                    stderr: format!("{err}\n"),
                    code: 1,
                })
            }
        };
        filter_devices(&mut devices, &args);

        if args.json {
            let stdout = format!(
                "{}\n",
                serde_json::to_string_pretty(&Value::Object(devices))
                    .map_err(|err| format!("failed to render devices JSON: {err}"))?
            );
            return Ok(NativeOutput {
                stdout,
                stderr: String::new(),
                code: 0,
            });
        }

        if devices.is_empty() {
            return Ok(NativeOutput {
                stdout: "No devices found.\n".to_string(),
                stderr: String::new(),
                code: 0,
            });
        }

        let rows = devices
            .iter()
            .map(|(name, dev)| {
                vec![
                    name.to_string(),
                    string_field(dev, "host"),
                    string_field(dev, "owner"),
                    tags_field(dev),
                    string_field(dev, "description"),
                ]
            })
            .collect::<Vec<_>>();
        Ok(NativeOutput {
            stdout: format_table(&rows, &["NAME", "HOST", "OWNER", "TAGS", "DESCRIPTION"]),
            stderr: String::new(),
            code: 0,
        })
    }

    fn status(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_status_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let devices =
            match self.select_devices(args.device.as_deref(), &args.tags, args.owner.as_deref()) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            };
        let mut results = devices
            .iter()
            .map(|device| probe_device(device))
            .collect::<Vec<_>>();
        results.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str))
        });

        if args.json {
            return json_output(Value::Array(results), 0);
        }
        if results.is_empty() {
            return Ok(stdout("No devices found.\n"));
        }
        let rows = results
            .iter()
            .map(|result| {
                let online = result
                    .get("online")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let status = if online {
                    "ONLINE".to_string()
                } else if let Some(gateway) = result.get("gateway").and_then(Value::as_str) {
                    format!("OFFLINE [gw:{gateway}]")
                } else {
                    "OFFLINE".to_string()
                };
                let disk = result
                    .get("disk")
                    .and_then(|disk| disk.get("avail"))
                    .and_then(Value::as_str)
                    .map(|avail| format!("{avail} free"))
                    .unwrap_or_else(|| "-".to_string());
                let mem = result.get("memory").and_then(Value::as_object);
                let memory = mem
                    .map(|mem| {
                        format!(
                            "{}/{} MB",
                            mem.get("used_mb").and_then(Value::as_str).unwrap_or("?"),
                            mem.get("total_mb").and_then(Value::as_str).unwrap_or("?")
                        )
                    })
                    .unwrap_or_else(|| "-".to_string());
                let cpu = result
                    .get("cpu_load")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string();
                let gpu = result
                    .get("gpu")
                    .and_then(|gpu| gpu.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string();
                vec![
                    result
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    status,
                    disk,
                    memory,
                    cpu,
                    gpu,
                ]
            })
            .collect::<Vec<_>>();
        Ok(NativeOutput {
            stdout: format_table(
                &rows,
                &["NAME", "STATUS", "DISK", "MEMORY", "CPU LOAD", "GPU"],
            ),
            stderr: String::new(),
            code: 0,
        })
    }

    fn match_devices(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_match_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let devices = match self.select_devices(None, &args.tags, args.owner.as_deref()) {
            Ok(devices) => devices,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if devices.is_empty() {
            return if args.json {
                Ok(stdout("[]\n"))
            } else {
                Ok(stdout("No devices match the specified tags.\n"))
            };
        }
        let mut results = devices
            .iter()
            .map(|device| probe_device(device))
            .filter(|result| {
                result
                    .get("online")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        sort_match_results(&mut results, args.sort.as_deref());
        if results.is_empty() {
            return if args.json {
                Ok(stdout("[]\n"))
            } else {
                Ok(stdout("No online devices match the specified tags.\n"))
            };
        }
        if args.json {
            return json_output(Value::Array(results), 0);
        }
        let rows = results
            .iter()
            .map(|result| {
                let disk = result
                    .get("disk")
                    .and_then(|disk| disk.get("avail"))
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string();
                let mem = result.get("memory").and_then(Value::as_object);
                let memory = mem
                    .map(|mem| {
                        format!(
                            "{} MB free",
                            mem.get("free_mb").and_then(Value::as_str).unwrap_or("?")
                        )
                    })
                    .unwrap_or_else(|| "-".to_string());
                vec![
                    result
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    disk,
                    memory,
                    result
                        .get("cpu_load")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                    format!(
                        "ssh {}",
                        result.get("host").and_then(Value::as_str).unwrap_or("")
                    ),
                ]
            })
            .collect::<Vec<_>>();
        Ok(NativeOutput {
            stdout: format_table(&rows, &["NAME", "DISK", "MEMORY", "CPU LOAD", "SSH"]),
            stderr: String::new(),
            code: 0,
        })
    }

    fn exec(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_exec_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if args.stream {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let devices = match self.select_devices(Some(&args.device), &args.tags, None) {
            Ok(devices) => devices,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let targets = if args.tags.is_empty() {
            devices
        } else {
            match self.select_devices(None, &args.tags, None) {
                Ok(devices) if !devices.is_empty() => devices,
                Ok(_) => {
                    return Ok(cli_error(
                        "No devices match the specified tags.".to_string(),
                        1,
                    ))
                }
                Err(err) => return Ok(cli_error(err, 1)),
            }
        };

        if args.detach {
            if targets.len() != 1 {
                return Ok(cli_error(
                    "Error: --detach requires a single target.".to_string(),
                    2,
                ));
            }
            if targets[0].is_windows() {
                return Ok(cli_error("Error: --detach uses nohup/sh and does not work on Windows devices.\n       Use: fleet exec <device> -- powershell -Command \"Start-Process ... -WindowStyle Hidden\"".to_string(), 2));
            }
            return self.exec_detach(&targets[0], &args);
        }

        let mut results = Map::new();
        let command = if args.literal {
            shlex_join(&args.command)
        } else {
            args.command.join(" ")
        };
        for device in &targets {
            let mut device = device.clone();
            if let Some(host) = &args.host {
                device.host = host.clone();
            }
            let run = ssh_exec(
                &device,
                &command,
                Duration::from_secs(args.timeout),
                args.sudo,
                args.raw,
            );
            match run {
                Ok(run) => {
                    results.insert(
                        device.name.clone(),
                        json!({"success": run.success, "output": run.output}),
                    );
                }
                Err(err) => {
                    results.insert(
                        device.name.clone(),
                        json!({"success": false, "output": err}),
                    );
                }
            }
        }
        if args.json {
            return json_output(Value::Object(results), 0);
        }
        let mut stdout_text = String::new();
        let mut stderr_text = String::new();
        let mut ok_all = true;
        for (index, device) in targets.iter().enumerate() {
            if targets.len() > 1 {
                stdout_text.push_str(&format!("=== {} ===\n", device.name));
            }
            let result = results
                .get(&device.name)
                .and_then(Value::as_object)
                .unwrap();
            if result
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                stdout_text.push_str(result.get("output").and_then(Value::as_str).unwrap_or(""));
                stdout_text.push('\n');
            } else {
                ok_all = false;
                stderr_text.push_str(&format!(
                    "Error: {}\n",
                    result.get("output").and_then(Value::as_str).unwrap_or("")
                ));
                if let Some(gateway) = &device.gateway {
                    stderr_text.push_str(&format!(
                        "[fleet] Hint: {} is unreachable but has a gateway -> {gateway}\n",
                        device.name
                    ));
                    stderr_text.push_str(&format!(
                        "  Check WSL state : fleet wsl {} status\n",
                        device.name
                    ));
                    stderr_text.push_str(&format!(
                        "  Restart WSL     : fleet wsl {} restart\n",
                        device.name
                    ));
                }
            }
            if targets.len() > 1 && index + 1 < targets.len() {
                stdout_text.push('\n');
            }
        }
        Ok(NativeOutput {
            stdout: stdout_text,
            stderr: stderr_text,
            code: if ok_all { 0 } else { 1 },
        })
    }

    fn docker(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_device_json_args("docker", args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let command = "docker ps --format '{{json .}}'";
        let run = match ssh_exec(&device, command, Duration::from_secs(60), false, false) {
            Ok(run) if run.success => run,
            Ok(run) => return Ok(cli_error(format!("Error: {}", run.output), 1)),
            Err(err) => return Ok(cli_error(format!("Error: {err}"), 1)),
        };
        let containers = run
            .output
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .map(|value| {
                json!({
                    "name": value.get("Names").and_then(Value::as_str).unwrap_or(""),
                    "image": value.get("Image").and_then(Value::as_str).unwrap_or(""),
                    "status": value.get("Status").and_then(Value::as_str).unwrap_or(""),
                    "ports": value.get("Ports").and_then(Value::as_str).unwrap_or(""),
                })
            })
            .collect::<Vec<_>>();
        if args.json {
            return json_output(json!({"device": args.device, "containers": containers}), 0);
        }
        if containers.is_empty() {
            return Ok(stdout(&format!("No containers on {}.\n", args.device)));
        }
        let rows = containers
            .iter()
            .map(|container| {
                vec![
                    string_field(container, "name"),
                    string_field(container, "image"),
                    string_field(container, "status"),
                    string_field(container, "ports"),
                ]
            })
            .collect::<Vec<_>>();
        Ok(NativeOutput {
            stdout: format!(
                "Containers on {} ({}):\n\n{}",
                args.device,
                device.host,
                format_table(&rows, &["NAME", "IMAGE", "STATUS", "PORTS"])
            ),
            stderr: String::new(),
            code: 0,
        })
    }

    fn jobs(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_device_json_args("jobs", args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let command = r#"for f in /tmp/fleet-jobs/*.json; do [ -e "$f" ] || continue; echo __FLEET_JOB__; cat "$f"; echo; pidfile="${f%.json}.pid"; alive=no; if [ -f "$pidfile" ]; then pid=$(cat "$pidfile" 2>/dev/null); if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then alive=yes; fi; fi; echo "__PID_ALIVE__:$alive"; done"#;
        let run = match ssh_exec(&device, command, Duration::from_secs(10), false, false) {
            Ok(run) if run.success => run,
            Ok(run) => return Ok(cli_error(format!("Error: {}", run.output), 1)),
            Err(err) => return Ok(cli_error(format!("Error: {err}"), 1)),
        };
        let jobs = parse_jobs_output(&run.output);
        if args.json {
            return json_output(json!({"device": args.device, "jobs": jobs}), 0);
        }
        if jobs.is_empty() {
            return Ok(stdout(&format!("No detached jobs on {}.\n", args.device)));
        }
        let rows = jobs
            .iter()
            .map(|job| {
                let running = job.get("status").and_then(Value::as_str) == Some("running");
                let alive = job.get("_pid_alive").and_then(Value::as_str) == Some("yes");
                let status = if running && !alive {
                    "stale".to_string()
                } else {
                    job.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string()
                };
                vec![
                    string_field(job, "id"),
                    status,
                    string_field(job, "started_at"),
                    string_field(job, "command").chars().take(60).collect(),
                ]
            })
            .collect::<Vec<_>>();
        Ok(NativeOutput {
            stdout: format!(
                "Jobs on {}:\n\n{}",
                args.device,
                format_table(&rows, &["JOB ID", "STATUS", "STARTED", "COMMAND"])
            ),
            stderr: String::new(),
            code: 0,
        })
    }

    fn log(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_log_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if !valid_job_id(&args.job_id) {
            return Ok(cli_error("Error: invalid job ID format".to_string(), 1));
        }
        if args.follow {
            return Ok(stdout(&format!(
                "Use 'fleet ssh {}' then: tail -f /tmp/fleet-jobs/{}.log\n",
                args.device, args.job_id
            )));
        }
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let command = format!(
            "tail -n {} /tmp/fleet-jobs/{}.log 2>/dev/null || echo '[fleet] log file not found: /tmp/fleet-jobs/{}.log'",
            args.tail, args.job_id, args.job_id
        );
        match ssh_exec(&device, &command, Duration::from_secs(10), false, false) {
            Ok(run) => Ok(stdout(&(run.output + "\n"))),
            Err(err) => Ok(cli_error(err, 1)),
        }
    }

    fn kill_job(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_kill_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if !valid_job_id(&args.job_id) {
            return Ok(cli_error("Error: invalid job ID format".to_string(), 1));
        }
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let signal = if args.force { "9" } else { "15" };
        let signal_name = if args.force { "SIGKILL" } else { "SIGTERM" };
        let command = format!(
            "pid=$(cat /tmp/fleet-jobs/{id}.pid 2>/dev/null); if [ -z \"$pid\" ]; then echo 'no-pid-file'; exit 0; fi; kill -{signal} $pid 2>/dev/null && echo 'killed' || echo 'not-found'; tmp=/tmp/fleet-jobs/{id}.json.tmp; sed 's/\"status\":\"running\"/\"status\":\"killed\"/; s/\"status\": \"running\"/\"status\": \"killed\"/' /tmp/fleet-jobs/{id}.json > \"$tmp\" 2>/dev/null && mv \"$tmp\" /tmp/fleet-jobs/{id}.json",
            id = args.job_id
        );
        let run = match ssh_exec(&device, &command, Duration::from_secs(10), args.sudo, false) {
            Ok(run) if run.success => run,
            Ok(run) => return Ok(cli_error(format!("Error: {}", run.output), 1)),
            Err(err) => return Ok(cli_error(format!("Error: {err}"), 1)),
        };
        let output = if run.output.contains("killed") {
            format!(
                "Job {} killed ({signal_name}) on {}.\n",
                args.job_id, args.device
            )
        } else if run.output.contains("not-found") {
            format!("Job {}: PID not alive. Marked as killed.\n", args.job_id)
        } else {
            format!("Job {}: {}\n", args.job_id, run.output)
        };
        Ok(stdout(&output))
    }

    fn push(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_transfer_args("push", args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let local = PathBuf::from(&args.first_path);
        if local.is_dir() {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        match sftp_put(&device, &local, &args.second_path) {
            Ok(size) => {
                let local_hash = match local_md5(&local) {
                    Ok(hash) => hash,
                    Err(err) => return Ok(cli_error(err, 1)),
                };
                let remote_hash = remote_md5(&device, &args.second_path).ok().flatten();
                let mut stderr = String::new();
                if let Some(remote_hash) = remote_hash {
                    if remote_hash == local_hash {
                        stderr.push_str(&format!("  verify: OK (md5: {local_hash})\n"));
                    } else {
                        return Ok(NativeOutput {
                            stdout: String::new(),
                            stderr: format!(
                                "  verify: FAILED! src={local_hash} dst={remote_hash}\n"
                            ),
                            code: 1,
                        });
                    }
                } else {
                    stderr.push_str("  verify: SKIP (md5 unavailable on remote)\n");
                }
                Ok(NativeOutput {
                    stdout: format!(
                        "{} -> {}:{} ({})\n",
                        args.first_path,
                        args.device,
                        args.second_path,
                        human_size(size)
                    ),
                    stderr,
                    code: 0,
                })
            }
            Err(err) => Ok(cli_error(format!("Error: {err}"), 1)),
        }
    }

    fn pull(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_transfer_args("pull", args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let remote_type = ssh_exec(
            &device,
            &format!(
                "test -d {} && echo DIR || echo FILE",
                sh_quote(&args.first_path)
            ),
            Duration::from_secs(10),
            false,
            false,
        );
        if matches!(remote_type, Ok(run) if run.output.trim() == "DIR") {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let local = PathBuf::from(&args.second_path);
        match sftp_get(&device, &args.first_path, &local) {
            Ok(size) => {
                let remote_hash = remote_md5(&device, &args.first_path).ok().flatten();
                let local_hash = match local_md5(&local) {
                    Ok(hash) => hash,
                    Err(err) => return Ok(cli_error(err, 1)),
                };
                let mut stderr = String::new();
                if let Some(remote_hash) = remote_hash {
                    if remote_hash == local_hash {
                        stderr.push_str(&format!("  verify: OK (md5: {local_hash})\n"));
                    } else {
                        return Ok(NativeOutput {
                            stdout: String::new(),
                            stderr: format!(
                                "  verify: FAILED! src={remote_hash} dst={local_hash}\n"
                            ),
                            code: 1,
                        });
                    }
                } else {
                    stderr.push_str("  verify: SKIP (md5 unavailable on remote)\n");
                }
                Ok(NativeOutput {
                    stdout: format!(
                        "{}:{} -> {} ({})\n",
                        args.device,
                        args.first_path,
                        args.second_path,
                        human_size(size)
                    ),
                    stderr,
                    code: 0,
                })
            }
            Err(err) => Ok(cli_error(format!("Error: {err}"), 1)),
        }
    }

    fn exec_detach(&self, device: &Device, args: &ExecArgs) -> Result<NativeOutput, String> {
        let command = if args.literal {
            shlex_join(&args.command)
        } else {
            args.command.join(" ")
        };
        let job_id = make_job_id();
        let started_at = timestamp_seconds();
        let metadata = json!({
            "id": job_id,
            "command": command,
            "started_at": started_at,
            "status": "running"
        })
        .to_string();
        let setup = format!(
            "mkdir -p /tmp/fleet-jobs && cat > /tmp/fleet-jobs/{job_id}.json << 'FLEETEOF'\n{metadata}\nFLEETEOF"
        );
        match ssh_exec(device, &setup, Duration::from_secs(10), false, false) {
            Ok(run) if run.success => {}
            Ok(run) => {
                return Ok(cli_error(
                    format!("Error: failed to create job metadata: {}", run.output),
                    1,
                ))
            }
            Err(err) => {
                return Ok(cli_error(
                    format!("Error: failed to create job metadata: {err}"),
                    1,
                ))
            }
        }
        let launch = if args.sudo {
            let script_path = format!("/tmp/fleet-jobs/{job_id}.sh");
            let write_script = format!(
                "cat > {script_path} << 'JOBEOF'\n#!/bin/sh\necho $$ > /tmp/fleet-jobs/{job_id}.pid\n{{ {command}; }} >> /tmp/fleet-jobs/{job_id}.log 2>&1\nJOBEOF\nchmod +x {script_path}"
            );
            match ssh_exec(device, &write_script, Duration::from_secs(5), false, false) {
                Ok(run) if run.success => {}
                Ok(run) => {
                    return Ok(cli_error(
                        format!("Error: failed to write launcher script: {}", run.output),
                        1,
                    ))
                }
                Err(err) => {
                    return Ok(cli_error(
                        format!("Error: failed to write launcher script: {err}"),
                        1,
                    ))
                }
            }
            format!("trap '' HUP; setsid sh -c 'exec </dev/null; nohup {script_path} >> /tmp/fleet-jobs/{job_id}.log 2>&1 &'")
        } else {
            format!(
                "nohup sh -c {} >> /tmp/fleet-jobs/{job_id}.log 2>&1 & echo $! > /tmp/fleet-jobs/{job_id}.pid",
                sh_quote(&command)
            )
        };
        match ssh_exec(device, &launch, Duration::from_secs(15), args.sudo, false) {
            Ok(run) if run.success => {}
            Ok(run) => {
                return Ok(cli_error(
                    format!("Error: failed to start detached job: {}", run.output),
                    1,
                ))
            }
            Err(err) => {
                return Ok(cli_error(
                    format!("Error: failed to start detached job: {err}"),
                    1,
                ))
            }
        }
        let pid = ssh_exec(
            device,
            &format!("cat /tmp/fleet-jobs/{job_id}.pid 2>/dev/null"),
            Duration::from_secs(5),
            false,
            false,
        )
        .ok()
        .map(|run| run.output.trim().to_string())
        .filter(|pid| !pid.is_empty());
        if let Some(pid) = pid {
            let update = format!(
                "tmp=/tmp/fleet-jobs/{job_id}.json.tmp; sed 's/}}/,\"pid\":{pid}}}/' /tmp/fleet-jobs/{job_id}.json > \"$tmp\" && mv \"$tmp\" /tmp/fleet-jobs/{job_id}.json"
            );
            let _ = ssh_exec(device, &update, Duration::from_secs(5), false, false);
        }
        if args.json {
            json_output(
                json!({"device": device.name, "job_id": job_id, "log": format!("/tmp/fleet-jobs/{job_id}.log")}),
                0,
            )
        } else {
            Ok(stdout(&format!(
                "Job {job_id} started on {}\n  Log:    /tmp/fleet-jobs/{job_id}.log\n  Status: fleet jobs {}\n  Tail:   fleet log {} {job_id}\n  Kill:   fleet kill-job {} {job_id}\n",
                device.name, device.name, device.name, device.name
            )))
        }
    }

    fn select_devices(
        &self,
        device_name: Option<&str>,
        tags: &[String],
        owner: Option<&str>,
    ) -> Result<Vec<Device>, String> {
        let mut devices = load_inventory(&self.inventory_path, false)?;
        if let Some(device_name) = device_name {
            if !tags.is_empty() {
                // Python ignores the positional device when --tag is present.
            } else {
                let Some(device) = devices
                    .into_iter()
                    .find(|device| device.name == device_name)
                else {
                    return Err(format!("Error: device '{device_name}' not found"));
                };
                return Ok(vec![device]);
            }
        }
        devices.retain(|device| {
            let owner_ok = owner.map(|owner| device.owner == owner).unwrap_or(true);
            let tags_ok = tags
                .iter()
                .all(|tag| device.tags.iter().any(|existing| existing == tag));
            owner_ok && tags_ok
        });
        Ok(devices)
    }

    fn get_device(&self, name: &str) -> Result<Device, String> {
        self.select_devices(Some(name), &[], None)?
            .into_iter()
            .next()
            .ok_or_else(|| format!("Error: device '{name}' not found"))
    }

    fn add(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_add_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if args.scan {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let mut root = match load_inventory_root(&self.inventory_path) {
            Ok(root) => root,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let devices = root
            .get_mut("devices")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                format!(
                    "{} must contain a top-level 'devices' object",
                    self.inventory_path.display()
                )
            })?;
        if devices.contains_key(&args.name) {
            return Ok(cli_error(
                format!("Error: device '{}' already exists", args.name),
                1,
            ));
        }
        devices.insert(
            args.name.clone(),
            json!({
                "host": args.host,
                "user": args.user,
                "password": args.password.unwrap_or_default(),
                "owner": args.owner,
                "tags": args.tags,
                "specs": {},
                "description": args.description,
            }),
        );
        if let Err(err) = save_inventory_root(&self.inventory_path, &root) {
            return Ok(cli_error(err, 1));
        }
        Ok(stdout(&format!("Added '{}' ({})\n", args.name, args.host)))
    }

    fn remove(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_remove_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if !args.force {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let mut root = match load_inventory_root(&self.inventory_path) {
            Ok(root) => root,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let devices = root
            .get_mut("devices")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                format!(
                    "{} must contain a top-level 'devices' object",
                    self.inventory_path.display()
                )
            })?;
        if devices.remove(&args.name).is_none() {
            return Ok(cli_error(
                format!("Error: device '{}' not found", args.name),
                1,
            ));
        }
        if let Err(err) = save_inventory_root(&self.inventory_path, &root) {
            return Ok(cli_error(err, 1));
        }
        Ok(stdout(&format!("Removed '{}'\n", args.name)))
    }

    fn bootstrap(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_bootstrap_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let targets = if args.all {
            match self.select_devices(None, &[], None) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            }
        } else if !args.tags.is_empty() {
            match self.select_devices(None, &args.tags, None) {
                Ok(devices) if !devices.is_empty() => devices,
                Ok(_) => {
                    return Ok(cli_error(
                        "No devices match the specified tags.".to_string(),
                        1,
                    ))
                }
                Err(err) => return Ok(cli_error(err, 1)),
            }
        } else if let Some(device) = &args.device {
            match self.select_devices(Some(device), &[], None) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            }
        } else {
            return Ok(cli_error(
                "Error: specify a device, --all, or --tag".to_string(),
                1,
            ));
        };

        let script = self.inventory_path.with_file_name("bootstrap.sh");
        if !script.exists() {
            return Ok(cli_error(
                format!("Error: bootstrap.sh not found at {}", script.display()),
                1,
            ));
        }
        let remote_script = format!("/tmp/fleet-bootstrap-{}.sh", make_job_id());
        let remote_args = args.remote_args().join(" ");
        let mut results = Map::new();
        for device in &targets {
            let result = match sftp_put(device, &script, &remote_script) {
                Ok(_) => ssh_exec(
                    device,
                    &format!("chmod +x {remote_script} && bash {remote_script} {remote_args}; rm -f {remote_script}"),
                    Duration::from_secs(60),
                    false,
                    false,
                ),
                Err(err) => Err(err),
            };
            match result {
                Ok(run) => {
                    results.insert(
                        device.name.clone(),
                        json!({"success": run.success, "output": run.output}),
                    );
                }
                Err(err) => {
                    results.insert(
                        device.name.clone(),
                        json!({"success": false, "output": err}),
                    );
                }
            }
        }
        if args.json {
            return json_output(Value::Object(results), 0);
        }
        let mut stdout_text = String::new();
        let mut code = 0;
        for device in &targets {
            if targets.len() > 1 {
                stdout_text.push_str(&format!("=== {} ===\n", device.name));
            }
            let result = results
                .get(&device.name)
                .and_then(Value::as_object)
                .unwrap();
            if !result
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                code = 1;
            }
            stdout_text.push_str(result.get("output").and_then(Value::as_str).unwrap_or(""));
            stdout_text.push('\n');
            if targets.len() > 1 {
                stdout_text.push('\n');
            }
        }
        Ok(NativeOutput {
            stdout: stdout_text,
            stderr: String::new(),
            code,
        })
    }

    fn scan(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_scan_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let targets = if let Some(device) = &args.device {
            match self.select_devices(Some(device), &[], None) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            }
        } else if !args.tags.is_empty() {
            match self.select_devices(None, &args.tags, None) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            }
        } else {
            match self.select_devices(None, &[], None) {
                Ok(devices) => devices,
                Err(err) => return Ok(cli_error(err, 1)),
            }
        };
        let mut scan_results = Map::new();
        let mut stdout_text = String::new();
        let mut root = match load_inventory_root(&self.inventory_path) {
            Ok(root) => root,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let Some(devices_map) = root.get_mut("devices").and_then(Value::as_object_mut) else {
            return Ok(cli_error(
                format!(
                    "{} must contain a top-level 'devices' object",
                    self.inventory_path.display()
                ),
                1,
            ));
        };
        let mut updated = 0;
        for device in &targets {
            match scan_device_specs(device) {
                Ok(info) => {
                    scan_results.insert(device.name.clone(), info.clone());
                    let Some(dev) = devices_map
                        .get_mut(&device.name)
                        .and_then(Value::as_object_mut)
                    else {
                        continue;
                    };
                    let mut changes = Vec::new();
                    let specs = info
                        .get("specs")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let existing_specs = dev.entry("specs").or_insert_with(|| json!({}));
                    if let Some(existing) = existing_specs.as_object_mut() {
                        let mut added = Vec::new();
                        for (key, value) in specs {
                            if !existing.contains_key(&key) {
                                existing.insert(key.clone(), value);
                                added.push(key);
                            }
                        }
                        if !added.is_empty() {
                            changes.push(format!("specs: +{}", added.join(", ")));
                        }
                    }
                    let suggested = info
                        .get("suggested_tags")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let tags_value = dev.entry("tags").or_insert_with(|| json!([]));
                    if let Some(tags) = tags_value.as_array_mut() {
                        let mut added = Vec::new();
                        for tag in suggested.iter().filter_map(Value::as_str) {
                            if !tags.iter().any(|existing| existing.as_str() == Some(tag)) {
                                tags.push(Value::String(tag.to_string()));
                                added.push(tag.to_string());
                            }
                        }
                        if !added.is_empty() {
                            added.sort();
                            changes.push(format!("tags: +{}", added.join(", ")));
                        }
                    }
                    if changes.is_empty() {
                        stdout_text.push_str(&format!("  {}: OK (no changes)\n", device.name));
                    } else {
                        updated += 1;
                        stdout_text.push_str(&format!(
                            "  {}: UPDATED ({})\n",
                            device.name,
                            changes.join("; ")
                        ));
                    }
                }
                Err(err) => {
                    scan_results.insert(device.name.clone(), json!({"error": err}));
                    stdout_text.push_str(&format!("  {}: SKIP ({err})\n", device.name));
                }
            }
        }
        if updated > 0 && !args.dry_run {
            if let Err(err) = save_inventory_root(&self.inventory_path, &root) {
                return Ok(cli_error(err, 1));
            }
            stdout_text.push_str(&format!(
                "\nSaved {updated} device(s) to {}\n",
                self.inventory_path.display()
            ));
        } else if updated > 0 {
            stdout_text.push_str(&format!(
                "\nDry run: {updated} device(s) would be updated\n"
            ));
        }
        if args.json {
            let mut json_stdout = serde_json::to_string_pretty(&Value::Object(scan_results))
                .map_err(|err| format!("failed to render JSON: {err}"))?;
            json_stdout.push('\n');
            stdout_text.push_str(&json_stdout);
        }
        Ok(stdout(&stdout_text))
    }

    fn transfer(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_remote_transfer_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let _relay_mode = args.relay;
        let (src_name, src_path) = match parse_device_path(&args.source) {
            Ok(value) => value,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let (dst_name, dst_path) = match parse_device_path(&args.dest) {
            Ok(value) => value,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let src = match self.get_device(&src_name) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let mut dst = match self.get_device(&dst_name) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.dest_host {
            dst.host = host;
        }
        let src_type = ssh_exec(
            &src,
            &format!("test -d {} && echo DIR || echo FILE", sh_quote(&src_path)),
            Duration::from_secs(10),
            false,
            false,
        );
        if matches!(src_type, Ok(run) if run.output.trim() == "DIR") {
            return Err(NATIVE_FALLBACK.to_string());
        }
        let tmp = std::env::temp_dir().join(format!("rpty-transfer-{}", make_job_id()));
        let size = match sftp_get(&src, &src_path, &tmp) {
            Ok(size) => size,
            Err(err) => return Ok(cli_error(format!("Error: {err}"), 1)),
        };
        if let Err(err) = sftp_put(&dst, &tmp, &dst_path) {
            let _ = std::fs::remove_file(&tmp);
            return Ok(cli_error(format!("Error: {err}"), 1));
        }
        let _ = std::fs::remove_file(&tmp);
        let src_hash = remote_md5(&src, &src_path).ok().flatten();
        let dst_hash = remote_md5(&dst, &dst_path).ok().flatten();
        let mut stderr = String::new();
        if let (Some(src_hash), Some(dst_hash)) = (src_hash, dst_hash) {
            if src_hash == dst_hash {
                stderr.push_str(&format!("  verify: OK (md5: {src_hash})\n"));
            } else {
                return Ok(NativeOutput {
                    stdout: String::new(),
                    stderr: format!("  verify: FAILED! src={src_hash} dst={dst_hash}\n"),
                    code: 1,
                });
            }
        } else {
            stderr.push_str("  verify: SKIP (md5 unavailable on remote)\n");
        }
        Ok(NativeOutput {
            stdout: format!(
                "{}:{} -> {}:{} ({})\n",
                src_name,
                src_path,
                dst_name,
                dst_path,
                human_size(size)
            ),
            stderr,
            code: 0,
        })
    }

    fn wsl(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_wsl_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let target = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        let Some(gateway_name) = &target.gateway else {
            return Ok(cli_error(
                format!(
                    "Error: device '{}' has no 'gateway' field in devices.json",
                    target.name
                ),
                1,
            ));
        };
        let gateway = match self.get_device(gateway_name) {
            Ok(device) => device,
            Err(_) => {
                return Ok(cli_error(
                    format!("Error: gateway device '{gateway_name}' not found in devices.json"),
                    1,
                ))
            }
        };
        let distro = args
            .distro
            .or_else(|| target.wsl_distro.clone())
            .unwrap_or_default();
        let distro_flag = if distro.is_empty() {
            String::new()
        } else {
            format!("-d {}", sh_quote(&distro))
        };
        match args.action.as_str() {
            "status" => {
                let run =
                    match ssh_exec(&gateway, "wsl -l -v", Duration::from_secs(30), false, true) {
                        Ok(run) => run,
                        Err(err) => {
                            return Ok(cli_error(
                                format!("Error connecting to gateway {gateway_name}: {err}"),
                                1,
                            ))
                        }
                    };
                if run.success {
                    Ok(stdout(&format!(
                        "[fleet] Querying WSL state via {} ({})...\n{}\n",
                        gateway.name, gateway.host, run.output
                    )))
                } else {
                    Ok(cli_error(
                        format!("Error connecting to gateway {gateway_name}: {}", run.output),
                        1,
                    ))
                }
            }
            "exec" => {
                if args.command.is_empty() {
                    return Ok(cli_error(
                        "Error: no command specified. Usage: fleet wsl <device> exec -- <cmd>"
                            .to_string(),
                        1,
                    ));
                }
                let inner = shlex_join(&args.command);
                let wsl_cmd = format!("wsl {distro_flag} -e bash -lc {}", win_quote(&inner));
                let run = match ssh_exec(
                    &gateway,
                    &wsl_cmd,
                    Duration::from_secs(args.timeout),
                    false,
                    true,
                ) {
                    Ok(run) => run,
                    Err(err) => return Ok(cli_error(err, 1)),
                };
                Ok(NativeOutput {
                    stdout: format!(
                        "[fleet] Running via {} -> WSL: {}\n{}\n",
                        gateway.name, wsl_cmd, run.output
                    ),
                    stderr: String::new(),
                    code: if run.success { 0 } else { 1 },
                })
            }
            "restart" => Err(NATIVE_FALLBACK.to_string()),
            other => Ok(cli_error(
                format!("Error: unknown WSL action '{other}'. Use: status | restart | exec"),
                1,
            )),
        }
    }

    fn work_sync(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_work_sync_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        if !(args.push ^ args.pull) {
            return Ok(cli_error(
                "Error: must specify --push or --pull".to_string(),
                1,
            ));
        }
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let mut command = Vec::<String>::new();
        if !device.password.is_empty() && command_exists("sshpass") {
            command.extend([
                "sshpass".to_string(),
                "-p".to_string(),
                device.password.clone(),
            ]);
        }
        command.extend([
            "rsync".to_string(),
            "-az".to_string(),
            "--info=progress2".to_string(),
            "--stats".to_string(),
        ]);
        if device.port != 22 {
            command.extend(["-e".to_string(), format!("ssh -p {}", device.port)]);
        }
        for exclude in EXCLUDE_ALWAYS {
            command.extend(["--exclude".to_string(), exclude.to_string()]);
        }
        let gitignore = Path::new(&args.local).join(".gitignore");
        if let Ok(content) = std::fs::read_to_string(gitignore) {
            for line in content.lines().map(str::trim) {
                if !line.is_empty() && !line.starts_with('#') {
                    command.extend(["--exclude".to_string(), line.to_string()]);
                }
            }
        }
        if args.dry_run {
            command.push("--dry-run".to_string());
            command.push("--itemize-changes".to_string());
        }
        let local = args.local.trim_end_matches('/');
        let remote = args.remote.trim_end_matches('/');
        if args.push {
            command.push(format!("{local}/"));
            command.push(format!("{}@{}:{remote}/", device.user, device.host));
        } else {
            command.push(format!("{}@{}:{remote}/", device.user, device.host));
            command.push(format!("{local}/"));
        }
        let program = command.remove(0);
        let output = std::process::Command::new(&program)
            .args(&command)
            .output()
            .map_err(|err| format!("failed to run {program}: {err}"))?;
        Ok(NativeOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            code: output.status.code().unwrap_or(1),
        })
    }

    fn ssh(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_ssh_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let mut stderr = String::new();
        let mut command = Vec::<String>::new();
        if !device.password.is_empty() && command_exists("sshpass") {
            command.extend([
                "sshpass".to_string(),
                "-p".to_string(),
                device.password.clone(),
            ]);
        } else if !device.password.is_empty() {
            stderr.push_str(
                "Tip: install sshpass for auto-login; password is configured but hidden.\n",
            );
        }
        command.push("ssh".to_string());
        command.extend([
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            "UserKnownHostsFile=/dev/null".to_string(),
        ]);
        if device.port != 22 {
            command.extend(["-p".to_string(), device.port.to_string()]);
        }
        command.push(format!("{}@{}", device.user, device.host));
        let program = command.remove(0);
        let status = std::process::Command::new(&program)
            .args(&command)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .map_err(|err| format!("failed to run {program}: {err}"))?;
        Ok(NativeOutput {
            stdout: String::new(),
            stderr,
            code: status.code().unwrap_or(1),
        })
    }

    fn work_enter(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_work_enter_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let basename = args
            .remote_dir
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("work");
        let session = args.session.unwrap_or_else(|| format!("claude-{basename}"));
        let check = format!(
            "tmux has-session -t {} 2>/dev/null && echo EXISTS || echo NEW",
            sh_quote(&session)
        );
        let exists = ssh_exec(&device, &check, Duration::from_secs(10), false, false)
            .map(|run| run.output.contains("EXISTS"))
            .unwrap_or(false);
        let remote_cmd = if exists {
            format!("tmux attach -t {}", sh_quote(&session))
        } else {
            format!(
                "cd {} && tmux new-session -s {} claude",
                sh_quote(&args.remote_dir),
                sh_quote(&session)
            )
        };
        run_system_ssh(&device, Some(&remote_cmd), true)
    }

    fn work_monitor(&self, args: &[String]) -> Result<NativeOutput, String> {
        let args = match parse_work_monitor_args(args) {
            Ok(args) => args,
            Err(err) => return Ok(cli_error(err, 2)),
        };
        let mut device = match self.get_device(&args.device) {
            Ok(device) => device,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if let Some(host) = args.host {
            device.host = host;
        }
        let script = format!(
            "#!/bin/sh\nSESSION={}\nON_EXIT={}\nwhile tmux has-session -t \"$SESSION\" 2>/dev/null; do sleep 2; done\necho \"Session $SESSION detached, executing: $ON_EXIT\"\neval \"$ON_EXIT\"\n",
            sh_quote(&args.session),
            sh_quote(&args.on_exit)
        );
        let script_path = format!("/tmp/monitor-{}.sh", args.session);
        let command = format!(
            "cat > {} << 'MONITOR_EOF'\n{}MONITOR_EOF\nchmod +x {} && nohup {} > /tmp/monitor-{}.log 2>&1 &",
            sh_quote(&script_path),
            script,
            sh_quote(&script_path),
            sh_quote(&script_path),
            args.session
        );
        let run = match ssh_exec(&device, &command, Duration::from_secs(10), false, false) {
            Ok(run) => run,
            Err(err) => return Ok(cli_error(err, 1)),
        };
        if run.success {
            Ok(stdout(&format!(
                "Monitor started for session '{}' on {}\nOn exit will run: {}\n",
                args.session, args.device, args.on_exit
            )))
        } else {
            Ok(cli_error(format!("Error: {}", run.output), 1))
        }
    }
}

fn parse_list_args(args: &[String]) -> Result<ListArgs, String> {
    let mut parsed = ListArgs {
        tags: Vec::new(),
        owner: None,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => {
                index += 1;
                let tag = args
                    .get(index)
                    .ok_or_else(|| "fleet list: --tag requires a value".to_string())?;
                parsed.tags.push(tag.clone());
            }
            "--owner" => {
                index += 1;
                let owner = args
                    .get(index)
                    .ok_or_else(|| "fleet list: --owner requires a value".to_string())?;
                match owner.as_str() {
                    "personal" | "company" => parsed.owner = Some(owner.clone()),
                    _ => {
                        return Err("fleet list: --owner must be either 'personal' or 'company'"
                            .to_string())
                    }
                }
            }
            "--json" => parsed.json = true,
            other => return Err(format!("fleet list: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_status_args(args: &[String]) -> Result<StatusArgs, String> {
    let mut parsed = StatusArgs {
        device: None,
        tags: Vec::new(),
        owner: None,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => {
                index += 1;
                parsed.tags.push(required_value(
                    args,
                    index,
                    "fleet status: --tag requires a value",
                )?);
            }
            "--owner" => {
                index += 1;
                let owner = required_value(args, index, "fleet status: --owner requires a value")?;
                validate_owner(&owner, "fleet status")?;
                parsed.owner = Some(owner);
            }
            "--json" => parsed.json = true,
            value if parsed.device.is_none() => parsed.device = Some(value.to_string()),
            other => return Err(format!("fleet status: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_match_args(args: &[String]) -> Result<MatchArgs, String> {
    let mut parsed = MatchArgs {
        tags: Vec::new(),
        owner: None,
        sort: None,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => {
                index += 1;
                parsed.tags.push(required_value(
                    args,
                    index,
                    "fleet match: --tag requires a value",
                )?);
            }
            "--owner" => {
                index += 1;
                let owner = required_value(args, index, "fleet match: --owner requires a value")?;
                validate_owner(&owner, "fleet match")?;
                parsed.owner = Some(owner);
            }
            "--sort" => {
                index += 1;
                let sort = required_value(args, index, "fleet match: --sort requires a value")?;
                if !matches!(sort.as_str(), "disk" | "memory" | "cpu") {
                    return Err("fleet match: --sort must be disk, memory, or cpu".to_string());
                }
                parsed.sort = Some(sort);
            }
            "--json" => parsed.json = true,
            other => return Err(format!("fleet match: unknown argument: {other}")),
        }
        index += 1;
    }
    if parsed.tags.is_empty() {
        return Err("fleet match: --tag is required".to_string());
    }
    Ok(parsed)
}

fn parse_exec_args(args: &[String]) -> Result<ExecArgs, String> {
    let mut parsed = ExecArgs {
        tags: Vec::new(),
        sudo: false,
        timeout: 60,
        host: None,
        json: false,
        literal: false,
        stream: false,
        detach: false,
        raw: false,
        device: String::new(),
        command: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => {
                index += 1;
                parsed.tags.push(required_value(
                    args,
                    index,
                    "fleet exec: --tag requires a value",
                )?);
            }
            "--sudo" => parsed.sudo = true,
            "--timeout" => {
                index += 1;
                parsed.timeout =
                    required_value(args, index, "fleet exec: --timeout requires a value")?
                        .parse()
                        .map_err(|_| "fleet exec: --timeout must be an integer".to_string())?;
            }
            "--host" => {
                index += 1;
                parsed.host = Some(required_value(
                    args,
                    index,
                    "fleet exec: --host requires a value",
                )?);
            }
            "--json" => parsed.json = true,
            "--literal" => parsed.literal = true,
            "--stream" => parsed.stream = true,
            "--detach" => parsed.detach = true,
            "--raw" => parsed.raw = true,
            value => {
                parsed.device = value.to_string();
                index += 1;
                if index < args.len() && args[index] == "--" {
                    index += 1;
                }
                parsed.command = args[index..].to_vec();
                break;
            }
        }
        index += 1;
    }
    if parsed.device.is_empty() {
        return Err("fleet exec: device is required".to_string());
    }
    if parsed.command.is_empty() {
        return Err(
            "Error: no command specified. Usage: fleet exec <device> -- <command>".to_string(),
        );
    }
    if parsed.command.first().map(String::as_str) == Some("sudo") && !parsed.sudo {
        return Err(format!(
            "Error: don't prefix the command with 'sudo' - non-interactive SSH can't prompt for a password.\n       Use the --sudo flag instead, which auto-injects the device password:\n           fleet exec --sudo {} -- {}",
            parsed.device,
            parsed.command.iter().skip(1).cloned().collect::<Vec<_>>().join(" ")
        ));
    }
    if parsed.stream && parsed.sudo {
        return Err(
            "Error: --stream is not compatible with --sudo (sudo path already streams via PTY)."
                .to_string(),
        );
    }
    if parsed.stream && parsed.json {
        return Err("Error: --stream is not compatible with --json.".to_string());
    }
    if parsed.detach && parsed.stream {
        return Err("Error: --detach is not compatible with --stream.".to_string());
    }
    Ok(parsed)
}

fn parse_device_json_args(command: &str, args: &[String]) -> Result<DeviceJsonArgs, String> {
    let mut host = None;
    let mut json = false;
    let mut device = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    &format!("fleet {command}: --host requires a value"),
                )?);
            }
            "--json" => json = true,
            value if device.is_none() => device = Some(value.to_string()),
            other => return Err(format!("fleet {command}: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(DeviceJsonArgs {
        host,
        device: device.ok_or_else(|| format!("fleet {command}: device is required"))?,
        json,
    })
}

fn parse_log_args(args: &[String]) -> Result<LogArgs, String> {
    let mut host = None;
    let mut device = None;
    let mut job_id = None;
    let mut tail = 50;
    let mut follow = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet log: --host requires a value",
                )?);
            }
            "--tail" => {
                index += 1;
                tail = required_value(args, index, "fleet log: --tail requires a value")?
                    .parse()
                    .map_err(|_| "fleet log: --tail must be an integer".to_string())?;
            }
            "--follow" | "-f" => follow = true,
            value if device.is_none() => device = Some(value.to_string()),
            value if job_id.is_none() => job_id = Some(value.to_string()),
            other => return Err(format!("fleet log: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(LogArgs {
        host,
        device: device.ok_or_else(|| "fleet log: device is required".to_string())?,
        job_id: job_id.ok_or_else(|| "fleet log: job ID is required".to_string())?,
        tail,
        follow,
    })
}

fn parse_kill_args(args: &[String]) -> Result<KillArgs, String> {
    let mut host = None;
    let mut sudo = false;
    let mut force = false;
    let mut device = None;
    let mut job_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet kill-job: --host requires a value",
                )?);
            }
            "--sudo" => sudo = true,
            "--force" | "-9" => force = true,
            value if device.is_none() => device = Some(value.to_string()),
            value if job_id.is_none() => job_id = Some(value.to_string()),
            other => return Err(format!("fleet kill-job: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(KillArgs {
        host,
        sudo,
        device: device.ok_or_else(|| "fleet kill-job: device is required".to_string())?,
        job_id: job_id.ok_or_else(|| "fleet kill-job: job ID is required".to_string())?,
        force,
    })
}

fn parse_transfer_args(command: &str, args: &[String]) -> Result<TransferArgs, String> {
    let mut host = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    &format!("fleet {command}: --host requires a value"),
                )?);
            }
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 3 {
        return Err(format!("fleet {command}: expected <device> <src> <dst>"));
    }
    Ok(TransferArgs {
        host,
        device: positionals[0].clone(),
        first_path: positionals[1].clone(),
        second_path: positionals[2].clone(),
    })
}

fn parse_add_args(args: &[String]) -> Result<AddArgs, String> {
    let mut positionals = Vec::new();
    let mut user = "root".to_string();
    let mut password = None;
    let mut owner = "company".to_string();
    let mut tags = Vec::new();
    let mut description = String::new();
    let mut scan = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--user" | "-u" => {
                index += 1;
                user = required_value(args, index, "fleet add: --user requires a value")?;
            }
            "--password" | "-p" => {
                index += 1;
                password = Some(required_value(
                    args,
                    index,
                    "fleet add: --password requires a value",
                )?);
            }
            "--owner" => {
                index += 1;
                owner = required_value(args, index, "fleet add: --owner requires a value")?;
                validate_owner(&owner, "fleet add")?;
            }
            "--tag" => {
                index += 1;
                tags.push(required_value(
                    args,
                    index,
                    "fleet add: --tag requires a value",
                )?);
            }
            "--desc" => {
                index += 1;
                description = required_value(args, index, "fleet add: --desc requires a value")?;
            }
            "--scan" => scan = true,
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 2 {
        return Err("fleet add: expected <name> <host>".to_string());
    }
    Ok(AddArgs {
        name: positionals[0].clone(),
        host: positionals[1].clone(),
        user,
        password,
        owner,
        tags,
        description,
        scan,
    })
}

fn parse_remove_args(args: &[String]) -> Result<RemoveArgs, String> {
    let mut name = None;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            value if name.is_none() => name = Some(value.to_string()),
            other => return Err(format!("fleet remove: unknown argument: {other}")),
        }
    }
    Ok(RemoveArgs {
        name: name.ok_or_else(|| "fleet remove: name is required".to_string())?,
        force,
    })
}

fn parse_bootstrap_args(args: &[String]) -> Result<BootstrapArgs, String> {
    let mut parsed = BootstrapArgs {
        device: None,
        all: false,
        tags: Vec::new(),
        profile: None,
        check: false,
        force: false,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => parsed.all = true,
            "--tag" => {
                index += 1;
                parsed.tags.push(required_value(
                    args,
                    index,
                    "fleet bootstrap: --tag requires a value",
                )?);
            }
            "--profile" => {
                index += 1;
                let profile =
                    required_value(args, index, "fleet bootstrap: --profile requires a value")?;
                if !matches!(profile.as_str(), "wsl2-proxy" | "edge-mirror" | "isolated") {
                    return Err(
                        "fleet bootstrap: --profile must be wsl2-proxy, edge-mirror, or isolated"
                            .to_string(),
                    );
                }
                parsed.profile = Some(profile);
            }
            "--check" => parsed.check = true,
            "--force" => parsed.force = true,
            "--json" => parsed.json = true,
            value if parsed.device.is_none() => parsed.device = Some(value.to_string()),
            other => return Err(format!("fleet bootstrap: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_scan_args(args: &[String]) -> Result<ScanArgs, String> {
    let mut parsed = ScanArgs {
        device: None,
        tags: Vec::new(),
        dry_run: false,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => {
                index += 1;
                parsed.tags.push(required_value(
                    args,
                    index,
                    "fleet scan: --tag requires a value",
                )?);
            }
            "--dry-run" => parsed.dry_run = true,
            "--json" => parsed.json = true,
            value if parsed.device.is_none() => parsed.device = Some(value.to_string()),
            other => return Err(format!("fleet scan: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_remote_transfer_args(args: &[String]) -> Result<RemoteTransferArgs, String> {
    let mut relay = false;
    let mut dest_host = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--relay" => relay = true,
            "--dest-host" => {
                index += 1;
                dest_host = Some(required_value(
                    args,
                    index,
                    "fleet transfer: --dest-host requires a value",
                )?);
            }
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 2 {
        return Err("fleet transfer: expected <src-device>:<path> <dst-device>:<path>".to_string());
    }
    Ok(RemoteTransferArgs {
        source: positionals[0].clone(),
        dest: positionals[1].clone(),
        relay,
        dest_host,
    })
}

fn parse_device_path(value: &str) -> Result<(String, String), String> {
    let Some((device, path)) = value.split_once(':') else {
        return Err(format!(
            "invalid device path '{value}', expected <device>:<path>"
        ));
    };
    if device.is_empty() || path.is_empty() {
        return Err(format!(
            "invalid device path '{value}', expected <device>:<path>"
        ));
    }
    Ok((device.to_string(), path.to_string()))
}

fn parse_wsl_args(args: &[String]) -> Result<WslArgs, String> {
    let mut device = None;
    let mut action = None;
    let mut distro = None;
    let mut timeout = 60;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--distro" => {
                index += 1;
                distro = Some(required_value(
                    args,
                    index,
                    "fleet wsl: --distro requires a value",
                )?);
            }
            "--timeout" => {
                index += 1;
                timeout = required_value(args, index, "fleet wsl: --timeout requires a value")?
                    .parse()
                    .map_err(|_| "fleet wsl: --timeout must be an integer".to_string())?;
            }
            value if device.is_none() => device = Some(value.to_string()),
            value if action.is_none() => {
                action = Some(value.to_string());
                index += 1;
                if index < args.len() && args[index] == "--" {
                    index += 1;
                }
                return Ok(WslArgs {
                    device: device.unwrap(),
                    action: action.unwrap(),
                    distro,
                    timeout,
                    command: args[index..].to_vec(),
                });
            }
            other => return Err(format!("fleet wsl: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(WslArgs {
        device: device.ok_or_else(|| "fleet wsl: device is required".to_string())?,
        action: action.unwrap_or_else(|| "status".to_string()),
        distro,
        timeout,
        command: Vec::new(),
    })
}

fn parse_work_sync_args(args: &[String]) -> Result<WorkSyncArgs, String> {
    let mut host = None;
    let mut push = false;
    let mut pull = false;
    let mut dry_run = false;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet work-sync: --host requires a value",
                )?);
            }
            "--push" => push = true,
            "--pull" => pull = true,
            "--dry-run" => dry_run = true,
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 3 {
        return Err("fleet work-sync: expected <device> <local> <remote>".to_string());
    }
    Ok(WorkSyncArgs {
        host,
        device: positionals[0].clone(),
        local: positionals[1].clone(),
        remote: positionals[2].clone(),
        push,
        pull,
        dry_run,
    })
}

fn parse_ssh_args(args: &[String]) -> Result<SshArgs, String> {
    let mut host = None;
    let mut device = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet ssh: --host requires a value",
                )?);
            }
            value if device.is_none() => device = Some(value.to_string()),
            other => return Err(format!("fleet ssh: unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(SshArgs {
        host,
        device: device.ok_or_else(|| "fleet ssh: device is required".to_string())?,
    })
}

fn parse_work_enter_args(args: &[String]) -> Result<WorkEnterArgs, String> {
    let mut host = None;
    let mut session = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet work-enter: --host requires a value",
                )?);
            }
            "--session" => {
                index += 1;
                session = Some(required_value(
                    args,
                    index,
                    "fleet work-enter: --session requires a value",
                )?);
            }
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 2 {
        return Err("fleet work-enter: expected <device> <remote-dir>".to_string());
    }
    Ok(WorkEnterArgs {
        host,
        device: positionals[0].clone(),
        remote_dir: positionals[1].clone(),
        session,
    })
}

fn parse_work_monitor_args(args: &[String]) -> Result<WorkMonitorArgs, String> {
    let mut host = None;
    let mut on_exit = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host" => {
                index += 1;
                host = Some(required_value(
                    args,
                    index,
                    "fleet work-monitor: --host requires a value",
                )?);
            }
            "--on-exit" => {
                index += 1;
                on_exit = Some(required_value(
                    args,
                    index,
                    "fleet work-monitor: --on-exit requires a value",
                )?);
            }
            value => positionals.push(value.to_string()),
        }
        index += 1;
    }
    if positionals.len() != 2 {
        return Err(
            "fleet work-monitor: expected <device> <session> --on-exit <command>".to_string(),
        );
    }
    Ok(WorkMonitorArgs {
        host,
        device: positionals[0].clone(),
        session: positionals[1].clone(),
        on_exit: on_exit.ok_or_else(|| "fleet work-monitor: --on-exit is required".to_string())?,
    })
}

fn required_value(args: &[String], index: usize, message: &str) -> Result<String, String> {
    args.get(index).cloned().ok_or_else(|| message.to_string())
}

fn validate_owner(owner: &str, command: &str) -> Result<(), String> {
    if matches!(owner, "personal" | "company") {
        Ok(())
    } else {
        Err(format!(
            "{command}: --owner must be either 'personal' or 'company'"
        ))
    }
}

fn load_devices(path: &Path) -> Result<Map<String, Value>, String> {
    Ok(load_inventory(path, true)?
        .into_iter()
        .map(|device| (device.name, device.value))
        .collect())
}

fn load_inventory(path: &Path, mask_passwords: bool) -> Result<Vec<Device>, String> {
    let value = load_inventory_root(path)?;
    let devices = value
        .get("devices")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "{} must contain a top-level 'devices' object",
                path.display()
            )
        })?;

    devices_from_object(devices, mask_passwords)
}

fn load_inventory_root(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        let example = path.with_file_name("devices.example.json");
        let mut message = format!("Error: {} not found", path.display());
        if example.exists() {
            message.push_str(&format!(
                "\nCopy {} to {} and fill in your devices.",
                example.display(),
                path.display()
            ));
            message.push_str("\nOr set FLEET_DEVICES_FILE=/path/to/devices.json.");
        }
        return Err(message);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str::<Value>(&content)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn save_inventory_root(path: &Path, root: &Value) -> Result<(), String> {
    let content = format!(
        "{}\n",
        serde_json::to_string_pretty(root)
            .map_err(|err| format!("failed to render {}: {err}", path.display()))?
    );
    std::fs::write(path, content)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn devices_from_object(
    devices: &Map<String, Value>,
    mask_passwords: bool,
) -> Result<Vec<Device>, String> {
    let mut parsed = Vec::new();
    for (name, dev) in devices {
        let mut dev = dev.clone();
        let object = dev
            .as_object()
            .ok_or_else(|| format!("device '{name}' must be a JSON object"))?;
        let password = object
            .get("password")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let tags = object
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let port = object
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(22);
        let host = object
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let user = object
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let owner = object
            .get("owner")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let gateway = object
            .get("gateway")
            .and_then(Value::as_str)
            .map(str::to_string);
        let wsl_distro = object
            .get("wsl_distro")
            .and_then(Value::as_str)
            .map(str::to_string);
        if mask_passwords {
            if let Some(object) = dev.as_object_mut() {
                if !password.is_empty() {
                    object.insert("password".to_string(), Value::String("***".to_string()));
                }
            }
        }
        parsed.push(Device {
            name: name.clone(),
            value: dev,
            host,
            user,
            password,
            port,
            tags,
            owner,
            description,
            gateway,
            wsl_distro,
        });
    }
    Ok(parsed)
}

fn filter_devices(devices: &mut Map<String, Value>, args: &ListArgs) {
    devices.retain(|_, dev| {
        let owner_ok = args
            .owner
            .as_ref()
            .map(|owner| dev.get("owner").and_then(Value::as_str) == Some(owner.as_str()))
            .unwrap_or(true);
        let tags_ok = args.tags.iter().all(|tag| {
            dev.get("tags")
                .and_then(Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .any(|value| value.as_str() == Some(tag.as_str()))
                })
                .unwrap_or(false)
        });
        owner_ok && tags_ok
    });
}

fn string_field(dev: &Value, key: &str) -> String {
    dev.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn tags_field(dev: &Value) -> String {
    dev.get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn ssh_exec(
    device: &Device,
    command: &str,
    timeout: Duration,
    sudo: bool,
    raw: bool,
) -> Result<RemoteRun, String> {
    if sudo && device.is_windows() {
        return Ok(RemoteRun {
            success: false,
            output: "[fleet] --sudo is not supported on Windows devices (no sudo/PTY semantics). Run an elevated command via an admin SSH account or Start-Process -Verb RunAs.".to_string(),
        });
    }
    let session = ssh_session(device, timeout)?;
    let mut channel = session
        .channel_session()
        .map_err(|err| format!("failed to open SSH channel: {err}"))?;
    let wrapped = if sudo {
        channel
            .request_pty("xterm", None, Some((80, 24, 0, 0)))
            .map_err(|err| format!("failed to request PTY: {err}"))?;
        format!(
            "sudo -S -p '' env DEBIAN_FRONTEND=noninteractive PATH=\"$HOME/.local/bin:$PATH\" {command}"
        )
    } else if raw || device.is_windows() {
        command.to_string()
    } else {
        format!(
            "export PATH=\"$HOME/.local/bin:$PATH\"; [ -f \"$HOME/.profile.d/mirrors.sh\" ] && . \"$HOME/.profile.d/mirrors.sh\"; {command}"
        )
    };

    channel
        .exec(&wrapped)
        .map_err(|err| format!("failed to execute remote command: {err}"))?;
    if sudo {
        channel
            .write_all(format!("{}\n", device.password).as_bytes())
            .map_err(|err| format!("failed to send sudo password: {err}"))?;
        let _ = channel.send_eof();
    }

    let mut stdout_bytes = Vec::new();
    channel
        .read_to_end(&mut stdout_bytes)
        .map_err(|err| format!("failed to read remote stdout: {err}"))?;
    let mut stderr_bytes = Vec::new();
    channel
        .stderr()
        .read_to_end(&mut stderr_bytes)
        .map_err(|err| format!("failed to read remote stderr: {err}"))?;
    channel
        .wait_close()
        .map_err(|err| format!("failed to close remote channel: {err}"))?;
    let exit = channel.exit_status().unwrap_or(1);
    let mut output = decode_remote(&stdout_bytes);
    let stderr = decode_remote(&stderr_bytes);
    if exit != 0 && !stderr.trim().is_empty() {
        if output.trim().is_empty() {
            output = stderr;
        } else {
            output = format!("{}\n{}", stderr.trim(), output.trim());
        }
    }
    if sudo {
        output = output
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.starts_with("[sudo]") && trimmed != device.password
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let output = output.trim().to_string();
    if exit != 0 {
        let mut detail = if output.is_empty() {
            format!("exit code {exit}")
        } else {
            output
        };
        let lower = detail.to_lowercase();
        if !sudo
            && (lower.contains("permission denied")
                || lower.contains("not allowed")
                || lower.contains("are you root"))
        {
            detail.push_str("\n[fleet] Hint: this command likely needs --sudo. Retry with: fleet exec --sudo <device> -- <command>");
        }
        return Ok(RemoteRun {
            success: false,
            output: format!("[fleet] command failed (exit {exit}): {detail}"),
        });
    }
    Ok(RemoteRun {
        success: true,
        output,
    })
}

#[cfg(windows)]
fn ssh_exec(
    device: &Device,
    command: &str,
    timeout: Duration,
    sudo: bool,
    raw: bool,
) -> Result<RemoteRun, String> {
    win_ssh::exec(device, command, timeout, sudo, raw)
}

#[cfg(not(windows))]
fn ssh_session(device: &Device, timeout: Duration) -> Result<ssh2::Session, String> {
    let addr = format!("{}:{}", device.host, device.port);
    let socket = addr
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve {addr}: {err}"))?
        .next()
        .ok_or_else(|| format!("failed to resolve {addr}"))?;
    let tcp = TcpStream::connect_timeout(&socket, timeout)
        .map_err(|err| format!("failed to connect to {addr}: {err}"))?;
    tcp.set_read_timeout(Some(timeout)).ok();
    tcp.set_write_timeout(Some(timeout)).ok();
    let mut session =
        ssh2::Session::new().map_err(|err| format!("failed to create SSH session: {err}"))?;
    session.set_tcp_stream(tcp);
    session.set_timeout(timeout.as_millis().min(u128::from(u32::MAX)) as u32);
    session
        .handshake()
        .map_err(|err| format!("SSH handshake failed for {addr}: {err}"))?;

    if !device.password.is_empty() {
        if session
            .userauth_password(&device.user, &device.password)
            .is_ok()
            && session.authenticated()
        {
            return Ok(session);
        }
    }
    if session.userauth_agent(&device.user).is_ok() && session.authenticated() {
        return Ok(session);
    }
    for key in ["id_ed25519", "id_rsa"] {
        let path = home_dir().join(".ssh").join(key);
        if path.exists()
            && session
                .userauth_pubkey_file(&device.user, None, &path, None)
                .is_ok()
            && session.authenticated()
        {
            return Ok(session);
        }
    }
    Err(format!(
        "authentication failed for {}@{}:{}",
        device.user, device.host, device.port
    ))
}

fn probe_device(device: &Device) -> Value {
    let command = "echo '---DISK---' && df -h / | tail -1 && echo '---MEM---' && free -m | awk 'NR==2{print}' && echo '---CPU---' && uptime && echo '---GPU---' && (nvidia-smi --query-gpu=name,memory.used,memory.total,utilization.gpu --format=csv,noheader,nounits 2>/dev/null || echo 'N/A')";
    let mut result = json!({
        "name": device.name,
        "host": device.host,
        "online": false,
        "tags": device.tags,
        "description": device.description,
        "gateway": device.gateway,
        "wsl_distro": device.wsl_distro,
    });
    let Ok(run) = ssh_exec(device, command, Duration::from_secs(10), false, false) else {
        return result;
    };
    if !run.success {
        return result;
    }
    result["online"] = Value::Bool(true);
    let mut current = "";
    let mut sections: Map<String, Value> = Map::new();
    for line in run.output.lines() {
        if line.starts_with("---") && line.ends_with("---") {
            current = line.trim_matches('-');
            sections.insert(current.to_string(), Value::Array(Vec::new()));
        } else if !current.is_empty() {
            if let Some(values) = sections.get_mut(current).and_then(Value::as_array_mut) {
                values.push(Value::String(line.to_string()));
            }
        }
    }
    if let Some(line) = section_line(&sections, "DISK") {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 4 {
            result["disk"] = json!({
                "total": parts[1],
                "used": parts[2],
                "avail": parts[3],
                "use_pct": parts.get(4).copied().unwrap_or("")
            });
        }
    }
    if let Some(line) = section_line(&sections, "MEM") {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 3 {
            result["memory"] = json!({
                "total_mb": parts[1],
                "used_mb": parts[2],
                "free_mb": parts.get(3).copied().unwrap_or("")
            });
        }
    }
    if let Some(line) = section_line(&sections, "CPU") {
        if let Some((_, load)) = line.split_once("load average:") {
            result["cpu_load"] = Value::String(load.trim().to_string());
        }
    }
    if let Some(line) = section_line(&sections, "GPU") {
        if line != "N/A" {
            let parts = line.split(", ").collect::<Vec<_>>();
            if parts.len() >= 4 {
                result["gpu"] = json!({
                    "name": parts[0],
                    "mem_used_mb": parts[1],
                    "mem_total_mb": parts[2],
                    "util_pct": parts[3],
                });
            }
        }
    }
    result
}

fn scan_device_specs(device: &Device) -> Result<Value, String> {
    let command = "echo '---ARCH---' && uname -m && echo '---OS---' && (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\" || echo \"macOS $(sw_vers -productVersion 2>/dev/null || echo unknown)\") && echo '---MODEL---' && (cat /proc/device-tree/model 2>/dev/null || system_profiler SPHardwareDataType 2>/dev/null | grep 'Model Name\\|Chip' | head -2 || echo 'N/A') && echo '---CPU---' && (nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'N/A') && echo '---MEM---' && (if command -v free >/dev/null 2>&1; then free -b | awk 'NR==2{print $2}'; else sysctl -n hw.memsize 2>/dev/null || echo 'N/A'; fi) && echo '---DISK---' && (if df -B1 / >/dev/null 2>&1; then df -B1 / | tail -1 | awk '{print $2}'; else df -k / | tail -1 | awk '{print $2 * 1024}'; fi) && echo '---GPU---' && (nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits 2>/dev/null || echo 'N/A') && echo '---ACCEL---' && (ls /dev/hailo* 2>/dev/null && hailortcli fw-control identify 2>/dev/null | grep 'Board Name\\|Device Architecture' || echo 'N/A') && echo '---NET---' && (curl -s --connect-timeout 3 --max-time 5 -o /dev/null -w '%{http_code}' https://pypi.org 2>/dev/null || echo '0') && echo '---HOSTNAME---' && hostname";
    let run = ssh_exec(device, command, Duration::from_secs(10), false, false)
        .map_err(|err| format!("offline: {err}"))?;
    if !run.success {
        return Err(format!("offline: {}", run.output));
    }
    let sections = parse_sections(&run.output);
    let mut specs = Map::new();
    let mut tags = Vec::<String>::new();

    if let Some(arch) = section_line(&sections, "ARCH").map(|v| v.trim().to_string()) {
        specs.insert("arch".to_string(), Value::String(arch.clone()));
        if matches!(arch.as_str(), "aarch64" | "arm64") {
            push_tag(&mut tags, "arm64");
        } else if matches!(arch.as_str(), "x86_64" | "amd64") {
            push_tag(&mut tags, "x86_64");
        }
    }
    if let Some(os) = section_line(&sections, "OS").map(|v| v.trim().to_string()) {
        specs.insert("os".to_string(), Value::String(os.clone()));
        if os.to_lowercase().contains("macos") || os == "Darwin" {
            push_tag(&mut tags, "macos");
        }
    }
    if let Some(model_raw) = section_line(&sections, "MODEL") {
        let raw_model = model_raw.split('\0').next().unwrap_or("").trim();
        let mut model = raw_model.to_string();
        if raw_model.contains(':') {
            let mut parts = Map::new();
            if let Some(lines) = sections.get("MODEL").and_then(Value::as_array) {
                for line in lines.iter().filter_map(Value::as_str) {
                    if let Some((key, value)) = line.split_once(':') {
                        parts.insert(
                            key.trim().to_string(),
                            Value::String(value.trim().to_string()),
                        );
                    }
                }
            }
            model = parts
                .get("Model Name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(chip) = parts.get("Chip").and_then(Value::as_str) {
                specs.insert("cpu".to_string(), Value::String(chip.to_string()));
                push_tag(&mut tags, &chip.to_lowercase().replace(' ', "-"));
            }
        }
        if !model.is_empty() && model != "N/A" {
            specs.insert("model".to_string(), Value::String(model.clone()));
            let lower = model.to_lowercase();
            if lower.contains("jetson") {
                push_tag(&mut tags, "jetson");
                if lower.contains("orin") {
                    push_tag(&mut tags, "orin");
                    if lower.contains("agx") {
                        push_tag(&mut tags, "agx-orin");
                    } else if lower.contains("nano") {
                        push_tag(&mut tags, "orin-nano");
                    } else if lower.contains("nx") {
                        push_tag(&mut tags, "orin-nx");
                    }
                }
                if lower.contains("xavier") {
                    push_tag(&mut tags, "xavier");
                }
            } else if lower.contains("raspberry") {
                push_tag(&mut tags, "rpi");
            }
        }
    }
    if let Some(cpu) = section_line(&sections, "CPU").and_then(|v| v.trim().parse::<i64>().ok()) {
        specs.insert("cpu_cores".to_string(), Value::Number(cpu.into()));
    }
    if let Some(mem) = section_line(&sections, "MEM").and_then(|v| v.trim().parse::<f64>().ok()) {
        let gb = (mem / 1024_f64.powi(3)).round() as i64;
        specs.insert("ram_gb".to_string(), Value::Number(gb.into()));
        specs.insert("ram".to_string(), Value::String(format!("{gb}GB")));
    }
    if let Some(disk) = section_line(&sections, "DISK").and_then(|v| v.trim().parse::<f64>().ok()) {
        let gb = (disk / 1024_f64.powi(3)).round() as i64;
        specs.insert("storage_gb".to_string(), Value::Number(gb.into()));
        specs.insert("storage".to_string(), Value::String(format!("{gb}GB")));
    }
    if let Some(gpu_line) = section_line(&sections, "GPU").map(|v| v.trim().to_string()) {
        if gpu_line != "N/A" {
            let parts = gpu_line.split(',').map(str::trim).collect::<Vec<_>>();
            if let Some(name) = parts.first() {
                specs.insert("gpu".to_string(), Value::String((*name).to_string()));
                push_tag(&mut tags, "gpu");
            }
            if let Some(mem_mb) = parts.get(1).and_then(|v| v.parse::<i64>().ok()) {
                specs.insert("gpu_mem_mb".to_string(), Value::Number(mem_mb.into()));
                specs.insert(
                    "gpu_mem".to_string(),
                    Value::String(format!("{}GB", (mem_mb as f64 / 1024.0).round() as i64)),
                );
            }
        }
    }
    if let Some(lines) = sections.get("ACCEL").and_then(Value::as_array) {
        let lines = lines.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if !lines.is_empty() && lines.first().copied() != Some("N/A") {
            let hailo = lines
                .iter()
                .filter(|line| line.contains("/dev/hailo"))
                .map(|line| Value::String((*line).to_string()))
                .collect::<Vec<_>>();
            if !hailo.is_empty() {
                specs.insert(
                    "accelerators".to_string(),
                    json!([{"type": "hailo", "devices": hailo}]),
                );
                push_tag(&mut tags, "hailo");
            }
        }
    }
    if let Some(hostname) = section_line(&sections, "HOSTNAME").map(|v| v.trim().to_string()) {
        specs.insert("hostname".to_string(), Value::String(hostname));
    }
    if let Some(code) = section_line(&sections, "NET").and_then(|v| v.trim().parse::<i64>().ok()) {
        if (200..500).contains(&code) {
            push_tag(&mut tags, "direct-internet");
        }
    }
    Ok(json!({"specs": specs, "suggested_tags": tags}))
}

fn parse_sections(output: &str) -> Map<String, Value> {
    let mut sections = Map::new();
    let mut current = String::new();
    for line in output.lines() {
        if line.starts_with("---") && line.ends_with("---") {
            current = line.trim_matches('-').to_string();
            sections.insert(current.clone(), Value::Array(Vec::new()));
        } else if !current.is_empty() {
            if let Some(values) = sections.get_mut(&current).and_then(Value::as_array_mut) {
                values.push(Value::String(line.to_string()));
            }
        }
    }
    sections
}

fn push_tag(tags: &mut Vec<String>, tag: &str) {
    if !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_string());
    }
}

fn section_line(sections: &Map<String, Value>, name: &str) -> Option<String> {
    sections
        .get(name)
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn sort_match_results(results: &mut [Value], sort: Option<&str>) {
    match sort {
        Some("disk") => results.sort_by(|a, b| disk_avail(b).total_cmp(&disk_avail(a))),
        Some("memory") => results.sort_by_key(|a| std::cmp::Reverse(mem_free(a))),
        Some("cpu") => results.sort_by(|a, b| cpu_load(a).total_cmp(&cpu_load(b))),
        _ => results.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str))
        }),
    }
}

fn disk_avail(value: &Value) -> f64 {
    let Some(avail) = value
        .get("disk")
        .and_then(|disk| disk.get("avail"))
        .and_then(Value::as_str)
    else {
        return 0.0;
    };
    let upper = avail.to_uppercase();
    let (number, multiplier) = match upper.chars().last() {
        Some('K') => (&upper[..upper.len() - 1], 1.0),
        Some('M') => (&upper[..upper.len() - 1], 1024.0),
        Some('G') => (&upper[..upper.len() - 1], 1024.0 * 1024.0),
        Some('T') => (&upper[..upper.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => (upper.as_str(), 1.0),
    };
    number.parse::<f64>().unwrap_or(0.0) * multiplier
}

fn mem_free(value: &Value) -> i64 {
    value
        .get("memory")
        .and_then(|memory| memory.get("free_mb"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn cpu_load(value: &Value) -> f64 {
    value
        .get("cpu_load")
        .and_then(Value::as_str)
        .and_then(|load| load.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(99.0)
}

#[cfg(not(windows))]
fn sftp_put(device: &Device, local: &Path, remote: &str) -> Result<u64, String> {
    let session = ssh_session(device, Duration::from_secs(60))?;
    let sftp = session
        .sftp()
        .map_err(|err| format!("failed to open SFTP: {err}"))?;
    let mut remote_file = sftp
        .create(Path::new(remote))
        .map_err(|err| format!("failed to create remote file {remote}: {err}"))?;
    let mut local_file = std::fs::File::open(local)
        .map_err(|err| format!("failed to open {}: {err}", local.display()))?;
    let size = std::io::copy(&mut local_file, &mut remote_file)
        .map_err(|err| format!("failed to upload {}: {err}", local.display()))?;
    Ok(size)
}

#[cfg(windows)]
fn sftp_put(device: &Device, local: &Path, remote: &str) -> Result<u64, String> {
    win_ssh::sftp_put(device, local, remote)
}

#[cfg(not(windows))]
fn sftp_get(device: &Device, remote: &str, local: &Path) -> Result<u64, String> {
    let session = ssh_session(device, Duration::from_secs(60))?;
    let sftp = session
        .sftp()
        .map_err(|err| format!("failed to open SFTP: {err}"))?;
    let mut remote_file = sftp
        .open(Path::new(remote))
        .map_err(|err| format!("failed to open remote file {remote}: {err}"))?;
    if let Some(parent) = local.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
    }
    let mut local_file = std::fs::File::create(local)
        .map_err(|err| format!("failed to create {}: {err}", local.display()))?;
    let size = std::io::copy(&mut remote_file, &mut local_file)
        .map_err(|err| format!("failed to download {remote}: {err}"))?;
    Ok(size)
}

#[cfg(windows)]
fn sftp_get(device: &Device, remote: &str, local: &Path) -> Result<u64, String> {
    win_ssh::sftp_get(device, remote, local)
}

#[cfg(windows)]
mod win_ssh {
    use super::{decode_remote, home_dir, Device, RemoteRun};
    use base64::Engine;
    use russh::client::{self, AuthResult, Handle};
    use russh::keys::{load_secret_key, PrivateKeyWithHashAlg, PublicKey};
    use russh::{ChannelMsg, Disconnect};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::runtime::Runtime;
    use tokio::time::timeout;

    #[derive(Clone)]
    struct ClientHandler;

    impl client::Handler for ClientHandler {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    pub fn exec(
        device: &Device,
        command: &str,
        timeout_duration: Duration,
        sudo: bool,
        raw: bool,
    ) -> Result<RemoteRun, String> {
        let device = device.clone();
        let command = command.to_string();
        run_with_stack("fleet-win-ssh-exec", move || {
            if sudo && device.is_windows() {
                return Ok(RemoteRun {
                    success: false,
                    output: "[fleet] --sudo is not supported on Windows devices (no sudo/PTY semantics). Run an elevated command via an admin SSH account or Start-Process -Verb RunAs.".to_string(),
                });
            }

            let runtime = runtime()?;
            let result = runtime.block_on(async {
                timeout(
                    timeout_duration,
                    exec_async(&device, &command, timeout_duration, sudo, raw),
                )
                .await
                .map_err(|_| {
                    format!(
                        "[fleet] command timed out after {}s",
                        timeout_duration.as_secs()
                    )
                })?
            })?;
            Ok(result)
        })
    }

    pub fn sftp_put(device: &Device, local: &Path, remote: &str) -> Result<u64, String> {
        let device = device.clone();
        let local = local.to_path_buf();
        let remote = remote.to_string();
        run_with_stack("fleet-win-ssh-put", move || {
            let runtime = runtime()?;
            runtime.block_on(async {
                timeout(
                    Duration::from_secs(60),
                    ssh_put_async(&device, &local, &remote),
                )
                .await
                .map_err(|_| "SFTP upload timed out after 60s".to_string())?
            })
        })
    }

    pub fn sftp_get(device: &Device, remote: &str, local: &Path) -> Result<u64, String> {
        let device = device.clone();
        let remote = remote.to_string();
        let local = local.to_path_buf();
        run_with_stack("fleet-win-ssh-get", move || {
            let runtime = runtime()?;
            runtime.block_on(async {
                timeout(
                    Duration::from_secs(60),
                    ssh_get_async(&device, &remote, &local),
                )
                .await
                .map_err(|_| "SFTP download timed out after 60s".to_string())?
            })
        })
    }

    fn run_with_stack<T, F>(name: &str, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        std::thread::Builder::new()
            .name(name.to_string())
            .stack_size(16 * 1024 * 1024)
            .spawn(f)
            .map_err(|err| format!("failed to spawn {name}: {err}"))?
            .join()
            .map_err(|_| format!("{name} panicked"))?
    }

    fn runtime() -> Result<Runtime, String> {
        Runtime::new().map_err(|err| format!("failed to create tokio runtime: {err}"))
    }

    async fn exec_async(
        device: &Device,
        command: &str,
        _timeout_duration: Duration,
        sudo: bool,
        raw: bool,
    ) -> Result<RemoteRun, String> {
        let session = connect(device).await?;
        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|err| format!("failed to open SSH channel: {err}"))?;

        let wrapped = if sudo {
            channel
                .request_pty(true, "xterm", 80, 24, 0, 0, &[])
                .await
                .map_err(|err| format!("failed to request PTY: {err}"))?;
            format!(
                "sudo -S -p '' env DEBIAN_FRONTEND=noninteractive PATH=\"$HOME/.local/bin:$PATH\" {command}"
            )
        } else if raw || device.is_windows() {
            command.to_string()
        } else {
            format!(
                "export PATH=\"$HOME/.local/bin:$PATH\"; [ -f \"$HOME/.profile.d/mirrors.sh\" ] && . \"$HOME/.profile.d/mirrors.sh\"; {command}"
            )
        };

        channel
            .exec(true, wrapped)
            .await
            .map_err(|err| format!("failed to execute remote command: {err}"))?;
        if sudo {
            channel
                .data_bytes(format!("{}\n", device.password).into_bytes())
                .await
                .map_err(|err| format!("failed to send sudo password: {err}"))?;
            let _ = channel.eof().await;
        }

        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let mut exit = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => stdout_bytes.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => stderr_bytes.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => exit = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = channel.close().await;
        let _ = session
            .disconnect(Disconnect::ByApplication, "done", "")
            .await;

        let exit = exit.unwrap_or(1);
        let mut output = decode_remote(&stdout_bytes);
        let stderr = decode_remote(&stderr_bytes);
        if exit != 0 && !stderr.trim().is_empty() {
            if output.trim().is_empty() {
                output = stderr;
            } else {
                output = format!("{}\n{}", stderr.trim(), output.trim());
            }
        }
        if sudo {
            output = output
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("[sudo]") && trimmed != device.password
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        let output = output.trim().to_string();
        if exit != 0 {
            let mut detail = if output.is_empty() {
                format!("exit code {exit}")
            } else {
                output
            };
            let lower = detail.to_lowercase();
            if !sudo
                && (lower.contains("permission denied")
                    || lower.contains("not allowed")
                    || lower.contains("are you root"))
            {
                detail.push_str("\n[fleet] Hint: this command likely needs --sudo. Retry with: fleet exec --sudo <device> -- <command>");
            }
            return Ok(RemoteRun {
                success: false,
                output: format!("[fleet] command failed (exit {exit}): {detail}"),
            });
        }
        Ok(RemoteRun {
            success: true,
            output,
        })
    }

    async fn ssh_put_async(device: &Device, local: &Path, remote: &str) -> Result<u64, String> {
        let mut local_file = tokio::fs::File::open(local)
            .await
            .map_err(|err| format!("failed to open {}: {err}", local.display()))?;
        let command = if device.is_windows() {
            powershell_encoded(&format!(
                "$ErrorActionPreference='Stop'; $p={}; $i=[Console]::OpenStandardInput(); $o=[IO.File]::Open($p,[IO.FileMode]::Create,[IO.FileAccess]::Write); try {{$i.CopyTo($o)}} finally {{$o.Close()}}",
                ps_quote(remote)
            ))
        } else {
            format!("cat > {}", super::sh_quote(remote))
        };
        let mut channel = open_exec_channel(device, &command).await?;
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024];
        loop {
            let n = local_file
                .read(&mut buffer)
                .await
                .map_err(|err| format!("failed to read {}: {err}", local.display()))?;
            if n == 0 {
                break;
            }
            channel
                .data_bytes(buffer[..n].to_vec())
                .await
                .map_err(|err| format!("failed to upload {}: {err}", local.display()))?;
            size += n as u64;
        }
        let _ = channel.eof().await;
        let run = collect_channel(channel).await;
        if run.exit != 0 {
            return Err(format!(
                "failed to upload {}: {}",
                local.display(),
                run.error_detail()
            ));
        }
        Ok(size)
    }

    async fn ssh_get_async(device: &Device, remote: &str, local: &Path) -> Result<u64, String> {
        if let Some(parent) = local.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
        }
        let command = if device.is_windows() {
            powershell_encoded(&format!(
                "$ErrorActionPreference='Stop'; $p={}; $i=[IO.File]::OpenRead($p); $o=[Console]::OpenStandardOutput(); try {{$i.CopyTo($o); $o.Flush()}} finally {{$i.Close()}}",
                ps_quote(remote)
            ))
        } else {
            format!("cat {}", super::sh_quote(remote))
        };
        let channel = open_exec_channel(device, &command).await?;
        let run = collect_channel(channel).await;
        if run.exit != 0 {
            return Err(format!(
                "failed to read remote file {remote}: {}",
                run.error_detail()
            ));
        }
        let mut local_file = tokio::fs::File::create(local)
            .await
            .map_err(|err| format!("failed to create {}: {err}", local.display()))?;
        local_file
            .write_all(&run.stdout)
            .await
            .map_err(|err| format!("failed to download {remote}: {err}"))?;
        local_file
            .shutdown()
            .await
            .map_err(|err| format!("failed to close {}: {err}", local.display()))?;
        Ok(run.stdout.len() as u64)
    }

    async fn open_exec_channel(
        device: &Device,
        command: &str,
    ) -> Result<russh::Channel<client::Msg>, String> {
        let session = connect(device).await?;
        let channel = session
            .channel_open_session()
            .await
            .map_err(|err| format!("failed to open SSH channel: {err}"))?;
        channel
            .exec(true, command)
            .await
            .map_err(|err| format!("failed to execute remote transfer command: {err}"))?;
        Ok(channel)
    }

    struct ByteRun {
        exit: u32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    impl ByteRun {
        fn error_detail(&self) -> String {
            let stderr = decode_remote(&self.stderr);
            if stderr.trim().is_empty() {
                format!("exit code {}", self.exit)
            } else {
                format!("exit {}: {}", self.exit, stderr.trim())
            }
        }
    }

    async fn collect_channel(mut channel: russh::Channel<client::Msg>) -> ByteRun {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => exit = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = channel.close().await;
        ByteRun {
            exit: exit.unwrap_or(1),
            stdout,
            stderr,
        }
    }

    fn powershell_encoded(script: &str) -> String {
        let mut bytes = Vec::with_capacity(script.len() * 2);
        for unit in script.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        format!(
            "powershell -NoProfile -EncodedCommand {}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn ps_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    async fn connect(device: &Device) -> Result<Handle<ClientHandler>, String> {
        let addr = format!("{}:{}", device.host, device.port);
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(60)),
            ..Default::default()
        });
        let mut session = client::connect(config, addr.clone(), ClientHandler)
            .await
            .map_err(|err| format!("SSH handshake failed for {addr}: {err}"))?;

        if !device.password.is_empty()
            && session
                .authenticate_password(device.user.clone(), device.password.clone())
                .await
                .map_err(|err| format!("password auth failed for {}@{}: {err}", device.user, addr))?
                .success()
        {
            return Ok(session);
        }

        for key_path in key_candidates() {
            if !key_path.exists() {
                continue;
            }
            let Ok(key) = load_secret_key(&key_path, None) else {
                continue;
            };
            let auth_key = PrivateKeyWithHashAlg::new(Arc::new(key), None);
            if matches!(
                session
                    .authenticate_publickey(device.user.clone(), auth_key)
                    .await,
                Ok(AuthResult::Success)
            ) {
                return Ok(session);
            }
        }

        Err(format!(
            "authentication failed for {}@{}:{}",
            device.user, device.host, device.port
        ))
    }

    fn key_candidates() -> Vec<PathBuf> {
        let ssh = home_dir().join(".ssh");
        ["id_ed25519", "id_rsa"]
            .into_iter()
            .map(|key| ssh.join(key))
            .collect()
    }
}

fn local_md5(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut context = md5::Context::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if n == 0 {
            break;
        }
        context.consume(&buffer[..n]);
    }
    Ok(format!("{:x}", context.compute()))
}

fn remote_md5(device: &Device, remote: &str) -> Result<Option<String>, String> {
    let command = if device.is_windows() {
        let path = ps_quote(remote);
        format!(
            "powershell -NoProfile -Command \"if (Test-Path {path}) {{ (Get-FileHash -Algorithm MD5 {path}).Hash.ToLowerInvariant() }}\""
        )
    } else {
        format!(
            "md5sum {} 2>/dev/null | awk '{{print $1}}' || md5 -q {} 2>/dev/null",
            sh_quote(remote),
            sh_quote(remote)
        )
    };
    let run = ssh_exec(device, &command, Duration::from_secs(30), false, false)?;
    if run.success && !run.output.trim().is_empty() {
        Ok(run.output.split_whitespace().next().map(str::to_string))
    } else {
        Ok(None)
    }
}

fn parse_jobs_output(output: &str) -> Vec<Value> {
    let mut jobs = Vec::new();
    let mut pending: Option<Value> = None;
    for line in output.lines() {
        if line == "__FLEET_JOB__" {
            pending = None;
            continue;
        }
        if let Some(alive) = line.strip_prefix("__PID_ALIVE__:") {
            if let Some(mut job) = pending.take() {
                if let Some(object) = job.as_object_mut() {
                    object.insert("_pid_alive".to_string(), Value::String(alive.to_string()));
                }
                jobs.push(job);
            }
            continue;
        }
        if let Ok(job) = serde_json::from_str::<Value>(line) {
            pending = Some(job);
        }
    }
    jobs
}

fn valid_job_id(job_id: &str) -> bool {
    job_id.len() == 8 && job_id.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn make_job_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:08x}", (nanos & 0xffff_ffff) as u32)
}

fn timestamp_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn json_output(value: Value, code: i32) -> Result<NativeOutput, String> {
    Ok(NativeOutput {
        stdout: format!(
            "{}\n",
            serde_json::to_string_pretty(&value)
                .map_err(|err| format!("failed to render JSON: {err}"))?
        ),
        stderr: String::new(),
        code,
    })
}

fn stdout(value: &str) -> NativeOutput {
    NativeOutput {
        stdout: value.to_string(),
        stderr: String::new(),
        code: 0,
    }
}

fn cli_error(message: String, code: i32) -> NativeOutput {
    NativeOutput {
        stdout: String::new(),
        stderr: if message.ends_with('\n') {
            message
        } else {
            format!("{message}\n")
        },
        code,
    }
}

fn decode_remote(data: &[u8]) -> String {
    if data.len() >= 2 {
        let nul_count = data.iter().filter(|byte| **byte == 0).count();
        if nul_count > data.len() / 4 {
            let mut units = Vec::new();
            for chunk in data.chunks_exact(2) {
                units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            if let Ok(decoded) = String::from_utf16(&units) {
                return decoded;
            }
        }
    }
    String::from_utf8_lossy(data).to_string()
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn shlex_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| sh_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sh_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | ',' | '=' | '+')
    }) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn win_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|path| {
                let candidate = path.join(name);
                candidate.exists()
            })
        })
        .unwrap_or(false)
}

fn run_system_ssh(
    device: &Device,
    remote_cmd: Option<&str>,
    force_tty: bool,
) -> Result<NativeOutput, String> {
    let mut stderr = String::new();
    let mut command = Vec::<String>::new();
    if !device.password.is_empty() && command_exists("sshpass") {
        command.extend([
            "sshpass".to_string(),
            "-p".to_string(),
            device.password.clone(),
        ]);
    } else if !device.password.is_empty() {
        stderr
            .push_str("Tip: install sshpass for auto-login; password is configured but hidden.\n");
    }
    command.push("ssh".to_string());
    command.extend([
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "UserKnownHostsFile=/dev/null".to_string(),
    ]);
    if device.port != 22 {
        command.extend(["-p".to_string(), device.port.to_string()]);
    }
    if force_tty {
        command.push("-t".to_string());
    }
    command.push(format!("{}@{}", device.user, device.host));
    if let Some(remote_cmd) = remote_cmd {
        command.push(remote_cmd.to_string());
    }
    let program = command.remove(0);
    let status = std::process::Command::new(&program)
        .args(&command)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
    Ok(NativeOutput {
        stdout: String::new(),
        stderr,
        code: status.code().unwrap_or(1),
    })
}

fn human_size(nbytes: u64) -> String {
    let mut size = nbytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 {
            if unit == "B" {
                return format!("{nbytes}B");
            }
            return format!("{size:.1}{unit}");
        }
        size /= 1024.0;
    }
    format!("{size:.1}TB")
}

fn format_table(rows: &[Vec<String>], headers: &[&str]) -> String {
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }

    let mut out = String::new();
    push_row(
        &mut out,
        &headers.iter().map(|v| (*v).to_string()).collect::<Vec<_>>(),
        &widths,
    );
    push_row(
        &mut out,
        &widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>(),
        &widths,
    );
    for row in rows {
        push_row(&mut out, row, &widths);
    }
    out
}

fn push_row(out: &mut String, row: &[String], widths: &[usize]) {
    for (index, value) in row.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        out.push_str(value);
        for _ in value.len()..widths[index] {
            out.push(' ');
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::{parse_list_args, NativeFleet};

    fn write_fixture(name: &str, content: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rpty-native-fleet-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.json");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn fixture() -> &'static str {
        r#"{
  "_meta": {"description": "test"},
  "devices": {
    "radxa": {
      "host": "100.77.150.16",
      "user": "radxa",
      "password": "secret",
      "owner": "personal",
      "tags": ["linux", "arm64", "rk3588", "edge"],
      "description": "Radxa board"
    },
    "home-win": {
      "host": "100.73.210.80",
      "user": "harve",
      "password": "",
      "owner": "personal",
      "tags": ["windows", "x86_64", "desktop"],
      "description": "Windows host"
    },
    "seeed-pi": {
      "host": "10.8.0.184",
      "user": "pi",
      "password": "pi-secret",
      "owner": "company",
      "tags": ["rpi", "arm64", "edge"],
      "description": "Company Pi"
    }
  }
}"#
    }

    #[test]
    fn parses_repeated_tags_and_owner() {
        let args = parse_list_args(&[
            "--tag".to_string(),
            "arm64".to_string(),
            "--tag".to_string(),
            "edge".to_string(),
            "--owner".to_string(),
            "personal".to_string(),
            "--json".to_string(),
        ])
        .unwrap();
        assert_eq!(args.tags, ["arm64", "edge"]);
        assert_eq!(args.owner.as_deref(), Some("personal"));
        assert!(args.json);
    }

    #[test]
    fn list_json_masks_passwords_and_preserves_device_fields() {
        let path = write_fixture("json", fixture());
        let fleet = NativeFleet::new(path);
        let output = fleet
            .capture(&["list".to_string(), "--json".to_string()])
            .unwrap()
            .unwrap();
        assert!(output.success);
        assert!(output.stdout.contains("\"radxa\""));
        assert!(output.stdout.contains("\"password\": \"***\""));
        assert!(output.stdout.contains("\"password\": \"\""));
        assert!(!output.stdout.contains("secret"));
        assert!(output.stdout.contains("\"specs\"") == false);
    }

    #[test]
    fn list_filters_by_all_tags_and_owner() {
        let path = write_fixture("filter", fixture());
        let fleet = NativeFleet::new(path);
        let output = fleet
            .capture(&[
                "list".to_string(),
                "--tag".to_string(),
                "arm64".to_string(),
                "--tag".to_string(),
                "edge".to_string(),
                "--owner".to_string(),
                "personal".to_string(),
                "--json".to_string(),
            ])
            .unwrap()
            .unwrap();
        assert!(output.stdout.contains("\"radxa\""));
        assert!(!output.stdout.contains("\"seeed-pi\""));
        assert!(!output.stdout.contains("\"home-win\""));
    }

    #[test]
    fn list_table_matches_python_shape() {
        let path = write_fixture("table", fixture());
        let fleet = NativeFleet::new(path);
        let output = fleet.capture(&["list".to_string()]).unwrap().unwrap();
        assert!(output.stdout.starts_with("NAME"));
        assert!(output.stdout.contains("HOST"));
        assert!(output.stdout.contains("OWNER"));
        assert!(output.stdout.contains("TAGS"));
        assert!(output.stdout.contains("DESCRIPTION"));
        assert!(output.stdout.contains("radxa"));
    }

    #[test]
    fn unsupported_commands_fall_through() {
        let path = write_fixture("unsupported", fixture());
        let fleet = NativeFleet::new(path);
        assert!(fleet
            .capture(&["unknown-native".to_string()])
            .unwrap()
            .is_none());
    }

    #[test]
    fn status_json_reports_offline_without_password_leak() {
        let path = write_fixture("status", fixture());
        let fleet = NativeFleet::new(path);
        let output = fleet
            .capture(&[
                "status".to_string(),
                "radxa".to_string(),
                "--json".to_string(),
            ])
            .unwrap()
            .unwrap();
        assert!(output.success);
        assert!(output.stdout.contains("\"name\": \"radxa\""));
        assert!(output.stdout.contains("\"online\": false"));
        assert!(!output.stdout.contains("secret"));
    }

    #[test]
    fn list_argument_errors_are_cli_failures() {
        let path = write_fixture("badarg", fixture());
        let fleet = NativeFleet::new(path);
        let output = fleet
            .capture(&[
                "list".to_string(),
                "--owner".to_string(),
                "unknown".to_string(),
            ])
            .unwrap()
            .unwrap();
        assert!(!output.success);
        assert_eq!(output.code, 2);
        assert!(output.stderr.contains("--owner"));
    }

    #[test]
    fn add_and_remove_update_inventory_file() {
        let path = write_fixture("addremove", fixture());
        let fleet = NativeFleet::new(path.clone());
        let added = fleet
            .capture(&[
                "add".to_string(),
                "new-box".to_string(),
                "192.0.2.10".to_string(),
                "--user".to_string(),
                "dev".to_string(),
                "--password".to_string(),
                "new-secret".to_string(),
                "--owner".to_string(),
                "personal".to_string(),
                "--tag".to_string(),
                "linux".to_string(),
                "--desc".to_string(),
                "Test box".to_string(),
            ])
            .unwrap()
            .unwrap();
        assert!(added.success);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"new-box\""));
        assert!(content.contains("\"password\": \"new-secret\""));

        let listed = fleet
            .capture(&["list".to_string(), "--json".to_string()])
            .unwrap()
            .unwrap();
        assert!(listed.stdout.contains("\"new-box\""));
        assert!(!listed.stdout.contains("new-secret"));

        let removed = fleet
            .capture(&[
                "remove".to_string(),
                "new-box".to_string(),
                "--force".to_string(),
            ])
            .unwrap()
            .unwrap();
        assert!(removed.success);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("\"new-box\""));
    }
}
