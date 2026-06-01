# User Guide

This guide covers the supported user-facing surface of `cue-rust`: the `cue-rs`
CLI and the public Rust SDK.

## Install

From the repository root:

```bash
cargo install --path apps/cue --force
cue-rs version
```

During development you can run the binary without installing it:

```bash
cargo run -p cue-rs -- version
```

## Evaluate CUE

Create a file:

```cue
// config.cue
package config

app: {
    name: "api"
    port: *8080 | int
}
```

Evaluate it:

```bash
cue-rs eval config.cue
```

Select one field or expression:

```bash
cue-rs eval -e app.name config.cue
cue-rs eval -e 'app.port + 1' config.cue
```

Show definition, hidden, or optional fields when you need schema-oriented output:

```bash
cue-rs eval --show-definitions --show-hidden --show-optional config.cue
```

## Export Data

Export concrete values as JSON, YAML, TOML, or CUE-like syntax:

```bash
cue-rs export --out json config.cue
cue-rs export --out yaml config.cue
cue-rs export --out toml config.cue
cue-rs export --out cue config.cue
```

`export` requires concrete values. Incomplete values such as `int`, `string`,
open lists, or unresolved disjunctions are reported as errors instead of being
silently dropped.

## Validate Data

Validate a CUE value by itself:

```bash
cue-rs vet schema.cue
```

Validate external data against a CUE schema:

```bash
cue-rs vet schema.cue --data data.json
cue-rs vet schema.cue --data data.yaml --data-format yaml
cue-rs vet schema.cue --data data.toml --data-format toml
```

Data files can also be passed positionally with an encoding prefix:

```bash
cue-rs vet schema.cue json:data.json
```

## Load Packages And Imports

`cue-rs` supports local package loading and module-local imports under
`cue.mod/pkg`.

Example layout:

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

Evaluate from the module root:

```bash
cue-rs eval main.cue
```

Use `--module-root` when running from another directory:

```bash
cue-rs --module-root /path/to/project eval /path/to/project/main.cue
```

## Tags And Stdin

Inject tag values:

```bash
cue-rs -t env=prod eval config.cue
cue-rs -t debug=false eval config.cue
```

Read CUE source from stdin with `-`:

```bash
printf 'x: 1\n' | cue-rs eval -
```

## SDK Basics

Add the workspace crate from this repository, or depend on `cue-rust` once it is
published for your target use case.

Parse, compile, evaluate, and encode:

```rust
use cue_rust::{
    Context, EncodeOptions, Encoding, EvaluatedValue, encode_value,
};

let context = Context::new();
let value = context.compile_source("example.cue", "x: 1 + 2")?;

assert_eq!(
    EvaluatedValue::Number("3".to_owned()),
    value.lookup_path(&["x"])?.evaluate()?,
);

let mut options = EncodeOptions::default();
options.encoding = Encoding::Json;
let json = encode_value(&value, options)?;
assert!(json.contains("\"x\": 3"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Load local files asynchronously:

```rust
use camino::Utf8PathBuf;
use cue_rust::{Context, LoadConfig};

let context = Context::new();
let instances = context
    .load(LoadConfig::default(), &[Utf8PathBuf::from("config.cue")])
    .await?;

let value = context.build_instance(&instances[0])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## What To Expect

Use `cue-rust` for local evaluation, validation, embedding, compatibility
experiments, and Rust-native CUE workflows. Keep using the Go `cue` command when
you need remote registry operations, full schema import/export, LSP features, or
exact behavior for every upstream edge case.
