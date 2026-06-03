# cue-rust

`cue-rust` is a Rust-native implementation of the core CUE workflow. It includes
a scanner, parser, compiler, evaluator, loader, encoders, public SDK, and a CLI
binary named `cue`.

Version `0.2.0` is useful for local evaluation, validation, data export, and Rust
embedding. It is still intentionally narrower than the Go `cue` command: registry
operations, LSP support, and full import/export parity for OpenAPI,
JSON-Schema, Proto, and Go types are outside the stable surface today.

## Install

From this repository:

```bash
cargo install --path apps/cue --force
cue version
```

During development:

```bash
cargo run -p cue-rs -- version
```

## Quick Start

Evaluate a CUE file:

```bash
cue eval ./config.cue
```

Export concrete data:

```bash
cue export --out json ./config.cue
cue export --out yaml ./config.cue
cue export --out toml ./config.cue
```

Validate data against a schema:

```bash
cue vet ./schema.cue --data ./data.json
```

Use the SDK:

```rust
use cue_rust::{Context, EvaluatedValue, Path, ValueExt};

let context = Context::new();
let value = context.compile_source(
    "example.cue",
    "app: { name: \"api\", port: *8080 | int }",
)?;

assert_eq!(
    EvaluatedValue::Number("8080".to_owned()),
    value
        .lookup(&Path::new().field("app").field("port"))?
        .default_value()?
        .evaluate()?,
);

let json = value.to_json()?;
assert!(json.contains("\"api\""));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Documentation

English:

- [User Guide](docs/guides/user-guide.md)
- [Developer Guide](docs/guides/developer-guide.md)

中文：

- [用户指南](docs/guides/user-guide.zh-CN.md)
- [开发指南](docs/guides/developer-guide.zh-CN.md)

More project documents are listed in [docs/index.md](docs/index.md).

## What Works In 0.2.0

- CUE scanning and tolerant parsing for the supported language subset
- AST lowering into an ADT-style runtime
- evaluation of structs, lists, defaults, disjunctions, references, constraints,
  comprehensions, interpolation, dynamic labels, closedness, and selected cycle
  behavior
- local package loading, stdin, overlays, tags, external data files, and
  module-local imports under `cue.mod/pkg`
- JSON, YAML, TOML, and CUE-like output
- broad `strings`, `list`, and finite `math` builtin coverage
- Rust SDK helpers for structured path lookup, default selection, validation,
  JSON export, and `serde_json::Value` export
- CLI commands: `parse`, `eval`, `export`, `vet`, and `version`

The current implementation is strict about source and decode limits, malformed
number literals, local import paths, symlink inputs, escape handling, and exact
numeric export for YAML/TOML.

## Current Limits

Use the Go `cue` command when you need:

- registry login, publish, or module proxy workflows
- LSP/editor integration
- complete upstream diagnostic wording parity
- full OpenAPI, JSON-Schema, Proto/Protobuf, or Go type import/export
- complete standard-library behavior for every upstream edge case

For embedding, also note:

- There is no `FillPath` or mutable value builder yet. Compose values by
  compiling inputs separately and calling `Value::unify`.
- There is no direct typed decode into Rust structs. Export concrete values to
  JSON, then deserialize with `serde`.
- There is no `Subsume` API yet.

## Development

Common targets:

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
make ci
```

Full local gate:

```bash
make check
make check-agent-sync
make fuzz-smoke
```

`make check` runs workspace build, tests, nightly rustfmt check, pedantic clippy,
docs, `cargo audit`, and `cargo deny check`.

## License

MIT. See [LICENSE.md](LICENSE.md).

Copyright 2026 Tyr Chen
