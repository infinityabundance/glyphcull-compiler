//! The `cull` binary: a thin wrapper over [`glyphcull_cli::run`].

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    glyphcull_cli::run(&args)
}
