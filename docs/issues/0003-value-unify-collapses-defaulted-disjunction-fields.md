# Issue 0003 — `Value::unify` collapses defaulted-disjunction fields before unifying (0001 regression on the API path)

Status: open · Severity: **high (blocks embedders)** · Found: 2026-06-02
Reported against: `cc57fa3` ("Fix defaulted disjunction and optional export")
Component: `crates/eval` — `Value::unify` (`crates/eval/src/lib.rs:814`) interacting
with `evaluate`/`evaluate_export` default resolution.
Relationship: this is the **residual half of [0001]**. `cc57fa3` fixed the
*compile-time* unification path (`unify_values` now uses `default_disjuncts_from`),
but the **`Value::unify` API path** — the one embedders use for
`validate_artifact` — still fails.

## Summary

After `cc57fa3`, unifying a defaulted disjunction with a non-default member works
when both sides are compiled **together** in one instance
(`#Actor & {type:"service"}` in CUE source ⇒ OK). But the public **`Value::unify`
API** — compile a schema `Value`, separately compile/decode a data `Value`, then
`schema.unify(&data)` — still yields
`cue.eval.bottom: conflicting values string and string` for any non-default
member. This is the exact path a Rust embedder uses to validate an in-memory JSON
artifact against a schema kind (`compile_instance_expression("#Kind")` +
`decode_bytes(Json)` + `unify` + `validate`).

## Minimal reproduction (Rust / SDK)

```rust
use cue_rust::{Context, DecodeOptions, Encoding, ValidateOptions, decode_bytes};

let ctx = Context::new();
// schema with a defaulted-disjunction field
let schema = ctx
    .compile_source("s.cue", "#Actor: { id: string, type: *\"user\" | \"bot\" | \"service\" }\nout: #Actor\n")
    .unwrap()
    .lookup_path(&["out"]).unwrap();

// data supplies the NON-default member
let data = decode_bytes(Encoding::Json, br#"{"id":"x","type":"service"}"#, DecodeOptions::default()).unwrap();

let unified = schema.unify(&data).unwrap();
unified.validate(ValidateOptions::default()).unwrap();
// ^ panics: $.type: conflicting values string and string
```

`type:"user"` (the default) validates fine; `type:"service"` / `type:"bot"` fail.

### Contrast — the compile-time path is already fixed

```console
$ cat t.cue
package t
#Actor: { id: string, type: *"user" | "bot" | "service" }
direct: #Actor & {id:"x", type:"service"}
nested: ({actor: #Actor}) & {actor: {id:"x", type:"service"}}
$ cue export -e direct t.cue --out json   # OK -> {"id":"x","type":"service"}
$ cue export -e nested t.cue --out json   # OK -> {"actor":{"id":"x","type":"service"}}
```

So the defect is specific to **`Value::unify` of two already-built values**, not to
nesting or to compile-time `&`.

## Root cause

`Value::unify` (`crates/eval/src/lib.rs:814`) evaluates **both** operands to an
`EvaluatedValue` *before* calling `unify_values`:

```rust
pub fn unify(&self, other: &Self) -> Result<Self, EvalError> {
    let options = ExportOptions { include_optional: true, ..ExportOptions::default() };
    let left = self.evaluate_export(options)?;   // <-- collapses nested defaults
    let right = other.evaluate_export(options)?;
    let unified = unify_values(left, right, None);
    Ok(Self::from_evaluated(unified))
}
```

The problem: evaluating a struct whose field is a defaulted disjunction
**collapses that field to the bare default scalar** — the `Default`/`Disjunction`
wrapper does not survive evaluation. Probe against the schema value:

```text
schema #Actor.type . evaluate()        = Ok(String("user"))
schema #Actor.type . evaluate_export() = Ok(String("user"))
```

The alternatives `"bot" | "service"` are already gone before `unify_values` runs.
`unify_values`'s new `default_disjuncts_from` machinery (the `cc57fa3` fix) is
therefore never reached for these fields — by the time it runs, `left` is
`String("user")` and `right` is `String("service")`, which conflict.

`default_disjuncts_from` only helps when the operand is still
`Default(_)`/`Disjunction(_)`. Inside an evaluated **struct**, the field has
already been reduced to the default scalar (`String("user")`), so the wrapper is
lost.

## Expected behaviour

`schema.unify(&data)` must behave identically to compiling
`schema & data` in one instance:

- `#Actor{type: *"user"|"bot"|"service"} . unify(data{type:"service"})` ⇒
  a value whose `type` is `"service"`, validating OK.
- `… . unify(data{type:"user"})` ⇒ `"user"` OK (default also accepted).
- Required vs optional and closedness behaviour unchanged.

The API path and the compile-time path should be the same function of the same
inputs.

## Suggested fix direction

The collapse happens during struct evaluation, so fixing only `unify_values` is
insufficient for `Value::unify`. Options:

1. **Preserve default-disjunction structure through struct evaluation** when the
   value is going to be unified (i.e. do not eagerly reduce
   `Default(Disjunction[...])` fields to the default scalar during
   `evaluate`/`evaluate_export`; keep the wrapper and only resolve the default at
   the *final* concrete-export/validate step). This is the most correct fix —
   default selection is a terminal step, not a per-field reduction — and it makes
   `Value::unify` Just Work.

2. **Make `Value::unify` not pre-evaluate to collapsed trees.** Unify at a layer
   where the disjunction structure is still present (e.g. unify the underlying
   compiled/ADT representations, or re-run compile-time-style unification),
   instead of `evaluate_export` → `unify_values` on two reduced trees.

3. As a narrower mitigation: have struct field evaluation retain a
   `Default(Disjunction[...])` (rather than the bare default member) so that
   `default_disjuncts_from` in `unify_values` can still recover non-default
   members; resolve the default only at concrete export/validate.

Option 1/3 (keep the wrapper, resolve default last) aligns with how `cc57fa3`
already models defaults in `unify_values`.

## Impact on the SRE-Suite embedder

This is the **only** remaining spike failure after `cc57fa3`. It blocks
`CueEvaluator::validate_artifact` ([21-cue-evaluator](https://…)) for any artifact
that sets a non-default value of a defaulted-disjunction field — which is most of
them (`#Actor.type` ≠ `user`, `#Visibility` ≠ `internal`,
`#FileRef.mediaType` = `application/x-ndjson`, `safety.canAutoExecute` = true,
`#ActionItem.priority` ≠ `p2`, …). The compile-time **vet/export** path
(round-trip of whole artifacts) already works post-`cc57fa3`; it is specifically
the `compile schema` + `unify(decoded data)` validation flow that remains broken.

## Suggested regression tests

```rust
// SDK-level: the embedder path must match the compile-time path.
fn unify_validate(schema_src: &str, expr: &str, data_json: &str) -> Result<(), String> { /* … */ }

assert!(unify_validate(
    "#A: {t: *\"x\" | \"y\" | \"z\"}\nout: #A",
    "out",
    r#"{"t":"y"}"#,
).is_ok());            // currently FAILS
assert!(unify_validate("#A:{t:*\"x\"|\"y\"}\nout:#A", "out", r#"{"t":"x"}"#).is_ok()); // default, OK today
assert!(unify_validate("#A:{t:*\"x\"|\"y\"}\nout:#A", "out", r#"{"t":"q"}"#).is_err()); // not a member
```
