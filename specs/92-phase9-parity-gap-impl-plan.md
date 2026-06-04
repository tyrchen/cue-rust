# Phase 9 Parity Gap Implementation Plan

Status: Active
Last updated: 2026-05-31
Reference research:
- [CUE architecture study](../docs/research/study-cue-architecture.md)
- [CUE test corpus spike](../docs/research/spike-cue-test-corpus.md)

## Purpose

This plan records the current Phase 9 gap triage before implementation. The goal is not to claim full upstream parity. The goal is to choose gaps that are both important to user-visible compatibility and small enough to implement correctly without weakening the Rust architecture.

## Gap Triage

Worth building in this batch:

1. Builtin package imports for high-frequency `strings` and `list` calls.
   - Vendor evidence: upstream CLI scripts and builtin corpus repeatedly use `strings.Join`, `strings.Split`, `strings.TrimSpace`, `strings.ToUpper`, `list.Contains`, `list.Concat`, and `list.Repeat`.
   - Importance: these imports unblock practical CUE files without implementing remote module loading.
   - Rust shape: keep parsing unchanged, record supported import aliases in compiler scope, lower imported selectors to fully qualified builtin names, and evaluate through typed builtin functions with resource caps.

2. Compatibility report expansion.
   - Vendor evidence: the current generated report only covered a narrow set of supported examples and hid major language gaps from the machine-readable summary.
   - Importance: Phase 9 needs visible pass/fail accounting, otherwise parity work can regress silently.
   - Rust shape: keep supported cases executable and mark known gaps explicitly with stable categories and reasons.

3. CLI binary name restoration to `cue`.
   - Spec evidence: README and CLI specs define the installed command as `cue`; integration tests already use `CARGO_BIN_EXE_cue`.
   - Importance: all CLI workflows and install smoke tests depend on the public binary name.
   - Rust shape: keep the app source in `apps/cue`, publish it as `cue-rust-cli`, and set the binary name to `cue`.

Not worth building in this batch:

- String interpolation, comprehensions, dynamic labels, aliases, open-list ellipsis constraints, cycle scheduling, and definition/pattern closedness. These are real gaps, but each requires new AST or ADT forms and should be implemented as dedicated batches rather than folded into builtin imports.

## Implementation Notes

- Supported builtin imports are deliberately local and explicit: `list` and `strings` only.
- Supported builtin imports include default names and explicit aliases such as `import s "strings"`.
- The compatibility report must keep the unsupported `strings` and `list` package surface visible as expected gaps.
- Unsupported imports remain compile diagnostics so registry/module work stays visible.
- Generated outputs from `strings` and `list` builtins are bounded by byte and item caps.
- Builtin behavior must return bottom values for bad arguments rather than panicking.
- Vendor-borrowed tests must assert the upstream fixture contains the borrowed behavior and then execute a reduced case through the Rust SDK.

## Verification Plan

Run once after all code in this batch is complete:

- `cargo build --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo +nightly fmt -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `cargo audit`
- `cargo deny check`
- `make vendor-corpus compat-report fuzz-smoke`
- `cargo install --path apps/cue --force`
- CLI smoke for parse, eval, export, vet, data input, tags, stdin, and import-backed builtin calls.
