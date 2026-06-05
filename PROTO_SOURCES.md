# Protocol Source Files

This file records the provenance of vendored `.proto` files in this workspace.

## zmqcontainer.proto

- **Local path:** `crates/exa-proto/proto/zmqcontainer.proto`
- **Commit SHA:** `75dc742299d3bfa5fb1d6e587097984017868364`
- **Fetch date:** 2026-06-05

### URL used

```
https://raw.githubusercontent.com/exasol/script-languages/75dc742299d3bfa5fb1d6e587097984017868364/exaudfclient/base/exaudflib/zmqcontainer.proto
```

### Note on the source location

The original task specified this URL:

```
https://raw.githubusercontent.com/exasol/script-languages-release/75dc742299d3bfa5fb1d6e587097984017868364/flavors/standard-EXASOL-all-python3/flavor_base/context/global_exaudfclient/zmqcontainer.proto
```

That URL returns HTTP 404. The pinned commit SHA
`75dc742299d3bfa5fb1d6e587097984017868364` does not exist in the
`exasol/script-languages-release` repository; it is a valid commit in the
`exasol/script-languages` repository, where the file lives at
`exaudfclient/base/exaudflib/zmqcontainer.proto`. The file was fetched from
that repository at the same pinned commit, so the recorded SHA is authoritative
and reproducible. The byte content is `proto2`, `optimize_for = LITE_RUNTIME`,
defining the `column_type` / `iter_type` enums and the `exascript_*` messages
used by the ZMQ exaudfclient protocol.
