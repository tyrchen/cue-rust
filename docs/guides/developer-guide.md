# Developer Guide

This guide is for contributors working on `cue-rust` itself.

## Repository Layout

```text
apps/cue/        CLI binary, published as cue-rs
crates/adt/      ADT runtime data structures
crates/compiler/ AST-to-ADT lowering
crates/encoding/ JSON, YAML, TOML, and CUE-like output
crates/eval/     evaluator, validation, builtins, export profiles
crates/loader/   local package loader, module-local imports, stdin, tags
crates/sdk/      public cue-rust facade
crates/source/   source files, limits, diagnostics
crates/syntax/   scanner, parser, AST
docs/research/   prior-art and design research
specs/           product, design, roadmap, and implementation plans
vendors/         vendored upstream CUE source used for parity work
```

The public crate is `cue-rust`; the CLI package is `cue-rs`.

## Development Loop

Start with the existing Makefile targets:

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
```

Run the full gate before a production-facing commit:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

Do not use `cargo clean`. The repository policy forbids it unless a user
explicitly approves it.

## Implementation Rules

Follow `AGENTS.md` as binding project policy. The most important rules for this
codebase are:

- keep crates on Rust 2024 and forbid unsafe code
- use `thiserror` for library error types and `anyhow` in the CLI
- avoid `unwrap`, `expect`, `todo`, `unimplemented`, and panics on external input
- validate input at loader, parser, CLI, decoding, and encoding boundaries
- prefer typed domain values and explicit invariants over loosely shaped strings
- keep public items documented
- keep edits scoped to the phase or gap being addressed

When adding automation, prefer a Makefile target over a loose shell script.

## Architecture

The pipeline is intentionally layered:

1. `source` validates bytes, names, limits, and diagnostics.
2. `syntax` scans and parses into a tolerant AST.
3. `loader` groups files into build instances, resolves local inputs, injects
   tags, records data files, and resolves module-local imports.
4. `compiler` lowers AST files into the ADT runtime.
5. `eval` evaluates vertices, applies constraints, handles builtins, and exposes
   validation/export behavior.
6. `encoding` serializes concrete evaluated values.
7. `sdk` provides the stable facade used by applications and tests.
8. `apps/cue` wires the SDK into the CLI.

Do not bypass earlier layers from later layers. For example, the evaluator should
not parse files, and the encoder should not reinterpret source syntax.

## Compatibility Work

Upstream CUE is vendored under `vendors/cue`. Before implementing a parity gap:

1. Find the upstream fixture or implementation that defines the behavior.
2. Decide whether the gap is high value for the current maturity level.
3. Add or extend a Rust integration test under `crates/sdk/tests/`.
4. Implement the smallest coherent architectural batch, not a one-off fixture
   hack.
5. Update `target/compatibility/core.json` coverage through
   `make compat-report` only when executable evidence proves the gap is closed.

Good parity batches usually touch parser/compiler/evaluator together. Avoid
claiming support by only matching one fixture string.

## Tests

Use these test layers deliberately:

- unit tests in the crate module for focused behavior
- `crates/sdk` tests for public API behavior
- `crates/sdk/tests/vendor_corpus.rs` for borrowed upstream fixture behavior
- `apps/cue` integration tests for CLI workflows
- compatibility report tests for machine-readable pass/expected-fail accounting
- fuzz smoke targets for scanner and decoder crash resistance

Test names should use the `test_should_...` style already used in the repo.

## CLI Changes

The CLI should remain a thin orchestration layer. It may handle command-line
parsing, IO, process exit codes, and user-facing error context. Core behavior
belongs in the SDK or lower crates.

When adding a CLI feature, add a vendor-style script test under `apps/cue/tests`
when the behavior is observable from the command line.

## Documentation Changes

Specs live under `specs/`. User and developer documentation lives under `docs/`.
Research memos live under `docs/research/`. When adding docs, update
`docs/index.md`; when adding specs, update `specs/index.md`.

Keep documentation honest about maturity. If a feature is a compatibility gap,
say so directly instead of implying full upstream parity.
