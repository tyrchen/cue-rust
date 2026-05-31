# Spike: Dependency Audit

Status: Done
Date: 2026-05-31
Depends on: `specs/61-cue-rust-crates-features-design.md`

## Summary

Dependency selection follows the spec rule: pure Rust where possible, current stable releases, explicit features, workspace-level versions, and no dependency unless it removes real implementation risk.

Rust stable was verified against Rust Forge and the Rust Blog for the 2026-05-28 stable release, Rust 1.96.0. Crate metadata was checked with `cargo search` / `cargo info` against crates.io on 2026-05-31.

## Audited Candidates

| Area | Candidate | Observed version | Decision |
| --- | --- | ---: | --- |
| parser combinators | `winnow` | 1.0.3 | Use selectively for grammar fragments. |
| diagnostics | `miette` | 7.6.0 | Use for diagnostic protocol/rendering. |
| errors | `thiserror` | 2.0.18 | Use for library errors. |
| app errors | `anyhow` | 1.0.102 | CLI only. |
| YAML | `noyalib` | 0.0.6 | Preferred over deprecated `serde_yml`; keep use narrow. |
| arena | local `Vec` arena | n/a | Preferred initially; `slotmap` 1.1.1 remains fallback. |
| ordered maps | `indexmap` | spec-cited 2.14.0 | Use for deterministic arcs/exports. |
| decimals | `dashu` | 0.4.2 | Preferred candidate for expanded arbitrary precision. |
| fixed decimal | `rust_decimal` | 1.42.0 | Rejected for semantic CUE numbers. |
| regex | `regex` | 1.12.3 | Preferred linear-time regex engine. |
| CLI | `clap` | spec-cited 4.6.1 | Use derive for CLI. |
| snapshots | `insta` | spec-cited 1.47.2 | Use for golden tests. |
| property tests | `proptest` | spec-cited 1.11.0 | Use for non-panic/span invariants. |
| fuzz | `libfuzzer-sys` | spec-cited 0.4.12 | Use through cargo-fuzz smoke target. |
| benchmarks | `criterion` | spec-cited 0.8.2 | Add when Phase 9 benchmarks land. |

## Risk Notes

- `noyalib` is young. Keep YAML integration behind a narrow adapter and maintain JSON/TOML paths independently.
- The local arena avoids adding lifetime/deletion complexity before the semantic graph is stable.
- Numeric behavior must remain encapsulated behind cue-rust types so the implementation can switch from preserved text to `dashu` without public API churn.
- `regex` compatibility is not identical to upstream in every detail; unsupported constructs become compatibility gaps, not silent behavior changes.

