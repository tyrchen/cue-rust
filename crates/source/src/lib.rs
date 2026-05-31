//! Source files, byte spans, and source-boundary validation for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::num::{NonZeroU32, NonZeroUsize};

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

        Ok(Self { name, content })
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
