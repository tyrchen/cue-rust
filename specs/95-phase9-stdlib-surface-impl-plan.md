# Phase 9 Stdlib Surface Implementation Plan

Status: Implemented
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

This batch narrows the `stdlib/strings-full-surface` and `stdlib/list-full-surface` compatibility gaps with a broad, pure builtin pass. The previous Phase 9 import batch made `strings` and `list` imports usable for high-frequency calls; this batch fills the remaining deterministic functions that can be implemented cleanly against the current evaluated-value model.

## Vendor Evidence

- `vendors/cue/pkg/strings/pkg.go` registers the `strings` package surface.
- `vendors/cue/pkg/list/pkg.go` registers the `list` package surface.
- `vendors/cue/pkg/strings/testdata/*.txtar` and `vendors/cue/pkg/list/testdata/*.txtar` exercise generated builtin examples.
- `vendors/cue/cue/testdata/eval/issue500.txtar` and `vendors/cue/cue/testdata/scalars/embed.txtar` use imported `strings` and `list` functions from ordinary CUE expressions.

## Scope

Worth building in this batch:

- Complete the pure `strings` native function surface registered in `vendors/cue/pkg/strings/pkg.go`.
- Add deterministic `list` manipulation functions: `Drop`, `Take`, `Slice`, `Reverse`, `FlattenN`, `UniqueItems`, `SortStrings`, `IsSortedStrings`, plus numeric aggregate/range functions.
- Keep resource caps on generated strings and lists.
- Keep concrete type validation explicit and return domain bottoms on invalid arity, invalid type, invalid indexes, empty numeric aggregates, and exhausted resource limits.
- Add SDK and vendor-borrowed integration coverage.
- Move `stdlib/strings-full-surface` to supported coverage, and narrow the list gap reason to advanced comparator/schema functions that need richer package values or evaluator hooks.

Out of scope for this batch:

- `list.Sort`, `list.SortStable`, and `list.IsSorted` with arbitrary comparator schemas.
- Package-provided CUE constants such as `list.Ascending` and `list.Descending`.
- Partial-application validator syntax beyond ordinary concrete builtin calls.

## Rust Shape

- Eval: extend `evaluate_builtin_call` with pure `strings.*` and `list.*` names.
- Eval helpers: use one shared set of typed-argument extractors, checked count/index conversion, generated-size guards, and value equality for uniqueness.
- Numeric list aggregates and ranges use exact decimal arithmetic rather than binary floats.
- Tests: add focused SDK tests and vendor-borrowed reduced cases that assert both successful values and representative errors.
- Compatibility: add supported cases for complete strings surface and broad list surface while keeping an expected gap only for comparator/schema-backed list functions.

## Verification Plan

Run once after the batch is fully implemented and reviewed:

- `cargo build --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo +nightly fmt -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `cargo audit`
- `cargo deny check`
- `make vendor-corpus compat-report fuzz-smoke`
- `cargo install --path apps/cue --force`
- CLI smoke covering newly supported strings/list builtins and previous Phase 9 flows.
