# Issue 0005 — SDK facade does not re-export `Severity` / `Span` / `Diagnostic` / `ByteOffset`

Status: fixed · Severity: low (embedder ergonomics) · Found: 2026-06-02
Reported against: `04fc3ae`
Component: `crates/sdk` (the `cue_rust` facade re-export list)

## Summary

The `cue_rust` SDK facade re-exports `DiagnosticReport`, but **not** the types
needed to inspect the diagnostics it contains:

- `cue_rust_source::Severity` (the `Error | Warning | Info` enum),
- `cue_rust_source::Span` and `cue_rust_source::ByteOffset`,
- `cue_rust_source::Diagnostic` (reachable only as the element type of
  `DiagnosticReport::diagnostics()`, but not nameable).

`DiagnosticReport::diagnostics()` returns `&[Diagnostic]`, and
`Diagnostic::severity() -> Severity`, `Diagnostic::primary_span() -> Option<Span>`,
`Span::start()/end() -> ByteOffset`, `ByteOffset::get() -> u32` are all callable.
But an embedder that wants to **map** a diagnostic into its own type cannot name
`Severity` to `match` on it, nor `Span`/`ByteOffset` to read offsets in a
type-checked way — those paths are not exported by the facade, and reaching into
`cue_rust_source` directly defeats the "consume only the façade" guidance.

## Impact

An embedder mapping `cue_rust` diagnostics into its own report type (e.g. to
render structured CLI/UI errors) must currently either:

1. depend on `cue_rust_source` directly (breaks the single-façade contract), or
2. derive severity from the `Debug` spelling of `severity()`
   (`format!("{:?}", d.severity())` → `"Error"`/`"Warning"`/`"Info"`), which is
   not a stable contract.

The SRE-Suite evaluator (`sre-cue-eval`) hit this mapping `cue_rust` diagnostics
into its `VetReport`/`Diagnostic`. It is using the documented-interim Debug
mapping, isolated in one helper, pending this facade addition.

## Suggested fix

Add to the `crates/sdk` facade re-export list:

```rust
pub use cue_rust_source::{ByteOffset, Diagnostic, Severity, Span};
```

(`DiagnosticReport` is already exported; these complete the diagnostic-inspection
surface so an embedder can map diagnostics without naming a lower crate.)

Implemented by re-exporting the diagnostic inspection types from `cue_rust` and
adding `ByteOffset::get()` so embedders can read offsets without accessing the
tuple field directly.

## Suggested regression test

```rust
// compiles only if the facade exports the diagnostic-inspection types by name.
fn _facade_surface(d: &cue_rust::Diagnostic) -> (cue_rust::Severity, Option<cue_rust::Span>) {
    (d.severity(), d.primary_span())
}
```
