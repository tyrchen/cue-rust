# Phase 9 Open List Implementation Plan

Status: Implemented
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

This batch removes the `eval/open-list-ellipsis` compatibility gap from the Phase 9 dashboard. Open lists are a high-value CUE semantic primitive: schemas commonly use `[...]`, `[...T]`, and fixed prefixes with ellipsis tails to validate arbitrary-length data.

## Vendor Evidence

- `vendors/cue/cue/testdata/basicrewrite/010_lists.txtar` contains fixed list unification and open-list tail validation cases such as `[1, 2, ...>=4 & <=5] & [1, 2, 4, 8]`.
- `vendors/cue/cue/testdata/definitions/typocheck.txtar` and synced compile fixtures repeatedly use `[...]`, `[...string]`, and open list fields inside definitions.

## Scope

Worth building in this batch:

- Parse list ellipsis tails as list shape, not as concrete list items.
- Lower open lists into ADT list expressions with an optional tail constraint expression.
- Evaluate open lists as list values with prefix items plus a tail constraint.
- Unify open lists with closed lists by applying the tail constraint to extra elements.
- Unify open lists with other open lists by preserving a prefix and intersecting tail constraints.
- Keep open lists non-concrete for JSON/YAML/TOML export and concrete validation.
- Display open lists in CUE output as `[prefix, ...]` or `[prefix, ...constraint]`.
- Move `eval/open-list-ellipsis` from expected gap to supported dashboard coverage.

Out of scope for this batch:

- Struct ellipsis/open struct semantics.
- Postfix spread or explicit-open experiments.
- Comprehensions, dynamic labels, aliases, interpolation, and cycle scheduling.

## Rust Shape

- Syntax: change `Expr::List` to hold `items` plus optional `tail`.
- ADT: change `SemanticExpr::List` similarly.
- Eval: add `EvaluatedValue::OpenList { items, tail }` and centralize list unification in a helper.
- Encoding: treat open lists as schema values in CUE output and incomplete values in concrete encoders.
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
- CLI smoke covering open list eval/export profile behavior.
