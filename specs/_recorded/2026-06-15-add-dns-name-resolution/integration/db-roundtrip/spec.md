# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container across the supported version matrix — registering the slim SLC, uploading the UDF artifacts to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. A single `it-runner` binary, compiled once, drives every version in the matrix. The DNS-gate scenario added here requires outbound network access from the runner so the external hostname resolves.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: DNS gate resolves an external hostname end-to-end

* *GIVEN* a running Exasol container with the slim Alpine SLC registered for the session
* *AND* the `resolv-udf` UDF uploaded and a SCALAR `RUST` script `resolv_udf` referencing its BucketFS `.so` path
* *WHEN* the harness runs `SELECT resolv_udf('www.exasol.com')` as part of the roundtrip suite
* *THEN* the query MUST return a single non-null VARCHAR value
* *AND* the returned string MUST parse as a valid `IpAddr`
<!-- /DELTA:NEW -->
