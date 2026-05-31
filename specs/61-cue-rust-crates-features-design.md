# cue-rust Crates And Dependency Design

Status: Draft
Last updated: 2026-05-31
Depends on: [SDK design](20-cue-rust-sdk-design.md)

## Design Summary

Use a Rust workspace with narrow internal crates and one ergonomic public facade. Dependencies are selected for safety, maintenance, semantic fit, and compile-time cost. Exact versions are reviewed during each implementation phase with `cargo info`, `cargo audit`, and `cargo deny`.

## Workspace Crates

```text
crates/
  source/       source files, spans, line indexes, diagnostics glue
  syntax/       scanner, parser, source AST, formatter syntax services
  loader/       package/module/data file loading
  adt/          semantic graph, runtime, features, vertices, environments
  compiler/     AST to ADT lowering and reference resolution
  eval/         evaluator, validation, export profiles
  encoding/     JSON/YAML/TOML/CUE/text/binary encode/decode
  sdk/          public facade re-exported as cue_rust
apps/
  cue-rs/       CLI binary
```

The initial repository currently has `crates/core`; implementation should either replace it with the crate layout above or keep it as a temporary facade only during the first workspace restructuring phase.

## Crate Boundaries

- `source` has no dependency on parser or evaluator.
- `syntax` depends on `source` and diagnostics only.
- `loader` depends on `source`, `syntax`, and `encoding`.
- `adt` depends on `source` for spans and diagnostics, but not on parser AST.
- `compiler` depends on `syntax`, `loader`, and `adt`.
- `eval` depends on `adt`.
- `encoding` depends on `eval` public export trees, not evaluator internals.
- `sdk` depends on all stable internal crates.
- `apps/cue-rs` depends on `sdk` and application-only crates.

## Feature Flags

Default features:

- `std`
- `json`
- `yaml`
- `toml`

Optional features:

- `cli`
- `registry`
- `lsp`
- `jsonschema`
- `openapi`
- `protobuf`
- `bench`
- `experimental-salsa`

Feature rules:

- Internal semantic correctness must not depend on optional CLI or LSP features.
- Network registry code is behind `registry`.
- `salsa` is experimental until incremental behavior is proven useful and stable enough for this project.

## Dependency Candidates

Verified with current registry metadata on 2026-05-31:

| Area | Candidate | Observed version | Decision |
| --- | --- | ---: | --- |
| parser combinators | `winnow` | 1.0.3 | Preferred for grammar productions |
| CLI | `clap` | 4.6.1 | Preferred with derive |
| diagnostics | `miette` | 7.6.0 | Preferred for rendering/protocol |
| library errors | `thiserror` | 2.0.18 | Required |
| app errors | `anyhow` | 1.0.102 in workspace | CLI only |
| serialization | `serde` | 1.0.228 | Required |
| JSON | `serde_json` | 1.0.150 latest reported | Preferred with numeric care |
| YAML | `noyalib` | 0.0.6 | Preferred over deprecated `serde_yml` |
| TOML | `toml` | 1.1.2+spec-1.1.0 | Preferred |
| UTF-8 paths | `camino` | 1.2.2 | Preferred for loader APIs |
| ordered maps | `indexmap` | 2.14.0 | Preferred where deterministic order matters |
| string interning | `lasso` | 0.7.3 | Candidate |
| compact strings | `compact_str` | 0.9.1 | Candidate |
| arenas | `slotmap` | 1.1.1 | Candidate |
| arbitrary decimal | `bigdecimal` | 0.4.10 | Candidate |
| big numbers | `dashu` | 0.4.2 | Candidate |
| regex | `regex` | 1.12.3 | Preferred for linear-time matching |
| test tables | `rstest` | 0.26.1 | Preferred |
| property tests | `proptest` | 1.11.0 | Preferred |
| snapshots | `insta` | 1.47.2 | Preferred |
| fuzz | `libfuzzer-sys` | 0.4.12 | Preferred via cargo-fuzz |
| tracing | `tracing` | 0.1.44 | Required |
| tracing output | `tracing-subscriber` | 0.3.23 | CLI/application |
| benchmarks | `criterion` | 0.8.2 | M4+ only |
| directory walk | `ignore` | 0.4.25 | Loader recursive walk |

Notes:

- `serde_yml` reported itself deprecated and should not be selected directly.
- `smallvec` latest is `2.0.0-alpha.12`; prefer stable `1.15.1` if small vectors are needed before 2.0 stabilizes.
- `rust_decimal` is fixed precision and is not sufficient for CUE's arbitrary precision numeric model.
- `rowan` is useful for future lossless syntax tooling but is not required for the M0 parser unless the parser spike proves it reduces complexity.

## Version Policy

- Workspace dependencies live in `[workspace.dependencies]`.
- Use current stable versions, not alpha releases, unless a phase explicitly accepts prerelease risk.
- Use explicit Tokio features when async registry or LSP code lands.
- Prefer pure Rust crates over FFI.
- Add a dependency only when it replaces real complexity or provides a hardened implementation.

## Tooling

Required root files:

- `rust-toolchain.toml` pinned to latest stable Rust 2024-compatible toolchain.
- `deny.toml` with license and ban policy.
- Makefile targets for build, test, fmt, clippy, audit, deny, fuzz, and conformance as they land.

## AGENTS Binding

- Audit new dependencies for maintenance, license, unsafe usage, and transitive footprint.
- Keep crate public APIs documented and `Debug`.
- Enable strict linting at crate roots.
- Do not add scripts for automation when Makefile targets are the discoverable project interface.
