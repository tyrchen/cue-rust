# cue-rust SDK Design

Status: Draft
Last updated: 2026-06-02
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
    pub fn with_config(config: ContextConfig) -> Self;
    pub fn parse_source(&self, source: Source) -> ParseResult;
    pub fn compile_source(&self, source: Source) -> Result<Value, CueError>;
    pub fn load(&self, config: LoadConfig) -> Result<Vec<BuildInstance>, CueError>;
    pub fn build_instance(&self, instance: &BuildInstance) -> Result<Value, CueError>;
}
```

`ContextConfig::builder()` configures parser mode, source limits, and comment
retention for current local embedding workflows. Future feature knobs for
registries, builtins, tracing, or compatibility profiles can extend this type
without changing `Context::new()`.

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

Current stable implementation:

```rust
pub struct Value { /* private */ }

impl Value {
    pub fn kind(&self) -> Result<ValueKind, EvalError>;
    pub fn evaluate(&self) -> Result<EvaluatedValue, EvalError>;
    pub fn evaluate_export(&self, options: ExportOptions) -> Result<EvaluatedValue, EvalError>;
    pub fn validate(&self, options: ValidateOptions) -> Result<(), EvalError>;
    pub fn unify(&self, other: &Value) -> Result<Value, EvalError>;
    pub fn default_value(&self) -> Result<Value, EvalError>;
    pub fn lookup(&self, path: &Path) -> Result<Value, EvalError>;
    pub fn lookup_path(&self, path: &[&str]) -> Result<Value, EvalError>;
}
```

The handle is immutable. Methods can force evaluation internally but do not mutate public state in a way callers can observe except through cached performance.

Structured path support:

```rust
pub struct Path { /* private */ }

pub enum Selector {
    Field(String),
    Definition(String),
    Hidden(String),
    Index(usize),
}
```

`Path` supports builder-style construction for regular fields, definitions,
hidden fields, and zero-based list indexes. `Path::parse` and `FromStr` support a
conservative string subset such as `a.b[0]`, `#Schema`, and `_scratch`.
`lookup_path(&[&str])` remains as a legacy string-field convenience wrapper.

Facade convenience API:

```rust
pub trait ValueExt {
    pub fn to_json(&self) -> Result<String, CueError>;
    pub fn to_json_with(&self, options: EncodeOptions) -> Result<String, CueError>;
    pub fn to_serde_json_value(&self) -> Result<serde_json::Value, CueError>;
    pub fn to_serde_json_value_with(&self, options: EncodeOptions) -> Result<serde_json::Value, CueError>;
}
```

The crate also exposes free-function equivalents for callers that prefer not to
import extension traits.

Stable top-level re-exports are limited to app embedding types: context,
diagnostics, loading, value operations, structured paths, validation, and
encoding. Parser AST, source-file internals, and compiler result types are
available under `cue_rust::experimental` until their stability contract is clear.

Current embedder compatibility gaps:

- No `FillPath` or builder-style mutation API is exposed. Callers compose base
  values and overlays by compiling each one and using `Value::unify`.
- No typed `Decode` API decodes directly into Rust structs. Callers export a
  concrete value with `encode_value` using `Encoding::Json`, then deserialize
  with `serde`.
- No `Subsume`, and no value-level reads for attributes, positions, source
  files, or documentation comments are exposed.

Any future addition of these Go CUE parity APIs must specify semantics,
diagnostics, validation limits, and compatibility tests before becoming part of
the stable facade.

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
- Experimental lower-level APIs are grouped under `cue_rust::experimental`.
- No public type exposes arena ids until their lifetime and stability contract is clear.
- Workspace internal crate dependencies include both `path` and exact `version`
  metadata so local development and crates.io packaging use the same dependency
  graph shape.
- Publishing is ordered from dependency leaves to the facade:
  `cue-rust-source`, `cue-rust-adt`, `cue-rust-syntax`, `cue-rust-eval`,
  `cue-rust-loader`, `cue-rust-compiler`, `cue-rust-encoding`, then
  `cue-rust`.

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
