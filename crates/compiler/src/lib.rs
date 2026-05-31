//! Compiler boundary between parsed CUE syntax and semantic ADT values.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Compiler options shared by future lowering passes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompileOptions {
    /// Whether experimental syntax and semantic features are allowed.
    pub allow_experimental: bool,
}
