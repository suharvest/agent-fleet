use std::process::ExitCode;

fn main() -> ExitCode {
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "rpty".to_string());

    match pty_router::cli::run_invoked_as(&argv0, std::env::args().skip(1)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("rpty: {err}");
            ExitCode::from(1)
        }
    }
}
