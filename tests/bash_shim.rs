//! The installed runtime answers to `bash`, so every bin target must keep the
//! shim's passthrough.
//!
//! `install` copies one binary and symlinks `rpty`, `fleet` and `bash` at it.
//! The `fleet` bin used to call the subcommand parser directly, so invoking it
//! as `bash` produced `unknown or unimplemented command: -c` — which broke
//! every `#!/usr/bin/env bash` script (git hooks, pre-commit, credential
//! helpers) on a machine with the shim dir on PATH.
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
