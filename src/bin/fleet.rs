use std::process::ExitCode;

fn main() -> ExitCode {
    match pty_router::cli::run(std::env::args().skip(1)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("fleet: {err}");
            ExitCode::from(1)
        }
    }
}
