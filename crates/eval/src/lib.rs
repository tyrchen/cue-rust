//! Evaluator, validation, and export profile types.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Validation options for CUE values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ValidateOptions {
    /// Require concrete values.
    pub concrete: bool,
    /// Report all validation errors instead of stopping at the first one.
    pub all_errors: bool,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        Self {
            concrete: true,
            all_errors: false,
        }
    }
}
