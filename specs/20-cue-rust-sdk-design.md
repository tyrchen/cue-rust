# cue-rust SDK Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Compiler design](14-cue-rust-compiler-design.md), [Evaluator design](15-cue-rust-evaluator-design.md), [Encoding design](21-cue-rust-encoding-design.md)

## Design Summary

The SDK exposes a small stable facade over the parser, loader, compiler, evaluator, validator, and encoders. It should feel idiomatic to Rust users while mapping cleanly to upstream CUE's `cue.Context`, `cue.Value`, `cue/load`, and encoding packages.

## Crate Facade

The top-level crate is `cue-rust` in Cargo and `cue_rust` in code. It re-exports stable API types from internal workspace crates.

Primary public modules:

- `context`
- `source`
- `load`
- `value`
- `diagnostic`
- `encoding`
- `syntax`

Internal implementation crates may have more granular names, but users should not need to depend on them for common workflows.

## Context API

```rust
pub struct Context { /* private */ }

impl Context {
    pub fn new() -> Self;
    pub fn with_config(config: ContextConfig) -> Result<Self, ContextError>;
    pub fn parse_source(&self, source: Source) -> ParseResult;
    pub fn compile_source(&self, source: Source) -> Result<Value, CueError>;
    pub fn load(&self, config: LoadConfig) -> Result<Vec<BuildInstance>, CueError>;
    pub fn build_instance(&self, instance: &BuildInstance) -> Result<Value, CueError>;
}
```

`ContextConfig` uses `typed-builder` because it will exceed five fields as limits, features, registries, builtins, and tracing options are added.

## Source API

```rust
pub struct Source { /* private */ }

impl Source {
    pub fn named(name: impl Into<SourceName>, content: impl Into<String>) -> Result<Self, SourceError>;
    pub fn from_path(path: impl AsRef<Utf8Path>) -> Result<Self, SourceError>;
}
```

`SourceName` rejects empty names, NUL bytes, path traversal where path semantics are requested, and names over the configured byte limit.

## Value API

```rust
pub struct Value { /* private */ }

impl Value {
    pub fn kind(&self) -> Result<ValueKind, CueError>;
    pub fn defaulted(&self) -> Result<DefaultResult, CueError>;
    pub fn validate(&self, options: ValidateOptions) -> Result<(), CueError>;
    pub fn unify(&self, other: &Value) -> Result<Value, CueError>;
    pub fn lookup_path(&self, path: &CuePath) -> Result<Value, CueError>;
    pub fn syntax(&self, options: SyntaxOptions) -> Result<SyntaxNode, CueError>;
    pub fn to_json(&self, options: JsonOptions) -> Result<Vec<u8>, CueError>;
}
```

The handle is immutable. Methods can force evaluation internally but do not mutate public state in a way callers can observe except through cached performance.

## Diagnostics API

```rust
pub struct DiagnosticReport { /* private */ }
pub struct CueError { /* thiserror enum */ }
```

`CueError` can contain one diagnostic or a report. CLI rendering uses `miette` fancy output. SDK users can inspect structured diagnostics without parsing strings.

## Options Types

Options use explicit structs:

- `ParseOptions`
- `LoadConfig`
- `CompileOptions`
- `EvalOptions`
- `ValidateOptions`
- `SyntaxOptions`
- `JsonOptions`
- `YamlOptions`

Every public options type is `Debug`, `Clone` when cheap enough, and `#[non_exhaustive]` unless builder-only construction is required.

## Stability Policy

- Top-level SDK facade evolves conservatively.
- Internal crates use `pub(crate)` aggressively.
- Experimental APIs are behind crate features with `experimental_` prefix.
- No public type exposes arena ids until their lifetime and stability contract is clear.

## Examples And Docs

Every public SDK method includes doc comments with:

- a short purpose statement
- an example where practical
- `# Errors` for fallible methods
- `# Panics` only when no panics are expected and that is worth documenting

Examples are doctested.

## AGENTS Binding

- No `unwrap` or `expect` in public SDK implementation.
- Public structs derive or implement `Debug`; secrets are not expected in core CUE values.
- `anyhow` is not used in library public APIs.
- Use explicit imports at file tops and avoid fully qualified paths in impl bodies.
