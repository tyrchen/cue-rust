# cue-rust Data Model Design

Status: Draft
Last updated: 2026-05-31
Depends on: [PRD](00-cue-rust-prd.md)

## Design Summary

The data model has three layers:

1. Source layer: files, spans, tokens, comments, and source-preserving AST nodes.
2. Semantic layer: compiled ADT expressions, vertices, arcs, conjuncts, environments, references, and bottom values.
3. Public layer: immutable SDK handles, export trees, diagnostics, and encoding outputs.

This mirrors upstream CUE, where `ast.File`, `internal/core/adt`, and `cue.Value` stay separate. Rust must keep that split. Evaluating directly from parser nodes would duplicate compiler scope logic and make CUE's reference semantics hard to preserve.

## Source Layer

Core types:

```rust
pub struct SourceId(NonZeroU32);
pub struct Span {
    source: SourceId,
    start: ByteOffset,
    end: ByteOffset,
}
pub struct LineIndex { /* private */ }
pub struct SourceFile { /* private */ }
```

Requirements:

- Spans are byte-indexed UTF-8 ranges.
- `SourceFile` validates maximum byte length before parsing.
- `LineIndex` maps byte offsets to line/column for diagnostics without changing semantic spans.
- Comments and whitespace trivia are retained for formatting and syntax export.
- Parser recovery nodes preserve the failing span instead of dropping malformed input.

Rust crates:

- Use `text-size` only if it materially improves span arithmetic; otherwise use local newtypes.
- Use `camino` for filesystem paths at loader boundaries, not inside AST nodes.
- Keep `SourceId` independent from filesystem identity so in-memory and overlay sources behave the same.

## AST Layer

Core node groups:

- `AstFile`: package clause, imports, declarations, comments, language version, unresolved identifiers.
- `Decl`: field, let, alias, import, embedding, comprehension, ellipsis, attributes.
- `Expr`: literal, identifier, selector, index, slice, call, unary, binary, list, struct, interpolation, bottom.
- `Label`: identifier, string label, integer label, pattern label, dynamic label, alias label.
- `BadDecl`, `BadExpr`, `BadLabel`: recovery nodes with diagnostics.

Invariants:

- AST nodes are source-preserving and do not carry evaluated values.
- Duplicate field labels remain duplicate declarations.
- Defaults are syntax on disjunction alternatives and are not normalized during parsing.
- The AST may contain unresolved identifiers. Resolution diagnostics are produced by compile.

## Semantic Layer

Core ids:

```rust
pub struct Feature(u32);
pub struct VertexId(NonZeroU32);
pub struct ExprId(NonZeroU32);
pub struct EnvironmentId(NonZeroU32);
pub struct ConjunctId(NonZeroU32);
```

Core graph values:

- `Runtime`: owns interning, loaded instance cache, builtin packages, and compiled vertices.
- `FeatureInterner`: stable bidirectional mapping from labels to compact features.
- `Vertex`: graph node for a CUE value; owns arcs, conjuncts, base value, status, closedness, and errors.
- `Arc`: feature-to-child relation with required/optional/definition/hidden metadata.
- `Conjunct`: pair of environment and semantic expression, plus close information.
- `Environment`: lexical parent chain plus current vertex and dynamic label data.
- `OpContext`: operation-local evaluator state, caches, current stack, and diagnostics.
- `Bottom`: semantic error value with severity, code, path, and source positions.

Important upstream equivalents:

- `Feature` encodes string/int labels compactly (`vendors/cue/internal/core/adt/feature.go:28`).
- `Environment` links lexical scope to a vertex (`vendors/cue/internal/core/adt/composite.go:83`).
- `Vertex` stores arcs, conjuncts, status, closedness, dynamic/shared state, and errors (`vendors/cue/internal/core/adt/composite.go:144`).
- `Conjunct` pairs environment with expression or node (`vendors/cue/internal/core/adt/composite.go:1471`).

## Public Layer

Public SDK handles:

```rust
pub struct Context { /* Arc<Runtime> */ }
pub struct Value { /* runtime handle + root vertex id + path context */ }
pub struct BuildInstance { /* source/package boundary */ }
pub struct DiagnosticReport { /* stable diagnostics */ }
```

Rules:

- `Value` is immutable from the caller's perspective.
- Methods may evaluate lazily through operation-local state.
- Public handles do not expose arena ids unless a typed handle is explicitly part of the API.
- Public structs with more than five fields use `typed-builder`.
- Library error types use `thiserror`; CLI wrapping uses `anyhow`.

## Error And Diagnostic Model

Internal errors are represented in two forms:

- Recoverable API errors: `Result<T, CueError>`.
- CUE bottom values: semantic invalidity inside the value lattice.

Diagnostics use:

- Stable diagnostic code, such as `cue.syntax.unexpected_token`.
- Primary span and secondary spans.
- Optional CUE path.
- Severity.
- Human message.
- Source-safe rendering through `miette::Diagnostic`.

Upstream CUE's `errors.Error` exposes position, input positions, path, and localized message (`vendors/cue/cue/errors/errors.go:106`). The Rust model keeps the same information while using Rust error enums.

## Serialization Model

Public JSON-facing structs use:

- `#[serde(rename_all = "camelCase")]`
- `#[serde(skip_serializing_if = "Option::is_none")]`
- Strongly typed fields instead of `serde_json::Value` except for truly dynamic exported data.

Internal ADT structures are not serde-stable. Persistence and cache formats require separate versioned snapshot types.

## AGENTS Binding

- Error handling: `thiserror` for library errors, `anyhow` for binaries, no `unwrap` or `expect` in production code.
- Safety: crate roots use `#![forbid(unsafe_code)]`; parser and evaluator reject hostile inputs instead of panicking.
- Type design: newtype ids make illegal states unrepresentable; zero-invalid ids use `NonZeroU32`.
- Performance: use interning and arena ids for hot graph paths; avoid repeated `String` keys.
- Testing: every public invariant gets focused unit tests plus property tests for parser spans and feature interning.
