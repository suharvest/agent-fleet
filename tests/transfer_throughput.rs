//! Device-gated throughput check for `fleet push` / `fleet pull`.
//!
//! `fleet push` used to stream SFTP through `std::io::copy`, whose 8 KiB
//! buffer forced one SFTP round trip per 8 KiB and pinned LAN transfers near
//! 1 MB/s. This test is the regression guard: it moves a real file to a real
//! device and fails if throughput collapses back to that range.
//!
//! It is skipped unless a device is named, because CI has no fleet:
//!
//! ```bash
//! RPTY_BENCH_DEVICE=wsl2-local cargo test --test transfer_throughput -- --nocapture
//! ```
//!
//! The floor is 15 MB/s rather than link speed. On a 1 GbE LAN (93 MB/s over
//! plain HTTP, 3.8 ms RTT) the SSH transport itself delivers about 30 MB/s,
//! and paramiko measures the same, so the floor guards against the 1 MB/s
//! regression without failing on a slower device or a busy link.
//!
//! Optional knobs:
//! - `RPTY_BENCH_SIZE_MB`     payload size, default 200
//! - `RPTY_BENCH_MIN_MBPS`    minimum accepted throughput, default 15
//! - `RPTY_BENCH_REMOTE_DIR`  remote scratch directory, default `/tmp`
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

fn env_or<T: std::str::FromStr>(key: &str, fallback: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn fleet(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rpty"))
        .args(args)
        .output()
        .expect("run fleet binary")
}

/// Deterministic filler so the payload does not compress to nothing.
fn payload(bytes: usize) -> Vec<u8> {
    let mut data = vec![0u8; bytes];
    let mut state: u32 = 0x9e37_79b9;
    for slot in data.iter_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *slot = (state >> 24) as u8;
    }
    data
}

#[test]
fn push_and_pull_sustain_lan_throughput() {
    let Ok(device) = std::env::var("RPTY_BENCH_DEVICE") else {
        eprintln!("skipped: set RPTY_BENCH_DEVICE=<device> to run the transfer benchmark");
        return;
    };

    let size_mb: usize = env_or("RPTY_BENCH_SIZE_MB", 200);
    let min_mbps: f64 = env_or("RPTY_BENCH_MIN_MBPS", 15.0);
    let remote_dir = std::env::var("RPTY_BENCH_REMOTE_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let bytes = size_mb * 1024 * 1024;

    let tag = format!("rpty-bench-{}", std::process::id());
    let local: PathBuf = std::env::temp_dir().join(format!("{tag}.bin"));
    let back: PathBuf = std::env::temp_dir().join(format!("{tag}.back.bin"));
    let remote = format!("{remote_dir}/{tag}.bin");

    let data = payload(bytes);
    std::fs::write(&local, &data).expect("write payload");

    let started = Instant::now();
    let push = fleet(&["push", &device, local.to_str().unwrap(), &remote]);
    let push_secs = started.elapsed().as_secs_f64();

    let cleanup = || {
        let _ = fleet(&["exec", &device, "--", &format!("rm -f -- '{remote}'")]);
        let _ = std::fs::remove_file(&local);
        let _ = std::fs::remove_file(&back);
    };

    if !push.status.success() {
        cleanup();
        panic!(
            "push failed: {}{}",
            String::from_utf8_lossy(&push.stdout),
            String::from_utf8_lossy(&push.stderr)
        );
    }

    // md5 verification must survive the fast path.
    let push_log = format!(
        "{}{}",
        String::from_utf8_lossy(&push.stdout),
        String::from_utf8_lossy(&push.stderr)
    );
    if !push_log.contains("verify: OK") {
        cleanup();
        panic!("push did not report a successful md5 verify:\n{push_log}");
    }

    let started = Instant::now();
    let pull = fleet(&["pull", &device, &remote, back.to_str().unwrap()]);
    let pull_secs = started.elapsed().as_secs_f64();

    if !pull.status.success() {
        cleanup();
        panic!(
            "pull failed: {}{}",
            String::from_utf8_lossy(&pull.stdout),
            String::from_utf8_lossy(&pull.stderr)
        );
    }

    let round_trip = std::fs::read(&back).expect("read pulled file");
    let identical = round_trip == data;
    cleanup();

    assert!(identical, "pulled bytes differ from the pushed payload");

    let push_mbps = size_mb as f64 / push_secs;
    let pull_mbps = size_mb as f64 / pull_secs;
    eprintln!(
        "push {push_mbps:.1} MB/s ({push_secs:.1}s), pull {pull_mbps:.1} MB/s ({pull_secs:.1}s)"
    );

    assert!(
        push_mbps >= min_mbps,
        "push throughput {push_mbps:.1} MB/s is below the {min_mbps:.1} MB/s floor"
    );
    assert!(
        pull_mbps >= min_mbps,
        "pull throughput {pull_mbps:.1} MB/s is below the {min_mbps:.1} MB/s floor"
    );
}
