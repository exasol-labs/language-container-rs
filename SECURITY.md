# Security Policy

## Reporting a vulnerability

Please report security issues privately to the maintainers via GitHub Security
Advisories (the "Report a vulnerability" button under the repository's *Security*
tab) rather than opening a public issue.

## Dependency auditing

Every push runs `cargo deny check advisories` and `cargo deny check licenses`
(see `deny.toml`). The dependency tree carries no copyleft licenses; all licenses
are MIT/Apache-2.0/BSD/ISC-compatible, and third-party notices for the
distributed binary are bundled in `THIRD-PARTY-LICENSES.md`.

## Known accepted advisories

Two advisories are currently ignored in `deny.toml`, both transitive via
`parquet` (Apache Arrow) and outside our control:

- **RUSTSEC-2024-0436** — `paste` is unmaintained but not vulnerable.
- **GHSA-2f9f-gq7v-9h6m** (CVE-2026-43868, CWE-789, CVSS 5.3) — `thrift <0.23.0`.
  The fix (`thrift 0.23.0`) is not yet on crates.io, and `parquet`'s `^0.17`
  semver constraint blocks a `[patch.crates-io]` override. `parquet 59.x` drops
  `thrift` entirely but is not yet released, and `adbc_core` currently caps
  `arrow-schema <59`. **Re-evaluate when** `parquet 59.x` is on crates.io **and**
  `adbc_core` supports `arrow-schema >=59`.
