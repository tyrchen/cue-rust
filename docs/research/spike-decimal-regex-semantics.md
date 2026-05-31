# Spike: Decimal And Regex Semantics

Status: Done
Date: 2026-05-31
Depends on: `specs/13-cue-rust-adt-runtime-design.md`, `specs/21-cue-rust-encoding-design.md`

## Question

Which crates should cue-rust use for CUE number and regex compatibility?

## Decision

Use a local `Number` wrapper that initially preserves exact numeric source text and supports the core integer/decimal comparisons needed by early phases. Keep `dashu` as the preferred arbitrary-precision implementation candidate for expanded semantics. Do not use `rust_decimal` for semantic CUE numbers because it is fixed precision.

Use `regex` for regular expression behavior where Rust's linear-time engine is compatible with CUE's supported surface. Enforce pattern byte limits before compilation and compile with explicit size limits for hostile input.

## Rationale

CUE numbers are arbitrary precision and must not be routed through `f64`. Early phases do not need the full numeric lattice, but they must avoid irreversible precision loss. Preserving numeric text lets JSON/TOML decoding and AST literals remain exact until the evaluator needs richer operations.

`dashu` provides big-number building blocks with permissive licensing and no FFI. `rust_decimal` is maintained and useful for fixed-precision domains, but CUE needs arbitrary precision. `bigdecimal` remains an alternative if decimal ergonomics outweigh `dashu` integration.

Rust's `regex` crate guarantees linear-time matching and is the right default for hostile input. More expressive engines are rejected for untrusted patterns because backtracking behavior would violate the security plan.

## Consequences

- Phase 4 introduces a semantic `Number` newtype rather than leaking external numeric crates through public APIs.
- Phase 7 JSON/YAML/TOML decoders preserve numeric text.
- Regex support is bounded and can report compatibility gaps for unsupported upstream constructs.
- Expanded arithmetic and numeric normalization are Phase 9 compatibility items.

