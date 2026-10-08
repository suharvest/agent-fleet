//! The bundled backend must never be truncated by the command that seeds it.
//!
//! `install_bundled_backend` picks its source from the running binary's own
//! directory first. Once the runtime is installed there, source and destination
//! are the same path, and a plain `fs::copy(src, src)` truncates the file to
//! zero bytes before reading it — which silently emptied `~/.rpty/bin/fleet_backend/`
//! every time `doctor --fix` ran from `~/.rpty/bin/fleet-router`.
//!
//! `doctor --fix` is the only path that copies the bundled backend
//! (`install_base_shims` -> `install_bundled_backend`); plain `install` only
//! writes the runtime and the command shims. These tests therefore drive
//! `doctor --fix` from a copy of the binary placed inside the install dir, with
//! `RPTY_HOME` pointed at the scratch root so the install dir is isolated.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The files `install_bundled_backend` copies out of `fleet_backend/`.
const BACKEND_FILES: &[&str] = &[
    "fleet.py",
    "bootstrap.sh",
    "pyproject.toml",
    "devices.example.json",
];

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(tag: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rpty-backend-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    ScratchDir(dir)
}

fn repo_backend() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fleet_backend")
}

fn md5(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    format!("{:x}", md5::compute(&bytes))
}

/// Run `doctor --fix` from a copy of the rpty binary that lives in the install
/// dir (`$RPTY_HOME/bin/fleet-router`), which is the layout every machine has
/// once the runtime is installed. Returns the install directory.
fn doctor_fix_from_inside(root: &Path) -> PathBuf {
    let install_dir = root.join("bin");
    std::fs::create_dir_all(&install_dir).expect("create install bin dir");
    let runtime = install_dir.join("fleet-router");
    // Unlink before copying: overwriting a signed Mach-O in place leaves a
    // stale code-signature cache entry and macOS then SIGKILLs the copy.
    let _ = std::fs::remove_file(&runtime);
    std::fs::copy(env!("CARGO_BIN_EXE_rpty"), &runtime).expect("copy binary into install dir");

    let out = Command::new(&runtime)
        .args(["doctor", "--fix"])
        .env("RPTY_HOME", root)
        .env_remove("RPTY_ARGV0")
        .output()
        .expect("run doctor --fix from install dir");
    assert!(
        out.status.success(),
        "doctor --fix failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    install_dir
}

fn assert_backend_matches_repo(install_dir: &Path) {
    let repo = repo_backend();
    for name in BACKEND_FILES {
        let installed = install_dir.join("fleet_backend").join(name);
        let source = repo.join(name);
        assert!(
            source.is_file(),
            "test precondition: repo copy {} is missing",
            source.display()
        );
        assert!(
            installed.is_file(),
            "{} must exist after doctor --fix",
            installed.display()
        );
        let len = std::fs::metadata(&installed)
            .expect("stat installed backend file")
            .len();
        assert!(len > 0, "{} was truncated to 0 bytes", installed.display());
        assert_eq!(
            md5(&installed),
            md5(&source),
            "{} must match the repo copy",
            installed.display()
        );
    }
}

#[test]
fn doctor_fix_seeds_the_backend_in_a_fresh_install_dir() {
    let root = scratch("fresh");
    let install_dir = doctor_fix_from_inside(&root.0);
    assert_backend_matches_repo(&install_dir);
}

#[test]
fn doctor_fix_run_from_inside_the_install_dir_keeps_the_backend_intact() {
    let root = scratch("reinstall");

    // The first run seeds `fleet_backend/` in the install dir, which is what
    // makes `bundled_backend_source()` resolve to the exe-relative directory on
    // every later run.
    let install_dir = doctor_fix_from_inside(&root.0);
    assert_backend_matches_repo(&install_dir);

    // Second run: source and destination are now the same path.
    let install_dir = doctor_fix_from_inside(&root.0);
    assert_backend_matches_repo(&install_dir);
}
