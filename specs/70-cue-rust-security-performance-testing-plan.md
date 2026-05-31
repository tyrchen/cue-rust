# cue-rust Security, Performance, And Testing Plan

Status: Draft
Last updated: 2026-05-31
Depends on: all component designs

## Security Model

Every value crossing a trust boundary is hostile:

- CLI arguments
- environment variables
- stdin
- file paths
- file contents
- overlays
- decoded JSON/YAML/TOML
- module metadata
- registry responses
- future LSP requests

Validation happens immediately at the boundary. Once data enters domain types such as `SourceName`, `PackageName`, `ImportPath`, `CuePath`, or `SourceFile`, it must already satisfy invariants.

## Mandatory Protections

- `#![forbid(unsafe_code)]` at every crate root.
- No `unwrap`, `expect`, `todo!`, or `panic!` reachable from external input.
- Byte limits on source files, strings, decoded data, streams, and diagnostics.
- Depth limits on parser, decoder, and evaluator recursion.
- Collection size limits for lists, structs, decoded maps, and generated dynamic fields.
- Checked arithmetic for externally influenced sizes and offsets.
- Path traversal controls for loader paths and overlays.
- Structured tracing without concatenating hostile user data into log messages.
- Linear-time `regex` for user-influenced regex behavior; size limits for user-supplied patterns.

## Registry And Network Security

Network registry support is out of the initial milestones. When added:

- use `rustls` with `aws-lc-rs` backend
- allowlist HTTPS schemes
- reject private, loopback, and link-local resolved IPs unless explicitly configured for local development
- pin resolved IPs for a request to avoid DNS rebinding
- enforce request timeout, body size, redirect count, and concurrency limits
- keep auth tokens in `secrecy` types and never log them

## Performance Model

Primary performance goal: correctness first, then predictable scaling on common configuration workloads.

Performance-sensitive structures:

- feature interner
- source line indexes
- scanner
- AST allocation
- vertex arena
- arc maps
- scheduler task queues
- disjunction normalization
- JSON/YAML decode and encode

Initial budgets:

- parser is linear in input size for valid source
- malformed source cannot create unbounded diagnostic output
- evaluating duplicate fields is proportional to conjunct count with bounded overhead
- export order is deterministic without repeated full-map sorting where possible

Profiling starts after M2 when parser, compiler, and evaluator can run end to end. Use `criterion` for stable microbenchmarks and `samply` or flamegraph tools for whole-command profiling.

## Test Layers

Unit tests:

- source span arithmetic
- scanner tokens
- parser productions
- feature interning
- AST lowering
- unification primitives
- validation options
- encoders and decoders

Integration tests:

- SDK compile/evaluate/export flows
- loader package discovery
- data file validation
- CLI stdout/stderr/exit codes
- module-root behavior

Property tests:

- parser never panics on arbitrary bytes
- spans are monotonic and within source bounds
- feature interner round-trips
- unification is commutative for supported pure value subsets
- validation all-errors is a superset of first-error mode

Fuzz tests:

- scanner
- parser
- JSON/YAML/TOML decoders
- AST-to-ADT compiler on recovered syntax
- evaluator on generated bounded ASTs

Conformance tests:

- mirror selected upstream CUE testdata into a Rust harness
- record expected pass, expected fail, and unsupported categories
- never hide failures by broad ignore rules
- report semantic gaps in a machine-readable summary

## Verification Commands

Every implementation phase must run:

```text
cargo build
cargo test
cargo +nightly fmt
cargo clippy -- -D warnings -W clippy::pedantic
cargo audit
cargo deny check
```

When new automation is needed, add Makefile targets rather than standalone scripts.

## Documentation Tests

Public SDK examples are doctests. Any example that requires filesystem setup should use temporary directories and explicit error handling.

## AGENTS Binding

- Security and safety are separate review sections for every implementation phase.
- Tests must include error cases and hostile inputs, not only successful examples.
- No benchmark work before a meaningful end-to-end path exists.
- Remove dead code instead of suppressing warnings.
