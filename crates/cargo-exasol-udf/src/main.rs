mod build;
mod elf;
mod new;
mod slc_surface;
mod validate;

use std::env;
use std::io::{self, Write};
use std::process;

fn write_usage(mut w: impl Write) -> io::Result<()> {
    writeln!(w, "Usage: cargo exasol-udf <subcommand> [args]")?;
    writeln!(w, "Subcommands:")?;
    writeln!(
        w,
        "  new <path>                Scaffold a new UDF crate at <path>"
    )?;
    writeln!(w, "  build [<path>] [--target <triple>]")?;
    writeln!(
        w,
        "                            Build the UDF crate as a host glibc-dynamic"
    )?;
    writeln!(
        w,
        "                            cdylib (defaults to .); --target overrides the"
    )?;
    writeln!(
        w,
        "                            build target (native builds only)"
    )?;
    writeln!(w, "  validate <path> [--deny-unknown-deps]")?;
    writeln!(
        w,
        "                            Validate a compiled UDF .so: entry points, ABI"
    )?;
    writeln!(
        w,
        "                            version and SDK fingerprint, the artifact's glibc"
    )?;
    writeln!(
        w,
        "                            symbol floor, and its dynamic dependencies against"
    )?;
    writeln!(
        w,
        "                            the SLC library surface; --deny-unknown-deps fails"
    )?;
    writeln!(
        w,
        "                            on any dependency outside that surface"
    )?;
    Ok(())
}

fn usage() -> ! {
    let _ = write_usage(io::stderr());
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

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
