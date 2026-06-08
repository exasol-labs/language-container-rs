//! Integration-test harness for the Rust SLC.
//!
//! Helpers to start `exasol/docker-db`, upload the slim language container and
//! UDF artifacts into BucketFS, register the `RUST` script language, and run
//! SQL through `exarrow-rs`. Everything here is gated behind the `integration`
//! feature so a default build never drags in Docker or a live database.
#![cfg(feature = "integration")]

use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use exarrow_rs::adbc::{Connection, Driver};
use testcontainers::core::{ExecCommand, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// Database SQL port (Exasol native/TLS protocol).
pub const DB_PORT: u16 = 8563;
/// BucketFS HTTPS port for `exasol/docker-db` (HTTP is disabled by default).
pub const BUCKETFS_PORT: u16 = 2581;
/// Image holding the database under test. Pinned per project rules.
pub const DB_IMAGE: &str = "exasol/docker-db";
/// Slim Rust SLC image built earlier in the pipeline.
pub const SLC_IMAGE: &str = "slc-rs-slim:dev";

/// Exasol image tag. Reads `EXASOL_VERSION` env var; defaults to `2026.latest`.
pub fn db_tag() -> String {
    std::env::var("EXASOL_VERSION").unwrap_or_else(|_| "2026.latest".to_string())
}

/// Bucket name and BucketFS service name baked into the docker-db default.
pub const BUCKET: &str = "default";
pub const BFS_SERVICE: &str = "bfsdefault";
const BFS_WRITE_USER: &str = "w";

/// A running database plus the host/port mapping needed to reach it.
pub struct Harness {
    // Held to keep the container alive for the harness lifetime; dropping it
    // tears the database down. None in external (CI) mode where the container
    // is managed outside the test process.
    _container: Option<ContainerAsync<GenericImage>>,
    // ID or name used for `docker exec` / `docker logs` diagnostics.
    container_name: String,
    pub host: String,
    pub db_port: u16,
    pub bucketfs_port: u16,
    bucketfs_write_password: String,
}

impl Harness {
    /// Start the database harness.
    ///
    /// **Testcontainers mode** (default): starts `exasol/docker-db` privileged,
    /// maps ports, waits for "All stages finished.", and reads BucketFS
    /// credentials from `EXAConf` inside the container.
    ///
    /// **External mode** (when `EXASOL_HOST` is set): skips container startup
    /// and reads connection details from env vars. Used by CI where the
    /// container is started and health-checked externally.
    ///
    /// External env vars:
    /// - `EXASOL_HOST` — DB hostname (triggers external mode)
    /// - `EXASOL_PORT` — DB SQL port (default `8563`)
    /// - `BUCKETFS_PORT` — BucketFS HTTPS port (default `2581`)
    /// - `BUCKETFS_PASSWORD` — BucketFS `default` bucket write password (required)
    pub async fn start() -> Result<Self> {
        install_crypto_provider();

        if let Ok(host) = std::env::var("EXASOL_HOST") {
            let db_port = std::env::var("EXASOL_PORT")
                .unwrap_or_else(|_| "8563".to_string())
                .parse::<u16>()
                .context("EXASOL_PORT is not a valid port number")?;
            let bucketfs_port = std::env::var("BUCKETFS_PORT")
                .unwrap_or_else(|_| "2581".to_string())
                .parse::<u16>()
                .context("BUCKETFS_PORT is not a valid port number")?;
            let bucketfs_write_password = std::env::var("BUCKETFS_PASSWORD")
                .context("BUCKETFS_PASSWORD is required when EXASOL_HOST is set")?;
            return Ok(Self {
                _container: None,
                container_name: "exasol-db".to_string(),
                host,
                db_port,
                bucketfs_port,
                bucketfs_write_password,
            });
        }

        let container = GenericImage::new(DB_IMAGE, &db_tag())
            .with_exposed_port(DB_PORT.tcp())
            .with_exposed_port(BUCKETFS_PORT.tcp())
            .with_wait_for(WaitFor::message_on_stdout("All stages finished."))
            .with_privileged(true)
            .with_startup_timeout(Duration::from_secs(600))
            .start()
            .await
            .context("starting exasol/docker-db container")?;

        let host = container.get_host().await?.to_string();
        let db_port = container.get_host_port_ipv4(DB_PORT.tcp()).await?;
        let bucketfs_port = container.get_host_port_ipv4(BUCKETFS_PORT.tcp()).await?;
        let bucketfs_write_password = read_bucketfs_password(&container, "WritePasswd").await?;
        let container_name = container.id().to_string();

        Ok(Self {
            _container: Some(container),
            container_name,
            host,
            db_port,
            bucketfs_port,
            bucketfs_write_password,
        })
    }

    /// Open an `exarrow-rs` connection to the mapped DB port, retrying until the
    /// SQL layer accepts logins. The boot log firing does not guarantee the SQL
    /// engine has finished recovery, so we retry for a generous window.
    pub async fn connect(&self) -> Result<Connection> {
        let uri = format!(
            "exasol://sys:exasol@{}:{}/?validateservercertificate=0",
            self.host, self.db_port
        );
        let driver = Driver::new();

        let deadline = std::time::Instant::now() + Duration::from_secs(300);
        loop {
            match open_and_connect(&driver, &uri).await {
                Ok(conn) => return Ok(conn),
                Err(e) => {
                    if std::time::Instant::now() >= deadline {
                        bail!("could not connect to Exasol within timeout; last error: {e}");
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        }
    }

    /// Upload bytes to `<bucket>/<path_in_bucket>` over BucketFS HTTPS PUT.
    ///
    /// Uses blocking reqwest on a dedicated thread so a large `.so`/container
    /// upload never stalls the async runtime, and accepts the self-signed
    /// BucketFS certificate per project rules.
    pub async fn upload_to_bucketfs(&self, path_in_bucket: &str, data: Vec<u8>) -> Result<()> {
        let url = format!(
            "https://{}:{}/{}/{}",
            self.host, self.bucketfs_port, BUCKET, path_in_bucket
        );
        let user = BFS_WRITE_USER.to_string();
        let password = self.bucketfs_write_password.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let client = reqwest::blocking::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_secs(180))
                .build()?;
            let resp = client
                .put(&url)
                .basic_auth(user, Some(password))
                .body(data)
                .send()
                .with_context(|| format!("PUT {url}"))?;
            let status = resp.status();
            if !status.is_success() {
                bail!("BucketFS PUT {url} returned {status}");
            }
            Ok(())
        })
        .await
        .context("BucketFS upload task panicked")?
    }

    /// Export the locally built slim SLC image as a flat-filesystem `.tar.gz`
    /// and push it into BucketFS under `slc/<name>.tar.gz`.
    ///
    /// Exasol extracts archives uploaded to BucketFS, so the container-name in
    /// the language URL is the file name *without* the `.tar.gz` suffix. An SLC
    /// must be a flattened root filesystem (`/exaudf/exaudfclient`), which is
    /// what `docker export` produces — `docker save` would yield OCI layers
    /// that BucketFS cannot use.
    pub async fn load_slc(&self) -> Result<SlcRef> {
        let name = "rustslc";
        let tarball = tokio::task::spawn_blocking(|| export_image_filesystem(SLC_IMAGE))
            .await
            .context("image-export task panicked")??;
        self.upload_to_bucketfs(&format!("slc/{name}.tar.gz"), tarball)
            .await?;
        Ok(SlcRef {
            name: name.to_string(),
        })
    }

    /// Upload a precompiled musl UDF `.so` under `udf/<filename>` and return the
    /// in-database `/buckets/...` path for use in a `%udf_object` directive.
    pub async fn upload_udf(&self, filename: &str, so_bytes: Vec<u8>) -> Result<String> {
        let path = format!("udf/{filename}");
        self.upload_to_bucketfs(&path, so_bytes).await?;
        Ok(format!("/buckets/{BFS_SERVICE}/{BUCKET}/{path}"))
    }

    /// Return the Docker container ID or name for out-of-process inspection.
    pub fn container_id(&self) -> &str {
        &self.container_name
    }

    /// Return the container's primary eth0 IP address (e.g. `172.17.0.3`).
    pub async fn container_inner_ip(&self) -> Result<String> {
        let script = "ip addr show eth0 | awk '/inet /{print $2}' | cut -d/ -f1 | head -1";
        let ip = self
            .exec_in_container(script)
            .await
            .context("container_inner_ip")?;
        if ip.is_empty() {
            anyhow::bail!("could not determine container eth0 IP");
        }
        Ok(ip)
    }

    /// Return the connect-back address for UDFs: the Docker bridge gateway IP combined
    /// with the host-mapped DB port (e.g. `172.17.0.1:32768`).
    ///
    /// UDFs run inside the container. The Docker host gateway (reachable from inside
    /// the container) routes the connect-back as an external-client session through the
    /// host port-mapping. Note: on `2026.latest` the server-side SIGABRT (ADR-015)
    /// still fires regardless of address or transport — this address is architecturally
    /// correct, but the upstream core bug prevents a successful round-trip.
    pub async fn container_connect_back_address(&self) -> Result<String> {
        let script = "ip route show default | awk '/default/ {print $3}' | head -1";
        let gateway = self
            .exec_in_container(script)
            .await
            .context("container_connect_back_address")?;
        if gateway.is_empty() {
            anyhow::bail!("could not determine container default gateway");
        }
        Ok(format!("{}:{}", gateway, self.db_port))
    }

    /// Read UDF client diagnostic logs from inside the container and return them as a string.
    pub async fn dump_udf_logs(&self) -> String {
        let script = concat!(
            "echo '=== /tmp/cb_debug.txt ==='; cat /tmp/cb_debug.txt 2>/dev/null || echo '(not found)';",
            " echo '=== udf_diag.log search ===';",
            " find /buckets /exa/logs /tmp -name 'udf_diag.log' -o -name 'UDFClientDiag*'",
            " 2>/dev/null | sort | tail -10 | xargs -I{} sh -c 'echo \"=== {} ===\"; cat {}' 2>/dev/null"
        );
        match self.exec_in_container(script).await {
            Ok(out) if !out.trim().is_empty() => out,
            Ok(_) => "(no UDF diagnostic log found in /buckets, /exa/logs, or /tmp)".to_string(),
            Err(e) => format!("(exec failed: {e})"),
        }
    }

    /// Execute a shell script inside the container and return trimmed stdout.
    ///
    /// Uses `testcontainers` exec in local mode; falls back to `docker exec
    /// <container_name>` in external (CI) mode.
    async fn exec_in_container(&self, script: &str) -> Result<String> {
        match &self._container {
            Some(container) => {
                let mut result = container
                    .exec(ExecCommand::new(["bash", "-c", script]))
                    .await
                    .context("exec in container")?;
                let bytes = result.stdout_to_vec().await?;
                Ok(String::from_utf8_lossy(&bytes).trim().to_string())
            }
            None => {
                let output = Command::new("docker")
                    .args(["exec", &self.container_name, "bash", "-c", script])
                    .output()
                    .context("docker exec")?;
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
        }
    }
}

/// A registered language-container reference, used to build the SCRIPT_LANGUAGES
/// activation string.
pub struct SlcRef {
    name: String,
}

impl SlcRef {
    /// The `RUST=...` definition string for `ALTER SESSION SET SCRIPT_LANGUAGES`.
    ///
    /// The URL points at the extracted container directory; the `#...` fragment
    /// is the path to the script client inside that directory.
    pub fn script_languages(&self) -> String {
        let dir = format!("{BFS_SERVICE}/{BUCKET}/slc/{}", self.name);
        format!("RUST=localzmq+protobuf:///{dir}?lang=rust#buckets/{dir}/exaudf/exaudfclient")
    }
}

/// Register the SLC for the current session.
pub async fn register_slc(conn: &mut Connection, slc: &SlcRef) -> Result<()> {
    let stmt = format!(
        "ALTER SESSION SET SCRIPT_LANGUAGES='{}'",
        slc.script_languages()
    );
    conn.execute(&stmt)
        .await
        .with_context(|| format!("registering SLC: {stmt}"))?;
    Ok(())
}

/// Run a query expected to yield exactly one VARCHAR cell and return it.
///
/// Callers wrap their projection in `CAST(... AS VARCHAR(...))`/`TO_CHAR` so the
/// result type is always Arrow `Utf8`, sidestepping DECIMAL representation
/// quirks in scalar assertions.
pub async fn query_single_string(conn: &mut Connection, sql: &str) -> Result<Option<String>> {
    use arrow::array::{Array, StringArray};

    let batches = conn
        .query(sql)
        .await
        .with_context(|| format!("query: {sql}"))?;
    let batch = batches
        .into_iter()
        .find(|b| b.num_rows() > 0)
        .ok_or_else(|| anyhow!("query returned no rows: {sql}"))?;
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow!("column 0 is not a Utf8 string array for: {sql}"))?;
    if col.is_null(0) {
        Ok(None)
    } else {
        Ok(Some(col.value(0).to_string()))
    }
}

/// Read the bytes of a precompiled UDF artifact from the workspace target directory.
pub fn read_udf_artifact(lib_filename: &str) -> Result<Vec<u8>> {
    let path = format!("{}/target/release/{}", workspace_root(), lib_filename);
    std::fs::read(&path).with_context(|| format!("reading UDF artifact {path}"))
}

fn workspace_root() -> String {
    // CARGO_MANIFEST_DIR points at crates/it; the workspace root is two up.
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| manifest.to_string())
}

/// Install the process-wide rustls `CryptoProvider` exactly once. Both `ring`
/// and `aws-lc-rs` end up in the dependency graph (via exarrow-rs / reqwest /
/// bollard), so rustls cannot pick a default on its own and panics on first TLS
/// use unless one is installed explicitly.
fn install_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

async fn open_and_connect(driver: &Driver, uri: &str) -> Result<Connection> {
    let db = driver.open(uri)?;
    Ok(db.connect().await?)
}

/// Read a BucketFS credential (`WritePasswd`/`ReadPasswd`) for the default
/// bucket out of the running container's `EXAConf` and base64-decode it.
async fn read_bucketfs_password(
    container: &ContainerAsync<GenericImage>,
    field: &str,
) -> Result<String> {
    let script = format!(
        "awk '/\\[\\[Bucket : default\\]\\]/{{f=1}} f && /{field}/{{print $3; exit}}' /exa/etc/EXAConf"
    );
    let mut result = container
        .exec(ExecCommand::new(["bash", "-c", &script]))
        .await
        .with_context(|| format!("exec reading {field} from EXAConf"))?;
    let raw = String::from_utf8(result.stdout_to_vec().await?)?;
    let b64 = raw.trim();
    if b64.is_empty() {
        bail!("{field} not found in EXAConf");
    }
    decode_base64(b64).with_context(|| format!("decoding {field}"))
}

/// Minimal standard-alphabet base64 decoder (no padding-stripping surprises);
/// avoids pulling a base64 crate into the harness just for one secret.
fn decode_base64(input: &str) -> Result<String> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let chars: Vec<u8> = input.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::new();
    for chunk in chars.chunks(4) {
        let mut buf = 0u32;
        let mut bits = 0;
        for &c in chunk {
            let v = val(c).ok_or_else(|| anyhow!("invalid base64 char"))?;
            buf = (buf << 6) | v;
            bits += 6;
        }
        // Emit the high-order full bytes accumulated in this chunk.
        let bytes_out = (chunk.len() * 6) / 8;
        for i in 0..bytes_out {
            let shift = bits - 8 * (i + 1);
            out.push(((buf >> shift) & 0xff) as u8);
        }
    }
    Ok(String::from_utf8(out)?)
}

/// `docker export` a fresh container of `image` into a gzipped flat-fs tarball.
///
/// The `export | gzip` pipeline runs inside a single shell so the OS streams
/// the multi-hundred-megabyte tar between the two processes. Doing the gzip
/// in-process would require concurrently writing stdin and draining stdout to
/// avoid a pipe-buffer deadlock; delegating to the shell sidesteps that.
fn export_image_filesystem(image: &str) -> Result<Vec<u8>> {
    let create = Command::new("docker")
        .args(["create", image])
        .output()
        .context("docker create")?;
    if !create.status.success() {
        bail!(
            "docker create {image} failed: {}",
            String::from_utf8_lossy(&create.stderr)
        );
    }
    let cid = String::from_utf8(create.stdout)?.trim().to_string();

    let result = (|| -> Result<Vec<u8>> {
        let out = Command::new("bash")
            .arg("-c")
            .arg(format!("set -o pipefail; docker export {cid} | gzip -c"))
            .output()
            .context("docker export | gzip")?;
        if !out.status.success() {
            bail!(
                "docker export {cid} | gzip failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(out.stdout)
    })();

    // Always clean up the throwaway container, regardless of export outcome.
    let _ = Command::new("docker").args(["rm", "-f", &cid]).output();
    result
}

#[cfg(test)]
mod tests {
    use super::decode_base64;

    #[test]
    fn decodes_known_base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), "hello");
        assert_eq!(decode_base64("aGpYMHM4dE5zSk1n").unwrap(), "hjX0s8tNsJMg");
    }
}
