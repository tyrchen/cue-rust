//! Scanner, parser, AST, and syntax-service entry points.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use cue_rust_source::{SourceFile, SourceLimits};

/// Parser mode requested by the SDK or CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParseMode {
    /// Parse a complete CUE source file.
    #[default]
    File,
}

/// Parser configuration shared by syntax entry points.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParseConfig {
    mode: ParseMode,
    limits: SourceLimits,
}

impl ParseConfig {
    /// Creates parser configuration from a mode and source limits.
    #[must_use]
    pub fn new(mode: ParseMode, limits: SourceLimits) -> Self {
        Self { mode, limits }
    }

    /// Returns the parser mode.
    #[must_use]
    pub fn mode(self) -> ParseMode {
        self.mode
    }

    /// Returns the source limits.
    #[must_use]
    pub fn limits(self) -> SourceLimits {
        self.limits
    }
}

/// Minimal syntax parse result used by the Phase 1 SDK facade.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedSource {
    source: SourceFile,
}

impl ParsedSource {
    /// Creates a parsed-source shell around a validated source.
    #[must_use]
    pub fn new(source: SourceFile) -> Self {
        Self { source }
    }

    /// Returns the source backing this parsed result.
    #[must_use]
    pub fn source(&self) -> &SourceFile {
        &self.source
    }
}
