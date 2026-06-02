# cue-rust issues

Tracked defects found while embedding `cue-rust` as the CUE engine for an
external project (the SRE-Suite Phase-0 parity spike, 2026-06-02). Each file is a
self-contained, reproducible bug report against a specific commit with a root
cause and a suggested fix direction.

History: 0001 + 0002 were filed first and **fixed in `cc57fa3`**. Re-running the
spike against `cc57fa3` surfaced two residuals on the *embedder API* path — 0003
(the `Value::unify`/`validate_artifact` half of 0001) and 0004 (default-expression
sibling resolution). Both were then **fixed in `24b733d` + `04fc3ae`**. As of
`04fc3ae` the full SRE-Suite spike passes **24/24** with every real consumption
path green (`validate_artifact` whole-artifact unify+validate with non-default
disjunction members; `#PostmortemRequired` predicate consumed via whole-struct
export).

| # | Title | Severity | Component | Status |
| --- | --- | --- | --- | --- |
| [0001](./0001-defaulted-disjunction-non-default-unify-conflict.md) | Unifying a defaulted disjunction (`*a \| b \| c`) with a non-default member yields a spurious `conflicting values` | high (blocks embedders) | `crates/eval` | **fixed `cc57fa3`** (compile-time) + `24b733d` (API path) |
| [0002](./0002-unset-optional-fields-block-concrete-export.md) | Unset optional fields (`b?: T`) block concrete export instead of being omitted | medium-high (blocks embedders) | `crates/encoding` + `crates/eval` | **fixed `cc57fa3`** |
| [0003](./0003-value-unify-collapses-defaulted-disjunction-fields.md) | `Value::unify` collapses defaulted-disjunction fields to the default before unifying (residual of 0001 on the API/`validate_artifact` path) | high (blocks embedders) | `crates/eval` (`Value::unify`) | **fixed `24b733d`** |
| [0004](./0004-default-expr-sibling-reference-not-resolved-after-unification.md) | A default expression referencing a sibling field doesn't see values supplied by later unification (`x: bool \| *(sibling == …)`) | medium (blocks computed-default policies) | `crates/eval` | **fixed `04fc3ae`** (whole-struct export) |
| [0005](./0005-sdk-facade-does-not-reexport-diagnostic-severity-span.md) | SDK facade doesn't re-export `Severity`/`Span`/`Diagnostic`/`ByteOffset`, so an embedder can't name them to map diagnostics | low (embedder ergonomics) | `crates/sdk` | open |

### Verified non-issue

The earlier note about selecting a `*default | T` field directly via
`lookup_path`/`compile_instance_expression("…​.field")` was rechecked on
`04fc3ae` and did not reproduce. Direct field selection preserves the defaulted
disjunction for `eval` (`*"x" | string`, `*false | bool`), and concrete export of
the selected field resolves the default correctly (`"x"`, `false`). This is now
covered by a SDK regression test so future changes cannot reintroduce the stale
edge.

## Reproduction environment

- Repo HEAD when filed: `d0181ce` (`chore: make binary cue not cue-rs`).
- Also reproduces on `ee8220e` and `da88884`.
- Repro via the `cue` CLI (`cargo build -p cue-rs` → binary `cue`) or the SDK
  library path (`Context::compile_source` → `lookup_path` → `evaluate` /
  `to_serde_json_value`).

## Why these matter together

Both are exercised by the single most common embedder operation: **unify a schema
(with defaults and optional fields) against a concrete instance, then export to
JSON** (the `cue vet` + `validate_artifact` flow). `*x | y` defaults and `field?:`
optionals appear in essentially every real-world CUE schema, so until both are
fixed an embedder can only round-trip artifacts that (a) set every defaulted
disjunction to its default and (b) supply every optional field. Plain
disjunctions, regex constraints (`=~` / `!~`), closedness, dynamic labels
(`{[string]: string}`), decimal-string number carriage, and nested struct
validation were all verified **working** in the same spike.
