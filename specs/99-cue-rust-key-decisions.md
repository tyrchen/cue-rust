# cue-rust Key Decisions

Status: Draft
Last updated: 2026-05-31

## Decisions

### D1: Build Native Rust, Not Go FFI

Decision: cue-rust will implement parser, compiler, evaluator, SDK, and CLI in Rust.

Reason: FFI would inherit Go runtime complexity, complicate distribution, and fail the goal of an embeddable Rust SDK.

### D2: Preserve Three-Layer Architecture

Decision: keep source AST, compiled ADT graph, and public immutable value handles separate.

Reason: upstream relies on this split, and it prevents parser, compiler, and evaluator responsibilities from blending.

### D3: Use A Custom Scanner

Decision: implement a custom scanner and use `winnow` selectively for grammar productions.

Reason: CUE comma insertion, interpolation, comments, recovery, and source positions need stateful scanner control.

### D4: Compile References Explicitly

Decision: compiler emits explicit reference nodes with lexical `up_count`.

Reason: this preserves CUE reference semantics without runtime AST scope walks or value copying.

### D5: Represent Values As A Graph

Decision: semantic values are vertices and arcs in an arena or graph, not owned trees.

Reason: CUE needs sharing, cycles, duplicate conjuncts, lazy finalization, and parent/path context.

### D6: Keep Evaluation Operation-Local

Decision: public values are immutable; evaluator mutation lives in `OpContext`.

Reason: upstream `Value` is immutable and operation contexts are intentionally not concurrent.

### D7: Choose Deterministic Single-Threaded Evaluation First

Decision: no async or parallel evaluator in early milestones.

Reason: the hard problem is graph semantics, dependencies, and cycles. Parallelism adds risk before correctness is proven.

### D8: Treat Validation And Export As Profiles

Decision: validation and export are explicit operations over evaluated values.

Reason: upstream uses validation configs and export profiles; parser and evaluator cannot bake in one policy.

### D9: Use `miette`, `thiserror`, And `anyhow` By Boundary

Decision: diagnostics render through `miette`, library errors use `thiserror`, and binaries use `anyhow`.

Reason: this gives structured SDK errors and ergonomic CLI context without leaking app error types into libraries.

### D10: Use `noyalib` For YAML

Decision: prefer `noyalib` over `serde_yml`.

Reason: current `serde_yml` metadata marks it deprecated and a shim over `noyalib`.

### D11: Do Not Commit To Decimal Crate Before Spike

Decision: run a numeric semantics spike before choosing `bigdecimal`, `dashu`, or a local wrapper.

Reason: CUE's arbitrary precision decimal behavior is central to compatibility, and fixed precision decimals are insufficient.

### D12: Gate Compatibility With Upstream Tests

Decision: compatibility claims require a corpus harness and visible pass/fail categories.

Reason: CUE has subtle semantics around defaults, disjunctions, closedness, cycles, and export that cannot be proven by a few hand-written examples.
