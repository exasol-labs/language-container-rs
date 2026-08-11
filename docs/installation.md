[language-container-rs](../README.md) › [docs](index.md) › Installation

# Installing the language container

There are three ways to get the `RUST` script language registered in an Exasol database:

| Path | When to use |
|------|-------------|
| [Automated](#automated-install-scriptsinstallsh) | `exapump` has direct network access to both BucketFS and the DB SQL port (e.g. a local Docker-db). One command does everything. |
| [Exasol Personal](#exasol-personal-install) | An Exasol Personal deployment. It publishes no BucketFS endpoint at all, so the container travels over SSH into the VM's BucketFS directory instead of being uploaded. |
| [Manual](#manual-install) | No `exapump`/BucketFS network access — e.g. Exasol SaaS, or any hosted platform that only exposes a BucketFS upload UI or REST API. Every step is a `curl`/SQL command or a UI action, no Docker or Rust toolchain required. |

## Release assets

Each release publishes two SLC tarballs on the [GitHub Releases](https://github.com/exasol-labs/language-container-rs/releases) page:

| Asset | Architecture |
|-------|--------------|
| `lc-rust-<version>.tar.gz` | x86_64 (the unsuffixed name is the x86_64 build) |
| `lc-rust-<version>-aarch64.tar.gz` | aarch64 |

## Platform support matrix

Pick a row by where the target database runs, then follow that row's install path.

| Platform | Release tarball | Install path | UDF build target |
|----------|------------------|---------------|-------------------|
| Docker-db (local, x86_64) | `lc-rust-<version>.tar.gz` | [Automated](#automated-install-scriptsinstallsh) | `cargo exasol-udf build` (glibc cdylib, host default `x86_64-unknown-linux-gnu`) |
| Exasol SaaS (x86_64 backend) | `lc-rust-<version>.tar.gz` | [Manual](#manual-install) | `cargo exasol-udf build` on an x86_64 host (glibc cdylib), or `--target x86_64-unknown-linux-gnu` |
| Exasol Personal (Apple Silicon, aarch64) | `lc-rust-<version>-aarch64.tar.gz` | [Exasol Personal](#exasol-personal-install) | `cargo exasol-udf build` on/in a Linux aarch64 host (glibc cdylib, host default `aarch64-unknown-linux-gnu`); a macOS host cannot emit a Linux `.so` natively |

The UDF `.so`'s architecture must match the SLC's. Build on a host of the same architecture as the target database, or cross the gap with `--target` (see [Writing a Rust UDF §13](writing-a-udf.md#13-build-and-deploy)).

## Automated install (`scripts/install.sh`)

`scripts/install.sh` builds the Docker image, exports the container filesystem, uploads it to BucketFS, and registers the `RUST` script language — all in one command:

```bash
scripts/install.sh \
  --host localhost \
  --password exasol \
  --bfs-password <write-password>
```

The BucketFS write password for the Docker image can be read with:

```bash
docker exec exasol-db bash -c \
  "xmllint --xpath '//BucketFSService[@id=\"bfsdefault\"]/Bucket[@id=\"default\"]/WritePasswd/text()' \
  /exa/etc/EXAConf"
```

Full option reference: `scripts/install.sh --help`

## Exasol Personal install

Exasol Personal publishes only the SQL port (`8563`) from its VM — there is no
BucketFS HTTP endpoint to upload to, so the [automated path](#automated-install-scriptsinstallsh)
dead-ends at its upload step. Personal's engine reconciles BucketFS from the VM
filesystem instead: extracting the container into
`/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM creates a real
bucket within about a second, visible to UDFs at
`/buckets/<service>/<bucket>/<slc-name>/`.

`scripts/install.sh` switches to this SSH transport when you pass `--deployment`;
it copies the container over SSH, extracts it, then registers the language:

```bash
scripts/install.sh --deployment my-db --password <db-password>
```

It builds the container for the host architecture (on Apple Silicon: an aarch64
SLC), or reuses a prebuilt tarball when `SLC_TARBALL` is set:

```bash
SLC_TARBALL=/path/to/lc-rs.tar.gz \
  scripts/install.sh --deployment my-db --password <db-password>
```

The `--deployment` path needs `jq`, `ssh`/`scp`, `exapump`, and Docker unless
`SLC_TARBALL` is set; it fixes the SQL host/port at `127.0.0.1:8563`, registers
with `ALTER SYSTEM`, and needs no BucketFS password. Full option reference:
`scripts/install.sh --help`.

> **Building UDFs for Personal:** the UDF `.so` itself must be built on — or
> inside — a Linux environment matching the deployment's architecture (an aarch64
> Linux host or container for an Apple Silicon Personal). A macOS host cannot emit
> a Linux `.so` natively: there is no Linux cross-linker or sysroot, so neither a
> native `cargo` build nor `cargo exasol-udf build` produces a loadable artifact
> there. Build in the same Linux aarch64 environment the SLC uses.

> **Deploying a UDF `.so` on Personal:** Personal has no BucketFS HTTP endpoint,
> so `writing-a-udf.md` §13's "upload via the HTTP API" step does not apply. Copy
> the built `.so` into the VM's BucketFS directory over the same SSH transport the
> install script uses — the `udf/` prefix maps to `/buckets/<service>/<bucket>/udf/`:
>
> ```bash
> # sshPort is reassigned on every `exasol start`; read it fresh each time.
> ssh_port="$(jq -r '.connection.sshPort' \
>   ~/.exasol/personal/deployments/<name>/deployment.json)"
> scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
>   -i ~/.exasol/personal/deployments/<name>/local/node_access.pem -P "$ssh_port" \
>   libmy_udf.so root@127.0.0.1:/var/lib/exa/bucketfs/bfsdefault/default/udf/
> ```
>
> The `.so` is then visible to `CREATE SCRIPT` at
> `/buckets/bfsdefault/default/udf/libmy_udf.so`.

| Step | What it does, and why |
|------|-----------------------|
| Read the connection details | Takes `connection.sshPort` and the node key from `~/.exasol/personal/deployments/<name>/` on **every** run. `exasol start` reassigns the SSH port, so a remembered one is wrong after the first restart. |
| Copy and extract | `scp` over that port, then extract into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` and confirm `exaudf/exaudfclient` landed executable. |
| Register | `ALTER SYSTEM SET SCRIPT_LANGUAGES` over `8563`, so the registration survives a restart. The current value is read first and the `RUST` entry appended to it, so entries added by `exasol slc install` are preserved. |

Re-running after `exasol stop && exasol start` is the supported way to recover:
it picks up the reassigned SSH port and replaces its own `RUST` entry without
disturbing the others.

### Exasol Personal on a cloud backend

The steps above cover a **local** Personal deployment (a VM on this machine).
Personal can also run on a cloud backend (`aws`/`azure`/`exoscale`/`stackit`):
that VM reaches the DB over the network and exposes the ordinary BucketFS HTTP
endpoint, so the SSH transport above does not apply to it — it uses the same
HTTP upload-and-register path as the [automated install](#automated-install-scriptsinstallsh).

`scripts/install.sh --deployment` handles both cases from the same flag: it
reads `deployment.json`'s `.backend` field in
`~/.exasol/personal/deployments/<name>/` and picks the transport at runtime —
`"local"` keeps the SSH path above unchanged; any other value resolves the DB
host, port, and user from `deployment.json`'s `.connection` object and the DB
password from the sibling `secrets.json`'s `.dbPassword`, then uploads over
HTTP exactly as the automated path does.

Because cloud Personal provisions no BucketFS password, `--bfs-password` is
**required** for a cloud deployment (unlike the local path, which needs none):

```bash
scripts/install.sh --deployment my-cloud-db --bfs-password <bfs-write-password>
```

`--host`, `--port`, `--user`, and `--password` still work as explicit
overrides of the descriptor-derived values, and `--scope` behaves as it does
without `--deployment` (default `SESSION`) — the cloud path does not force
`SYSTEM` the way the local path does. Full option reference:
`scripts/install.sh --help`.

## Manual install

Use this path when `exapump` can't reach BucketFS or the DB directly. Every step below
is a plain `curl`/SQL command (or a UI action), so it works from any machine with
network access to the platform's API — no Docker, no Rust toolchain.

### Step 1 — Download the prebuilt release tarball

Every version-bumped merge to `main` publishes a [GitHub Release](https://github.com/exasol-labs/language-container-rs/releases)
with a single `lc-rust-<version>.tar.gz` asset — CI builds it from `Dockerfile.alpine`'s
`artifact` stage and renames it for release (e.g. `lc-rust-0.21.0.tar.gz` for `v0.21.0`).

```bash
curl -fsSL -o rustslc.tar.gz \
  https://github.com/exasol-labs/language-container-rs/releases/download/v<VERSION>/lc-rust-<VERSION>.tar.gz
```

Pick `<VERSION>` from the releases page, or `gh release list --repo exasol-labs/language-container-rs`.

**Naming — this is the one thing that must stay consistent for the rest of the guide:**
the filename must remain `rustslc.tar.gz`. This name (minus `.tar.gz`) is embedded
verbatim in the BucketFS upload destination, the directory BucketFS auto-extracts it
to, and the `SCRIPT_LANGUAGES` string in step 4. If you need several SLC versions
installed side by side, pick a different name for the `-o` flag above and swap
`rustslc` for it everywhere below — otherwise just use `rustslc` as-is.

### Step 2 — Upload `rustslc.tar.gz` to BucketFS

Pick whichever channel is available on your platform. In all three, upload straight to
the bucket root as `rustslc.tar.gz` — no subfolder needed, which keeps the destination
path (and the path embedded in `SCRIPT_LANGUAGES` in step 4) as short as possible.

#### a) BucketFS upload UI

Any platform that provides a BucketFS file browser (e.g. Exasol SaaS's "Files" tab):
drop `rustslc.tar.gz` at the bucket root. BucketFS auto-extracts recognized archives on
upload regardless of which channel does the uploading, so there's no explicit "extract"
step — right after upload you should see `rustslc.tar.gz` appear in the browser.

The BucketFS service/bucket names differ per platform — the SaaS ones are given in 2c
below; for any other platform confirm the names via its UI or docs before step 4.

#### b) Raw HTTP PUT

For an on-prem/Docker BucketFS that's reachable over the network, but without
`exapump` installed:

```bash
curl -X PUT -T rustslc.tar.gz -u w:<BFS_WRITE_PASSWORD> \
  http://<HOST>:2580/bfsdefault/default/rustslc.tar.gz
```

BucketFS endpoints are addressed as `/<service>/<bucket>/<path>` — `bfsdefault/default`
is the on-prem/Docker default; swap it for whatever service/bucket your platform uses.
Use `https://` and port `2581` (add `--insecure` for the self-signed Docker-db cert) if
the BucketFS service requires TLS. `w` is the fixed BucketFS write-username; the
password is the bucket's write password (for a local Docker-db, read it with the
`xmllint`/`EXAConf` snippet in the [automated install](#automated-install-scriptsinstallsh) section above).
`-u` sends the same Basic-Auth credential as embedding it in the URL, but avoids putting credentials in the URL (so they don’t end up in URL-logging proxies/access logs).
To keep the password out of shell history and `ps` output, pass only the username (`-u w`) and let `curl` prompt for the password.

#### c) Exasol SaaS REST API

SaaS doesn't expose the raw BucketFS ports at all, so on SaaS this is the only path
that isn't the UI. A couple of SaaS-specific things to know first:

- The BucketFS service/bucket on SaaS is always `uploads/default` (**not**
  `bfsdefault/default`, which is the on-prem/Docker default).
- Auth is `Authorization: Bearer <PAT>` — a SaaS personal access token.
- The API needs your SaaS `accountID` and the target `databaseID` as inputs — there is
  **no** API endpoint to list or discover them (the SaaS OpenAPI spec at
  `https://cloud.exasol.com/openapi.json` has no bare `/accounts` listing route;
  `accountId` only ever appears as a required path parameter). Get both from the SaaS
  web console before starting this step. Once you have `accountID`, you can confirm
  `databaseID` by listing databases in that account and matching by name:
  ```bash
  curl -H "Authorization: Bearer <PAT>" \
    https://cloud.exasol.com/api/v1/accounts/<accountID>/databases
  ```
  Use `cloud-staging.exasol.com` instead of `cloud.exasol.com` if you're on the staging
  environment.

Upload is a two-step presigned-URL dance (root key, so no `/`-encoding to worry about):

```bash
curl -X POST -H "Authorization: Bearer <PAT>" \
  "https://cloud.exasol.com/api/v1/accounts/<accountID>/databases/<databaseID>/files/rustslc.tar.gz"
# → {"url": "<presigned PUT URL>"}

curl -X PUT --upload-file rustslc.tar.gz "<presigned PUT URL>"
```

The presigned URL expires in ~600s and is signed for `host` only — don't add extra
headers, and run both commands back-to-back.

### Step 3 — Confirm the tarball is uploaded

List the bucket through whichever channel you used in step 2 (UI file browser,
`GET .../files`, or `exapump bucketfs ls` if you have it) and confirm `rustslc.tar.gz`
is present — that's the full confirmation available at this stage, on every platform.

BucketFS extracts the archive internally so the language container can load it, but
that extracted content is for the container's own use, not a browsable part of the
bucket listing — so `rustslc.tar.gz` being present is the green light to continue. The
definitive end-to-end confirmation comes from step 4 (the language registers without
error) and from actually writing and running a UDF (see
[Writing a Rust UDF](writing-a-udf.md)).

### Step 4 — Register the language via SQL

First check whether `SCRIPT_LANGUAGES` already has a value — a real cluster likely
already has `PYTHON3`/`JAVA`/`R` registered, and the `ALTER ... SET` below **replaces**
the whole value, so the new `RUST` entry must be appended to whatever is already there,
not used to overwrite it:

```sql
SELECT * FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES';
```

Build the registration string from the BucketFS service/bucket used in step 2
(on-prem/Docker: `bfsdefault/default`; Exasol SaaS: `uploads/default`) plus the
`rustslc` name fixed in step 1, and append it to the existing value:

> This guide uploads to the bucket root, so `rustslc` is the whole path. If your
> platform's upload UI forces a destination folder, or you choose to mirror
> `scripts/install.sh`'s own layout (which uploads under `slc/<name>` — see
> `BFS_PATH` in the script), prefix that folder onto `rustslc` in *both* URIs below,
> e.g. `slc/rustslc` instead of `rustslc`.

```sql
-- current session only
ALTER SESSION SET SCRIPT_LANGUAGES='<existing value, if any> RUST=localzmq+protobuf:///<bfs-service>/<bucket>/rustslc?lang=rust#buckets/<bfs-service>/<bucket>/rustslc/exaudf/exaudfclient';

-- persists across sessions (requires admin)
ALTER SYSTEM SET SCRIPT_LANGUAGES='<existing value, if any> RUST=localzmq+protobuf:///<bfs-service>/<bucket>/rustslc?lang=rust#buckets/<bfs-service>/<bucket>/rustslc/exaudf/exaudfclient';
```

## Troubleshooting

### `22002 VM crashed`

`22002 VM crashed` almost always means the engine could not execute the UDF client — not a bug in the UDF's Rust code. Check the `SCRIPT_LANGUAGES` registration first.

Fix: confirm the `#` fragment in the `RUST=...` entry points at the `exaudfclient` **executable**, not its containing directory, and has **no leading slash**:

```
# correct
...#buckets/bfsdefault/default/rustslc/exaudf/exaudfclient

# wrong: points at the directory, not the executable
...#buckets/bfsdefault/default/rustslc/exaudf/

# wrong: leading slash
...#/buckets/bfsdefault/default/rustslc/exaudf/exaudfclient
```

Re-run the `ALTER SESSION`/`ALTER SYSTEM SET SCRIPT_LANGUAGES` statement (Step 4 above, or `scripts/install.sh` with or without `--deployment`) with the corrected fragment, then retry the failing UDF call.

## Next step — write your first UDF

With `RUST` registered, you're ready to write, build, and deploy an actual UDF. That's
covered in full in [Writing a Rust UDF](writing-a-udf.md) — scaffolding a UDF crate,
building the `.so` with `cargo-exasol-udf`, and the `CREATE SCRIPT` SQL to register it.
