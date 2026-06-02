# Issue 0004 — A default expression that references a sibling field does not see values supplied by later unification

Status: open · Severity: medium (blocks computed-default policies) · Found: 2026-06-02
Reported against: `cc57fa3`
Component: `crates/eval` — lazy field-reference resolution inside a default
expression (`x: bool | *(<expr over sibling fields>)`) under unification.

## Summary

A field whose value is a **default expression** that references **sibling
fields** — e.g.

```cue
#P: {
    severity: string
    required: bool | *(severity == "sev1" || …)
}
```

computes the default against the field's **declared type/default**, not against
the **concrete value supplied by a later unification** (`#P & {severity:"sev1"}`).
So `severity` inside the default expression resolves to `string` (the type), the
comparison `severity == "sev1"` is `false`, and `required` is wrong.

Writing the *same* fields as a single inline struct literal (no separate
unification step) computes the default correctly. So the bug is specifically that
a default expression's sibling references are bound/evaluated **before** external
unification fills those siblings, and are not re-resolved afterwards.

## Minimal reproduction

```cue
// repro.cue
package r
#P: {
    severity: string
    echo: *(severity)              // should mirror severity once unified
    eq:   *(severity == "sev1")    // should be true when severity == "sev1"
}
a: #P & {severity: "sev1"}
```

```console
$ cue eval -e a repro.cue
{echo: string, eq: false, severity: "sev1"}     # echo == `string` (the type!), eq == false

$ cue export -e a repro.cue --out json
Error: cue.eval.incomplete: $.echo: incomplete value   # echo never became "sev1"
```

Expected (Go parity): `echo: "sev1"`, `eq: true`.

### Contrast — inline struct (no separate unification) is correct

```cue
c: {
    severity: "sev1"
    required: bool | *(severity == "sev1")
}
// c.required == true   (correct)
```

And the comparison / boolean / numeric machinery itself is fine in isolation:

```cue
sev: "sev1"
orv:      sev == "sev0" || sev == "sev1"     // true
gt:       75 > 60                            // true
defform:  bool | *(sev == "sev1")            // true   (default expr over a *concrete* sibling)
```

`defform` works because `sev` is already concrete in the same scope; the bug only
appears when the referenced sibling becomes concrete via a **later `&`
unification** of the enclosing definition.

## Root cause (hypothesis)

The default expression's sibling references are resolved against the definition's
own field environment at the point the `Default(expr)` is built/evaluated, i.e.
against `severity: string`. When `#P & {severity:"sev1"}` unifies, the `severity`
*field* is narrowed to `"sev1"`, but the already-evaluated default expression
(and its captured reference to `severity`) is not recomputed against the unified
environment — it keeps the pre-unification binding (`string`), yielding
`incomplete`/`false`.

The fix should make the default expression a lazy thunk that resolves its field
references against the **final unified struct environment**, so default
computation happens after sibling fields are filled (consistent with CUE's lazy,
order-independent field semantics).

## Expected behaviour (Go parity)

- `(#P & {severity:"sev1"}).echo` ⇒ `"sev1"`.
- `(#P & {severity:"sev1"}).eq` ⇒ `true`.
- `(#P & {severity:"sev3", durationMinutes:75}).required` ⇒ `true`
  (`durationMinutes > 60`).
- Default expressions referencing siblings must observe values filled by any
  later unification of the enclosing struct/definition.

## Impact on the SRE-Suite embedder

Affects the `policies/postmortem_required.cue` predicate
([20-cue-schemas § 2a.2](…)):

```cue
#PostmortemRequired: {
    severity: #Severity
    userVisible: *false | bool
    durationMinutes: *0 | int
    required: bool | *(
        severity == "sev0" || severity == "sev1" ||
        (severity == "sev2" && userVisible) ||
        durationMinutes > 60)
}
```

`(#PostmortemRequired & {severity:"sev1", …}).required` returns `false` for every
input (should be `true` for sev0/sev1, user-visible sev2, or >60min). This is a
P1-policy capability (org-tunable overlays), **not** on the M0 critical path —
the M0 round-trip/vet/validate flow does not depend on computed-default
predicates — but it blocks the policy-overlay feature whenever it lands. A
workaround until fixed: compute the `required` decision in Rust
(`sre-core`/`sre-workflow`) rather than as a CUE default expression, or pass the
predicate inputs already-concrete in one struct literal.

## Suggested regression tests

```rust
assert_eq!(export_json("#P:{s:string, e: *(s)}\na: #P & {s:\"x\"}"), json!({"s":"x","e":"x"}));
assert_eq!(export_json("#P:{s:string, eq: *(s==\"x\")}\na:#P & {s:\"x\"}").pointer("/eq"), Some(&json!(true)));
```
