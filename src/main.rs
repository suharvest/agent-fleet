use std::process::ExitCode;

fn main() -> ExitCode {
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "rpty".to_string());
    let program_name = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rpty");

    let routed_name = std::env::var("RPTY_ARGV0").unwrap_or_else(|_| program_name.to_string());

    let result = if routed_name == "bash" {
        pty_router::cli::run_bash_shim(std::env::args().skip(1))
    } else {
        pty_router::cli::run(std::env::args().skip(1))
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("rpty: {err}");
            ExitCode::from(1)
        }
    }
}
