# cue-rust Compiler Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Parser design](11-cue-rust-parser-design.md), [ADT runtime](13-cue-rust-adt-runtime-design.md), [Loader design](12-cue-rust-loader-design.md)

## Design Summary

The compiler lowers parsed AST files into ADT expressions and a root vertex. It resolves lexical references, imports, aliases, labels, and lets into explicit ADT reference nodes. Evaluation should not rediscover lexical scope from AST nodes.

Upstream `compile.Instance` creates a compiler, precomputes package-level scope, converts files into struct literals, builds an environment chain, and inserts one root conjunct per file (`vendors/cue/internal/core/compile/compile.go:65`, `vendors/cue/internal/core/compile/compile.go:285`).

## Inputs And Outputs

Input:

- `BuildInstance`
- `Runtime`
- optional external scope
- compile options

Output:

- root `VertexId`
- compile diagnostics
- import dependencies
- source provenance table

```rust
pub struct CompileOptions {
    pub allow_experimental: bool,
    pub language_version: LanguageVersion,
}
```

## Compiler Passes

### Pass 1: Package Scope

Build package-level bindings:

- top-level fields
- definitions
- hidden fields
- imports
- aliases
- let declarations

Duplicate field names stay as multiple declarations. Duplicate non-field bindings produce diagnostics when CUE rules require uniqueness.

### Pass 2: AST Lowering

Lower AST expressions to semantic expressions:

- literals to base values
- structs to struct literals with declarations
- lists to list expressions
- fields to field conjuncts
- ellipses to openness constraints
- binary operators to ADT binary expressions
- disjunctions and defaults to disjunction expressions
- comprehensions to comprehension expressions
- attributes to metadata expressions

Parser recovery nodes become compile diagnostics and bottom expressions that keep paths and spans usable.

### Pass 3: Reference Resolution

Resolve identifiers into explicit reference variants:

- `FieldReference`: field in a relative environment with `up_count`.
- `ValueReference`: current or ancestor value.
- `LabelReference`: current label value.
- `DynamicReference`: computed label lookup.
- `ImportReference`: imported package or builtin package.
- `LetReference`: lexical let binding with cache identity.

Upstream lowers these reference variants in the compiler (`vendors/cue/internal/core/compile/compile.go:423`, `vendors/cue/internal/core/compile/compile.go:521`, `vendors/cue/internal/core/compile/compile.go:604`). Rust must preserve the `up_count` concept.

### Pass 4: Root Vertex Assembly

Create a root vertex and add one conjunct per source file. This preserves file provenance and supports CUE's package-level unification model.

## Import Handling

Imports compile to references against runtime-loaded instances or builtin packages. Missing imports produce compile diagnostics and bottom references, not parser failures.

Compilation recursively requests imported `BuildInstance`s through the runtime instance cache. Cycles are reported as import cycle diagnostics before evaluation.

## Diagnostics

Compiler diagnostics include:

- unresolved identifier
- invalid duplicate binding
- invalid package clause
- invalid import alias
- invalid attribute placement
- unsupported language version feature
- syntax recovery propagated into semantic bottom

All diagnostics carry AST spans and, where known, CUE paths.

## Rust API

```rust
pub struct Compiler<'rt> {
    runtime: &'rt Runtime,
}

impl Compiler<'_> {
    pub fn compile_instance(
        &mut self,
        instance: &BuildInstance,
        options: CompileOptions,
    ) -> Result<CompiledInstance, CompileError>;
}
```

`CompileError` represents infrastructure failure. Normal CUE errors are diagnostics inside `CompiledInstance`.

## Tests

- Golden tests for AST-to-ADT lowering.
- Scope tests for nested structs, aliases, lets, dynamic labels, and imports.
- Regression tests against upstream reference examples.
- Property tests that every emitted reference target has either a valid binding or a diagnostic.

## AGENTS Binding

- Implement conversions with `From`, `TryFrom`, and `FromStr` where appropriate.
- Keep compiler functions small; split passes by semantic responsibility.
- No eager evaluation in compile.
- Use `thiserror` for compile infrastructure errors.
- Public compile APIs document `# Errors`.
