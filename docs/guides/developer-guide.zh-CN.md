# 开发指南

这份文档写给要改 `cue-rust` 的开发者。

## 目录结构

```text
apps/cue/        命令行 package，安装后的命令叫 cue
crates/adt/      语义图和运行时数据结构
crates/compiler/ AST 到 ADT 的 lowering
crates/encoding/ JSON、YAML、TOML、CUE-like 编解码
crates/eval/     求值、校验、内建函数、默认值、导出规则
crates/loader/   本地包加载、stdin、overlay、tag、data file
crates/sdk/      对外的 cue-rust facade
crates/source/   源文件、大小限制、span、诊断
crates/syntax/   scanner、parser、AST
docs/guides/     用户和开发文档
docs/issues/     详细问题报告和修复记录
docs/research/   调研记录
specs/           产品、设计、路线图和实现计划
vendors/         vendored upstream CUE，用来做兼容性对照
```

对外 SDK crate 是 `cue-rust`。命令行 package 是 `cue-rs`，二进制命令是 `cue`。

## 架构分层

这个项目按层组织。改代码时，尽量让逻辑留在它该在的层。

1. `source` 负责源文件名、字节上限、UTF-8、span、行号索引和诊断。
2. `syntax` 把 CUE 扫描、解析成容错 AST。
3. `loader` 把本地输入整理成 build instance，处理 overlay、tag、外部数据文件和模块内导入。
4. `compiler` 把 AST 降到 ADT runtime。
5. `eval` 做求值、unify、约束检查、默认值、内建函数、validate/export。
6. `encoding` 在 evaluated value 和 JSON/YAML/TOML/CUE-like 数据之间转换。
7. `sdk` 提供业务代码使用的稳定入口。
8. `apps/cue` 负责命令行参数、IO、错误上下文和退出码。

不要跨层抄近路。比如 evaluator 不应该读文件，encoder 不应该重新解释源码。

## 日常开发

优先用 Makefile：

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
```

生产相关改动收尾前，至少跑：

```bash
make check
make check-agent-sync
make fuzz-smoke
```

`make check` 包含：

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

不要跑 `cargo clean`，除非用户明确同意。

## 代码要求

仓库规则写在 `AGENTS.md`。平时最该注意的是这些：

- 使用 Rust 2024
- 禁止 unsafe
- public item 要有文档注释
- library error 用 `thiserror`，CLI 串流程用 `anyhow`
- 生产代码不要在外部输入路径上用 `unwrap`、`expect`、`todo`、`unimplemented` 或会被用户输入触发的 panic
- source、parser、loader、decoder、encoder、CLI 边界都要校验输入
- 文件大小、decode 深度、集合大小、内建函数生成量、正则大小、disjunction 展开、parse/compile/eval 递归都要有明确上限
- 能用类型和 checked arithmetic 表达的约束，不要藏在随意拼出来的字符串里
- 改动要围绕当前问题收住，不要顺手大重构

新增自动化时，优先加 Makefile target，不要散落临时脚本。

## 做兼容性改动

upstream CUE 放在 `vendors/cue`。

处理兼容性缺口时，建议按这个顺序：

1. 先找 upstream fixture、实现代码或 spec，确认真实行为。
2. 判断这个缺口是否属于当前版本要覆盖的范围。
3. 先补一个聚焦的 Rust 测试。公开行为通常放在 `crates/sdk/tests` 或 `apps/cue/tests`。
4. 在正确的层实现，不要写只服务某个 fixture 的特判。
5. 只有可执行测试证明缺口关闭后，才更新 compatibility report。

不少兼容性问题会同时影响 parser、compiler 和 evaluator。只把某个输出字符串凑对，不代表语义真的对。

## 测试怎么放

按问题范围选测试层级：

- crate 内 unit test：测局部逻辑
- SDK 测试：测公开 API
- CLI 测试：测命令行流程、输出和退出码
- vendor fixture：对照 upstream 行为
- compatibility report：记录 pass 和 expected-fail
- fuzz smoke：保证 scanner 和 decoder 不容易被随机输入打崩

测试名沿用仓库里的 `test_should_...` 风格。

修 bug 时，能先写复现就先写复现。修安全或资源边界时，要直接测边界本身，不要只测下游现象。

## 改 CLI

CLI 保持薄层。它负责：

- 参数解析
- stdin/stdout/stderr
- 文件 IO 的用户上下文
- 退出码
- 用户能看懂的错误信息

CUE 的核心行为应该放在 SDK 或更底层 crate。命令行可见行为有变化时，在 `apps/cue/tests` 里补集成测试。

## 改文档

- 用户和开发文档放 `docs/guides/`
- 问题报告和修复记录放 `docs/issues/`
- 调研记录放 `docs/research/`
- 产品和实现规格放 `specs/`

新增 docs 时更新 `docs/index.md`。新增 specs 时更新 `specs/index.md`。

文档要诚实。还没支持的行为就明确说是缺口，不要写得像已经完整对齐 upstream。

## 发版检查

版本升级时：

1. 更新 `Cargo.toml` 里的 `workspace.package.version` 和 workspace 内部依赖版本。
2. 重新生成 `Cargo.lock`。
3. 如果能力范围或成熟度有变化，同步更新 README 和用户文档。
4. 跑完整检查：

```bash
make check
make check-agent-sync
make fuzz-smoke
```

发布前按依赖顺序 dry-run：

```bash
cargo publish -p cue-rust-source --dry-run
cargo publish -p cue-rust-adt --dry-run
cargo publish -p cue-rust-syntax --dry-run
cargo publish -p cue-rust-eval --dry-run
cargo publish -p cue-rust-loader --dry-run
cargo publish -p cue-rust-compiler --dry-run
cargo publish -p cue-rust-encoding --dry-run
cargo publish -p cue-rust --dry-run
cargo publish -p cue-rs --dry-run
```
