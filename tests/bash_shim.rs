//! The installed runtime answers to `bash`, so every bin target must keep the
//! shim's passthrough.
//!
//! `install` copies one binary and symlinks `rpty`, `fleet` and `bash` at it.
//! The `fleet` bin used to call the subcommand parser directly, so invoking it
//! as `bash` produced `unknown or unimplemented command: -c` — which broke
//! every `#!/usr/bin/env bash` script (git hooks, pre-commit, secret-run) on a
//! machine with the shim dir on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Symlink `exe` as `bash` inside a fresh dir and return the link path.
fn link_as_bash(exe: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rpty-bash-shim-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let link = dir.join("bash");
    #[cfg(unix)]
    std::os::unix::fs::symlink(Path::new(exe), &link).expect("symlink runtime as bash");
    (dir, link)
}

/// Run the shim with an isolated RPTY_HOME so no real session state is read.
fn run_shim(link: &Path, home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(link)
        .args(args)
        .env("RPTY_HOME", home)
        .env_remove("RPTY_ARGV0")
        .env_remove("RPTY_BASH_PASSTHROUGH")
        .output()
        .expect("run bash shim")
}

fn assert_passthrough(exe: &str, tag: &str) {
    let (dir, link) = link_as_bash(exe, tag);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create rpty home");

    let out = run_shim(&link, &home, &["-c", "echo hi-from-shim"]);
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
    let script = dir.join("hook.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\necho hi-from-script\n").expect("write script");
    let out = run_shim(&link, &home, &[script.to_str().expect("utf8 path")]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hi-from-script"),
        "`bash <script>` must reach the real bash (stdout={stdout:?})"
    );

    // A failing command still reports its own exit code, not the parser's.
    let out = run_shim(&link, &home, &["-c", "exit 3"]);
    assert_eq!(out.status.code(), Some(3), "exit code must come from bash");

    let _ = std::fs::remove_dir_all(&dir);
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
fn own_subcommands_still_work_under_the_bash_name() {
    // Routing control has to stay reachable through the shim name.
    let (dir, link) = link_as_bash(env!("CARGO_BIN_EXE_fleet"), "subcmd");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create rpty home");

    let out = run_shim(&link, &home, &["-c", "echo routed-check"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("routed-check"),
        "sanity: passthrough works before checking subcommands"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
