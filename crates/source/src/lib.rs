//! Source files, byte spans, line indexes, and diagnostics for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    fmt,
    num::{NonZeroU32, NonZeroUsize},
};

use miette::Diagnostic as MietteDiagnosticTrait;
use thiserror::Error;

/// Default maximum source size used by early SDK and CLI entry points.
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 1_048_576;

/// A non-zero identifier for a source file inside a source database.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(NonZeroU32);

impl SourceId {
    /// Creates a source id from a one-based integer.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::ZeroSourceId`] when `value` is zero.
    pub fn new(value: u32) -> Result<Self, SourceError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(SourceError::ZeroSourceId)
    }

    /// Returns the underlying one-based id.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// Byte offset into a source file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteOffset(pub u32);

/// A half-open byte span inside a source file.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    source: SourceId,
    start: ByteOffset,
    end: ByteOffset,
}

impl Span {
    /// Creates a checked span.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidSpan`] when `end` is before `start`.
    pub fn new(source: SourceId, start: ByteOffset, end: ByteOffset) -> Result<Self, SourceError> {
        if end < start {
            return Err(SourceError::InvalidSpan { start, end });
        }
        Ok(Self { source, start, end })
    }

    /// Returns the span source id.
    #[must_use]
    pub fn source(self) -> SourceId {
        self.source
    }

    /// Returns the start byte offset.
    #[must_use]
    pub fn start(self) -> ByteOffset {
        self.start
    }

    /// Returns the end byte offset.
    #[must_use]
    pub fn end(self) -> ByteOffset {
        self.end
    }
}

/// One-based line and column position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineColumn {
    /// One-based line number.
    pub line: NonZeroUsize,
    /// One-based UTF-8 byte column.
    pub column: NonZeroUsize,
}

/// Line-start index for byte-to-line/column rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineIndex {
    line_starts: Vec<ByteOffset>,
}

impl LineIndex {
    /// Builds a line index from UTF-8 source text.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::OffsetTooLarge`] if the source length cannot fit
    /// into the project's current span representation.
    pub fn new(content: &str) -> Result<Self, SourceError> {
        let mut line_starts = vec![ByteOffset(0)];
        for (offset, byte) in content.bytes().enumerate() {
            if byte == b'\n' {
                let next = offset.checked_add(1).ok_or(SourceError::OffsetTooLarge)?;
                line_starts.push(ByteOffset(
                    u32::try_from(next).map_err(|_| SourceError::OffsetTooLarge)?,
                ));
            }
        }
        Ok(Self { line_starts })
    }

    /// Converts a byte offset to one-based line and byte-column coordinates.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidLineIndex`] if the offset is inconsistent
    /// with this index.
    pub fn line_column(&self, offset: ByteOffset) -> Result<LineColumn, SourceError> {
        let insertion = self
            .line_starts
            .binary_search(&offset)
            .unwrap_or_else(|index| index);
        let line_index = insertion.saturating_sub(1);
        let line_start = self
            .line_starts
            .get(line_index)
            .copied()
            .ok_or(SourceError::InvalidLineIndex)?;
        let line =
            NonZeroUsize::new(line_index.saturating_add(1)).ok_or(SourceError::InvalidLineIndex)?;
        let delta = offset
            .0
            .checked_sub(line_start.0)
            .ok_or(SourceError::InvalidLineIndex)?;
        let column_value = usize::try_from(delta)
            .map_err(|_| SourceError::OffsetTooLarge)?
            .saturating_add(1);
        let column = NonZeroUsize::new(column_value).ok_or(SourceError::InvalidLineIndex)?;
        Ok(LineColumn { line, column })
    }
}

/// Validated source limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLimits {
    max_file_bytes: NonZeroUsize,
}

impl SourceLimits {
    /// Creates source limits with an explicit maximum byte count.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::ZeroLimit`] when `max_file_bytes` is zero.
    pub fn new(max_file_bytes: usize) -> Result<Self, SourceError> {
        NonZeroUsize::new(max_file_bytes)
            .map(|max_file_bytes| Self { max_file_bytes })
            .ok_or(SourceError::ZeroLimit)
    }

    /// Returns the maximum accepted source size in bytes.
    #[must_use]
    pub fn max_file_bytes(self) -> usize {
        self.max_file_bytes.get()
    }
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: NonZeroUsize::new(DEFAULT_MAX_SOURCE_BYTES)
                .unwrap_or(NonZeroUsize::MIN),
        }
    }
}

/// A validated named source buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFile {
    name: String,
    content: String,
    line_index: LineIndex,
}

impl SourceFile {
    /// Creates a named source after validating name and byte limits.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] when the name is empty, contains NUL, or the
    /// content exceeds the configured byte limit.
    pub fn named(
        name: impl Into<String>,
        content: impl Into<String>,
        limits: SourceLimits,
    ) -> Result<Self, SourceError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SourceError::EmptySourceName);
        }
        if name.as_bytes().contains(&0) {
            return Err(SourceError::SourceNameContainsNul);
        }

        let content = content.into();
        let bytes = content.len();
        if bytes > limits.max_file_bytes() {
            return Err(SourceError::SourceTooLarge {
                actual: bytes,
                limit: limits.max_file_bytes(),
            });
        }

        let line_index = LineIndex::new(&content)?;
        Ok(Self {
            name,
            content,
            line_index,
        })
    }

    /// Creates a named source from raw bytes after UTF-8 validation.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] when the bytes are invalid UTF-8 or violate
    /// source boundary rules.
    pub fn named_bytes(
        name: impl Into<String>,
        bytes: &[u8],
        limits: SourceLimits,
    ) -> Result<Self, SourceError> {
        let content =
            String::from_utf8(bytes.to_vec()).map_err(|error| SourceError::InvalidUtf8 {
                valid_up_to: error.utf8_error().valid_up_to(),
            })?;
        Self::named(name, content, limits)
    }

    /// Returns the display name of the source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the UTF-8 source content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Returns the source line index.
    #[must_use]
    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }
}

/// Severity for cue-rust diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Severity {
    /// Informational diagnostic.
    Info,
    /// Warning diagnostic.
    Warning,
    /// Error diagnostic.
    Error,
}

/// Structured diagnostic emitted by scanner, parser, compiler, or evaluator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    severity: Severity,
    code: &'static str,
    message: String,
    primary_span: Option<Span>,
}

impl Diagnostic {
    /// Creates a diagnostic.
    #[must_use]
    pub fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        primary_span: Option<Span>,
    ) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            primary_span,
        }
    }

    /// Returns the diagnostic severity.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the stable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the primary span, if known.
    #[must_use]
    pub fn primary_span(&self) -> Option<Span> {
        self.primary_span
    }

    /// Converts this diagnostic into a `miette` report.
    #[must_use]
    pub fn to_miette(&self) -> miette::Report {
        MietteReport::from(self).into()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// A collection of diagnostics with helper predicates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticReport {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    /// Creates an empty diagnostic report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Returns true when at least one error diagnostic exists.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// Returns diagnostics in emission order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// `miette` diagnostic adapter used by CLI rendering.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct MietteReport {
    code: &'static str,
    message: String,
}

impl From<&Diagnostic> for MietteReport {
    fn from(value: &Diagnostic) -> Self {
        Self {
            code: value.code,
            message: value.message.clone(),
        }
    }
}

impl MietteDiagnosticTrait for MietteReport {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(self.code))
    }
}

/// Errors produced while validating source data.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum SourceError {
    /// Source ids are one-based.
    #[error("source id must be non-zero")]
    ZeroSourceId,
    /// Source limits must be non-zero.
    #[error("source byte limit must be non-zero")]
    ZeroLimit,
    /// Byte offsets currently fit in `u32`.
    #[error("source offset is too large")]
    OffsetTooLarge,
    /// Line index lookup failed.
    #[error("invalid line index")]
    InvalidLineIndex,
    /// Source bytes must be valid UTF-8.
    #[error("source is not valid UTF-8 near byte {valid_up_to}")]
    InvalidUtf8 {
        /// Last valid byte offset reported by UTF-8 decoding.
        valid_up_to: usize,
    },
    /// Span end must not precede span start.
    #[error("invalid span {start:?}..{end:?}")]
    InvalidSpan {
        /// Start offset.
        start: ByteOffset,
        /// End offset.
        end: ByteOffset,
    },
    /// Source names must not be empty.
    #[error("source name must not be empty")]
    EmptySourceName,
    /// Source names reject NUL bytes.
    #[error("source name must not contain NUL bytes")]
    SourceNameContainsNul,
    /// Source content exceeded the configured limit.
    #[error("source is too large: {actual} bytes exceeds limit {limit} bytes")]
    SourceTooLarge {
        /// Observed byte length.
        actual: usize,
        /// Configured byte limit.
        limit: usize,
    },
}
