# cue-rust Parser Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Data model](10-cue-rust-data-model-design.md)

## Design Summary

Implement a Rust-native scanner and tolerant parser for CUE. Use a custom scanner because CUE needs comma insertion, nested string interpolation, comment collection, BOM/NUL handling, and source position control. Use `winnow` for grammar productions where parser-combinator structure improves clarity and recovery.

Upstream parser behavior is the compatibility target: `ParseFile` can read from filename or in-memory source, returns a partial AST on syntax errors, records positions, and sanitizes sorted error lists (`vendors/cue/cue/parser/interface.go:166`).

## Inputs

```rust
pub struct ParseConfig {
    pub mode: ParseMode,
    pub language_version: LanguageVersion,
    pub limits: SourceLimits,
}
```

`ParseMode` includes:

- package clause only
- imports only
- parse comments
- declaration errors
- all errors
- allow partial

This mirrors upstream parser modes (`vendors/cue/cue/parser/interface.go:74`).

## Scanner

Scanner responsibilities:

- Decode UTF-8 and reject invalid sequences with diagnostics.
- Skip a UTF-8 BOM at offset 0 and reject unexpected BOM elsewhere.
- Reject NUL bytes.
- Insert commas according to CUE grammar rules unless disabled.
- Track string quote stack for interpolation and multiline strings.
- Preserve comments as trivia when requested.
- Produce token spans and literal text slices.

Upstream scanner keeps current rune, offsets, comma-insertion state, quote stack, and error count (`vendors/cue/cue/scanner/scanner.go:45`). The Rust scanner should use the same state categories with Rust newtypes and bounded counters.

## Parser

Parser responsibilities:

- Build `AstFile`, declarations, labels, expressions, comments, and import specs.
- Retain malformed fragments as `Bad*` AST nodes.
- Accumulate diagnostics instead of failing fast.
- Resolve package clause and language version metadata.
- Leave identifier resolution to the compiler.

The parser uses Pratt or precedence-climbing parsing for expressions. `winnow` parsers are used for local grammar sequences where they remain readable; the outer parser remains stateful because recovery and comma insertion need mutable parser state.

## Recovery

Recovery strategy:

- On unexpected token in declarations, advance to the next declaration boundary.
- On unexpected token in expressions, produce `BadExpr` and advance to the nearest synchronizing delimiter.
- Cap repeated diagnostics per line unless `all_errors` is enabled.
- Preserve source spans for skipped regions.

The recovery contract is that formatting, diagnostics, and editor tooling can still inspect as much syntax as possible after errors.

## Source Limits

All external source text is bounded before scanning:

```rust
pub struct SourceLimits {
    pub max_file_bytes: NonZeroUsize,
    pub max_string_literal_bytes: NonZeroUsize,
    pub max_interpolation_depth: NonZeroUsize,
    pub max_parser_depth: NonZeroUsize,
    pub max_errors: NonZeroUsize,
}
```

Default limits are conservative and configurable through SDK and CLI options. Limits are enforced in bytes, not Unicode scalar counts.

## AST Compatibility Scope

M0 syntax coverage:

- package clauses and imports
- field declarations
- let declarations
- identifiers, selectors, indexes, slices
- scalar literals
- lists and structs
- unary and binary operators
- disjunction marker `*`
- ellipsis
- attributes as parsed syntax

M1 expands coverage to comprehensions, dynamic fields, aliases, interpolation, byte literals, multiline strings, and version-gated syntax.

## Diagnostics

Every syntax diagnostic includes:

- source id and byte span
- token found
- expected token class where useful
- recovery action
- stable code

Use `miette` labels for rendered CLI diagnostics. Do not let `miette` shape internal parser data structures.

## Tests

- Unit tests for scanner tokenization and comma insertion.
- `rstest` tables for grammar productions.
- `proptest` for span monotonicity and parser non-panics on arbitrary bytes.
- Snapshot tests with `insta` for diagnostics and recovered ASTs.
- Fuzz target: arbitrary bytes into `parse_file` must return diagnostics or AST, never panic.

## AGENTS Binding

- Use the latest stable `winnow` release verified during dependency review unless the parser spike proves it is a poor fit.
- No `unsafe`, no panics reachable from external input, no indexing without bounds checks.
- Reject invalid source at the boundary; do not sanitize hostile bytes into different semantic input.
- Public parser APIs document `# Errors` and include doctested examples.
