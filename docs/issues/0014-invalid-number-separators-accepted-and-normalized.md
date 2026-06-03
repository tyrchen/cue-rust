# Issue 0014 — Invalid number separators are accepted and normalized

Status: fixed in working tree · Severity: medium (invalid source accepted; values can change) · Found: 2026-06-03
Component: `crates/syntax`

## Summary

The scanner accepts `_` anywhere inside the digit run for a number token. That
means malformed number literals such as `1_`, `1__2`, `1e`, `1e_`, and `1._2`
parse successfully even though the separator or exponent placement is invalid.

Because later encoders normalize digit separators before external export, some
of these invalid literals silently become different concrete numbers. For
example, `1_` exports as `1`, `1__2` exports as `12`, and `1._2` exports as
`1.2`.

## Reproduction

```console
$ printf 'x: 1_\n' | cue parse -
field x
  number 1_

$ printf 'x: 1_\n' | cue export --out json -
{
  "x": 1
}
```

The same class reproduces with `1__2`, `1e`, `1e_`, and `1._2`.

## Expected behavior

Malformed numeric literals should be rejected at the scanner/parser boundary.
Digit separators should only be valid between digits, and exponent markers must
be followed by at least one digit after an optional sign.

## Suggested fix

- Validate each scanned number token before emitting it.
- Report a `cue.scan.invalid_number` diagnostic for malformed separator,
  decimal, or exponent placement.
- Keep accepting valid separators such as `1_000`, `1_000.5_0`, `1e1_0`, and
  `1_0e+2`.
- Add scanner regression tests for both valid and invalid separator forms.

## Resolution

The scanner now validates each number token before parser recovery. Digit runs
reject leading, trailing, or consecutive `_`, fractional parts must contain at
least one digit, and exponents must contain at least one digit after an optional
sign. Malformed literals now produce `cue.scan.invalid_number`, while valid
separator forms such as `1_000` and `1_0e+2` continue to scan and export
correctly.
