//! Encoding adapters for external data formats.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Supported external data encodings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Encoding {
    /// JavaScript Object Notation.
    Json,
    /// YAML data streams.
    Yaml,
    /// TOML documents.
    Toml,
}
