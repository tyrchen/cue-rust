# Phase 9 Alias Label Implementation Plan

Status: Implemented
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

This batch removes the `compile/aliases` compatibility gap for the high-value alias label subset. CUE fixtures use aliases to bind a local identifier to a field label, then reference that identifier from lets or sibling fields.

## Vendor Evidence

- `vendors/cue/cue/testdata/basicrewrite/aliases/aliases.txtar` covers `a=_a: _` and `c=d: 3`, including let references through aliases.

## Scope

Worth building in this batch:

- Parse field heads with an optional identifier alias before `=`.
- Preserve the real field label for output and feature interning.
- Resolve alias identifiers to the aliased field feature during AST-to-ADT lowering.
- Allow local lets to reference sibling fields and alias labels in the same struct scope.
- Move `compile/aliases` from expected gap to supported dashboard coverage.
- Borrow reduced vendor coverage from the upstream alias fixture.

Out of scope for this batch:

- Dynamic labels, pattern labels, and label expressions.
- Alias labels in comprehension clauses.
- General value aliases outside the field-label subset already represented by `let`.

## Rust Shape

- Syntax: add `FieldDecl::alias: Option<String>` and parse `alias=label: value`.
- Compiler: replace field-presence scope entries with reference bindings from identifier text to actual field label text.
- Eval: reuse existing field reference evaluation; alias references lower to the same feature as the real field label.
- Tests: add parser, SDK, compatibility dashboard, and vendor-borrowed reduced tests.

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
- CLI smoke covering alias-backed eval/export and existing Phase 9 core flows.
