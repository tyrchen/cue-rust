# 用户指南

这份文档介绍 `cue-rust` `0.2.0` 里已经适合日常使用的部分：命令行工具 `cue` 和
Rust SDK。

## 安装和运行

在仓库根目录安装命令行工具：

```bash
cargo install --path apps/cue --force
cue version
```

开发时也可以不安装：

```bash
cargo run -p cue-rust-cli -- version
```

这里有两个名字容易混：Cargo package 叫 `cue-rust-cli`，安装出来的命令叫 `cue`。

## 计算 CUE 文件

新建 `config.cue`：

```cue
package config

app: {
    name: "api"
    port: *8080 | int
    replicas: 2
}
```

查看计算结果：

```bash
cue eval config.cue
```

只看某个字段：

```bash
cue eval -e app.name config.cue
```

也可以在当前文件上下文里计算表达式：

```bash
cue eval -e 'app.port + 1' config.cue
```

如果你正在看 schema，需要显示定义字段、隐藏字段或可选字段，可以打开这些开关：

```bash
cue eval --show-definitions --show-hidden --show-optional config.cue
```

## 导出数据

可以导出 JSON、YAML、TOML，也可以导出接近 CUE 的文本：

```bash
cue export --out json config.cue
cue export --out yaml config.cue
cue export --out toml config.cue
cue export --out cue config.cue
```

`export` 只接受具体值。`int`、`string`、开放列表、还没选定的 disjunction 等不完整值会报错，不会被静默丢掉。

导出前可以先选中一段：

```bash
cue export --out json -e app config.cue
```

## 校验数据

只校验 CUE 文件本身：

```bash
cue vet schema.cue
```

用 CUE schema 校验外部数据：

```bash
cue vet schema.cue --data data.json
cue vet schema.cue --data data.yaml --data-format yaml
cue vet schema.cue --data data.toml --data-format toml
```

数据文件也可以直接放在位置参数里，并带上格式前缀：

```bash
cue vet schema.cue json:data.json
```

从标准输入读取 JSON：

```bash
printf '{"name":"api","port":8080}\n' | cue vet schema.cue --data - --data-format json
```

## 包和本地导入

`cue-rust` 支持本地 package，也支持 `cue.mod/pkg` 下的模块内导入。

目录示例：

```text
.
├── main.cue
└── cue.mod
    └── pkg
        └── example.com
            └── lib
                └── lib.cue
```

`main.cue`：

```cue
package app

import "example.com/lib"

value: lib.value + 1
```

`cue.mod/pkg/example.com/lib/lib.cue`：

```cue
package lib

value: 2
```

在模块根目录运行：

```bash
cue eval main.cue
```

从别的目录运行时，指定模块根目录：

```bash
cue --module-root /path/to/project eval /path/to/project/main.cue
```

本地导入会被限制在 `cue.mod/pkg` 里。包含 `..` 的导入路径和符号链接输入都会被拒绝。

## Tag、标准输入和大小限制

注入 tag：

```bash
cue -t env=prod eval config.cue
cue -t debug=false eval config.cue
```

从标准输入读取 CUE：

```bash
printf 'x: 1\n' | cue eval -
```

设置输入大小上限：

```bash
cue --source-limit 1048576 eval config.cue
```

CLI 和 loader 都会按限制读取输入。文件或标准输入超出限制时会直接报错。

## 在 Rust 里使用

公开入口在 `cue-rust` crate。

编译 CUE、选字段、取默认值、导出 JSON：

```rust
use cue_rust::{Context, EvaluatedValue, Path, ValueExt};

let context = Context::new();
let value = context.compile_source(
    "config.cue",
    "app: { name: \"api\", port: *8080 | int }",
)?;

let port = value
    .lookup(&Path::new().field("app").field("port"))?
    .default_value()?
    .evaluate()?;

assert_eq!(EvaluatedValue::Number("8080".to_owned()), port);

let json = value.lookup_path(&["app"])?.to_json()?;
assert!(json.contains("\"api\""));
# Ok::<(), Box<dyn std::error::Error>>(())
```

异步加载本地文件：

```rust
use camino::Utf8PathBuf;
use cue_rust::{Context, LoadConfig};

let context = Context::new();
let instances = context
    .load(LoadConfig::default(), &[Utf8PathBuf::from("config.cue")])
    .await?;

let value = context.build_instance(&instances[0])?;
# let _ = value;
# Ok::<(), Box<dyn std::error::Error>>(())
```

把外部 JSON 和 schema 合并后校验：

```rust
use cue_rust::{Context, DecodeOptions, Encoding, ValidateOptions, decode_bytes};

let context = Context::new();
let schema = context
    .compile_source("schema.cue", "#App: { name: string, port?: int }\nout: #App")?
    .lookup_path(&["out"])?;

let data = decode_bytes(
    Encoding::Json,
    br#"{"name":"api","port":8080}"#,
    DecodeOptions::default(),
)?;

schema.unify(&data)?.validate(ValidateOptions::default())?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

如果 Rust 代码里需要 `serde_json::Value`：

```rust
use cue_rust::{Context, ValueExt};

let context = Context::new();
let value = context.compile_source("config.cue", "x: { ok: true }")?;
let json = value.lookup_path(&["x"])?.to_serde_json_value()?;
assert_eq!(json["ok"], true);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## SDK 目前的边界

稳定 API 主要围绕 `Context`、`ContextConfig`、`Value`、`Path`、`Selector`、校验选项、编码选项和诊断/错误类型。

`cue_rust::experimental` 里有更底层的 parser、source、compiler 类型。它适合做工具链实验，不建议当作长期稳定的业务接口。

现在还没有这些能力：

- 没有 `FillPath`，也没有可变的 value builder；要叠加数据时，分别编译后用 `Value::unify`
- 不能直接 decode 成 Rust struct；先导出 JSON，再交给 `serde`
- 没有 `Subsume`
- 还不能从 `Value` 直接读取 attribute、源码文件、源码位置或文档注释

适合使用 `cue-rust` 的场景：本地配置计算、数据校验、Rust 项目内嵌 CUE、兼容性验证。需要 registry、LSP、完整 schema 导入导出，或者必须和 Go 版 CUE 每个边角行为完全一致时，先继续使用 Go 版 `cue`。
