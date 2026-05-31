//! Public SDK facade for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub use cue_rust_eval::ValidateOptions;
pub use cue_rust_loader::{LoadConfig, PackageSelector};
pub use cue_rust_source::{SourceError, SourceFile, SourceLimits};
pub use cue_rust_syntax::{
    AstFile, Decl, Expr, FieldDecl, ImportDecl, Label, LetDecl, PackageClause, ParseConfig,
    ParseMode, ParseResult, ParsedSource, ScanResult, Token, TokenKind,
};

/// Current SDK version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Top-level SDK context.
#[derive(Clone, Debug, Default)]
pub struct Context {
    parse_config: ParseConfig,
}

impl Context {
    /// Creates a context with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a context with an explicit parser configuration.
    #[must_use]
    pub fn with_parse_config(parse_config: ParseConfig) -> Self {
        Self { parse_config }
    }

    /// Parses a named source into a tolerant AST and diagnostics.
    #[must_use]
    pub fn parse_source(&self, name: impl Into<String>, content: impl Into<String>) -> ParseResult {
        let content = content.into();
        cue_rust_syntax::parse_bytes(name, content.as_bytes(), self.parse_config)
    }

    /// Parses raw source bytes into a tolerant AST and diagnostics.
    #[must_use]
    pub fn parse_source_bytes(&self, name: impl Into<String>, bytes: &[u8]) -> ParseResult {
        cue_rust_syntax::parse_bytes(name, bytes, self.parse_config)
    }

    /// Scans raw source bytes into syntax tokens and diagnostics.
    #[must_use]
    pub fn scan_source_bytes(&self, name: impl Into<String>, bytes: &[u8]) -> ScanResult {
        cue_rust_syntax::scan_bytes(name, bytes, self.parse_config)
    }
}
