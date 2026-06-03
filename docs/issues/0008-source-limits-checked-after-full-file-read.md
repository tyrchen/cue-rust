# Issue 0008 — Source limits are checked after fully reading files into memory

Status: fixed · Severity: high (resource exhaustion) · Found: 2026-06-03
Component: `apps/cue`, `crates/loader`, `crates/source`

## Summary

Stdin uses a bounded reader (`take(limit + 1)`) before allocation can grow
unbounded, but regular files are read with `tokio::fs::read` and only checked
after the whole file is already in memory. `SourceFile::named_bytes` also
converts the whole byte slice to `String` before applying the byte limit.

## Impact

`--source-limit` and SDK `SourceLimits` do not protect callers from large disk
files. A hostile or accidental multi-gigabyte file can force allocation before
the configured limit rejects it.

## Expected behavior

File input should reject oversized files before reading their full contents, and
byte-to-string conversion should check the byte length before cloning.

## Suggested fix

- Check file metadata length before `tokio::fs::read` where possible.
- For non-regular files, read at most `limit + 1` bytes.
- Move the `SourceFile::named_bytes` length check before UTF-8 conversion.
- Add tests that oversized files are rejected by metadata/limited read paths.

## Resolution

CLI and loader file reads now check metadata before opening regular files and
then read through a bounded `take(limit + 1)` stream. `SourceFile::named_bytes`
checks byte length before cloning into a UTF-8 `String`, so oversized bytes are
rejected before UTF-8 validation or allocation growth.
