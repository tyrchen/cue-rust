# cue-rust Loader Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Parser design](11-cue-rust-parser-design.md), [Encoding design](21-cue-rust-encoding-design.md)

## Design Summary

The loader turns CLI and SDK inputs into `BuildInstance`s. It owns package selection, module root discovery, overlays, stdin, file discovery, tags, data file routing, import paths, and registry hooks. Compilation starts only after the loader has produced coherent source/package boundaries.

Upstream `cue/load.Config` is broad by design: it includes module root, module path, package name, directory, tags, data files, overlays, file systems, stdin, registry, and environment (`vendors/cue/cue/load/config.go:126`). Rust needs an equivalent subsystem, not helper functions around `parse_file`.

## Public Types

```rust
pub struct LoadConfig { /* typed-builder */ }
pub struct Loader { /* private */ }
pub struct BuildInstance { /* public snapshot */ }
pub struct BuildFile { /* public snapshot */ }
pub enum PackageSelector { Default, Named(PackageName), Any, None }
```

`LoadConfig` fields:

- current directory
- module root override
- module path override
- package selector
- parser config
- source limits
- stdin source
- overlay map
- file system adapter
- tag values
- data file behavior
- registry provider
- environment provider

## Paths And Filesystems

Use `camino::Utf8PathBuf` for validated UTF-8 paths at the API boundary. The loader rejects non-UTF-8 paths unless a future compatibility mode provides byte-path support.

Path safety:

- Reject NUL bytes and path traversal in overlay names.
- Canonicalize real filesystem roots before walking.
- For user-supplied relative paths, canonicalize and verify they remain under the configured allowed root when a root is configured.
- Never follow symlink escapes for sandboxed load roots.

## Package Arguments

Supported argument forms:

- source files
- directories
- package-qualified paths, such as `.:foo`
- recursive package patterns, such as `./...`
- import paths inside the current module
- data file qualifiers, such as `json:` and `yaml:`

Initial implementation supports local files and directories first. Recursive package patterns and import paths enter after module root discovery is stable.

## Build Instances

`BuildInstance` stores:

- package name
- import path
- root directory
- build files
- parsed AST files
- data files
- direct imports
- load diagnostics
- parser diagnostics
- language version

Upstream `build.Instance` stores this boundary data before runtime compilation (`vendors/cue/cue/build/instance.go:35`). Rust should keep it serializable for debugging but not stable as a long-term cache format.

## Tags

Tags support:

- selecting files through `@if`
- injecting field values through `@tag`
- boolean tags
- typed tag values for string, int, number, and bool

Tag values are validated as CUE literal fragments before injection. Invalid injected values produce load diagnostics, not panics.

## Overlays And Stdin

Overlay entries are named sources with validated absolute or module-relative names. They cannot be combined with an arbitrary filesystem adapter unless the adapter explicitly supports overlay semantics.

Stdin is represented as a source named `-` with an optional declared encoding. The loader applies source byte limits before decoding stdin.

## Registry

The registry interface is asynchronous only at the boundary:

```rust
pub trait RegistryProvider {
    fn resolve(&self, import_path: &ImportPath) -> Result<ResolvedPackage, RegistryError>;
}
```

Initial local loading remains synchronous. A future network registry adapter may use Tokio actors and bounded channels internally, but the evaluator never depends on async.

## Tests

- Unit tests for argument parsing and package selection.
- Integration tests for overlays, stdin, recursive directory discovery, and mixed CUE/data files.
- Security tests for symlink escape, absolute paths, traversal, oversized files, and invalid UTF-8.
- Golden tests comparing selected upstream load behaviors from `vendors/cue/cue/load`.

## AGENTS Binding

- No direct shelling out for load behavior.
- All filesystem input is hostile until validated.
- Loader APIs return `Result` with `thiserror` errors and rich diagnostics.
- Use `ignore` for directory walking once recursive patterns are implemented.
- Use explicit configuration defaults rather than hidden process-global state.
