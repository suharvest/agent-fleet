use std::process::ExitCode;

fn main() -> ExitCode {
    // Dispatch on argv0 like the `rpty` bin does. `install` symlinks `bash` at
    // whichever runtime binary was copied into place, and that can be this one.
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "fleet".to_string());

    match pty_router::cli::run_invoked_as(&argv0, std::env::args().skip(1)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("fleet: {err}");
            ExitCode::from(1)
        }
    }
}
