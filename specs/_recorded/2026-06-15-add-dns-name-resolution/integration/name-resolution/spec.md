# Feature: name-resolution

Proves that a Rust UDF running inside the Exasol sandbox can resolve external DNS hostnames end-to-end, giving operators a one-line check that the SLC's resolver configuration (the `/conf/hosts` and `/conf/resolv.conf` symlinks) is wired correctly in any deployment.

## Background

The `resolv_udf` is a SCALAR `RUST` script of the form `resolv_udf(host VARCHAR) RETURNS VARCHAR`. Its body resolves the supplied hostname via the standard library's `getaddrinfo`-backed `ToSocketAddrs` and returns the first resolved IP address as a string. It performs no connect-back, opens no CONNECTION object, and starts no Exasol session — pure name resolution — so SCALAR is safe (no SIGABRT risk). DNS resolution depends on the SLC tarball shipping `/etc/resolv.conf` as a symlink into `/conf/`, which the database populates inside the sandbox. These scenarios require a running Exasol container with the slim Alpine SLC registered and outbound network access from the runner.

## Scenarios

### Scenario: resolv_udf resolves an external hostname to a valid IP

* *GIVEN* a running Exasol container with the slim Alpine SLC registered for the session
* *AND* the `resolv-udf` UDF uploaded and a SCALAR `RUST` script `resolv_udf` referencing its BucketFS `.so` path
* *WHEN* the harness runs `SELECT resolv_udf('www.exasol.com')`
* *THEN* the query MUST return a single non-null VARCHAR value
* *AND* the returned string MUST parse as a valid `IpAddr`

### Scenario: resolv_udf surfaces an error for an unresolvable hostname

* *GIVEN* a running Exasol container with the slim Alpine SLC registered for the session
* *AND* the `resolv-udf` UDF uploaded and a SCALAR `RUST` script `resolv_udf` referencing its BucketFS `.so` path
* *WHEN* the harness runs `resolv_udf` against a hostname that cannot be resolved
* *THEN* the query MUST fail rather than return a value
* *AND* the failure MUST surface a UDF error message rather than silently masking the resolution failure
