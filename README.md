# cue-rust

`cue-rust` is a Rust-native implementation of the core CUE toolchain: parser,
compiler, evaluator, SDK, encoders, loader, and a CLI named `cue-rs`.

The project is useful today for local CUE evaluation, validation, data export,
embedding in Rust applications, and compatibility work against selected upstream
CUE fixtures. It is not a full replacement for the Go `cue` command yet: remote
registry workflows, full schema import/export, LSP services, and exact parity for
every standard-library edge case are still outside the mature surface.

## Quick Start

```bash
cargo build --workspace --all-targets
cargo run -p cue-rs -- version
```

Evaluate a CUE file:

```bash
cargo run -p cue-rs -- eval ./config.cue
```

Export concrete data as JSON:

```bash
cargo run -p cue-rs -- export --out json ./config.cue
```

Validate JSON data against a CUE schema:

```bash
cargo run -p cue-rs -- vet ./schema.cue --data ./data.json
```

Use the SDK from Rust:

```rust
use cue_rust::{Context, EvaluatedValue};

let context = Context::new();
let value = context.compile_source("example.cue", "x: 1 + 2")?;
assert_eq!(
    EvaluatedValue::Number("3".to_owned()),
    value.lookup_path(&["x"])?.evaluate()?,
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Guides

- [User Guide](docs/guides/user-guide.md)
- [Developer Guide](docs/guides/developer-guide.md)
- [用户指南](docs/guides/user-guide.zh-CN.md)
- [开发指南](docs/guides/developer-guide.zh-CN.md)

## Current Scope

Implemented and actively tested:

- tolerant scanner and parser for the supported CUE subset
- compiler lowering into an ADT-style runtime
- evaluator support for structs, lists, defaults, disjunctions, constraints,
  references, comprehensions, interpolation, dynamic labels, closedness, and
  selected cycle behavior
- local package loading, module-local `cue.mod/pkg` imports, stdin, overlays,
  tags, and data files
- JSON, YAML, TOML, and CUE-like encoding for concrete values
- broad `strings`, `list`, and finite `math` standard-library coverage
- CLI commands: `parse`, `eval`, `export`, `vet`, and `version`

Known non-goals for the current maturity level:

- network registry client and publishing flows
- full OpenAPI, JSON Schema, Protobuf, and Go type import/export
- LSP integration
- complete upstream diagnostic wording parity
- every numerical corner case in the Go CUE standard library

## Development

The project follows the phased implementation plan in
[`specs/91-cue-rust-impl-plan.md`](specs/91-cue-rust-impl-plan.md). Useful
commands are exposed through the Makefile:

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
make ci
```

Before claiming a production-facing change is ready, run the full gate used by
the project:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

## Agent Support

Generated projects include agent-facing guidance for both Codex and Claude:

- `AGENTS.md` for Codex project instructions.
- `.agents/skills/{spec,research,impl}` for Codex skills.
- `CLAUDE.md` and `.claude/skills/{spec,research,impl}` for Claude Code
  compatibility.

## License

This project is distributed under the terms of MIT.

See [LICENSE](LICENSE.md) for details.

Copyright 2026 Tyr Chen
