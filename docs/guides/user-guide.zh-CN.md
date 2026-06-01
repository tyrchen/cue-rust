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
    Context, EncodeOptions, Encoding, EvaluatedValue, encode_value,
};

let context = Context::new();
let value = context.compile_source("example.cue", "x: 1 + 2")?;

assert_eq!(
    EvaluatedValue::Number("3".to_owned()),
    value.lookup_path(&["x"])?.evaluate()?,
);

let mut options = EncodeOptions::default();
options.encoding = Encoding::Json;
let json = encode_value(&value, options)?;
assert!(json.contains("\"x\": 3"));
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

## 什么时候该用它

适合用 `cue-rust` 的场景：本地配置计算、数据校验、Rust 项目内嵌 CUE、对 upstream
CUE 行为做兼容性实验。

暂时继续用 Go 版 `cue` 的场景：远程 registry、完整 schema 导入导出、LSP、以及必须和
upstream 每一个边角行为完全一致的生产流程。
