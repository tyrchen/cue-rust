# User Guide

This guide covers the user-facing surface of `cue-rust` `0.2.0`: the `cue` CLI
and the public Rust SDK.

## Install And Run

Install the CLI from the repository:

```bash
cargo install --path apps/cue --force
cue version
```

Run it without installing:

```bash
cargo run -p cue-rs -- version
```

The Cargo package is named `cue-rs`; the installed binary is named `cue`.

## Evaluate CUE

Create `config.cue`:

```cue
package config

app: {
    name: "api"
    port: *8080 | int
    replicas: 2
}
```

Evaluate the file:

```bash
cue eval config.cue
```

Select a field:

```bash
cue eval -e app.name config.cue
```

Evaluate an expression in the file context:

```bash
cue eval -e 'app.port + 1' config.cue
```

Schema-oriented output can include definitions, hidden fields, and optional
constraints:

```bash
cue eval --show-definitions --show-hidden --show-optional config.cue
```

## Export Concrete Data

Export supports JSON, YAML, TOML, and CUE-like text:

```bash
cue export --out json config.cue
cue export --out yaml config.cue
cue export --out toml config.cue
cue export --out cue config.cue
```

`export` requires concrete values. If a value is still `int`, `string`, an open
list, an unresolved disjunction, or another incomplete constraint, the command
returns an error instead of dropping the field.

Select a value before export:

```bash
cue export --out json -e app config.cue
```

## Validate Data

Validate CUE by itself:

```bash
cue vet schema.cue
```

Validate external data against CUE:

```bash
cue vet schema.cue --data data.json
cue vet schema.cue --data data.yaml --data-format yaml
cue vet schema.cue --data data.toml --data-format toml
```

External data may also be positional:

```bash
cue vet schema.cue json:data.json
```

For stdin:

```bash
printf '{"name":"api","port":8080}\n' | cue vet schema.cue --data - --data-format json
```

## Packages And Local Imports

`cue-rust` supports local package loading and module-local imports under
`cue.mod/pkg`.

Example:

```text
.
├── main.cue
└── cue.mod
    └── pkg
        └── example.com
            └── lib
                └── lib.cue
```

`main.cue`:

```cue
package app

import "example.com/lib"

value: lib.value + 1
```

`cue.mod/pkg/example.com/lib/lib.cue`:

```cue
package lib

value: 2
```

Run from the module root:

```bash
cue eval main.cue
```

Run from elsewhere:

```bash
cue --module-root /path/to/project eval /path/to/project/main.cue
```

Local import paths are intentionally constrained to `cue.mod/pkg`. Parent
directory traversal and symlink inputs are rejected.

## Tags, Stdin, And Limits

Inject tag values:

```bash
cue -t env=prod eval config.cue
cue -t debug=false eval config.cue
```

Read CUE from stdin:

```bash
printf 'x: 1\n' | cue eval -
```

Set a source byte limit:

```bash
cue --source-limit 1048576 eval config.cue
```

The CLI and loader use bounded reads. Oversized source and data inputs are
reported as errors.

## SDK Basics

The public facade is the `cue-rust` crate.

Compile, select, resolve a default, and export JSON:

```rust
use cue_rust::{Context, EvaluatedValue, Path, ValueExt};

let context = Context::new();
let value = context.compile_source(
    "config.cue",
    "app: { name: \"api\", port: *8080 | int }",
)?;

let port = value
    .lookup(&Path::new().field("app").field("port"))?
    .default_value()?
    .evaluate()?;

assert_eq!(EvaluatedValue::Number("8080".to_owned()), port);

let json = value.lookup_path(&["app"])?.to_json()?;
assert!(json.contains("\"api\""));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Load files asynchronously:

```rust
use camino::Utf8PathBuf;
use cue_rust::{Context, LoadConfig};

let context = Context::new();
let instances = context
    .load(LoadConfig::default(), &[Utf8PathBuf::from("config.cue")])
    .await?;

let value = context.build_instance(&instances[0])?;
# let _ = value;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Decode external data and unify it with a schema:

```rust
use cue_rust::{Context, DecodeOptions, Encoding, ValidateOptions, decode_bytes};

let context = Context::new();
let schema = context
    .compile_source("schema.cue", "#App: { name: string, port?: int }\nout: #App")?
    .lookup_path(&["out"])?;

let data = decode_bytes(
    Encoding::Json,
    br#"{"name":"api","port":8080}"#,
    DecodeOptions::default(),
)?;

schema.unify(&data)?.validate(ValidateOptions::default())?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Export to `serde_json::Value`:

```rust
use cue_rust::{Context, ValueExt};

let context = Context::new();
let value = context.compile_source("config.cue", "x: { ok: true }")?;
let json = value.lookup_path(&["x"])?.to_serde_json_value()?;
assert_eq!(json["ok"], true);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Embedding Notes

The stable SDK surface is centered on `Context`, `ContextConfig`, `Value`,
`Path`, `Selector`, validation options, encoding options, and diagnostic/error
types.

Lower-level parser, source, and compiler types are available through
`cue_rust::experimental`. Use that module for tooling experiments, not as a
long-term application contract.

Current limits:

- no `FillPath` or mutable value builder; compile inputs separately and use
  `Value::unify`
- no direct typed decode into Rust structs; export JSON and then use `serde`
- no `Subsume`
- no value-level API for attributes, source files, source positions, or
  documentation comments

Use `cue-rust` for local CUE evaluation, validation, Rust embedding, and parity
experiments. Keep the Go `cue` command for registry workflows, LSP, full schema
import/export, and exact upstream behavior across every edge case.
