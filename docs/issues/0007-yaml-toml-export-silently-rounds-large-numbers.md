# Issue 0007 — YAML/TOML export silently rounds large CUE numbers

Status: fixed · Severity: high (silent data corruption) · Found: 2026-06-03
Component: `crates/encoding`

## Summary

JSON export preserves arbitrary-precision CUE number text through
`serde_json::Number`, but YAML and TOML export currently convert values that do
not fit `i64` to `f64`. Large integers and high-precision decimals are therefore
rounded without an error.

## Reproduction

```cue
n: 9223372036854775808
```

Current exports:

```text
json: { "n": 9223372036854775808 }
yaml: n: 9.223372036854776e18
toml: n = 9223372036854776000.0
```

## Expected behavior

Encoding must never silently change a numeric value. TOML should emit integers
only when they fit TOML's signed 64-bit integer representation, and otherwise
return an error. YAML should preserve exact values where the backend can
represent them exactly; otherwise return an error instead of rounding to `f64`.

## Suggested fix

Centralize YAML/TOML number conversion behind exact conversion helpers:

- keep `i64` integer output when parsing as `i64` succeeds;
- reject integer text that cannot fit the target integer representation;
- only use floating output when the parsed `f64` round-trips exactly to the
  original decimal value;
- add regression tests for `9223372036854775808` and a high-precision decimal.

## Resolution

YAML/TOML number conversion now rejects values that cannot be represented
exactly by the target backend. `i64` integers still export as integers, and
floating output is only used when the `f64` short decimal representation is
numerically identical to the original CUE number text.
