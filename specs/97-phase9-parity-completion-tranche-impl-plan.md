# Phase 9 Parity Completion Tranche Implementation Plan

Status: Active
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

This tranche replaces one-gap-at-a-time patching with one integrated parity pass over the remaining high-value gaps surfaced by `target/compatibility/core.json` and vendored tests. The goal is not to fake full CUE parity by adding narrow fixtures; the goal is to implement the missing architectural surfaces that move the Rust implementation toward production use.

## Gap Set

Build in this tranche:

- Parser and AST support for dynamic labels, basic pattern labels, string interpolation, and for/if comprehensions.
- Compiler lowering for those new AST forms into explicit semantic nodes rather than ad-hoc string matching.
- Evaluator support for deterministic comprehensions, concrete string interpolation, dynamic field labels, first-class string validators, cycle-safe self references, and stricter closedness/pattern checks.
- Loader support for module-local import registry resolution before compilation.
- List comparator/schema surface sufficient for `list.Sort`, `list.SortStable`, `list.IsSorted`, `list.Ascending`, and `list.Descending` over currently evaluable number/string/struct inputs.
- Compatibility report updates that remove only gaps proven by broad executable coverage.

## Vendor Evidence

- `vendors/cue/cue/testdata/comprehensions/*.txtar`
- `vendors/cue/cue/testdata/interpolation/*.txtar`
- `vendors/cue/cue/testdata/compile/labels.txtar`
- `vendors/cue/cue/testdata/cycle/*.txtar`
- `vendors/cue/cue/testdata/definitions/*.txtar`
- `vendors/cue/cue/load/import_test.go`
- `vendors/cue/pkg/list/testdata/*.txtar`
- `vendors/cue/pkg/strings/testdata/gen.txtar`

## Implementation Shape

Parser and AST:

- Extend tokenization with `for` and `if` keyword recognition while preserving current identifier behavior for non-keywords.
- Add AST forms for interpolation segments, dynamic labels, pattern labels, and comprehension clauses.
- Keep the scanner tolerant and bounded; interpolation depth is capped by `SourceLimits`.

Compiler:

- Lower dynamic labels as expression labels and make the evaluator own label evaluation.
- Lower comprehensions as explicit semantic comprehension nodes with bound variable names and source spans.
- Preserve import paths on build instances so module-local imports can resolve without network access.

Evaluator:

- Evaluate dynamic labels only to concrete string labels; non-concrete labels become bottom.
- Evaluate comprehensions with bounded generated field/list counts.
- Evaluate interpolation by forcing concrete string/number/bool/null/bytes fragments and rejecting structs/lists.
- Replace depth-only recursion handling with an operation-local in-progress cache that returns cycle bottoms for unresolved cycles and reuses evaluated vertex results for resolved cycles.
- Model string validators as constraints and unify them with concrete strings.
- Add comparator helper evaluation for list sorting over the schema shapes currently used by upstream fixtures.

Loader:

- Resolve imports within the current module root by mapping import paths to loaded local package directories.
- Keep unsupported remote registry imports visible as diagnostics until a real registry provider exists.

## Non-Goals

- Network module registry client.
- Full OpenAPI/JSON Schema export.
- Full upstream scheduler internals. The implementation must be cycle-safe and deterministic, but it can be a Rust-native operation cache rather than a literal port.

## Done Criteria

- Each gap currently marked expected-fail either moves to supported executable coverage or has a narrower, truthful remaining reason.
- Vendor-borrowed tests exercise at least one realistic fixture per gap area.
- Full quality gates pass after one consolidated review.
- The CLI smoke covers imports, interpolation, comprehensions, dynamic labels, list sorting, string validators, cycle behavior, and closedness/pattern behavior.
