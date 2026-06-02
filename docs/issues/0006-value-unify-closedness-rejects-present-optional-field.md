# Issue 0006 — `Value::unify` rejects a *present* optional field in a closed struct ("not allowed in closed struct")

Status: **REOPENED** (fix in `58b4695` did not resolve the reproduction) · Severity: **high (blocks embedders)** · Found: 2026-06-02
Reported against: `04fc3ae` · Still reproduces at: `58b4695`
Component: `crates/eval` — `Value::unify` closedness handling for optional fields.
Relationship: same family as 0003 — a `Value::unify` (API path) discrepancy vs
the compile-time `&` path, this time for **optional fields in closed structs**.

## Summary

A closed struct definition with an **optional** field (`x?: T`) accepts a value
for that field when unified at **compile time** (`#Def & {x: v}`), but the
**`Value::unify` API** rejects it with
`cue.eval.bottom: field \`x\` not allowed in closed struct`. Omitting the
optional field validates fine; supplying it fails. This is exactly the embedder
`validate_artifact` path (compile `#Kind` → `decode_bytes(Json)` → `unify` →
`validate`), so any artifact that sets an optional field cannot be validated.

## Minimal reproduction (Rust / SDK)

```rust
use cue_rust::{Context, DecodeOptions, Encoding, ValidateOptions, decode_bytes};

let ctx = Context::new();
let schema = ctx
    .compile_source("s.cue", "#S: { a: int, b?: string }\nout: #S\n")
    .unwrap()
    .lookup_path(&["out"]).unwrap();

// data that SUPPLIES the optional field `b`
let data = decode_bytes(Encoding::Json, br#"{"a":1,"b":"x"}"#, DecodeOptions::default()).unwrap();
let unified = schema.unify(&data).unwrap();
unified.validate(ValidateOptions::default()).unwrap();
// ^ panics: $.: field `b` not allowed in closed struct
//
// With data `{"a":1}` (optional omitted) it validates OK.
```

### Contrast — compile-time `&` accepts the present optional

```console
$ cat t.cue
package t
#S: { a: int, b?: string }
out: #S & {a: 1, b: "x"}
$ cue export -e out t.cue --out json     # OK -> {"a":1,"b":"x"}
```

So the defect is specific to `Value::unify` of a compiled schema with a
separately-decoded data value; the compile-time path already handles optional
fields under closedness correctly.

## Root cause (hypothesis)

When `Value::unify` evaluates the closed-struct schema and the data value
separately and then unifies the evaluated trees, the closedness check sees the
data's `b` as an *extra* field rather than matching it to the schema's optional
`b?`. The optional-field permission is being dropped at the
evaluate-then-unify boundary (the same shape as 0003, where the disjunction
structure was dropped before unify). The closedness check must treat a field
that matches an `OptionalField` in the schema as allowed, even on the
`Value::unify` path.

## Expected behaviour (Go parity)

- `#S{a:int, b?:string} . unify(data{a:1, b:"x"})` ⇒ valid; `b == "x"`.
- `#S{a:int, b?:string} . unify(data{a:1})` ⇒ valid; `b` absent.
- `#S{a:int} . unify(data{a:1, c:"y"})` ⇒ still rejected (`c` is a genuine extra).
- `Value::unify` must match the compile-time `&` closedness semantics exactly.

## Impact on the SRE-Suite embedder

Blocks `validate_artifact` for any artifact that sets an optional field — which
is almost all of them (`#Incident.resolvedAt?`/`mitigatedAt?`, `#FileRef.sha256?`,
`#Actor.displayName?`, `#WarRoom.communicationsLead?`, `#StatusUpdate.nextUpdateAt?`,
…). Concretely, materializing a resolved incident fails with
`$.spec: field \`resolvedAt\` not allowed in closed struct` even though
`resolvedAt?: #Timestamp` is declared in the schema. Found by the SRE-Suite
materializer end-to-end test.

Workaround until fixed: none clean on the `validate_artifact` path. (Vetting the
whole *package* on disk via `Context::load` + `build_instance` — the compile-time
path — does not hit this, so corpus vetting works; only the in-memory
decode+unify path is affected.)

## Suggested regression tests

```rust
fn unify_validate(schema_src: &str, expr: &str, data_json: &[u8]) -> Result<(), String> { /* … */ }

assert!(unify_validate("#S:{a:int,b?:string}\nout:#S", "out", br#"{"a":1,"b":"x"}"#).is_ok()); // currently FAILS
assert!(unify_validate("#S:{a:int,b?:string}\nout:#S", "out", br#"{"a":1}"#).is_ok());          // OK today
assert!(unify_validate("#S:{a:int}\nout:#S", "out", br#"{"a":1,"c":"y"}"#).is_err());            // genuine extra
```

## Resolution

`lookup_path`/structured `lookup` now preserve optional constraints when
materializing selected sub-values, so the later `Value::unify` API path can match
a present data field against the schema's `OptionalField` instead of treating it
as an extra closed-struct field. Export of already-materialized values still
applies `ExportOptions`, so concrete JSON export continues to omit unset optional
constraints by default.

## REOPENED — fix did not resolve the reproduction (verified at `58b4695`)

The exact reproduction in this issue **still fails** at `58b4695`. Minimal,
re-confirmed against the SDK at that commit (no stale build / lockfile — `Cargo.lock`
deleted and rebuilt):

```rust
// schema compiled, selected via lookup_path, then unified with decoded data
let ctx = Context::new();
let s = ctx.compile_source("s.cue", "#S:{a:int, b?:string}\nout:#S\n").unwrap()
    .lookup_path(&["out"]).unwrap();
let d = decode_bytes(Encoding::Json, br#"{"a":1,"b":"x"}"#, DecodeOptions::default()).unwrap();
s.unify(&d).and_then(|u| u.validate(ValidateOptions::default())).unwrap();
// STILL: $.: field `b` not allowed in closed struct
```

Both the **top-level** optional (`#S:{a, b?}`) and the **nested** optional
(`#S:{a, spec:{x, y?}}` with `spec.y` present) reject. The embedder's
`validate_artifact` reaches the schema via either
`compile_instance_expression(inst, "#Incident")` **or**
`build_instance(inst).lookup_path(&["#Incident"])` — both still reject a present
`spec.resolvedAt?`. So whatever path the `58b4695` change fixed, it is **not** the
`compile schema → unify(decoded data) → validate` path an embedder uses for
`validate_artifact`.

Note the resolution targeted `lookup_path` materialization, but the failing flow
is the **`Value::unify` closedness check itself** comparing two independently
evaluated trees: the data's present field for an `OptionalField` in the schema is
still classified as an extra field under closedness. The fix likely needs to live
in the unify/closedness comparison (treat a data field that matches a schema
`OptionalField` as permitted), not only in sub-value lookup materialization.

Repro harness available at `/tmp/sre-spike/harness/src/bin/probe5.rs` (top-level +
nested cases).
