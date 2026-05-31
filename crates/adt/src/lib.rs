//! Semantic graph and runtime data structures for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Runtime configuration shared by semantic graph construction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeConfig {
    deterministic_export: bool,
}

impl RuntimeConfig {
    /// Creates a runtime configuration.
    #[must_use]
    pub fn new(deterministic_export: bool) -> Self {
        Self {
            deterministic_export,
        }
    }

    /// Returns whether exports should preserve deterministic field ordering.
    #[must_use]
    pub fn deterministic_export(&self) -> bool {
        self.deterministic_export
    }
}
