# Issue 0011 — Parser and compiler have no explicit nesting depth limit

Status: fixed in working tree · Severity: medium (stack exhaustion risk) · Found: 2026-06-03
Component: `crates/syntax`, `crates/compiler`, `crates/sdk`

## Summary

The evaluator has a maximum depth, but parser and compiler lowering recurse
through expressions, structs, lists, calls, interpolation expressions, and
comprehensions without a configurable nesting limit. Small but deeply nested
inputs can stress the process stack before evaluator limits apply.

## Expected behavior

Parsing and compilation should reject inputs whose syntactic or lowering depth
exceeds a documented limit. The limit should be carried through the SDK
configuration path so embedders can tune it.

## Suggested fix

- Extend `ParseConfig` with `max_depth`.
- Track parser recursion depth for expression/list/struct/call/slice parsing.
- Add compiler lowering depth checks as a second layer.
- Add tests for deeply nested arrays/structs and nested expressions.

## Resolution

Added `ParseConfig::max_depth` with a documented default and SDK builder
plumbing via `ContextConfigBuilder::max_parse_depth`. The parser now tracks
recursive expression/chained-field depth and emits
`cue.parse.max_depth_exceeded` before descending past the configured limit.
Parser recovery also now guarantees progress when an unexpected closing token is
left at top level.

Added `CompileOptions::max_depth` with `with_max_depth` plus SDK plumbing via
`ContextConfigBuilder::max_compile_depth`. Compiler lowering now emits
`cue.compile.max_depth_exceeded` as a second-layer guard. Regression tests cover
syntax parsing, direct compiler lowering, and SDK configuration behavior.
