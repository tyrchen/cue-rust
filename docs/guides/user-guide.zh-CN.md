# 用户指南

这份指南讲 `cue-rust` 目前能稳定使用的部分：`cue-rs` 命令行工具和 Rust SDK。

## 安装

在仓库根目录执行：

```bash
cargo install --path apps/cue --force
cue-rs version
```

开发时也可以不安装，直接运行：

```bash
cargo run -p cue-rs -- version
```

## 计算 CUE 文件

先写一个文件：

```cue
// config.cue
package config

app: {
    name: "api"
    port: *8080 | int
}
```

运行：

```bash
cue-rs eval config.cue
```

只看某个字段，或者直接计算一个表达式：

```bash
cue-rs eval -e app.name config.cue
cue-rs eval -e 'app.port + 1' config.cue
```

如果你在看 schema，而不是最终数据，可以打开定义字段、隐藏字段和可选字段：

```bash
cue-rs eval --show-definitions --show-hidden --show-optional config.cue
```

## 导出数据

可以导出成 JSON、YAML、TOML，也可以导出成接近 CUE 的文本：

```bash
cue-rs export --out json config.cue
cue-rs export --out yaml config.cue
cue-rs export --out toml config.cue
cue-rs export --out cue config.cue
```

`export` 要求结果是具体值。像 `int`、`string`、开放列表、还没选定的 disjunction
这类不完整值，会报错，不会悄悄吞掉。

## 校验数据

只校验 CUE 文件本身：

```bash
cue-rs vet schema.cue
```

用 CUE schema 校验外部数据：

```bash
cue-rs vet schema.cue --data data.json
cue-rs vet schema.cue --data data.yaml --data-format yaml
cue-rs vet schema.cue --data data.toml --data-format toml
```

也可以把数据文件作为位置参数传入，并带上格式前缀：

```bash
cue-rs vet schema.cue json:data.json
```

## 包和本地导入

`cue-rs` 支持本地包加载，也支持 `cue.mod/pkg` 下面的模块内导入。

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
cue-rs eval main.cue
```

如果从别的目录运行，可以显式指定模块根目录：

```bash
cue-rs --module-root /path/to/project eval /path/to/project/main.cue
```

## 标签和标准输入

注入 tag：

```bash
cue-rs -t env=prod eval config.cue
cue-rs -t debug=false eval config.cue
```

从标准输入读取 CUE：

```bash
printf 'x: 1\n' | cue-rs eval -
```

## 在 Rust 里使用

解析、编译、求值、编码：

```rust
use cue_rust::{
    Context, ContextConfig, EvaluatedValue, Path, SourceLimits, ValueExt,
};

let context = Context::with_config(
    ContextConfig::builder()
        .source_limits(SourceLimits::default())
        .include_comments(false)
        .build(),
);
let value = context.compile_source("example.cue", "x: { items: [*1 | 2, 3] }")?;

assert_eq!(
    EvaluatedValue::Number("1".to_owned()),
    value
        .lookup(&Path::new().field("x").field("items").index(0))?
        .default_value()?
        .evaluate()?,
);

let json = value.to_json()?;
assert!(json.contains("\"items\""));
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
# Ok::<(), Box<dyn std::error::Error>>(())
```

### SDK 兼容性说明

公开 SDK 适合做本地内嵌使用，但它还不是 Go CUE API 的完整克隆。当前内嵌方可以使用已经实现的
结构化 path 和默认值 API：

```rust
use cue_rust::{Context, EvaluatedValue, Path, ValueExt};

let context = Context::new();
let value = context.compile_source(
    "schema.cue",
    "#Schema: { _choices: [*\"default\" | \"other\", \"second\"] }",
)?;

let path = Path::parse("#Schema._choices[0]")?;
assert_eq!(
    EvaluatedValue::String("default".to_owned()),
    value.lookup(&path)?.default_value()?.evaluate()?,
);

let json_value = value
    .lookup(&Path::parse("#Schema._choices")?)?
    .to_serde_json_value()?;
assert!(json_value.is_array());
# Ok::<(), Box<dyn std::error::Error>>(())
```

稳定 facade 以 `Context`、`ContextConfig`、`Value`、`Path`、`Selector`、校验选项、编码选项和
诊断/错误类型为中心。更底层的 parser、source、compiler 内部类型放在
`cue_rust::experimental` 下；它们适合工具链实验，不适合作为稳定 app 内嵌契约。

当前内嵌方仍然需要注意：

- 没有 `FillPath` 或 builder 风格的可变构造 API。需要叠加数据或覆盖值时，分别编译成
  `Value`，再调用 `Value::unify`。
- 没有直接解码到 Rust struct 的 typed decode。先把具体的 `Value` 导出为 JSON，再交给
  `serde`：

```rust
use cue_rust::{Context, EncodeOptions, Encoding, encode_value};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AppConfig {
    name: String,
    port: u16,
}

let context = Context::new();
let value = context.compile_source(
    "config.cue",
    r#"app: { name: "api", port: *8080 | int }"#,
)?;

let mut options = EncodeOptions::default();
options.encoding = Encoding::Json;
let json = encode_value(&value.lookup_path(&["app"])?, options)?;
let config: AppConfig = serde_json::from_str(&json)?;
# let _ = config;
# Ok::<(), Box<dyn std::error::Error>>(())
```

- 没有 `Subsume`，也没有 value 级别的 attribute、源码位置、源码文件或文档注释读取 API。

## 什么时候该用它

适合用 `cue-rust` 的场景：本地配置计算、数据校验、Rust 项目内嵌 CUE、对 upstream
CUE 行为做兼容性实验。

暂时继续用 Go 版 `cue` 的场景：远程 registry、OpenAPI/JSON-Schema/Proto 导入导出、
LSP、以及必须和 upstream 每一个边角行为完全一致的生产流程。
