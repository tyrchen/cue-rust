# Spike: Rust Parser Stack

Status: Done
Date: 2026-05-31
Depends on: `specs/11-cue-rust-parser-design.md`, `docs/research/study-cue-architecture.md`

## Question

Which parser stack should cue-rust use for tolerant CUE syntax while preserving spans, comments, comma insertion, and recovery nodes?

## Decision

Use a custom scanner plus a handwritten recursive-descent parser with Pratt expression parsing. Use `winnow` only for small local grammar fragments once the scanner has produced bounded token streams.

## Rationale

CUE scanning is stateful in ways that are awkward for a pure parser-combinator pipeline:

- comma insertion depends on previous token class and newline state
- string interpolation needs a quote stack
- comments and attributes must be retained for formatting and syntax export
- malformed input must produce stable `Bad*` AST nodes instead of failing the whole parse
- diagnostics must preserve byte spans and recovery actions

The upstream parser follows this shape: scanner state is explicit, parsing is tolerant, and syntax errors return partial ASTs. A fully generated parser would make recovery and source preservation harder to control. A purely handwritten stack keeps the parser close to the compatibility target and lets the project introduce `winnow` where it improves clarity without letting combinators own global recovery.

## Selected Stack

| Layer | Choice | Notes |
| --- | --- | --- |
| byte/source boundary | local `SourceFile`, `LineIndex`, byte-span newtypes | Enforce limits before scanning. |
| scanner | custom tokenizer | Handles BOM, NUL, UTF-8, comments, comma insertion, string stack. |
| expression parser | Pratt parser | Compact handling of CUE precedence and disjunction defaults. |
| declaration parser | recursive descent | Easier recovery at field, import, let, and attribute boundaries. |
| grammar helpers | `winnow` 1.0.3 candidate | Use selectively for contained productions, not as the parser driver. |
| diagnostics | `miette` rendering over local diagnostic model | Keep SDK diagnostics independent from renderer shape. |

## Consequences

- Phase 2 can ship the scanner independently of the full parser.
- Phase 3 can build tolerant AST recovery on token streams rather than raw bytes.
- Parser tests should include token snapshots, AST snapshots, recovery snapshots, and arbitrary-byte non-panic tests.
- `winnow` is an optional implementation aid, not a public contract.

