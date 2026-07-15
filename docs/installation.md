[language-container-rs](../README.md) › [docs](index.md) › Installation

# Installing the language container

There are two ways to get the `RUST` script language registered in an Exasol database:

| Path | When to use |
|------|-------------|
| [Automated](#automated-install-scriptsinstallsh) | `exapump` has direct network access to both BucketFS and the DB SQL port (e.g. a local Docker-db). One command does everything. |
| [Manual](#manual-install) | No `exapump`/BucketFS network access — e.g. Exasol SaaS, or any hosted platform that only exposes a BucketFS upload UI or REST API. Every step is a `curl`/SQL command or a UI action, no Docker or Rust toolchain required. |

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

## Next step — write your first UDF

With `RUST` registered, you're ready to write, build, and deploy an actual UDF. That's
covered in full in [Writing a Rust UDF](writing-a-udf.md) — scaffolding a UDF crate,
building the `.so` with `cargo-exasol-udf`, and the `CREATE SCRIPT` SQL to register it.
