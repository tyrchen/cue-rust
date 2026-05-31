# cue-rust CLI Design

Status: Draft
Last updated: 2026-05-31
Depends on: [SDK design](20-cue-rust-sdk-design.md), [Encoding design](21-cue-rust-encoding-design.md)

## Design Summary

The CLI binary is `cue-rs`. It follows CUE command behavior where practical but does not pretend to be upstream `cue` until compatibility gates justify that alias. The CLI is a thin application layer over the SDK.

Upstream root command wires global flags, subcommands, current directory handling, help, benchmarking, and error printing in `cmd/cue/cmd/root.go` (`vendors/cue/cmd/cue/cmd/root.go:282`). Rust should use `clap` derive for command structure and keep business logic in library crates.

## Command Set

M0:

- `cue-rs parse`
- `cue-rs fmt --check`
- `cue-rs version`

M2:

- `cue-rs eval`
- `cue-rs export`
- `cue-rs vet`

M3:

- `cue-rs def`
- `cue-rs trim`
- `cue-rs import`
- `cue-rs mod`

M5:

- `cue-rs completion`
- `cue-rs lsp`
- `cue-rs registry`

Upstream has additional commands such as `cmd`, `get`, `login`, `refactor`, and experimental commands. Those remain outside the first compatibility target.

## Global Flags

- `--trace`
- `--all-errors`
- `--strict`
- `--source-limit <bytes>`
- `--module-root <path>`
- `--package <name>`
- `--inject tag=value`
- `--out <format>`
- `--verbose`
- `--quiet`

Flag parsing uses `clap` 4.6.1 or newer stable release verified at implementation time.

## eval

Behavior:

- load package args
- compile instances
- optionally select expression paths
- evaluate values
- render CUE syntax by default
- include definitions, hidden fields, optional fields, attributes, docs, or errors as selected by flags

This tracks upstream `eval`, which supports expression selection, concrete mode, hidden/optional/attributes/all flags, and errors-as-values (`vendors/cue/cmd/cue/cmd/eval.go:30`).

## export

Behavior:

- load and compile values
- require concrete output by default
- take defaults when needed
- encode as JSON by default
- support `--out cue|json|yaml|toml|text|binary` as formats land

Upstream export defaults to JSON and rejects incomplete values unless flags change behavior (`vendors/cue/cmd/cue/cmd/export.go:24`).

## vet

Behavior:

- validate CUE packages or data files against CUE schemas
- exit silently on success unless a listing flag is used
- print diagnostics on failure
- support concrete validation by default for data files

This follows upstream `vet`, which is designed for silent success and data/schema validation (`vendors/cue/cmd/cue/cmd/vet.go:25`).

## Output And Exit Codes

- stdout: successful data output only.
- stderr: diagnostics and human messages.
- exit 0: success.
- exit 1: CUE diagnostics or validation failure.
- exit 2: CLI usage error.
- exit 3: infrastructure failure such as unreadable file.

Diagnostic rendering uses `miette` fancy output when stderr is a terminal and plain structured text otherwise. JSON diagnostic output can be added as a future flag.

## Configuration

No hidden global configuration is required for core commands. Future registry auth and module settings use a YAML config file with explicit path selection.

## Tests

- `assert_cmd` integration tests for all commands.
- Golden stderr/stdout tests with `insta`.
- CLI tests for stdin, overlays, data files, path errors, and invalid flags.
- Snapshot tests for `--help`.

## AGENTS Binding

- CLI uses `anyhow` with context for application errors.
- CLI never string-concatenates user input into shell commands.
- CLI setup initializes `tracing-subscriber`.
- Automation is exposed through Makefile targets when new command workflows are added.
