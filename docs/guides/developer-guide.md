# Developer Guide

This guide is for contributors working on `cue-rust`.

## Repository Layout

```text
apps/cue/        CLI package; installs a binary named cue
crates/adt/      semantic graph and runtime data structures
crates/compiler/ AST-to-ADT lowering
crates/encoding/ JSON, YAML, TOML, and CUE-like encoding/decoding
crates/eval/     evaluator, validation, builtins, defaults, export profiles
crates/loader/   local package loader, stdin, overlays, tags, data files
crates/sdk/      public cue-rust facade
crates/source/   source files, byte limits, spans, diagnostics
crates/syntax/   scanner, parser, AST
docs/guides/     user and developer documentation
docs/issues/     detailed bug reports and fix notes
docs/research/   research notes and prior-art studies
specs/           product, design, roadmap, and implementation plans
vendors/         vendored upstream CUE checkout for compatibility work
```

The published SDK crate is `cue-rust`. The CLI package is `cue-rust-cli`, and its
binary is `cue`.

## Architecture

The implementation is layered. Keep behavior in the layer that owns it.

1. `source` validates source names, byte limits, UTF-8, spans, line indexes, and
   diagnostics.
2. `syntax` scans and parses CUE into a tolerant AST.
3. `loader` resolves local inputs into build instances, applies overlays and
   tags, records external data files, and resolves module-local imports.
4. `compiler` lowers AST files into the ADT runtime.
5. `eval` evaluates vertices, unifies values, enforces constraints, handles
   defaults, runs builtins, and validates/export profiles.
6. `encoding` converts between evaluated values and JSON/YAML/TOML/CUE-like
   data.
7. `sdk` exposes the application-facing facade.
8. `apps/cue` provides CLI orchestration, IO, error context, and exit codes.

Do not bypass earlier layers from later layers. For example, the evaluator
should not read files, and the encoder should not reinterpret source syntax.

## Development Loop

Prefer Makefile targets:

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
```

Before claiming a production-facing change is ready, run:

```bash
make check
make check-agent-sync
make fuzz-smoke
```

`make check` includes:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

Do not run `cargo clean` unless the user explicitly approves it.

## Code Rules

Project policy lives in `AGENTS.md`. The rules that matter most in daily work:

- Rust 2024 edition
- `#![forbid(unsafe_code)]`
- public items documented
- `thiserror` for library errors, `anyhow` for CLI orchestration
- no `unwrap`, `expect`, `todo`, `unimplemented`, or user-input panics in
  production code
- validate input at source, parser, loader, decoder, encoder, and CLI
  boundaries
- use explicit limits for source bytes, decoded depth, collection size,
  generated builtin output, regex size, disjunction expansion, and recursive
  parse/compile/eval paths
- prefer structured types and checked arithmetic over loosely shaped strings
- keep changes scoped to the behavior being fixed

For new automation, add a Makefile target instead of a one-off shell script.

## Compatibility Work

Upstream CUE is vendored under `vendors/cue`.

When closing a compatibility gap:

1. Find the upstream fixture, implementation, or spec text that defines the
   behavior.
2. Decide whether the gap is in scope for the current maturity level.
3. Add a focused Rust test. Public behavior usually belongs in `crates/sdk/tests`
   or `apps/cue/tests`.
4. Implement the behavior in the proper layer. Avoid fixture-specific branches.
5. Update compatibility reporting only when executable evidence proves the gap
   is closed.

Good parity batches often cross parser, compiler, and evaluator. Matching one
output string is not enough if the underlying value semantics are still wrong.

## Testing Strategy

Use the smallest test layer that proves the behavior:

- crate unit tests for local parser, compiler, evaluator, loader, source, and
  encoding behavior
- SDK tests for public API behavior
- CLI tests for command-line workflows and exit codes
- borrowed vendor fixtures for upstream compatibility
- compatibility reports for pass/expected-fail accounting
- fuzz smoke for scanner and decoder crash resistance

Test names should follow the existing `test_should_...` style.

For bug fixes, add the reproduction first when practical. If a fix protects a
security or resource boundary, test the boundary directly, not only a downstream
symptom.

## CLI Guidelines

The CLI should stay thin. It owns:

- argument parsing
- stdin/stdout/stderr
- file IO boundary context
- process exit codes
- user-facing error messages

Core CUE behavior belongs in the SDK or lower crates. If a CLI feature exposes
new behavior, add an integration test under `apps/cue/tests`.

## Documentation

- User and developer docs live under `docs/guides/`.
- Bug reports and fix notes live under `docs/issues/`.
- Research notes live under `docs/research/`.
- Product and implementation specs live under `specs/`.

Update `docs/index.md` when adding docs. Update `specs/index.md` when adding
specs.

Keep documentation honest about maturity. If a behavior is a known gap, name the
gap clearly.

## Release Checklist

For a version bump:

1. Update `workspace.package.version` and workspace internal dependency versions
   in `Cargo.toml`.
2. Regenerate `Cargo.lock`.
3. Update README and user-facing docs when the release changes behavior or
   maturity.
4. Run the full gate:

```bash
make check
make check-agent-sync
make fuzz-smoke
```

`make check` includes verified workspace packaging, which verifies every crate
can be packaged and built from its published archive. For the initial multi-crate
release, downstream `cargo publish --dry-run` commands can only pass after their
workspace dependencies are already available in the crates.io index. Publish in
dependency order:

```bash
cargo publish -p cue-rust-source
cargo publish -p cue-rust-adt
cargo publish -p cue-rust-syntax
cargo publish -p cue-rust-eval
cargo publish -p cue-rust-loader
cargo publish -p cue-rust-compiler
cargo publish -p cue-rust-encoding
cargo publish -p cue-rust
cargo publish -p cue-rust-cli
```
