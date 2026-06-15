use exa_udf_runtime::Runtime;
use tracing::error;

fn main() {
    // Must happen before any library code that might read HOME.
    // SAFETY: main() runs single-threaded before any other threads are spawned.
    unsafe { std::env::set_var("HOME", "/tmp") };

    // Debug tracing: write to /tmp so it survives even when BucketFS is read-only.
    // This file is read by the integration test harness via dump_udf_logs().
    let _ = std::fs::write(
        "/tmp/exaudf_started.txt",
        format!(
            "exaudfclient started; args: {:?}\n",
            std::env::args().collect::<Vec<_>>()
        ),
    );

    // Probe whether getrandom(2) is available; BoringSSL aborts if it is not
    // and /dev/urandom is also absent (BucketFS chroot strips device nodes).
    let mut probe = [0u8; 1];
    let gr_rc = unsafe {
        libc::syscall(
            libc::SYS_getrandom,
            probe.as_mut_ptr() as *mut libc::c_void,
            1usize,
            0u32,
        )
    };
    eprintln!("[slc] getrandom probe rc={gr_rc}");

    let diag_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("udf_diag.log")));
    match diag_path.as_ref().and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    }) {
        Some(f) => {
            tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("debug".parse().unwrap()),
                )
                .init();
            std::panic::set_hook(Box::new(move |info| {
                tracing::error!("PANIC: {info}");
            }));
        }
        None => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("info".parse().unwrap()),
                )
                .init();
        }
    }

    let args: Vec<String> = std::env::args().collect();

    match run(&args) {
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

fn run(args: &[String]) -> Result<(), Exit> {
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

    let parser_version = resolve_parser_version(args);
    tracing::debug!("parser_version={}", parser_version);

    // MT_CLIENT's client_name field is documented as "URL of the client in form:
    // tcp://10.10.1.1:2000". Send the actual ZMQ endpoint URL so Part:40 recognises
    // this as a valid SLC connection and allows connect-back sessions without crashing.
    let client_name = endpoint.clone();
    let runtime = Runtime::new(endpoint.clone(), client_name);
    runtime
        .run()
        .map_err(|e| Exit::new(1, format!("F-UDF-CL-RUST-0001: {}", e)))
}

/// Resolve parser version from env var, then from a `parser_version=N` CLI arg,
/// then default to "1".
pub fn resolve_parser_version(args: &[String]) -> String {
    if let Ok(v) = std::env::var("EXAUDF_PARSER_VERSION") {
        return v;
    }
    args.iter()
        .skip(3)
        .find(|a| a.starts_with("parser_version="))
        .map(|a| a.trim_start_matches("parser_version=").to_string())
        .unwrap_or_else(|| "1".to_string())
}

fn usage() -> String {
    "Usage: exaudfclient <endpoint> lang=rust [parser_version=N]\n\
     Exasol Rust UDF Client v1"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn too_few_args_returns_exit_code_1() {
        let result = run(&args(&["exaudfclient", "tcp://localhost:1234"]));
        let exit = result.unwrap_err();
        assert_eq!(exit.code, 1);
        assert!(exit.message.contains("F-UDF-CL-RUST-0003"));
    }

    #[test]
    fn unsupported_lang_returns_exit_code_2() {
        let result = run(&args(&[
            "exaudfclient",
            "tcp://localhost:1234",
            "lang=python",
        ]));
        let exit = result.unwrap_err();
        assert_eq!(exit.code, 2);
        assert!(exit.message.contains("F-UDF-CL-RUST-0002"));
    }

    /// All parser-version resolution cases in one sequential test to avoid
    /// env-var races between parallel test threads.
    #[test]
    fn resolve_parser_version_precedence() {
        // Ensure env var is absent for the non-env cases.
        // SAFETY: this test is intentionally single-threaded (see comment above);
        // no other threads read EXAUDF_PARSER_VERSION concurrently.
        unsafe { std::env::remove_var("EXAUDF_PARSER_VERSION") };

        // Default fallback.
        let v = resolve_parser_version(&args(&["exaudfclient", "tcp://x:1", "lang=rust"]));
        assert_eq!(v, "1");

        // Explicit arg overrides default.
        let v = resolve_parser_version(&args(&[
            "exaudfclient",
            "tcp://x:1",
            "lang=rust",
            "parser_version=7",
        ]));
        assert_eq!(v, "7");

        // Env var takes precedence over CLI arg.
        unsafe { std::env::set_var("EXAUDF_PARSER_VERSION", "42") };
        let v = resolve_parser_version(&args(&[
            "exaudfclient",
            "tcp://x:1",
            "lang=rust",
            "parser_version=7",
        ]));
        assert_eq!(v, "42");
        unsafe { std::env::remove_var("EXAUDF_PARSER_VERSION") };
    }
}
