# Issue 0009 — String and bytes escape decoding is incomplete and accepts invalid escapes

Status: fixed in working tree · Severity: medium (incorrect data and diagnostics) · Found: 2026-06-03
Component: `crates/compiler`, `crates/syntax`

## Summary

Literal lowering decodes only a small subset of CUE escapes. Unicode escapes
such as `"\u0041"` are left as the six literal bytes `\u0041` instead of `A`;
`\b` and `\f` are also not decoded. Unknown or malformed escapes are preserved
instead of producing diagnostics.

## Reproduction

```cue
u: "\u0041"
b: "a\b"
f: "a\f"
```

Current JSON export contains escaped backslash sequences rather than the decoded
characters.

## Expected behavior

Supported escapes should decode according to CUE string/bytes literal rules.
Unsupported or malformed escapes should produce parse/compile diagnostics rather
than silently changing the literal meaning.

## Suggested fix

Introduce a fallible literal decoder used by compiler lowering:

- support `\n`, `\r`, `\t`, `\b`, `\f`, quotes, slash/backslash, `\xNN`,
  `\uNNNN`, and `\UNNNNNNNN`;
- validate hex length and Unicode scalar values;
- emit diagnostics for malformed escapes;
- preserve bytes-specific behavior for `'\xNN'` while rejecting invalid Unicode
  in string literals.

## Resolution

Implemented a fallible compiler literal decoder for strings, bytes, and
interpolation text. It now decodes the supported control, quote, slash,
hex-byte, and Unicode escapes, reports `cue.compile.invalid_escape` for
malformed sequences, and keeps invalid escapes from silently compiling into
wrong data. SDK unit tests cover Unicode/control decoding, bytes decoding, and
invalid escape diagnostics.
