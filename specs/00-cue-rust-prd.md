# cue-rust PRD

Status: Draft for implementation planning
Owner: cue-rust
Last updated: 2026-05-31
Research basis: [CUE architecture study](../docs/research/study-cue-architecture.md), `vendors/cue` at `803c837a690c75343a0f82a1029819b38e52e649`

## Problem

Rust projects need a native CUE implementation that can parse, compile, evaluate, validate, and export CUE without shelling out to the Go CLI or embedding Go through FFI. The implementation must be suitable as both a command-line tool and a library SDK for Rust applications.

CUE is not just a syntax format. Upstream is centered on typed feature structures, graph unification, lexical environments, defaults, disjunctions, closedness, validation profiles, and lazy evaluation. The Rust product succeeds only if it preserves those semantic contracts while using Rust ownership, error handling, diagnostics, and security practices.

## Product Goal

Build a Rust-native CUE parser, compiler, evaluator, SDK, and CLI that is compatible enough with upstream CUE to become a practical substitute for common Rust workflows, then harden it against the upstream conformance corpus until semantic differences are intentional and documented.

## Target Users

- Rust application developers who want CUE validation or configuration loading in-process.
- Platform engineers who use CUE for Kubernetes, policy, service configuration, or schema workflows and want a Rust CLI in constrained environments.
- Tool authors building editors, formatters, package tooling, or CI validators on top of CUE.
- Maintainers of this repository who need a phased implementation plan with clear semantic gates.

## Goals

- Provide a public Rust SDK centered on `Context`, `BuildInstance`, and immutable `Value` handles.
- Provide a CLI with CUE-like commands for `eval`, `export`, `vet`, `fmt`, package loading, and version reporting.
- Preserve the upstream architecture split: source AST, compiled ADT graph, and public immutable value handle.
- Preserve tolerant parsing: syntax errors produce diagnostics and partial syntax trees where possible.
- Preserve CUE semantics for duplicate fields, references, disjunctions, defaults, closedness, bottom values, validation, and export profiles.
- Use Rust 2024, `#![forbid(unsafe_code)]`, rich `thiserror` errors, `miette` diagnostics, and `anyhow` only in binaries.
- Validate and bound all external input at loader, parser, encoding, registry, and CLI boundaries.
- Make conformance measurable through an upstream test corpus runner.

## Non-Goals

- No Go FFI, no subprocess dependency on `cue`, and no line-by-line Go port.
- No full registry, LSP, OpenAPI, JSON Schema, or module publishing support before the core evaluator is stable.
- No parallel evaluator until deterministic single-threaded graph semantics are proven.
- No public API that exposes evaluator mutation or arena internals.
- No unstable Rust, nightly-only runtime code, or `unsafe` hot path.

## Success Metrics

- `cargo build`, `cargo test`, `cargo +nightly fmt`, strict clippy, `cargo audit`, and `cargo deny check` pass for every implementation phase.
- M1 parses and compiles a focused core CUE subset with rich spans and no panics on malformed input.
- M2 validates scalar, struct, list, field reference, duplicate field, basic default, and disjunction cases against golden tests.
- M3 runs `eval`, `export`, and `vet` over local packages and JSON/YAML/TOML data files.
- M4 runs a selected upstream CUE test corpus with pass/fail accounting and categorized gaps.
- Public docs include examples for all stable SDK entry points.

## Product Shape

The SDK should feel like this:

```rust
use cue_rust::{Context, Source};

let ctx = Context::new();
let value = ctx.compile_source(Source::named("config.cue", "x: int & 3"))?;
value.validate(Default::default())?;
let bytes = value.to_json(Default::default())?;
```

The CLI should feel like this:

```text
cue export ./config.cue --out json
cue vet schema.cue data.yaml
cue eval ./... -e services.api.port
```

## Upstream Compatibility Anchors

- `cue.Context` wraps runtime state, and `cue.Value` is an immutable handle over an ADT vertex (`vendors/cue/cue/context.go:33`, `vendors/cue/cue/types.go:584`).
- `parser.ParseFile` returns partial ASTs plus sorted sanitized errors for syntax failures (`vendors/cue/cue/parser/interface.go:166`).
- `cue/load.Config` owns module roots, packages, tags, overlays, file systems, stdin, registries, and data file behavior (`vendors/cue/cue/load/config.go:126`).
- `compile.Instance` lowers AST files into a root `Vertex` with source conjuncts and environment references (`vendors/cue/internal/core/compile/compile.go:65`).
- `Vertex`, `Conjunct`, and `Environment` are load-bearing ADT structures (`vendors/cue/internal/core/adt/composite.go:83`, `vendors/cue/internal/core/adt/composite.go:144`, `vendors/cue/internal/core/adt/composite.go:1471`).
- Evaluation is demand-driven through an operation-local context and node-local scheduler (`vendors/cue/internal/core/eval/eval.go:23`, `vendors/cue/internal/core/adt/sched.go:21`).
- Validation and export are explicit policy/profile phases, not parser features (`vendors/cue/internal/core/adt/validate.go:17`, `vendors/cue/internal/core/export/export.go:34`).

## Release Milestones

- M0: Workspace architecture, source database, diagnostics, scanner, tolerant parser spine.
- M1: AST-to-ADT compiler for core constructs, feature interning, arena graph, basic references.
- M2: Deterministic evaluator for core unification, defaults, disjunctions, validation, and JSON export.
- M3: Loader, package resolution, data file decoding, `eval`, `export`, and `vet`.
- M4: Compatibility hardening against upstream tests, builtins, formatting, and performance gates.
- M5: Registry, modules beyond local packages, schema import/export, LSP-facing syntax services.
