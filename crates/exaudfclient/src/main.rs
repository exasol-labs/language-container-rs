use exa_udf_runtime::Runtime;
use tracing::error;
use tracing_subscriber::{EnvFilter, prelude::*, reload};

fn main() {
    // Must happen before any library code that might read HOME.
    // SAFETY: main() runs single-threaded before any other threads are spawned.
    unsafe { std::env::set_var("HOME", "/tmp") };

    // The filter starts at INFO (or RUST_LOG if set). After the handshake
    // delivers %udf_debug_level from the script source, Runtime::run() calls
    // the on_level_resolved hook which modifies this handle in-place — no
    // reload crate feature needed, reload is always available in tracing-subscriber.
    let initial_filter = EnvFilter::from_default_env().add_directive("info".parse().unwrap());
    let (filter_layer, filter_handle) = reload::Layer::new(initial_filter);
    // stderr flushes per write: std::io::stderr() is an unbuffered fd-level
    // write (no userspace BufWriter), so every tracing event is visible to the
    // DB's fd-2 redirect immediately.
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let args: Vec<String> = std::env::args().collect();
    tracing::debug!("exaudfclient started; args: {:?}", args);

    match run(&args, |level| {
        // Apply %udf_debug_level resolved from the script source post-handshake.
        let new_filter = EnvFilter::new(level.as_str());
        let _ = filter_handle.modify(|f| *f = new_filter);
    }) {
        Ok(()) => {
            // Force immediate process exit. The reference C++ exaudfclient_main
            // does `return 0` from a function whose caller immediately exits; the OS
            // then reaps the process via waitpid(). Without this call, Rust's normal
            // cleanup tries to join the static connect-back Tokio runtime (reactor +
            // blocking threads), delaying exit by ~10 s and causing Part:40's
            // TimerWatchDog to fire SIGABRT before waitpid() ever succeeds.
            std::process::exit(0);
        }
        Err(Exit { code, message }) => {
            eprintln!("{}", message);
            error!("{}", message);
            std::process::exit(code);
        }
    }
}

struct Exit {
    code: i32,
    message: String,
}

impl Exit {
    fn new(code: i32, msg: impl Into<String>) -> Self {
        Exit {
            code,
            message: msg.into(),
        }
    }
}

fn run(args: &[String], on_level_resolved: impl Fn(tracing::Level)) -> Result<(), Exit> {
    if args.len() < 3 {
        return Err(Exit::new(
            1,
            format!("F-UDF-CL-RUST-0003: wrong argument count\n{}", usage()),
        ));
    }

    let endpoint = &args[1];
    let lang_arg = &args[2];

    if lang_arg != "lang=rust" {
        return Err(Exit::new(
            2,
            format!(
                "F-UDF-CL-RUST-0002: unsupported language argument '{}'; expected 'lang=rust'",
                lang_arg
            ),
        ));
    }

    // MT_CLIENT's client_name field is documented as "URL of the client in form:
    // tcp://10.10.1.1:2000". Send the actual ZMQ endpoint URL so Part:40 recognises
    // this as a valid SLC connection and allows connect-back sessions without crashing.
    let client_name = endpoint.clone();
    let runtime = Runtime::new(endpoint.clone(), client_name);
    runtime
        .run(on_level_resolved)
        .map_err(|e| Exit::new(1, format!("F-UDF-CL-RUST-0001: {}", e)))
}

fn usage() -> &'static str {
    "Usage: exaudfclient <endpoint> lang=rust [scriptOptionsParserVersion=N]\n\
     Exasol Rust UDF Client v1"
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
