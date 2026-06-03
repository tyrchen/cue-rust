# Issue 0002 — Unset optional fields block concrete export instead of being omitted

Status: fixed in `cc57fa3` · Severity: **medium-high (blocking embedders)** · Found: 2026-06-02
Reported against: `d0181ce` (also reproduces on `ee8220e`, `da88884`)
Component: `crates/encoding` (`evaluated_to_json` struct encoding) and/or
`crates/eval` (concrete evaluation of optional fields)

## Summary

When a closed definition declares an **optional** field (`b?: T`) and a concrete
instance does **not** supply it, exporting the instance to JSON fails with
`cue.eval.incomplete: $.b: incomplete value` (the field's *type* leaks into the
concrete output). In Go `cue`, an optional field with no concrete value is simply
**absent** from concrete export — it does not make the whole value incomplete.

Because almost every real schema has optional fields (`sha256?`, `displayName?`,
`monitorId?`, `nextUpdateAt?`, …), this blocks exporting/round-tripping any
instance that leaves an optional field unset — i.e. essentially all of them.

## Minimal reproduction

```cue
// repro.cue
#S: {a: int, b?: string}
out: #S & {a: 1}
```

```console
$ cue eval -e out repro.cue
{b: string, a: 1}                       # <- optional type retained in the tree

$ cue export -e out repro.cue --out json
Error: cue.eval.incomplete: $.b: incomplete value
```

Expected (Go parity):

```json
{ "a": 1 }
```

Also fails identically with a constrained optional (e.g. a regex like
`#FileRef.sha256?: =~"^[a-f0-9]{64}$"`):

```cue
#S: {a: int, b?: =~"^x$"}
out: #S & {a: 1}     // export -> Error: $.b: incomplete value
```

## Root cause

Two layers contribute:

1. **Evaluation keeps the optional field's type as a value.** `cue eval` shows
   `{b: string, a: 1}` — the unset optional `b` evaluates to its type
   (`string`), not to "absent". So by the time encoding runs, `b` is present in
   the struct map with a non-concrete value.

2. **The JSON encoder inserts every field unconditionally.** In
   `crates/encoding/src/lib.rs` (`evaluated_to_json`, ~line 347):

   ```rust
   EvaluatedValue::Struct(values)
   | EvaluatedValue::PatternedStruct { fields: values, .. }
   | EvaluatedValue::ClosedStruct(values)
   | EvaluatedValue::ClosedPatternedStruct { fields: values, .. } => {
       let mut object = JsonMap::new();
       for (key, value) in values {
           object.insert(key, evaluated_to_json(value)?);   // <- no skip for optional/non-concrete
       }
       Ok(JsonValue::Object(object))
   }
   ```

   When `value` is `OptionalField(_)`, `Top`, `String`-as-kind, or any other
   non-concrete constraint, `evaluated_to_json` returns
   `unsupported(... "incomplete value")` instead of the field being skipped.

   (`EvaluatedValue::OptionalField(_)` does have an explicit arm at line ~377
   that returns `unsupported`, but an unset optional often surfaces as the bare
   *type* (`Top`/`String`-kind), which hits the generic "incomplete" arms — see
   the `eval` output `{b: string}` above.)

## Expected behaviour (Go parity)

During **concrete** export (`concrete: true`, the default `EncodeOptions`):

- An optional field (`b?:`) with **no concrete value** is **omitted** from the
  output object — it does not error and does not appear as its type.
- An optional field that **was** given a concrete value is exported normally.
- Required fields that are still non-concrete remain a genuine
  `incomplete value` error (that part is correct).

## Suggested fix direction

Prefer fixing at the **evaluation/export boundary** so both the encoder and
`Value::validate` agree:

1. When evaluating a struct for concrete export, drop optional fields whose value
   did not reduce to a concrete value (i.e. is still `OptionalField`, a bare
   `Kind`, `Top`, or an unresolved constraint). This matches Go, where optional
   fields are not part of the concrete value unless filled.

2. Failing that, in `evaluated_to_json` (and the YAML/TOML/CUE-concrete paths),
   when `concrete` export is requested, **skip** struct entries whose value is
   `OptionalField(_)` or a non-concrete constraint rather than erroring — while
   still erroring on a non-concrete **required** field.

Option 1 is cleaner because `evaluate_export(ExportOptions{ include_optional:
false, .. })` already exists as the intended mechanism; the unset-optional field
should not be materialized into the struct map at all when `include_optional` is
false. Today `Value::evaluate`/`to_serde_json_value` retains it.

## Impact on the SRE-Suite embedder

Blocks export/round-trip for nearly every artifact, because optional fields are
ubiquitous in the `sre-cue` schemas:

- `#FileRef.sha256?`, `#Actor.displayName?`, `#Link.title?`
- `#Runbook` `severityHint?`, `descriptionRef?`, `roles?`, `communication?`, `references?`
- `#WarRoom` `communicationsLead?`, `operationsLead?`, `scribe?`, `channels.zoom?`, …
- `#StatusUpdate.nextUpdateAt?`, `#Incident.mitigatedAt?/resolvedAt?`
- `#AlertBinding.match.monitorId?/alertName?`, `severityHint?`

The Phase-0 spike's `incident` example round-trips only because it happens to
set every field it declares; every other example (`service`, `slo`, `warroom`,
`postmortem`, `runbook`) fails on an unset optional.

## Suggested regression tests

```rust
assert_eq!(export_json("#S: {a: int, b?: string}\nout: #S & {a:1}"), json!({"a":1}));
assert_eq!(export_json("#S: {a: int, b?: string}\nout: #S & {a:1, b:\"x\"}"), json!({"a":1,"b":"x"}));
// required non-concrete field still errors:
assert!(export_json_err("#S: {a: int, b: string}\nout: #S & {a:1}").contains("incomplete"));
```
