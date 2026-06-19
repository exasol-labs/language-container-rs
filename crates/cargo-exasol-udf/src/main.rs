mod build;
mod new;
mod validate;

use std::env;
use std::process;

fn usage() -> ! {
    eprintln!("Usage: cargo exasol-udf <subcommand> [args]");
    eprintln!("Subcommands:");
    eprintln!("  new <path>       Scaffold a new UDF crate at <path>");
    eprintln!("  build [<path>]   Build the UDF crate (defaults to .)");
    eprintln!("  validate <path>  Validate a compiled UDF .so");
    process::exit(1);
}

fn main() {
    // When invoked as `cargo exasol-udf <cmd>`, argv is:
    //   ["cargo-exasol-udf", "exasol-udf", <cmd>, ...]
    // Skip argv[0] (binary name) and argv[1] ("exasol-udf" cargo-subcommand token).
    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(2).map(|s| s.as_str());
    let rest = args.get(3..).unwrap_or_default();

    match subcommand {
        Some("new") => {
            if let Err(e) = new::run(rest) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Some("build") => {
            if let Err(e) = build::run(rest) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        Some("validate") => {
            if let Err(e) = validate::run(rest) {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        _ => usage(),
    }
}
