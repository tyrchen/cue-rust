# cue-rust ADT Runtime Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Data model](10-cue-rust-data-model-design.md)

## Design Summary

The ADT runtime is the semantic center of cue-rust. It owns interned labels, compiled expressions, vertices, environments, conjuncts, builtin package registrations, loaded instance caches, and operation-local evaluator state.

The Rust implementation should model the upstream ADT directly enough that semantic tests map cleanly to upstream concepts, while using Rust ids and ownership instead of pointer graphs exposed through public APIs.

## Runtime Ownership

```rust
pub struct Runtime {
    features: FeatureInterner,
    strings: StringInterner,
    vertices: VertexArena,
    expressions: ExprArena,
    environments: EnvironmentArena,
    conjuncts: ConjunctArena,
    instances: InstanceCache,
    builtins: BuiltinRegistry,
}
```

Public `Context` owns an `Arc<RuntimeFacade>` or similar handle. The internal runtime can use single-threaded arenas during M1 if public values do not claim cross-thread mutation. Any `Send` or `Sync` public promises must be proven by type structure and tests.

## Feature Interner

`Feature` is a compact copyable label:

```rust
pub enum FeatureKind {
    String,
    Int,
    Definition,
    Hidden,
    Let,
}
```

Requirements:

- Stable ids within a runtime.
- Bidirectional lookup for diagnostics and export.
- Reserved feature for `_`.
- Total ordering matching CUE field ordering where required by export.
- No repeated `String` map keys in hot vertex arcs.

Use `lasso` or a small custom interner after the Phase 0 interner spike. The default should be context-local, not global, unless a compatibility or memory study proves a global interner is beneficial.

## Vertex Arena

`Vertex` stores:

- parent vertex id
- feature label
- arcs by feature
- source conjunct ids
- base value
- evaluator status
- closedness flags
- optionality and definition metadata
- pattern constraints
- dynamic field state
- bottom/error state
- source provenance

The arena representation is selected by `spike-rust-arena-vertex.md`. Candidate implementations:

- `slotmap` for generational ids and deletion safety.
- custom `Vec` arena with `NonZeroU32` ids for maximum performance.
- `Arc` graph only if immutable sharing proves simpler without losing lazy evaluation state.

The implementation phase must choose one and document why.

## Environments

`Environment` stores:

- parent environment id
- current vertex id
- dynamic label
- comprehension id
- per-environment cache for let references and dynamic evaluation

This mirrors upstream, where environments preserve lexical reference semantics without copying referenced values (`vendors/cue/internal/core/adt/doc.go:41`).

## Conjuncts

`Conjunct` stores:

- environment id
- expression id or vertex id
- close information
- source span

Duplicate fields append conjuncts. They are not map overwrites. This is required because duplicate labels denote unification.

## Base Values

Base value variants:

- top
- bottom
- null
- bool
- number
- string
- bytes
- list marker
- struct marker
- builtin

Numbers need arbitrary precision decimal semantics compatible with upstream's `apd` usage (`vendors/cue/internal/core/adt/decimal.go:17`). The Phase 0 numeric spike compares `bigdecimal`, `dashu`, and a local decimal wrapper before committing. `rust_decimal` is not sufficient for unbounded CUE numbers.

## Bottom Values

`Bottom` is a value-lattice member, not just an API error. It stores:

- diagnostic code
- message
- source spans
- path
- child errors
- incomplete vs concrete failure category

Validation and export decide whether bottom is reportable for a specific operation.

## Builtin Registry

Builtins are runtime packages, not parser special cases. The registry maps import paths and builtin names to typed functions over ADT values. Initial builtins are minimal and driven by conformance tests.

## AGENTS Binding

- No `unsafe`.
- Avoid `Mutex<HashMap>` for hot graph state. Use operation-local mutation or purpose-built arenas.
- Use newtypes for ids, features, package names, import paths, and language versions.
- Keep public API immutable; internal evaluation mutation remains scoped to `OpContext`.
- Add focused tests for feature interning, duplicate fields, environment lookup, and bottom propagation.
