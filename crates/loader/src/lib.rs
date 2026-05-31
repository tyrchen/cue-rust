//! Loader boundary for package arguments, files, overlays, and modules.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use camino::Utf8PathBuf;
use cue_rust_source::DiagnosticReport;
use cue_rust_syntax::AstFile;

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

/// Source/package boundary consumed by the compiler.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuildInstance {
    package_name: Option<String>,
    files: Vec<AstFile>,
    diagnostics: DiagnosticReport,
}

impl BuildInstance {
    /// Creates a build instance from parsed AST files.
    #[must_use]
    pub fn new(package_name: Option<String>, files: Vec<AstFile>) -> Self {
        Self {
            package_name,
            files,
            diagnostics: DiagnosticReport::new(),
        }
    }

    /// Returns the package name, if known.
    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        self.package_name.as_deref()
    }

    /// Returns parsed AST files.
    #[must_use]
    pub fn files(&self) -> &[AstFile] {
        &self.files
    }

    /// Returns load diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }
}
