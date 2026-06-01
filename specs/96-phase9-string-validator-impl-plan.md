# Phase 9 String Validator Implementation Plan

Status: Active
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

The previous stdlib surface batch implemented `strings.MinRunes` and `strings.MaxRunes` as ordinary two-argument boolean calls. Vendor fixtures also use these builtins as one-argument validators, for example `strings.MaxRunes(3) & "foo"`. This batch closes that semantic gap so the supported `strings` package claim matches upstream behavior more closely.

## Vendor Evidence

- `vendors/cue/pkg/strings/manual.go` documents `MinRunes` and `MaxRunes` as field constraints.
- `vendors/cue/pkg/strings/testdata/gen.txtar` asserts one-argument validator success and failure cases.
- `vendors/cue/cue/testdata/eval/issue545.txtar` uses `strings.MinRunes(3)` inside recursive schemas and disjunctions.
- JSON Schema and OpenAPI vendor code emit these validators for string length constraints.

## Scope

Worth building in this batch:

- Represent one-argument `strings.MinRunes(n)` and `strings.MaxRunes(n)` calls as first-class string constraints.
- Allow multiple string constraints to unify with each other and then with concrete strings.
- Return concrete strings when all rune constraints pass; return bottom when a concrete string violates a constraint.
- Treat string constraints as incomplete for concrete JSON/YAML/TOML export, while preserving readable CUE export.
- Add SDK coverage and vendor-borrowed integration tests for both passing and failing validator cases.
- Add compatibility report coverage so validator-style strings cannot regress silently.

Out of scope:

- General partial application for arbitrary builtins.
- `list.MatchN` validator semantics.
- OpenAPI or JSON Schema generation.

## Rust Shape

- Eval: add a small `StringConstraint` value variant alongside numeric and regex constraints.
- Eval builtin dispatch: `strings.MinRunes` and `strings.MaxRunes` accept either one integer argument for a constraint or two arguments for the existing boolean call.
- Eval unification: combine string constraints, and validate concrete strings against all accumulated constraints.
- Encoding: CUE formatting renders constraints as `strings.MinRunes(n)` / `strings.MaxRunes(n)`; concrete exports reject unresolved string constraints as incomplete.

## Verification Plan

Run once after implementation and review:

- `cargo build --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo +nightly fmt -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `cargo audit`
- `cargo deny check`
- `make vendor-corpus compat-report fuzz-smoke`
- `cargo install --path apps/cue --force`
- CLI smoke for one-argument string validators and the previous Phase 9 flows.
