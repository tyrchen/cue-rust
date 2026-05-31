//! Loader boundary for package arguments, files, overlays, and modules.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use camino::Utf8PathBuf;
use cue_rust_source::{
    Diagnostic, DiagnosticReport, Severity, SourceError, SourceFile, SourceLimits,
};
use cue_rust_syntax::{AstFile, ParseConfig, parse_bytes};
use thiserror::Error;
use typed_builder::TypedBuilder;

const CUE_EXTENSION: &str = "cue";
const MODULE_FILE: &str = "cue.mod/module.cue";

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

/// In-memory overlay source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlaySource {
    /// Logical source name.
    pub name: Utf8PathBuf,
    /// UTF-8 source content.
    pub content: String,
}

/// Loader configuration for local package loading.
#[derive(Clone, Debug, Eq, PartialEq, TypedBuilder)]
#[non_exhaustive]
pub struct LoadConfig {
    /// Current directory used to resolve relative arguments.
    #[builder(default)]
    current_dir: Option<Utf8PathBuf>,
    /// Module root override.
    #[builder(default)]
    module_root: Option<Utf8PathBuf>,
    /// Package selection policy.
    #[builder(default)]
    package: PackageSelector,
    /// Parser configuration.
    #[builder(default)]
    parse_config: ParseConfig,
    /// Source size limits.
    #[builder(default)]
    source_limits: SourceLimits,
    /// Optional stdin source content.
    #[builder(default)]
    stdin: Option<String>,
    /// Overlay sources keyed by logical path.
    #[builder(default)]
    overlays: BTreeMap<Utf8PathBuf, String>,
    /// Tag values injected as top-level fields.
    #[builder(default)]
    tags: BTreeMap<String, String>,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl LoadConfig {
    /// Creates a loader configuration.
    #[must_use]
    pub fn new(current_dir: Option<Utf8PathBuf>, package: PackageSelector) -> Self {
        Self::builder()
            .current_dir(current_dir)
            .package(package)
            .build()
    }

    /// Returns the configured current directory override.
    #[must_use]
    pub fn current_dir(&self) -> Option<&Utf8PathBuf> {
        self.current_dir.as_ref()
    }

    /// Returns the configured module root override.
    #[must_use]
    pub fn module_root(&self) -> Option<&Utf8PathBuf> {
        self.module_root.as_ref()
    }

    /// Returns the package selector.
    #[must_use]
    pub fn package(&self) -> &PackageSelector {
        &self.package
    }
}

/// Build file snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildFile {
    /// Logical file name.
    pub name: Utf8PathBuf,
    /// Whether the file came from an overlay.
    pub overlay: bool,
}

/// External data file snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataFile {
    /// Data encoding qualifier.
    pub encoding: String,
    /// File path.
    pub path: Utf8PathBuf,
}

/// Source/package boundary consumed by the compiler.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuildInstance {
    package_name: Option<String>,
    import_path: Option<String>,
    root_dir: Option<Utf8PathBuf>,
    build_files: Vec<BuildFile>,
    files: Vec<AstFile>,
    data_files: Vec<DataFile>,
    direct_imports: Vec<String>,
    diagnostics: DiagnosticReport,
}

impl BuildInstance {
    /// Creates a build instance from parsed AST files.
    #[must_use]
    pub fn new(package_name: Option<String>, files: Vec<AstFile>) -> Self {
        let direct_imports = collect_direct_imports(&files);
        Self {
            package_name,
            files,
            direct_imports,
            ..Self::default()
        }
    }

    /// Returns the package name, if known.
    #[must_use]
    pub fn package_name(&self) -> Option<&str> {
        self.package_name.as_deref()
    }

    /// Returns the import path, if known.
    #[must_use]
    pub fn import_path(&self) -> Option<&str> {
        self.import_path.as_deref()
    }

    /// Returns the root directory, if known.
    #[must_use]
    pub fn root_dir(&self) -> Option<&Utf8PathBuf> {
        self.root_dir.as_ref()
    }

    /// Returns build files.
    #[must_use]
    pub fn build_files(&self) -> &[BuildFile] {
        &self.build_files
    }

    /// Returns parsed AST files.
    #[must_use]
    pub fn files(&self) -> &[AstFile] {
        &self.files
    }

    /// Returns data files discovered by package arguments.
    #[must_use]
    pub fn data_files(&self) -> &[DataFile] {
        &self.data_files
    }

    /// Returns direct import paths.
    #[must_use]
    pub fn direct_imports(&self) -> &[String] {
        &self.direct_imports
    }

    /// Returns load diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticReport {
        &self.diagnostics
    }
}

/// Loader errors for local filesystem and source validation.
#[derive(Debug, Error)]
pub enum LoadError {
    /// Current directory could not be discovered.
    #[error("failed to discover current directory")]
    CurrentDir,
    /// Filesystem IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Path is not valid UTF-8.
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    /// Path escaped the configured root.
    #[error("path escapes load root: {path}")]
    PathEscapesRoot {
        /// Escaping path.
        path: Utf8PathBuf,
    },
    /// Path traversal was rejected.
    #[error("path traversal is not allowed: {path}")]
    PathTraversal {
        /// Rejected path.
        path: Utf8PathBuf,
    },
    /// Symlink input was rejected.
    #[error("symlink input is not allowed: {path}")]
    Symlink {
        /// Rejected path.
        path: Utf8PathBuf,
    },
    /// Source validation failed.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// Package name failed validation.
    #[error("invalid package name: {name}")]
    InvalidPackageName {
        /// Invalid package name.
        name: String,
    },
    /// Tag name failed validation.
    #[error("invalid tag name: {name}")]
    InvalidTagName {
        /// Invalid tag name.
        name: String,
    },
}

/// Local package loader.
#[derive(Clone, Debug)]
pub struct Loader {
    config: LoadConfig,
}

impl Loader {
    /// Creates a loader.
    #[must_use]
    pub fn new(config: LoadConfig) -> Self {
        Self { config }
    }

    /// Loads local package arguments into build instances.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] for invalid paths, IO failures, or source limit violations.
    pub async fn load_args(&self, args: &[Utf8PathBuf]) -> Result<Vec<BuildInstance>, LoadError> {
        let current_dir = self.current_dir().await?;
        let allowed_root = self.module_root(&current_dir).await?;
        let mut cue_files = Vec::new();
        let mut data_files = Vec::new();

        for arg in args {
            if arg.as_str() == "-" {
                self.push_stdin(&mut cue_files)?;
                continue;
            }
            if let Some((encoding, path)) = data_arg(arg) {
                let path = self
                    .resolve_existing_path(&current_dir, &allowed_root, &path)
                    .await?;
                data_files.push(DataFile { encoding, path });
                continue;
            }
            self.collect_arg(arg, &current_dir, &allowed_root, &mut cue_files)
                .await?;
        }

        for (name, content) in &self.config.overlays {
            validate_overlay_path(name)?;
            cue_files.push(LoadedSource {
                name: name.clone(),
                content: content.clone().into_bytes(),
                overlay: true,
            });
        }
        self.push_tags(&mut cue_files)?;

        let mut instance = self.build_instance(cue_files, data_files, allowed_root);
        self.apply_package_selector(&mut instance)?;
        Ok(vec![instance])
    }

    async fn current_dir(&self) -> Result<Utf8PathBuf, LoadError> {
        let path = if let Some(current_dir) = &self.config.current_dir {
            current_dir.as_std_path().to_path_buf()
        } else {
            std::env::current_dir().map_err(|_| LoadError::CurrentDir)?
        };
        path_to_utf8(tokio::fs::canonicalize(path).await?)
    }

    async fn module_root(&self, current_dir: &Utf8PathBuf) -> Result<Utf8PathBuf, LoadError> {
        if let Some(root) = &self.config.module_root {
            return Ok(root.clone());
        }
        discover_module_root(current_dir).await
    }

    async fn collect_arg(
        &self,
        arg: &Utf8PathBuf,
        current_dir: &Utf8PathBuf,
        allowed_root: &Utf8PathBuf,
        cue_files: &mut Vec<LoadedSource>,
    ) -> Result<(), LoadError> {
        if arg.as_str().ends_with("/...") {
            let base = arg.as_str().trim_end_matches("/...");
            let base = Utf8PathBuf::from(if base.is_empty() { "." } else { base });
            let path = self
                .resolve_existing_path(current_dir, allowed_root, &base)
                .await?;
            self.collect_directory(&path, allowed_root, cue_files, true)
                .await?;
            return Ok(());
        }

        let path = self
            .resolve_existing_path(current_dir, allowed_root, arg)
            .await?;
        let metadata = tokio::fs::symlink_metadata(path.as_std_path()).await?;
        if metadata.file_type().is_symlink() {
            return Err(LoadError::Symlink { path });
        }
        if metadata.is_dir() {
            self.collect_directory(&path, allowed_root, cue_files, false)
                .await
        } else if is_cue_path(&path) {
            self.push_file(path, cue_files).await
        } else {
            Ok(())
        }
    }

    async fn collect_directory(
        &self,
        dir: &Utf8PathBuf,
        allowed_root: &Utf8PathBuf,
        cue_files: &mut Vec<LoadedSource>,
        recursive: bool,
    ) -> Result<(), LoadError> {
        let mut pending = vec![dir.clone()];
        while let Some(next_dir) = pending.pop() {
            let mut entries = tokio::fs::read_dir(next_dir.as_std_path()).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = path_to_utf8(entry.path())?;
                assert_under_root(&path, allowed_root)?;
                let metadata = tokio::fs::symlink_metadata(path.as_std_path()).await?;
                if metadata.file_type().is_symlink() {
                    return Err(LoadError::Symlink { path });
                }
                if metadata.is_dir() {
                    if recursive {
                        pending.push(path);
                    }
                } else if metadata.is_file() && is_cue_path(&path) {
                    self.push_file(path, cue_files).await?;
                }
            }
        }
        cue_files.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(())
    }

    async fn push_file(
        &self,
        path: Utf8PathBuf,
        cue_files: &mut Vec<LoadedSource>,
    ) -> Result<(), LoadError> {
        let bytes = tokio::fs::read(path.as_std_path()).await?;
        SourceFile::named_bytes(path.as_str(), &bytes, self.config.source_limits)?;
        cue_files.push(LoadedSource {
            name: path,
            content: bytes,
            overlay: false,
        });
        Ok(())
    }

    fn push_stdin(&self, cue_files: &mut Vec<LoadedSource>) -> Result<(), LoadError> {
        if let Some(stdin) = &self.config.stdin {
            SourceFile::named("-", stdin, self.config.source_limits)?;
            cue_files.push(LoadedSource {
                name: Utf8PathBuf::from("-"),
                content: stdin.as_bytes().to_vec(),
                overlay: true,
            });
        }
        Ok(())
    }

    fn push_tags(&self, cue_files: &mut Vec<LoadedSource>) -> Result<(), LoadError> {
        if self.config.tags.is_empty() {
            return Ok(());
        }
        let mut content = String::new();
        for (name, value) in &self.config.tags {
            validate_identifier(name)
                .map_err(|()| LoadError::InvalidTagName { name: name.clone() })?;
            content.push_str(name);
            content.push_str(": ");
            content.push_str(value);
            content.push('\n');
        }
        SourceFile::named("tags.cue", &content, self.config.source_limits)?;
        cue_files.push(LoadedSource {
            name: Utf8PathBuf::from("tags.cue"),
            content: content.into_bytes(),
            overlay: true,
        });
        Ok(())
    }

    fn build_instance(
        &self,
        cue_files: Vec<LoadedSource>,
        data_files: Vec<DataFile>,
        root_dir: Utf8PathBuf,
    ) -> BuildInstance {
        let mut ast_files = Vec::with_capacity(cue_files.len());
        let mut build_files = Vec::with_capacity(cue_files.len());
        let mut diagnostics = DiagnosticReport::new();

        for source in cue_files {
            let parsed = parse_bytes(
                source.name.as_str(),
                &source.content,
                self.config.parse_config,
            );
            diagnostics.extend(parsed.diagnostics().diagnostics().iter().cloned());
            if let Some(ast) = parsed.ast() {
                ast_files.push(ast.clone());
                build_files.push(BuildFile {
                    name: source.name,
                    overlay: source.overlay,
                });
            }
        }

        let package_name = package_name_for(&ast_files);
        let direct_imports = collect_direct_imports(&ast_files);
        BuildInstance {
            package_name,
            import_path: None,
            root_dir: Some(root_dir),
            build_files,
            files: ast_files,
            data_files,
            direct_imports,
            diagnostics,
        }
    }

    async fn resolve_existing_path(
        &self,
        current_dir: &Utf8PathBuf,
        allowed_root: &Utf8PathBuf,
        arg: &Utf8PathBuf,
    ) -> Result<Utf8PathBuf, LoadError> {
        validate_no_traversal(arg)?;
        let path = if arg.is_absolute() {
            arg.clone()
        } else {
            current_dir.join(arg)
        };
        let canonical = tokio::fs::canonicalize(path.as_std_path()).await?;
        let canonical = path_to_utf8(canonical)?;
        assert_under_root(&canonical, allowed_root)?;
        Ok(canonical)
    }

    fn apply_package_selector(&self, instance: &mut BuildInstance) -> Result<(), LoadError> {
        match &self.config.package {
            PackageSelector::Default | PackageSelector::Any => {}
            PackageSelector::None => {
                instance.files.clear();
                instance.build_files.clear();
                instance.package_name = None;
            }
            PackageSelector::Named(name) => {
                validate_identifier(name)
                    .map_err(|()| LoadError::InvalidPackageName { name: name.clone() })?;
                if instance
                    .package_name
                    .as_deref()
                    .is_some_and(|package| package != name)
                {
                    instance.diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        "cue.load.package_mismatch",
                        format!("package does not match requested package `{name}`"),
                        None,
                    ));
                }
                instance.package_name = Some(name.clone());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoadedSource {
    name: Utf8PathBuf,
    content: Vec<u8>,
    overlay: bool,
}

fn collect_direct_imports(files: &[AstFile]) -> Vec<String> {
    let mut imports = BTreeSet::new();
    for file in files {
        for import in &file.imports {
            imports.insert(import.path.clone());
        }
    }
    imports.into_iter().collect()
}

fn package_name_for(files: &[AstFile]) -> Option<String> {
    files
        .iter()
        .find_map(|file| file.package.as_ref().map(|package| package.name.clone()))
}

fn data_arg(arg: &Utf8PathBuf) -> Option<(String, Utf8PathBuf)> {
    let (encoding, path) = arg.as_str().split_once(':')?;
    if matches!(encoding, "json" | "yaml" | "toml") && !path.is_empty() {
        return Some((encoding.to_owned(), Utf8PathBuf::from(path)));
    }
    None
}

fn validate_overlay_path(path: &Utf8PathBuf) -> Result<(), LoadError> {
    validate_no_traversal(path)?;
    if path.as_str().as_bytes().contains(&0) {
        return Err(LoadError::PathTraversal { path: path.clone() });
    }
    Ok(())
}

fn validate_no_traversal(path: &Utf8PathBuf) -> Result<(), LoadError> {
    for component in path.components() {
        if component.as_str() == ".." {
            return Err(LoadError::PathTraversal { path: path.clone() });
        }
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), ()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(());
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(());
    }
    if chars.all(|char| char == '_' || char.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(())
    }
}

fn is_cue_path(path: &Utf8PathBuf) -> bool {
    path.extension()
        .is_some_and(|extension| extension == CUE_EXTENSION)
}

fn path_to_utf8(path: PathBuf) -> Result<Utf8PathBuf, LoadError> {
    Utf8PathBuf::from_path_buf(path).map_err(LoadError::NonUtf8Path)
}

fn assert_under_root(path: &Utf8PathBuf, root: &Utf8PathBuf) -> Result<(), LoadError> {
    if path.as_std_path().starts_with(root.as_std_path()) {
        Ok(())
    } else {
        Err(LoadError::PathEscapesRoot { path: path.clone() })
    }
}

async fn discover_module_root(current_dir: &Utf8PathBuf) -> Result<Utf8PathBuf, LoadError> {
    let mut cursor = current_dir.as_std_path();
    loop {
        let marker = cursor.join(MODULE_FILE);
        if tokio::fs::try_exists(&marker).await? {
            return path_to_utf8(cursor.to_path_buf());
        }
        let Some(parent) = cursor.parent() else {
            return Ok(current_dir.clone());
        };
        cursor = parent;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        time::{SystemTime, UNIX_EPOCH},
    };

    use camino::Utf8PathBuf;
    use tokio::fs;

    use super::{LoadConfig, LoadError, Loader, PackageSelector};

    async fn fixture_dir() -> Result<Utf8PathBuf, Box<dyn Error>> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("cue-rust-loader-{nanos}"));
        fs::create_dir_all(&path).await?;
        Utf8PathBuf::from_path_buf(path).map_err(|path| format!("non-UTF-8 path: {path:?}").into())
    }

    #[tokio::test]
    async fn test_should_load_local_directory() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        fs::write(dir.join("a.cue"), "package p\nx: 1\n").await?;
        fs::write(dir.join("ignored.json"), "{}\n").await?;
        let loader = Loader::new(LoadConfig::builder().current_dir(Some(dir.clone())).build());
        let instances = loader.load_args(&[Utf8PathBuf::from(".")]).await?;
        let instance = instances.first().ok_or("missing instance")?;
        assert_eq!(Some("p"), instance.package_name());
        assert_eq!(1, instance.files().len());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_path_traversal() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        let loader = Loader::new(LoadConfig::builder().current_dir(Some(dir)).build());
        let result = loader.load_args(&[Utf8PathBuf::from("../x.cue")]).await;
        assert!(matches!(result, Err(LoadError::PathTraversal { .. })));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_oversized_source() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        fs::write(dir.join("a.cue"), "x: 12345\n").await?;
        let limits = cue_rust_source::SourceLimits::new(4)?;
        let loader = Loader::new(
            LoadConfig::builder()
                .current_dir(Some(dir))
                .source_limits(limits)
                .build(),
        );
        let result = loader.load_args(&[Utf8PathBuf::from("a.cue")]).await;
        assert!(matches!(result, Err(LoadError::Source(_))));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_load_overlay_stdin_and_tags() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        let mut overlays = std::collections::BTreeMap::new();
        overlays.insert(Utf8PathBuf::from("overlay.cue"), "x: 1\n".to_owned());
        let mut tags = std::collections::BTreeMap::new();
        tags.insert("env".to_owned(), "\"dev\"".to_owned());
        let loader = Loader::new(
            LoadConfig::builder()
                .current_dir(Some(dir))
                .stdin(Some("y: 2\n".to_owned()))
                .overlays(overlays)
                .tags(tags)
                .build(),
        );
        let instances = loader.load_args(&[Utf8PathBuf::from("-")]).await?;
        let instance = instances.first().ok_or("missing instance")?;
        assert_eq!(3, instance.files().len());
        assert_eq!(3, instance.build_files().len());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_record_data_files() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        fs::write(dir.join("data.json"), "{}\n").await?;
        let loader = Loader::new(LoadConfig::builder().current_dir(Some(dir)).build());
        let instances = loader
            .load_args(&[Utf8PathBuf::from("json:data.json")])
            .await?;
        let instance = instances.first().ok_or("missing instance")?;
        assert_eq!(1, instance.data_files().len());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_honor_named_package_selector() -> Result<(), Box<dyn Error>> {
        let dir = fixture_dir().await?;
        fs::write(dir.join("a.cue"), "package p\nx: 1\n").await?;
        let loader = Loader::new(
            LoadConfig::builder()
                .current_dir(Some(dir))
                .package(PackageSelector::Named("q".to_owned()))
                .build(),
        );
        let instances = loader.load_args(&[Utf8PathBuf::from("a.cue")]).await?;
        let instance = instances.first().ok_or("missing instance")?;
        assert!(instance.diagnostics().has_errors());
        Ok(())
    }
}
