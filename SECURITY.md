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

No advisories are currently ignored in `deny.toml`.
