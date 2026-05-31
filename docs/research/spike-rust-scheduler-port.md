# Spike: Rust Evaluator Scheduler Port

Status: Done
Date: 2026-05-31
Depends on: `specs/15-cue-rust-evaluator-design.md`, `docs/research/study-cue-architecture.md`

## Question

Should the first Rust evaluator port upstream's node-local scheduler directly, or start with a simpler evaluator for the core subset?

## Decision

Start with a deterministic single-threaded evaluator that finalizes bounded core values recursively, with explicit evaluation limits and cycle tracking. Preserve the ADT shapes and operation-context boundaries needed to replace the core with an upstream-style node scheduler before M4 compatibility hardening.

## Rationale

The upstream scheduler is load-bearing for complete CUE semantics, but implementing it before parser, compiler, and basic ADT tests exist would increase risk. The first evaluator must prove:

- scalar and struct unification
- duplicate field conjunct handling
- explicit reference resolution through compiled nodes
- defaults and simple disjunctions
- bottom propagation
- validation/export separation

Those behaviors can be implemented with a simpler finalizer while keeping scheduler-sensitive concepts visible in the data model: vertex status, operation stack, limits, dependency points, and bottom values.

## Replacement Boundary

The recursive evaluator is acceptable only through the core subset. Phase 9 compatibility work must treat the node-local scheduler as a hardening item, not as an optional optimization.

## Consequences

- No Tokio or async runtime in evaluator code.
- The evaluator has deterministic traversal order and explicit recursion limits.
- Cycle detection is implemented through `OpContext` stack state in the first pass.
- Scheduler gaps are tracked in the compatibility report rather than hidden behind broad ignores.

