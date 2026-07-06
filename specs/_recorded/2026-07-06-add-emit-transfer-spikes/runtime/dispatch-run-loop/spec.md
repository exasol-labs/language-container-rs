# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering bridge row materialisation, emit buffering, UDF error propagation, and connect-back availability. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant, formatting NUMERIC/DATE/TIMESTAMP/VARCHAR cells into the proto string block (`value_to_block_string`) and parsing them back on decode (`decode_string_block`). The exact wire-format strings the Exasol engine parses are fixed contracts: `DATE_FORMAT = "%Y-%m-%d"`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.9f"` (full nanosecond precision, engine-truncated to the declared column precision), and fixed-point decimal via `Decimal`'s `Display`. Any performance optimisation of the formatting/parsing path — whether a hand-rolled fast formatter, a promoted columnar transport spike, or a fast decimal/date parser — must leave those wire bytes and the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: A promoted emit fast-path encoder stays byte-identical to the row path

* *GIVEN* an `EmitBuffer` whose internal formatting of NUMERIC/DATE/TIMESTAMP/VARCHAR cells into the proto string block is produced by a performance-optimised encoder selected after benchmarking (for example a hand-rolled or `itoa`/`ryu`-based formatter replacing `chrono`'s generic `format` / the `Decimal` `Display` impl, or a columnar transport path promoted from a spike)
* *AND* the equivalent rows expressed through the current `chrono`/`Display`-based row path over the same declared `ColumnMeta` output schema
* *WHEN* `EmitBuffer::to_proto` serialises rows spanning the full `ExaType` range — including NULL cells and multiple columns sharing one block type
* *THEN* the resulting `ExascriptTableData` MUST be byte-identical to the output the current `chrono`/`Display`-based row path produces for every representable value, so downstream Exasol parsing — which depends on the exact `%Y-%m-%d` (`DATE_FORMAT`), `%Y-%m-%d %H:%M:%S%.9f` (`TIMESTAMP_EMIT`), and fixed-point decimal format strings — is unaffected
* *AND* the encoder MUST preserve the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged — the running byte estimate, the mid-run threshold flush, and the end-of-`run` tail flush
* *AND* a NULL cell MUST NOT occupy a slot in its type block, and the dense row-major-interleaved block layout `to_proto` produces MUST be preserved exactly
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A promoted ingest fast-path decoder round-trips byte-identically

* *GIVEN* an `InputRowSet` whose string-block parsing of NUMERIC/DATE/TIMESTAMP cells is produced by a performance-optimised decoder selected after benchmarking (the symmetric ingest mirror of the promoted emit fast-path), replacing `chrono`'s `parse_from_str` / decimal parsing in `decode_string_block`
* *WHEN* `InputRowSet::from_proto` decodes an `ExascriptTableData` covering the full `ExaType` range including NULL cells
* *THEN* each decoded `Value` MUST equal the `Value` the current `chrono`-based decode path produces for the same wire bytes, preserving lossless round-trip with the emit path
* *AND* the decoder MUST accept every format the emit path can produce — TIMESTAMP with 0..9 fractional-second digits (`%.f`), DATE as `%Y-%m-%d`, and fixed-point DECIMAL — with no loss of precision
* *AND* a NULL cell MUST NOT consume a slot in its type block, preserving the per-type cursor advancement `from_proto` guarantees
<!-- /DELTA:NEW -->
