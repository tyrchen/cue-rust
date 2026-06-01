# 开发指南

这份文档写给要改 `cue-rust` 本身的人。

## 仓库结构

```text
apps/cue/        命令行工具，二进制名是 cue-rs
crates/adt/      ADT 运行时数据结构
crates/compiler/ AST 到 ADT 的 lowering
crates/encoding/ JSON、YAML、TOML 和 CUE-like 输出
crates/eval/     evaluator、校验、内建函数、导出 profile
crates/loader/   本地包加载、模块内导入、stdin、tag
crates/sdk/      对外的 cue-rust facade
crates/source/   源文件、大小限制、诊断
crates/syntax/   scanner、parser、AST
docs/research/   调研记录
specs/           产品、设计、路线图和实现计划
vendors/         vendored upstream CUE，用来做 parity 对照
```

对外 crate 是 `cue-rust`，命令行 package 是 `cue-rs`。

## 日常开发命令

优先用 Makefile 里的目标：

```bash
make build
make test
make clippy-pedantic
make vendor-corpus
make compat-report
```

准备提交生产相关改动前，跑完整 gate：

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

不要跑 `cargo clean`。项目约定里明确禁止，除非用户单独批准。

## 代码规则

`AGENTS.md` 是本仓库的硬性规则，不是建议。最容易踩坑的几条：

- crate 使用 Rust 2024，禁止 unsafe
- library error 用 `thiserror`，CLI 里用 `anyhow`
- 外部输入路径上不要用 `unwrap`、`expect`、`todo`、`unimplemented`、`panic`
- loader、parser、CLI、decode、encode 边界都要做输入校验
- 能用类型表达的约束不要塞进松散字符串
- public item 要写文档注释
- 改动范围要收住，围绕当前 phase 或 gap 做完整一批

新增自动化时，优先加 Makefile target，不要散落新的 shell 脚本。

## 架构分层

主流程按层推进：

1. `source` 处理字节、文件名、大小限制和诊断。
2. `syntax` 扫描并解析成容错 AST。
3. `loader` 把文件组织成 build instance，处理本地输入、tag、data file、模块内导入。
4. `compiler` 把 AST 降到 ADT runtime。
5. `eval` 求值、应用约束、处理内建函数，并提供 validate/export 行为。
6. `encoding` 把具体值编码出去。
7. `sdk` 提供对外稳定入口。
8. `apps/cue` 把 SDK 接成命令行。

不要跨层抄近路。比如 evaluator 不应该读文件，encoder 不应该重新解释源码语法。

## 做兼容性改动

upstream CUE 放在 `vendors/cue`。处理 parity gap 时按这个顺序来：

1. 找到 upstream 里的 fixture 或实现代码，确认真实行为。
2. 判断这个 gap 对当前成熟度是否真的重要。
3. 在 `crates/sdk/tests/` 下补 Rust integration test。
4. 做一批架构上说得通的实现，不要为了一个 fixture 写特判。
5. 只有可执行覆盖证明 gap 已关闭，才通过 `make compat-report` 更新报告。

好的 parity 改动通常会同时碰 parser、compiler、evaluator。只靠对齐某个输出字符串，不能算真正支持。

## 测试分层

按目的选测试：

- crate 内 unit test：测局部行为
- `crates/sdk` 测试：测公开 API
- `crates/sdk/tests/vendor_corpus.rs`：承接 upstream fixture
- `apps/cue` integration test：测命令行行为
- compatibility report：维护 pass 和 expected-fail 的机器可读账本
- fuzz smoke：保证 scanner 和 decoder 不容易被输入打崩

测试名沿用仓库里的 `test_should_...` 风格。

## 改 CLI

CLI 应该保持薄层，只负责参数、IO、退出码和用户能看懂的错误上下文。核心逻辑要放进 SDK
或者更底层 crate。

新增命令行可见行为时，尽量在 `apps/cue/tests` 里补 vendor-style script test。

## 改文档

规格文档放 `specs/`，用户和开发文档放 `docs/`，调研记录放 `docs/research/`。新增
docs 时更新 `docs/index.md`；新增 specs 时更新 `specs/index.md`。

文档要说实话。还属于兼容性缺口的地方，就明确写缺口，不要暗示已经完全对齐 upstream。
