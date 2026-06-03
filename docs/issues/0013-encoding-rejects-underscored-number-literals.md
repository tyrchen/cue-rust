# Issue 0013 — External encoders reject underscored CUE number literals

Status: fixed in working tree · Severity: medium (valid values fail export) · Found: 2026-06-03
Component: `crates/encoding`

## Summary

The scanner accepts `_` as a digit separator in number literals and the
evaluator carries those numbers as valid `EvaluatedValue::Number` strings, but
external JSON/YAML/TOML encoders pass the original text directly to downstream
number parsers. Valid CUE numbers such as `1_000` therefore fail concrete export.

## Reproduction

```cue
x: 1_000
```

```console
$ cue eval -e x repro.cue
1_000

$ cue export --out json repro.cue
error: unsupported value for Json: invalid JSON number
```

## Expected behavior

Digit separators are source syntax, not numeric value. External encoders should
strip `_` before parsing numbers for JSON/YAML/TOML output while preserving exact
numeric value checks.

## Suggested fix

- Normalize number strings by removing `_` before external encoding.
- Keep exactness checks for YAML/TOML floats after normalization.
- Add regression coverage for JSON, YAML, and TOML export of underscored
  integer literals.

## Resolution

External encoders now normalize CUE digit separators before parsing numbers for
JSON/YAML/TOML output. The exact YAML/TOML float check still runs after
normalization, so inexact values are still rejected. Unit coverage verifies that
`1_000` exports as an external numeric `1000` for JSON, YAML, and TOML, and the
CLI repro now succeeds.
