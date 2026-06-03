# Issue 0016 — Symlink file arguments bypass the loader policy

Status: fixed in working tree · Severity: medium (loader boundary policy bypass) · Found: 2026-06-03
Component: `crates/loader`

## Summary

The loader has an explicit `LoadError::Symlink` policy and rejects symlink
entries during directory walks, but direct file arguments are canonicalized
before `symlink_metadata` is checked. Once canonicalized, `link.cue` has already
become the target path, so the symlink is no longer visible to the check.

This means direct symlink inputs are accepted even though the loader reports
that symlink inputs are not allowed.

## Reproduction

```console
$ printf 'x: 1\n' > real.cue
$ ln -s real.cue link.cue
$ cue export --out json link.cue
{
  "x": 1
}
```

## Expected behavior

The loader should reject a direct symlink argument before canonicalizing the
path, returning `LoadError::Symlink`.

## Suggested fix

- Check `symlink_metadata` on the resolved, pre-canonical path.
- Return `LoadError::Symlink` when the leaf argument is a symlink.
- Keep canonical root validation after the symlink check to preserve existing
  root-escape protection.
- Add a regression test for `link.cue -> real.cue`.

## Resolution

Direct loader arguments now run `symlink_metadata` on the resolved
pre-canonical path. A symlink leaf returns `LoadError::Symlink` before the path
is canonicalized, while canonical root validation still runs afterwards for
ordinary files and directories. A Unix regression test covers `link.cue ->
real.cue`, and the CLI repro now fails with the symlink policy error.
