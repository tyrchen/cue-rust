# Issue 0012 — CI misses local quality gates from the Makefile

Status: fixed in working tree · Severity: low (quality gate drift) · Found: 2026-06-03
Component: `.github/workflows/build.yml`

## Summary

The GitHub Actions workflow runs fmt, check, clippy, tests, and fuzz smoke, but
does not run the stricter local gates defined in the Makefile: docs with
`RUSTDOCFLAGS=-D warnings`, `cargo audit`, `cargo deny check`, and pedantic
clippy.

## Impact

Local completion criteria can pass or fail differently from CI. Security,
license, and documentation regressions may only be discovered manually.

## Expected behavior

CI should enforce the same quality gates expected before finishing a task.

## Suggested fix

Update `.github/workflows/build.yml` to run the stricter Makefile targets or
equivalent commands, including `cargo audit`, `cargo deny check`,
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, and
`cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`.

## Resolution

Updated `Makefile` so `make check` runs the stricter `clippy-pedantic` target in
addition to build, tests, fmt-check, docs, audit, and deny. Updated GitHub
Actions to install `cargo-audit` and `cargo-deny`, then run `make check`,
`make check-agent-sync`, and `make fuzz-smoke`, keeping CI aligned with the
local completion gates.
