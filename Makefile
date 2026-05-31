build:
	@cargo build --workspace --all-targets

test:
	@cargo test --workspace --all-targets

fmt:
	@cargo +nightly fmt

fmt-check:
	@cargo +nightly fmt -- --check

clippy:
	@cargo clippy --workspace --all-targets -- -D warnings

clippy-pedantic:
	@cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic

doc:
	@RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

audit:
	@cargo audit

deny:
	@cargo deny check

fuzz-smoke:
	@cargo +nightly fuzz run scanner -- -runs=1
	@cargo +nightly fuzz run decoder -- -runs=1

compat-report:
	@cargo test -p cue-rust --test compatibility -- --ignored --nocapture

bench-smoke:
	@cargo bench -p cue-rust --bench phase9 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.1 --save-baseline smoke

check: build test fmt-check clippy doc audit deny

ci: check check-agent-sync

check-agent-sync:
	@cmp -s CLAUDE.md AGENTS.md || { \
		echo "AGENTS.md must stay in sync with CLAUDE.md"; \
		echo "Update both files with the same shared project instructions."; \
		exit 1; \
	}
	@tmp_dir=$$(mktemp -d); \
	trap 'rm -rf "$$tmp_dir"' EXIT; \
	cp -R .claude/skills "$$tmp_dir/expected-skills"; \
	find "$$tmp_dir/expected-skills" -name SKILL.md -exec perl -0pi -e 's/CLAUDE\.md/AGENTS.md/g; s/Claude/Codex/g; s/claude/codex/g' {} +; \
	diff -ru --exclude agents "$$tmp_dir/expected-skills" .agents/skills || { \
		echo "Codex skills must stay in sync with Claude skills after Claude-to-Codex renaming."; \
		echo "Update .claude/skills first, then mirror the shared content into .agents/skills."; \
		exit 1; \
	}

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin master
	@cargo release push --execute

update-submodule:
	@git submodule update --init --recursive --remote

.PHONY: build test fmt fmt-check clippy clippy-pedantic doc audit deny fuzz-smoke compat-report bench-smoke check ci check-agent-sync release update-submodule
