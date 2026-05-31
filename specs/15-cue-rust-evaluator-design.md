# cue-rust Evaluator Design

Status: Draft
Last updated: 2026-05-31
Depends on: [ADT runtime](13-cue-rust-adt-runtime-design.md), [Compiler design](14-cue-rust-compiler-design.md)

## Design Summary

The evaluator finalizes ADT vertices on demand. It implements CUE unification as greatest lower bound, resolves references through environments, handles defaults and disjunctions, detects cycles, and produces bottom values for semantic failures. Validation and export are separate operations over evaluated vertices.

Upstream evaluation creates an operation context and finalizes the root vertex (`vendors/cue/internal/core/eval/eval.go:23`). The node scheduler is a deterministic local dependency engine, not an async runtime (`vendors/cue/internal/core/adt/sched.go:21`).

## Operation Context

```rust
pub struct OpContext<'rt> {
    runtime: &'rt Runtime,
    stack: EvaluationStack,
    diagnostics: DiagnosticSink,
    caches: OperationCaches,
    limits: EvaluationLimits,
}
```

`OpContext` is non-`Sync` by construction. Public `Value` handles can be cloned, but each operation gets its own context.

## Evaluation Limits

External input can trigger expensive evaluation. The evaluator enforces:

- maximum recursion depth
- maximum disjunction alternatives per normalization point
- maximum generated dynamic fields
- maximum list length from comprehensions
- maximum diagnostics
- optional wall-clock timeout for CLI operations

Limit failures become diagnostics and bottom values where they affect CUE data; infrastructure timeout can also return an API error.

## Scheduler

Phase 0 decides the precise Rust scheduler shape. The intended design is a direct Rust adaptation of upstream's node-local task scheduler:

- each vertex has readiness flags for scalar value, arcs, conjuncts, subfields, and recursive state
- tasks declare dependencies
- blocked tasks are retained in deterministic queues
- cycle placeholders prevent unbounded reentrancy
- finalization records enough state for later validation and export

If a simpler recursive evaluator is used during M1, it must be explicitly fenced to the core subset and replaced before M4 compatibility gating.

## Unification

Unification rules:

- duplicate fields add conjuncts
- scalar values intersect according to CUE kind and constraint rules
- structs unify by field feature and closedness rules
- lists unify by element position and list openness
- bottom propagates with source and path context
- defaults do not erase non-default alternatives until defaulting is requested

The public `Value::unify` creates a new root vertex with conjuncts from both operands, matching upstream `adt.Unify` (`vendors/cue/internal/core/adt/composite.go:929`).

## References

The evaluator resolves:

- field references through environment `up_count`
- value references to current or ancestor vertices
- label references from dynamic label context
- dynamic references by evaluating label expressions first
- import references through runtime instance or builtin registries
- let references through per-environment caches

The compiler must provide explicit reference nodes. The evaluator must not parse identifier names or scan AST scopes.

## Defaults And Disjunctions

Defaults are semantic and evaluation-time:

- defaults narrow disjunction choices only when a defaulted operation asks for them
- validation can choose concrete requirements separately from defaulting
- export profiles decide whether defaults are taken
- ambiguous or conflicting defaults produce bottom with alternative spans

This follows upstream default handling in `internal/core/adt/default.go`.

## Validation

Validation is a separate visitor over evaluated vertices:

```rust
pub struct ValidateOptions {
    pub concrete: bool,
    pub final_required: bool,
    pub allow_cycles: bool,
    pub all_errors: bool,
}
```

The validator distinguishes:

- incomplete values
- required fields
- bottom values
- structural cycles
- hidden and definition fields
- data mode vs schema mode

Upstream exposes these as validation config flags (`vendors/cue/internal/core/adt/validate.go:17`).

## Export

Export uses profiles:

- value export for concrete data
- definition export for schema
- simplified syntax export
- JSON export
- YAML/TOML export through concrete export tree

Export must force enough evaluation to honor the selected profile and must report incomplete values instead of silently emitting invalid data.

## Tests

- Unit tests for scalar unification and kind lattice.
- Golden tests for duplicate fields, defaults, disjunctions, references, closedness, bottom propagation, and validation.
- Cycle tests derived from upstream ADT scheduler cases.
- Fuzz target for compiled small ASTs into evaluation with strict limits.
- Differential harness comparing selected cases against vendored upstream CUE CLI or Go tests when available locally.

## AGENTS Binding

- Evaluator is deterministic and single-threaded first.
- Do not use Tokio inside the evaluator.
- Do not wrap non-Send arena state in `Mutex` to make the type system quiet.
- Use checked arithmetic for externally influenced sizes and indices.
- Avoid cloning large values; use ids, arenas, and operation-local caches.
