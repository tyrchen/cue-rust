//! Loader boundary for package arguments, files, overlays, and modules.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use camino::Utf8PathBuf;

/// Package selector used by loader configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PackageSelector {
    /// Use the package implied by the loaded files.
    #[default]
    Default,
    /// Require a named package.
    Named(String),
    /// Accept any package.
    Any,
    /// Do not load CUE package files.
    None,
}

/// Loader configuration for local package loading.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadConfig {
    current_dir: Option<Utf8PathBuf>,
    package: PackageSelector,
}

impl LoadConfig {
    /// Creates a loader configuration.
    #[must_use]
    pub fn new(current_dir: Option<Utf8PathBuf>, package: PackageSelector) -> Self {
        Self {
            current_dir,
            package,
        }
    }

    /// Returns the configured current directory override.
    #[must_use]
    pub fn current_dir(&self) -> Option<&Utf8PathBuf> {
        self.current_dir.as_ref()
    }

    /// Returns the package selector.
    #[must_use]
    pub fn package(&self) -> &PackageSelector {
        &self.package
    }
}
