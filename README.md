# cue-rust

Rust-native implementation of CUE's parser, compiler, evaluator, SDK, and CLI.

The project follows the phased implementation plan in
[`specs/91-cue-rust-impl-plan.md`](specs/91-cue-rust-impl-plan.md). The public
SDK crate is `cue-rust` and the command-line binary is `cue-rs`.

```bash
cargo build --workspace --all-targets
cargo run -p cue-rs -- version
```

## Agent support

Generated projects include agent-facing guidance for both Codex and Claude:

- `AGENTS.md` for Codex project instructions.
- `.agents/skills/{spec,research,impl}` for Codex skills.
- `CLAUDE.md` and `.claude/skills/{spec,research,impl}` for Claude Code compatibility.

## License

This project is distributed under the terms of MIT.

See [LICENSE](LICENSE.md) for details.

Copyright 2026 Tyr Chen
