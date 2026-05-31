//! Public SDK facade for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub use cue_rust_eval::ValidateOptions;
pub use cue_rust_loader::{LoadConfig, PackageSelector};
pub use cue_rust_source::{SourceError, SourceFile, SourceLimits};
pub use cue_rust_syntax::{ParseConfig, ParseMode, ParsedSource};

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

    /// Validates and records a named source for syntax processing.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] when the source name or content violates the
    /// configured limits.
    pub fn parse_source(
        &self,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<ParsedSource, SourceError> {
        SourceFile::named(name, content, self.parse_config.limits()).map(ParsedSource::new)
    }
}
