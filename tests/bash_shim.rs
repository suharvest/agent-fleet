//! The installed runtime answers to `bash`, so every bin target must keep the
//! shim's passthrough.
//!
//! `install` copies one binary and symlinks every command name at it. The bash
//! link is opt-in: `install` and `doctor --fix` skip it unless
//! `--with-bash-shim` is passed, and `install-shim` / `uninstall-shim` toggle
//! it on its own. The `fleet` bin used to call the subcommand parser directly,
//! so invoking it as `bash` produced `unknown or unimplemented command: -c` —
//! which broke every `#!/usr/bin/env bash` script (git hooks, pre-commit,
//! credential helpers) on a machine with the shim dir on PATH.
//!
//! Unix-only: passthrough execs `/bin/bash`, and the Windows shim is a `.cmd`
//! wrapper that passes the name through `RPTY_ARGV0` rather than a symlink.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

struct Shim {
    dir: PathBuf,
    link: PathBuf,
    home: PathBuf,
}

impl Drop for Shim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Symlink `exe` as `bash` in a fresh dir, with an isolated RPTY_HOME.
fn shim(exe: &str, tag: &str) -> Shim {
    let dir = std::env::temp_dir().join(format!("rpty-bash-shim-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create temp dirs");
    let link = dir.join("bash");
    std::os::unix::fs::symlink(Path::new(exe), &link).expect("symlink runtime as bash");
    Shim { dir, link, home }
}

impl Shim {
    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command(args).output().expect("run bash shim")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.link);
        cmd.args(args)
            .env("RPTY_HOME", &self.home)
            .env_remove("RPTY_ARGV0")
            .env_remove("RPTY_BASH_PASSTHROUGH")
            .env_remove("RPTY_SESSION");
        cmd
    }

    /// Pin a session to a host the way `fleet use <device>` does, so the shim
    /// believes it is routed without a device being reachable.
    fn route_session(&self, session: &str, host: &str) {
        let dir = self.home.join("state").join("sessions").join(session);
        std::fs::create_dir_all(&dir).expect("create session dir");
        std::fs::write(dir.join("current_host"), host).expect("write current_host");
    }
}

/// A scratch install dir that is removed on drop.
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(tag: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rpty-install-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    ScratchDir(dir)
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Run the `rpty` bin with `args`, isolating RPTY_HOME so nothing touches the
/// real install.
fn rpty(args: &[&str]) -> std::process::Output {
    let home = std::env::temp_dir().join("rpty-install-rpty-home");
    Command::new(env!("CARGO_BIN_EXE_rpty"))
        .args(args)
        .env("RPTY_HOME", home)
        .env_remove("RPTY_ARGV0")
        .output()
        .expect("run rpty")
}

#[test]
fn install_skips_the_bash_shim_by_default() {
    let dir = scratch("default");
    let out = rpty(&["install", dir.0.to_str().expect("utf8 path")]);
    assert!(
        out.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(exists(&dir.0.join("rpty")), "rpty shim must be installed");
    assert!(exists(&dir.0.join("fleet")), "fleet shim must be installed");
    assert!(
        !exists(&dir.0.join("bash")),
        "bash shim must NOT be installed by default"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("bash shim: not installed"),
        "install output must state the bash shim is off"
    );
}

#[test]
fn install_with_bash_shim_flag_creates_it() {
    let dir = scratch("with-flag");
    let out = rpty(&[
        "install",
        "--with-bash-shim",
        dir.0.to_str().expect("utf8 path"),
    ]);
    assert!(
        out.status.success(),
        "install --with-bash-shim failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        exists(&dir.0.join("bash")),
        "--with-bash-shim must create the bash link"
    );
}

#[test]
fn install_shim_still_creates_the_bash_link() {
    let dir = scratch("install-shim");
    let out = rpty(&["install-shim", dir.0.to_str().expect("utf8 path")]);
    assert!(
        out.status.success(),
        "install-shim failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        exists(&dir.0.join("bash")),
        "install-shim keeps its explicit opt-in semantics"
    );
}

#[test]
fn uninstall_shim_removes_the_bash_link() {
    let dir = scratch("uninstall");
    let path = dir.0.to_str().expect("utf8 path").to_string();
    assert!(rpty(&["install-shim", &path]).status.success());
    assert!(exists(&dir.0.join("bash")));

    let out = rpty(&["uninstall-shim", &path]);
    assert!(
        out.status.success(),
        "uninstall-shim failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !exists(&dir.0.join("bash")),
        "uninstall-shim must remove the bash link"
    );

    // Idempotent: a second run reports there was nothing to do and succeeds.
    let again = rpty(&["uninstall-shim", &path]);
    assert!(again.status.success());
    assert!(String::from_utf8_lossy(&again.stdout).contains("not installed"));
}

#[test]
fn uninstall_shim_refuses_a_real_bash_file() {
    let dir = scratch("refuse");
    let real = dir.0.join("bash");
    std::fs::write(&real, "#!/bin/sh\necho i-am-real-bash\n").expect("write real file");

    let out = rpty(&["uninstall-shim", dir.0.to_str().expect("utf8 path")]);
    assert!(
        !out.status.success(),
        "uninstall-shim must fail on a non-shim file"
    );
    assert!(exists(&real), "a real bash file must not be removed");
    assert_eq!(
        std::fs::read_to_string(&real).expect("read back"),
        "#!/bin/sh\necho i-am-real-bash\n"
    );
}

fn assert_passthrough(exe: &str, tag: &str) {
    let s = shim(exe, tag);

    let out = s.run(&["-c", "echo hi-from-shim"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("hi-from-shim"),
        "`bash -c` must reach the real bash (stdout={stdout:?} stderr={stderr:?})"
    );
    assert!(
        out.status.success(),
        "`bash -c` exited with {:?}",
        out.status
    );

    // A script path is the other shape git and pre-commit use.
    let script = s.dir.join("hook.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\necho hi-from-script\n").expect("write script");
    let out = s.run(&[script.to_str().expect("utf8 path")]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hi-from-script"),
        "`bash <script>` must reach the real bash (stdout={stdout:?})"
    );

    // A failing command still reports its own exit code, not the parser's.
    let out = s.run(&["-c", "exit 3"]);
    assert_eq!(out.status.code(), Some(3), "exit code must come from bash");
}

#[test]
fn rpty_bin_passes_bash_invocations_through() {
    assert_passthrough(env!("CARGO_BIN_EXE_rpty"), "rpty");
}

#[test]
fn fleet_bin_passes_bash_invocations_through() {
    assert_passthrough(env!("CARGO_BIN_EXE_fleet"), "fleet");
}

#[test]
fn rpty_argv0_overrides_the_link_name() {
    // The Windows `.cmd` wrapper carries the name in RPTY_ARGV0 instead of
    // argv0, so an explicit value has to win over the link it was invoked as.
    let s = shim(env!("CARGO_BIN_EXE_fleet"), "argv0");
    let out = s
        .command(&["-c", "echo via-env"])
        .env("RPTY_ARGV0", "bash")
        .output()
        .expect("run shim");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("via-env"),
        "RPTY_ARGV0=bash must dispatch to the shim"
    );

    // An empty value must not be taken as the invoked name: it falls back to
    // argv0, which is still `bash` here, so the call still passes through.
    let out = s
        .command(&["-c", "echo empty-env"])
        .env("RPTY_ARGV0", "")
        .output()
        .expect("run shim");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("empty-env"),
        "empty RPTY_ARGV0 must fall back to argv0"
    );
}

#[test]
fn passthrough_env_wins_over_a_routed_session() {
    // RPTY_BASH_PASSTHROUGH is the escape hatch for a routed session: it must
    // be checked before the host lookup, or a routed session could not run
    // anything locally.
    let s = shim(env!("CARGO_BIN_EXE_fleet"), "passthrough");
    s.route_session("test-session", "no-such-device");

    let out = s
        .command(&["-c", "echo still-local"])
        .env("RPTY_SESSION", "test-session")
        .env("RPTY_BASH_PASSTHROUGH", "1")
        .output()
        .expect("run shim");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("still-local"),
        "passthrough must run locally even while routed (stdout={stdout:?})"
    );
}

#[test]
fn unrouted_session_state_does_not_route() {
    // The `default` session id is treated as "no session", so a stale host file
    // there must not silently route an unrelated shell call.
    let s = shim(env!("CARGO_BIN_EXE_fleet"), "default-session");
    s.route_session("default", "no-such-device");

    let out = s.run(&["-c", "echo local-default"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("local-default"),
        "the default session must not route"
    );
}
