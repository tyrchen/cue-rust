# Spike: CUE Test Corpus Subset

Status: Done
Date: 2026-05-31
Depends on: `vendors/cue`, `specs/70-cue-rust-security-performance-testing-plan.md`

## Question

Which upstream CUE tests should cue-rust mirror first, and how should compatibility gaps be reported?

## Decision

Use a staged corpus:

1. scanner/parser syntax cases from `vendors/cue/cue/parser`, `vendors/cue/cue/scanner`, and selected `cmd/cue/cmd/testdata/script/fmt*.txtar`
2. core semantic cases from `vendors/cue/cue/testdata` and focused script cases for `eval`, `export`, and validation
3. loader cases from `vendors/cue/cue/load`, `vendors/cue/internal/mod/modpkgload`, and CLI package scripts
4. encoding cases from `vendors/cue/cmd/cue/cmd/testdata/script/export*.txtar`, `encoding_*.txtar`, and data-file validation scripts

Phase 9 adds a machine-readable compatibility summary with categories:

- `pass`
- `expected-fail`
- `unsupported-syntax`
- `unsupported-semantic`
- `loader-gap`
- `encoding-gap`

## Initial Hand-Written Smoke Set

Before the txtar harness is complete, each phase uses a small Rust-native fixture set:

- valid package clauses and fields
- malformed syntax with stable spans
- duplicate fields
- references to top-level fields
- scalar/struct unification
- JSON export of concrete data
- validation failure for conflicting data
- loader traversal, traversal rejection, symlink rejection, and oversized files

## Consequences

- Compatibility is measured by generated reports, not prose claims.
- Unsupported upstream cases are visible and categorized.
- Broad ignores are forbidden; every expected failure needs a reason.
- The compatibility Makefile target lands in Phase 9 after the CLI and loader can exercise end-to-end flows.

