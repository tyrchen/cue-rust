# Spike: Rust Arena And Vertex Representation

Status: Done
Date: 2026-05-31
Depends on: `specs/10-cue-rust-data-model-design.md`, `specs/13-cue-rust-adt-runtime-design.md`

## Question

How should cue-rust represent semantic vertices, expressions, environments, and conjuncts while preserving graph sharing, duplicate fields, and public `Value` immutability?

## Decision

Use context-local arenas backed by `Vec` plus non-zero typed ids for the initial implementation. Use `IndexMap` for deterministic field arcs. Keep public `Value` handles immutable and cloneable by storing an `Arc<Runtime>` plus root `VertexId`; evaluation mutation stays inside operation-local contexts and newly built result graphs.

## Rationale

The core graph is append-heavy in the early milestones. Deletion is not required for compiled instances, and generational deletion safety would add overhead without solving a current invariant. A small local arena gives:

- compact ids (`NonZeroU32`) that match the spec
- stable references across compile and evaluation
- deterministic storage and export order
- no `unsafe`
- simple serialization/debug export for tests

`slotmap` remains a good fit if later phases need deletion or long-lived invalidation detection. `Arc` node graphs were rejected for the initial runtime because lazy evaluation state and duplicate conjunct tracking are easier to reason about through ids and operation contexts.

## Public Concurrency Contract

`Value` is immutable from the SDK caller's perspective. Cloning a `Value` is cheap and does not expose evaluator mutation. The initial runtime uses a single-threaded construction path; public APIs do not promise `Send + Sync` until tests and type structure prove it.

Evaluation operations create an `OpContext` that is deliberately not shared across threads. Any future concurrent evaluator must preserve the same public handle contract.

## Consequences

- Phase 4 implements id newtypes and append-only arenas.
- Duplicate fields append conjuncts; they are never map overwrites.
- Export and diagnostics use interner lookups rather than storing repeated label strings in hot arcs.
- The code should avoid `Mutex<HashMap>` around graph state. Mutation belongs either in construction or in operation-local contexts.

