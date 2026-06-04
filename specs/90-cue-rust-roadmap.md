# cue-rust Roadmap

Status: Draft
Last updated: 2026-05-31
Depends on: [PRD](00-cue-rust-prd.md)

## Milestone Principles

- Each milestone lands a coherent user-visible or maintainer-visible capability.
- Risky semantic decisions are retired before broad implementation.
- Compatibility is measured by tests, not by claims.
- CLI behavior follows SDK behavior; the CLI is not a separate implementation.

## M0: Syntax And Workspace Spine

Outcome:

- Workspace split into source, syntax, diagnostics, SDK facade, and CLI skeleton crates.
- Rust toolchain, lint, audit, deny, and Makefile automation are complete.
- Scanner and tolerant parser handle core CUE syntax with diagnostics and partial ASTs.
- `cue parse` and `cue fmt --check` can run on local files.

Exit criteria:

- malformed input never panics in parser tests and fuzz smoke tests
- parser spans are stable and line/column rendering works
- public parser APIs have docs and examples

## M1: Core ADT And Compiler

Outcome:

- Feature interner, arenas, vertices, conjuncts, environments, and semantic expression ids exist.
- Compiler lowers core AST to ADT.
- Lexical references are explicit ADT nodes with `up_count`.
- Duplicate fields compile as conjuncts.

Exit criteria:

- AST-to-ADT golden tests cover scalar, struct, list, duplicate field, let, alias, selector, and reference cases
- unresolved identifiers become diagnostics
- no evaluator logic is hidden in parser or loader

## M2: Core Evaluation And Export

Outcome:

- Deterministic evaluator handles scalar and struct unification, references, simple disjunctions, defaults, bottom, validation, and JSON export.
- SDK exposes `compile_source`, `validate`, `unify`, `lookup_path`, `kind`, and `to_json`.
- `cue eval`, `cue export`, and `cue vet` work for local source files.

Exit criteria:

- compatibility tests pass for selected core CUE examples
- validation options are covered by tests
- JSON export rejects incomplete values with diagnostics

## M3: Loader And Data Files

Outcome:

- Loader supports local packages, directories, overlays, stdin, tags, and module root discovery.
- JSON/YAML/TOML decoding integrates with CUE validation.
- CLI supports common package and data-file workflows.

Exit criteria:

- `cue vet schema.cue data.yaml` works for streams and reports per-document diagnostics
- path traversal and oversized source tests pass
- loader behavior has golden tests against selected upstream cases

## M4: Compatibility Hardening

Outcome:

- Upstream CUE test corpus harness exists with categorized results.
- Scheduler, cycles, closedness, comprehensions, interpolation, numeric semantics, regex semantics, and export profiles are hardened.
- Performance profiling starts.

Exit criteria:

- conformance report is generated in CI
- known semantic gaps are tracked in specs or issue docs
- parser/evaluator fuzz targets run in CI smoke mode

## M5: Ecosystem Features

Outcome:

- Builtin packages expand.
- JSON Schema and OpenAPI import/export land behind features.
- Registry and module operations land behind `registry`.
- LSP-facing syntax services become possible.

Exit criteria:

- network registry code has explicit security review
- schema features have differential tests
- LSP APIs do not destabilize core SDK
