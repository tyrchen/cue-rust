# Issue 0001 — Unifying a defaulted disjunction with a non-default member yields a spurious conflict

Status: fixed in `cc57fa3` + `24b733d` · Severity: **high (blocking embedders)** · Found: 2026-06-02
Reported against: `d0181ce` (also reproduces on `ee8220e`, `da88884`)
Component: `crates/eval` (`unify_values` / `disjuncts_from` / `Default` handling)

## Summary

A defaulted disjunction such as `*"a" | "b" | "c"` unifies correctly with its
**default** member (`"a"`) but spuriously fails with
`cue.eval.conflict: conflicting values string and string` when unified with any
**non-default** member (`"b"`, `"c"`). The same holds for bool defaults
(`*false | bool`) and for the disjunction used as a *field type* inside a
definition. This is standard, extremely common CUE — `*x | y` defaults appear in
almost every real schema — so it blocks any embedder that unifies a schema with
concrete data (the canonical `vet` / `validate_artifact` flow).

Go `cue` accepts all of these.

## Minimal reproductions

```cue
// repro.cue
#H: {x: *"a" | "b" | "c"}
h_default: #H & {x: "a"}   // OK   -> {"x":"a"}
h_other:   #H & {x: "b"}   // FAIL -> conflicting values string and string
h_third:   #H & {x: "c"}   // FAIL -> conflicting values string and string

#B: {auto: *false | bool}
b_default: #B & {auto: false}  // OK
b_other:   #B & {auto: true}   // FAIL -> conflicting values bool and bool

#C: {n: *15 | int}
c_other:   #C & {n: 30}        // FAIL -> conflicting values number and number
```

The conflict message tracks the operand kind: `string and string`, `bool and
bool`, `number and number` — confirming it is the generic
`Default(default-member) & non-default-member` collapse, independent of type.

```console
$ cue export -e h_default repro.cue --out json
{ "x": "a" }
$ cue export -e h_other repro.cue --out json
Error: cue.eval.bottom: $.x: conflicting values string and string
$ cue export -e b_other repro.cue --out json
Error: cue.eval.bottom: $.auto: conflicting values bool and bool
```

Plain (non-defaulted) disjunctions are **not** affected:

```cue
#G: {x: "a" | "b" | "c"}
g_b: #G & {x: "b"}   // OK -> {"x":"b"}
g_c: #G & {x: "c"}   // OK -> {"x":"c"}
```

Inline restatement within one struct literal is **not** affected either:

```cue
out: {x: *"a" | "b" | "c", x: "b"}   // OK -> {"x":"b"}
```

It is specifically **`Default(disjunction)` unified with a value** that breaks.

## Root cause

When a field's value is a defaulted disjunction, evaluation produces an
`EvaluatedValue::Default(Box<…>)` wrapper. `disjuncts_from`
(`crates/eval/src/lib.rs:6679`) collapses that wrapper to **only the default
alternative**, discarding the other members:

```rust
fn disjuncts_from(value: EvaluatedValue) -> Vec<Disjunct> {
    match value {
        EvaluatedValue::Bottom(_) => Vec::new(),
        EvaluatedValue::Disjunction(disjuncts) => disjuncts,
        EvaluatedValue::Default(value) if matches!(value.as_ref(), EvaluatedValue::Bottom(_)) => Vec::new(),
        EvaluatedValue::Default(value) => vec![Disjunct { value, default: true }], // <-- only the default kept
        value => vec![Disjunct { value: Box::new(value), default: false }],
    }
}
```

Then in `unify_values` (`crates/eval/src/lib.rs:7548`):

```rust
(EvaluatedValue::Default(left), right) => unify_values(*left, right, span),
(left, EvaluatedValue::Default(right)) => unify_values(left, *right, span),
```

the `Default(left)` arm **unwraps to the bare default scalar** before unifying.
So `Default("a")` unified with `"b"` becomes `unify_values("a", "b")` →
`"a" & "b"` → `conflicting values string and string`.

The bug is the assumption that a `Default` wrapper contains only the chosen
default. For a disjunction with defaults, the wrapper must retain **all**
alternatives (with the default flag) so that unification can still select a
non-default member; default resolution should happen *after* unification, not
before.

Confirmed evaluated tree (probe):

```text
#H & {x: "b"}  ->  ClosedStruct({ "x": Bottom { code: "cue.eval.conflict",
                                 message: "conflicting values string and string" } })
```

## Expected behaviour (Go parity)

- `(*a | b | c) & b` ⇒ `b` (a non-default member is selected; no conflict).
- `(*a | b | c) & a` ⇒ `a`.
- `(*a | b | c)` with no further constraint ⇒ `a` (default wins only when the
  disjunction is left ambiguous).
- The default marker only decides which branch wins among **otherwise-valid**
  alternatives; it must never *exclude* a non-default branch during unification.

## Suggested fix direction

Preserve the disjunction structure across the `Default` boundary during
unification instead of eagerly unwrapping to the default scalar. Concretely,
one of:

1. Represent a defaulted disjunction as
   `Default(Box<Disjunction[ Disjunct{value, default:true}, … ]>)` and make the
   `(Default(left), right)` arm in `unify_values` recurse into the *disjunction*
   (so `unify_disjunction_with_value` runs over all members), then re-apply
   default resolution to the unified result. Today the wrapper already boxes a
   value — the loss happens earlier, in `disjuncts_from` / wherever the
   `Default` is built for a disjunction field type.

2. Alternatively, in the `(Default(left), right)` arm, if `right` is concrete
   and `*left` is the discarded default of a disjunction, fall back to unifying
   against the **full** disjunction rather than the default scalar.

Either way the invariant to restore: **default selection is a post-unification
step, not a pre-unification narrowing.**

## Impact on the SRE-Suite embedder

This is the blocking finding from the Phase-0 cue-rust parity spike. Defaulted
disjunctions are used throughout the `sre-cue` schemas:

- `#Actor.type: *"user" | "bot" | "ai_agent" | "service"` (any non-`user` actor fails)
- `#Visibility: *"internal" | "public" | "restricted" | "security" | "legal"`
- `#FileRef.mediaType: *"text/markdown" | … | "application/x-ndjson"`
  (`#WarRoom.timelineRef`, `#Postmortem.timelineRef` pin the non-default
  `"application/x-ndjson"` — both fail)
- `safety.canAutoExecute: *false | bool`, `requiresApproval: *true | bool`
- `#ActionItem.priority: *"p2" | "p0" | "p1" | "p3"`, `status: *"open" | …`
- `#Postmortem.metadata.status: *"draft" | "review" | …`

Until fixed, the embedder cannot validate or export any artifact that sets a
non-default value for one of these fields.

## Suggested regression tests

```rust
// crates/eval (or sdk) tests
assert_eq!(export_json("(*\"a\"|\"b\"|\"c\") & \"b\""), json!("b"));
assert_eq!(export_json("(*false|bool) & true"), json!(true));
assert_eq!(export_json("#H: {x: *\"a\"|\"b\"|\"c\"}\nout: #H & {x:\"b\"}"), json!({"x":"b"}));
// default still wins when unconstrained:
assert_eq!(export_json("*\"a\"|\"b\"|\"c\""), json!("a"));
```
