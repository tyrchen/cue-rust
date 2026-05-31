# cue-rust Encoding Design

Status: Draft
Last updated: 2026-05-31
Depends on: [Evaluator design](15-cue-rust-evaluator-design.md), [Loader design](12-cue-rust-loader-design.md)

## Design Summary

Encoding converts between CUE values and external data formats. It must be centralized so JSON, YAML, TOML, and future schema formats share the same evaluated ADT semantics instead of each format implementing its own validator.

Upstream has dedicated encoding packages and an internal shared decoder that routes CUE, JSON, JSONL, YAML, TOML, INI, XML, text, binary, protobuf, textproto, JSON Schema, and OpenAPI (`vendors/cue/internal/encoding/encoding.go:50`).

## Initial Formats

M2 supports:

- CUE source
- JSON
- YAML
- TOML
- text
- binary bytes for explicit bytes values

Later milestones add:

- JSONL
- INI
- XML
- protobuf
- textproto
- JSON Schema interpretation
- OpenAPI interpretation

## Decoder API

```rust
pub struct DecodeConfig { /* typed-builder */ }
pub struct Decoder { /* private */ }

impl Decoder {
    pub fn from_bytes(config: DecodeConfig, bytes: &[u8]) -> Result<Self, DecodeError>;
    pub fn next_value(&mut self) -> Result<Option<DecodedValue>, DecodeError>;
}
```

`DecodedValue` carries:

- source span table
- AST expression or CUE value
- detected encoding
- stream item index
- diagnostics

## Encoder API

```rust
pub trait Encoder {
    fn encode_value(&mut self, value: &Value, options: EncodeOptions) -> Result<(), EncodeError>;
    fn finish(self) -> Result<(), EncodeError>;
}
```

Concrete encoders:

- `JsonEncoder`
- `YamlEncoder`
- `TomlEncoder`
- `CueSyntaxEncoder`
- `TextEncoder`

## JSON

Use `serde_json` with arbitrary precision handling where required by CUE number semantics. `serde_json` reported `1.0.150` as latest while the local lock used `1.0.149`; specs should pin the workspace to current stable during implementation.

JSON decoding:

- rejects invalid JSON with spans when available
- preserves exact numeric text into the CUE number type
- applies source byte and nesting limits

JSON encoding:

- takes defaults when the selected profile requires concrete values
- rejects incomplete values
- emits deterministic object key order based on CUE export order

## YAML

Use `noyalib` rather than deprecated `serde_yml`. `serde_yml` reports itself deprecated and forwards to `noyalib`; direct dependency on `noyalib` is clearer for security review.

YAML decoding:

- supports YAML streams
- maps each document to a separate decoded value
- preserves source item index
- applies nesting and collection limits

## TOML

Use the current `toml` crate. TOML values decode into CUE data, not schema. TOML output requires concrete struct values compatible with TOML's data model.

## Number Semantics

External numeric text must not be parsed through `f64` for semantic values. The decoder stores numeric text and converts through the selected arbitrary precision decimal implementation. Invalid, overflowing, or non-representable values become diagnostics.

## Source Mapping

Every decoder should return source positions for diagnostics when the underlying parser exposes enough information. If a format parser cannot provide exact spans, diagnostics include file-level source context and the decoded path.

## Security Limits

All decoders enforce:

- byte length
- nesting depth
- collection element count
- string byte length
- stream item count
- decompressed byte count when compression is added

## Tests

- Golden tests for JSON/YAML/TOML decode to CUE export.
- Validation tests unifying decoded data with CUE schemas.
- Stream tests for YAML and later JSONL.
- Fuzz tests for JSON/YAML/TOML byte inputs.
- Differential tests against upstream CUE encoding behavior for selected cases.

## AGENTS Binding

- Validate at decode boundary.
- Reject invalid external data; do not sanitize.
- Use `serde_json::Value` only inside decoder adapters, not as the semantic value model.
- Add explicit dependency audit notes when selecting YAML and decimal crates.
