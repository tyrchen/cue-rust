# Issue 0015 — Local import path traversal escapes the module cache

Status: fixed in working tree · Severity: high (source-controlled path traversal) · Found: 2026-06-03
Component: `crates/loader`

## Summary

Local import resolution joins a source import string directly under
`cue.mod/pkg` without validating path components or canonicalizing the resolved
directory before traversal. A CUE file can therefore import paths such as
`"../../../outside"` and cause the loader to read CUE files outside
`cue.mod/pkg` and even outside the module root.

The command-line argument path path already rejects `..`, but import strings
come from source and bypass that boundary check.

## Reproduction

```console
$ tree /tmp/repro
/tmp/repro
├── outside
│   └── out.cue
└── root
    ├── cue.mod
    │   ├── module.cue
    │   └── pkg
    └── main.cue

$ cat /tmp/repro/root/main.cue
package p
import "../../../outside"
x: outside.y

$ cat /tmp/repro/outside/out.cue
package outside
y: 42

$ (cd /tmp/repro/root && cue export --out json main.cue)
{
  "x": 42
}
```

## Expected behavior

Local import paths should be interpreted strictly as module-cache paths under
`cue.mod/pkg`. They must not be absolute and must not contain parent-directory
components. Existing import directories should be canonicalized and verified to
remain under the canonical `cue.mod/pkg` root before any directory walk.

## Suggested fix

- Validate local import strings before joining them into filesystem paths.
- Reject `..`, absolute paths, empty paths, and NUL bytes with a load diagnostic.
- Canonicalize found import directories and assert they remain under
  `cue.mod/pkg`, not just the module root.
- Add a loader regression test for `../../../outside`.

## Resolution

Local import paths are now validated before filesystem resolution. The loader
rejects empty, absolute, NUL-containing, or parent-directory import paths with
`cue.load.invalid_local_import_path`. Existing local import directories are
canonicalized and checked against the canonical module cache root
(`cue.mod/pkg`) before directory traversal. A regression test covers the
`../../../outside` escape, and the CLI repro now fails instead of exporting the
external value.
