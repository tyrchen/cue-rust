//! Command-line interface for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use cue_rust::{
    CueError, DecodeOptions, EncodeError, EncodeOptions, Encoding, EvalError, ExportOptions, Value,
    decode_bytes, encode_value,
};
use tokio::io::AsyncReadExt;

/// cue-rust command-line arguments.
#[derive(Debug, Parser)]
#[command(name = "cue-rs", version, about = "Rust-native CUE CLI")]
struct Cli {
    /// Increase diagnostic verbosity.
    #[arg(long, global = true)]
    verbose: bool,
    /// Suppress non-data output.
    #[arg(long, global = true)]
    quiet: bool,
    /// Maximum accepted CUE source size in bytes.
    #[arg(long, global = true)]
    source_limit: Option<usize>,
    /// Module root used to resolve package arguments.
    #[arg(long, global = true)]
    module_root: Option<PathBuf>,
    /// Required CUE package name.
    #[arg(long, global = true)]
    package: Option<String>,
    /// Inject a top-level tag value. Bare names inject true; name=value infers bool, null, number,
    /// or string.
    #[arg(short = 't', long = "inject", global = true)]
    inject: Vec<String>,
    /// Command to run.
    #[command(subcommand)]
    command: Command,
}

/// Supported CLI commands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Scan CUE files and print source-level diagnostics.
    Parse {
        /// Files to scan.
        files: Vec<PathBuf>,
    },
    /// Evaluate CUE files and print CUE-like value syntax.
    Eval {
        /// Dot-separated expression path to evaluate.
        #[arg(short = 'e', long = "expr")]
        expressions: Vec<String>,
        /// Include definition fields such as #Schema.
        #[arg(long)]
        show_definitions: bool,
        /// Include hidden fields such as _scratch.
        #[arg(long)]
        show_hidden: bool,
        /// Include optional field constraints.
        #[arg(long)]
        show_optional: bool,
        /// CUE files to evaluate.
        files: Vec<PathBuf>,
    },
    /// Export concrete CUE values as external data.
    Export {
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        out: OutputFormat,
        /// Dot-separated expression path to export.
        #[arg(short = 'e', long = "expr")]
        expressions: Vec<String>,
        /// Include definition fields such as #Schema.
        #[arg(long)]
        show_definitions: bool,
        /// Include hidden fields such as _scratch.
        #[arg(long)]
        show_hidden: bool,
        /// Include optional field constraints.
        #[arg(long)]
        show_optional: bool,
        /// CUE files to export.
        files: Vec<PathBuf>,
    },
    /// Validate CUE values, optionally against external data files.
    Vet {
        /// CUE schema/value files.
        files: Vec<PathBuf>,
        /// External data files to validate against the first CUE file.
        #[arg(long)]
        data: Vec<PathBuf>,
        /// External data format. Defaults to file extension inference.
        #[arg(long, value_enum)]
        data_format: Option<OutputFormat>,
    },
    /// Print the cue-rust version.
    Version,
}

/// CLI output/data format flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    /// CUE-like value syntax.
    Cue,
    /// JSON.
    Json,
    /// YAML.
    Yaml,
    /// TOML.
    Toml,
}

impl OutputFormat {
    fn encoding(self) -> Encoding {
        match self {
            Self::Cue => Encoding::Cue,
            Self::Json => Encoding::Json,
            Self::Yaml => Encoding::Yaml,
            Self::Toml => Encoding::Toml,
        }
    }
}

fn main() -> ExitCode {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")
        .and_then(|runtime| runtime.block_on(run()));

    match result {
        Ok(code) => code,
        Err(error) => {
            let mut stderr = io::stderr().lock();
            let _ignored = writeln!(stderr, "error: {error:#}");
            ExitCode::from(3)
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet)?;
    let load_options = load_options_from_cli(&cli).await?;

    match cli.command {
        Command::Parse { files } => scan_files(&files, &load_options).await,
        Command::Eval {
            expressions,
            show_definitions,
            show_hidden,
            show_optional,
            files,
        } => {
            let export_options = export_options(show_definitions, show_hidden, show_optional);
            eval_files(&files, &expressions, export_options, &load_options).await
        }
        Command::Export {
            out,
            expressions,
            show_definitions,
            show_hidden,
            show_optional,
            files,
        } => {
            let export_options = export_options(show_definitions, show_hidden, show_optional);
            export_files(out, &files, &expressions, export_options, &load_options).await
        }
        Command::Vet {
            files,
            data,
            data_format,
        } => vet_files(&files, &data, data_format, &load_options).await,
        Command::Version => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", cue_rust::VERSION).context("failed to write version output")?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CliLoadOptions {
    source_limits: cue_rust::SourceLimits,
    module_root: Option<Utf8PathBuf>,
    package: Option<String>,
    inject: BTreeMap<String, String>,
}

async fn load_options_from_cli(cli: &Cli) -> Result<CliLoadOptions> {
    let source_limits = cli.source_limit.map_or_else(
        || Ok(cue_rust::SourceLimits::default()),
        cue_rust::SourceLimits::new,
    )?;
    let module_root = if let Some(path) = cli.module_root.as_deref() {
        Some(canonical_utf8_path(path).await?)
    } else {
        None
    };
    let inject = parse_injections(&cli.inject)?;
    Ok(CliLoadOptions {
        source_limits,
        module_root,
        package: cli.package.clone(),
        inject,
    })
}

fn parse_injections(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut tags = BTreeMap::new();
    for value in values {
        let (name, raw_value) = value
            .split_once('=')
            .map_or((value.as_str(), "true"), |(name, raw)| (name, raw));
        if name.is_empty() {
            return Err(anyhow!("injected tag name must not be empty"));
        }
        tags.insert(name.to_owned(), injected_value(raw_value)?);
    }
    Ok(tags)
}

fn injected_value(value: &str) -> Result<String> {
    if matches!(value, "true" | "false" | "null")
        || is_json_number_literal(value)
        || is_quoted_cue_literal(value)
    {
        return Ok(value.to_owned());
    }
    serde_json::to_string(value).context("failed to encode injected string value")
}

fn is_quoted_cue_literal(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(
        (bytes.first(), bytes.last()),
        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\''))
    ) && bytes.len() >= 2
}

fn is_json_number_literal(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Number>(value).is_ok()
}

fn export_options(
    include_definitions: bool,
    include_hidden: bool,
    include_optional: bool,
) -> ExportOptions {
    let mut options = ExportOptions::default();
    options.include_definitions = include_definitions;
    options.include_hidden = include_hidden;
    options.include_optional = include_optional;
    options
}

async fn eval_files(
    files: &[PathBuf],
    expressions: &[String],
    export_options: ExportOptions,
    load_options: &CliLoadOptions,
) -> Result<ExitCode> {
    write_encoded_files(
        files,
        expressions,
        OutputFormat::Cue,
        false,
        export_options,
        load_options,
    )
    .await
}

async fn export_files(
    out: OutputFormat,
    files: &[PathBuf],
    expressions: &[String],
    export_options: ExportOptions,
    load_options: &CliLoadOptions,
) -> Result<ExitCode> {
    write_encoded_files(files, expressions, out, true, export_options, load_options).await
}

async fn write_encoded_files(
    files: &[PathBuf],
    expressions: &[String],
    output_format: OutputFormat,
    concrete: bool,
    export_options: ExportOptions,
    load_options: &CliLoadOptions,
) -> Result<ExitCode> {
    if files.is_empty() {
        return Err(anyhow!("at least one CUE file is required"));
    }

    let ctx = context_for(load_options);
    let mut saw_error = false;
    let loaded = compile_cue_args(&ctx, files, load_options).await?;
    saw_error |= loaded.saw_diagnostics;
    for loaded_value in loaded.values {
        if expressions.is_empty() {
            saw_error |= write_value(&loaded_value.value, output_format, concrete, export_options)?;
            continue;
        }
        for expression in expressions {
            let selected = select_expression(&ctx, &loaded_value, expression)?;
            saw_error |= write_value(&selected, output_format, concrete, export_options)?;
        }
    }
    for data_file in loaded.data_files {
        let data = read_loaded_data_file(&data_file, load_options.source_limits).await?;
        if expressions.is_empty() {
            saw_error |= write_value(&data, output_format, concrete, export_options)?;
            continue;
        }
        for expression in expressions {
            let selected = select_value_path(&data, expression)?;
            saw_error |= write_value(&selected, output_format, concrete, export_options)?;
        }
    }

    if saw_error {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn write_value(
    value: &Value,
    output_format: OutputFormat,
    concrete: bool,
    export_options: ExportOptions,
) -> Result<bool> {
    let mut saw_error = false;
    let mut options = EncodeOptions::default();
    options.encoding = output_format.encoding();
    options.concrete = concrete;
    options.export_options = export_options;
    match encode_value(value, options) {
        Ok(output) => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{output}").context("failed to write encoded value")?;
        }
        Err(error) => {
            if write_encode_error(error)? {
                saw_error = true;
            }
        }
    }
    Ok(saw_error)
}

fn select_expression(
    ctx: &cue_rust::Context,
    loaded_value: &LoadedValue,
    expression: &str,
) -> Result<Value> {
    if expression.trim().is_empty() {
        return Err(anyhow!("expression path must not be empty"));
    }
    if let Ok(path) = parse_expression_path(expression)
        && let Ok(selected) = loaded_value.value.lookup_path(&path)
    {
        return Ok(selected);
    }
    ctx.compile_instance_expression(&loaded_value.instance, expression)
        .map_err(|error| anyhow!("failed to select expression `{expression}`: {error}"))
}

fn select_value_path(value: &Value, expression: &str) -> Result<Value> {
    let path = parse_expression_path(expression)?;
    value
        .lookup_path(&path)
        .map_err(|error| anyhow!("failed to select expression `{expression}`: {error}"))
}

fn parse_expression_path(expression: &str) -> Result<Vec<&str>> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("expression path must not be empty"));
    }
    let mut path = Vec::new();
    for segment in trimmed.split('.') {
        if segment.is_empty() {
            return Err(anyhow!(
                "expression path `{expression}` contains an empty segment"
            ));
        }
        path.push(segment);
    }
    Ok(path)
}

async fn vet_files(
    files: &[PathBuf],
    data_files: &[PathBuf],
    data_format: Option<OutputFormat>,
    load_options: &CliLoadOptions,
) -> Result<ExitCode> {
    if files.is_empty() {
        return Err(anyhow!("at least one CUE file is required"));
    }

    let ctx = context_for(load_options);
    let loaded = compile_cue_args(&ctx, files, load_options).await?;
    if loaded.saw_diagnostics {
        return Ok(ExitCode::from(1));
    }
    let LoadedValues {
        values,
        data_files: positional_data_files,
        ..
    } = loaded;
    let Some(schema) = unify_all(values.into_iter().map(|loaded| loaded.value))? else {
        return Ok(ExitCode::from(1));
    };

    let mut saw_error = false;
    if data_files.is_empty() && positional_data_files.is_empty() {
        if let Err(error) = schema.validate(cue_rust::ValidateOptions::default()) {
            write_eval_error(error)?;
            saw_error = true;
        }
    } else {
        for data_file in positional_data_files {
            let data = read_loaded_data_file(&data_file, load_options.source_limits).await?;
            if validate_data(&schema, &data)? {
                saw_error = true;
            }
        }
        for data_file in data_files {
            let data = read_cli_data_file(data_file, data_format, load_options).await?;
            if validate_data(&schema, &data)? {
                saw_error = true;
            }
        }
    }

    if saw_error {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

#[derive(Debug)]
struct LoadedValues {
    values: Vec<LoadedValue>,
    data_files: Vec<LoadedDataFile>,
    saw_diagnostics: bool,
}

#[derive(Debug)]
struct LoadedValue {
    instance: cue_rust::BuildInstance,
    value: Value,
}

#[derive(Debug)]
struct LoadedDataFile {
    encoding: OutputFormat,
    path: Utf8PathBuf,
}

async fn compile_cue_args(
    ctx: &cue_rust::Context,
    files: &[PathBuf],
    load_options: &CliLoadOptions,
) -> Result<LoadedValues> {
    let args = paths_to_utf8(files)?;
    let stdin = if args.iter().any(|arg| arg.as_str() == "-") {
        Some(read_stdin_string(load_options.source_limits).await?)
    } else {
        None
    };
    let config = load_config_for(files, load_options, stdin)?;
    let instances = ctx.load(config, &args).await?;
    let mut values = Vec::new();
    let mut data_files = Vec::new();
    let mut saw_diagnostics = false;
    for instance in instances {
        for data_file in instance.data_files() {
            data_files.push(LoadedDataFile {
                encoding: output_format_for_encoding(&data_file.encoding)?,
                path: data_file.path.clone(),
            });
        }
        if instance.files().is_empty() && !instance.data_files().is_empty() {
            continue;
        }
        match ctx.build_instance(&instance) {
            Ok(value) => values.push(LoadedValue { instance, value }),
            Err(CueError::Diagnostics(report)) => {
                write_diagnostics(&report)?;
                saw_diagnostics = true;
            }
            Err(error) => return Err(anyhow!("{error}")),
        }
    }
    Ok(LoadedValues {
        values,
        data_files,
        saw_diagnostics,
    })
}

fn load_config_for(
    files: &[PathBuf],
    load_options: &CliLoadOptions,
    stdin: Option<String>,
) -> Result<cue_rust::LoadConfig> {
    let current_dir = current_dir_for_args(files)?;
    let package = load_options
        .package
        .as_ref()
        .map_or(cue_rust::PackageSelector::Default, |name| {
            cue_rust::PackageSelector::Named(name.clone())
        });
    Ok(cue_rust::LoadConfig::builder()
        .current_dir(current_dir)
        .module_root(load_options.module_root.clone())
        .package(package)
        .parse_config(parse_config_for(load_options))
        .source_limits(load_options.source_limits)
        .stdin(stdin)
        .tags(load_options.inject.clone())
        .build())
}

fn current_dir_for_args(files: &[PathBuf]) -> Result<Option<Utf8PathBuf>> {
    let candidate = files.iter().find_map(candidate_path_for_current_dir);
    if let Some(first) = candidate
        && first.is_absolute()
    {
        let base = first.parent().unwrap_or_else(|| Path::new("."));
        return path_buf_to_utf8(base).map(Some);
    }
    Ok(None)
}

fn candidate_path_for_current_dir(path: &PathBuf) -> Option<PathBuf> {
    if path == Path::new("-") {
        return None;
    }
    let text = path.to_str()?;
    if let Some((encoding, rest)) = text.split_once(':')
        && matches!(encoding, "json" | "yaml" | "toml")
    {
        return Some(PathBuf::from(rest));
    }
    Some(path.clone())
}

fn path_buf_to_utf8(path: &Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|path| anyhow!("path is not valid UTF-8: {}", path.display()))
}

async fn canonical_utf8_path(path: &Path) -> Result<Utf8PathBuf> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .with_context(|| format!("failed to canonicalize path {}", path.display()))?;
    path_buf_to_utf8(&canonical)
}

fn output_format_for_encoding(encoding: &str) -> Result<OutputFormat> {
    match encoding {
        "json" => Ok(OutputFormat::Json),
        "yaml" => Ok(OutputFormat::Yaml),
        "toml" => Ok(OutputFormat::Toml),
        _ => Err(anyhow!("unsupported data encoding `{encoding}`")),
    }
}

fn validate_data(schema: &Value, data: &Value) -> Result<bool> {
    match schema.unify(data).and_then(|value| {
        value.validate(cue_rust::ValidateOptions::default())?;
        Ok(value)
    }) {
        Ok(_) => Ok(false),
        Err(error) => {
            write_eval_error(error)?;
            Ok(true)
        }
    }
}

async fn read_loaded_data_file(
    data_file: &LoadedDataFile,
    limits: cue_rust::SourceLimits,
) -> Result<Value> {
    let path = data_file.path.as_std_path().to_path_buf();
    read_data_file(&path, Some(data_file.encoding), limits).await
}

async fn read_stdin_string(limits: cue_rust::SourceLimits) -> Result<String> {
    let bytes = read_stdin_bytes(limits).await?;
    String::from_utf8(bytes).context("stdin is not valid UTF-8")
}

async fn read_stdin_bytes(limits: cue_rust::SourceLimits) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        let limit = limits.max_file_bytes();
        let take_limit = u64::try_from(limit)
            .ok()
            .and_then(|limit| limit.checked_add(1))
            .ok_or_else(|| anyhow!("source limit is too large"))?;
        let mut input = Vec::new();
        io::stdin()
            .take(take_limit)
            .read_to_end(&mut input)
            .context("failed to read stdin")?;
        if input.len() > limit {
            return Err(anyhow!(
                "stdin source exceeds maximum size of {} bytes",
                limits.max_file_bytes()
            ));
        }
        Ok::<_, anyhow::Error>(input)
    })
    .await
    .context("stdin read task failed")?
}

fn parse_config_for(load_options: &CliLoadOptions) -> cue_rust::ParseConfig {
    cue_rust::ParseConfig::new(cue_rust::ParseMode::File, load_options.source_limits)
}

fn context_for(load_options: &CliLoadOptions) -> cue_rust::Context {
    cue_rust::Context::with_parse_config(parse_config_for(load_options))
}

async fn read_file_or_stdin(file: &Path, limits: cue_rust::SourceLimits) -> Result<Vec<u8>> {
    if file == Path::new("-") {
        return read_stdin_bytes(limits).await;
    }
    let limit = limits.max_file_bytes();
    let metadata = tokio::fs::metadata(file)
        .await
        .with_context(|| format!("failed to inspect input file {}", file.display()))?;
    let limit_u64 = u64::try_from(limit).unwrap_or(u64::MAX);
    if metadata.is_file() && metadata.len() > limit_u64 {
        return Err(anyhow!(
            "input file {} exceeds maximum size of {} bytes",
            file.display(),
            limit,
        ));
    }
    let read_limit = limit_u64.saturating_add(1);
    let mut input = Vec::new();
    tokio::fs::File::open(file)
        .await
        .with_context(|| format!("failed to open input file {}", file.display()))?
        .take(read_limit)
        .read_to_end(&mut input)
        .await
        .with_context(|| format!("failed to read input file {}", file.display()))?;
    if input.len() > limit {
        return Err(anyhow!(
            "input file {} exceeds maximum size of {} bytes",
            file.display(),
            limit,
        ));
    }
    Ok(input)
}

async fn scan_files(files: &[PathBuf], load_options: &CliLoadOptions) -> Result<ExitCode> {
    let ctx = context_for(load_options);
    let mut saw_error = false;

    for file in files {
        let bytes = read_file_or_stdin(file, load_options.source_limits).await?;
        let name = file.to_string_lossy().into_owned();
        let result = ctx.parse_source_bytes(name, &bytes);
        write_diagnostics(result.diagnostics())?;
        if !result.diagnostics().has_errors()
            && let Some(ast) = result.ast()
        {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", ast.to_debug_tree()).context("failed to write parse tree")?;
        }
        saw_error |= result.diagnostics().has_errors();
    }

    if saw_error {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn paths_to_utf8(files: &[PathBuf]) -> Result<Vec<Utf8PathBuf>> {
    files
        .iter()
        .map(|file| {
            Utf8PathBuf::from_path_buf(file.clone())
                .map_err(|path| anyhow!("path is not valid UTF-8: {}", path.display()))
        })
        .collect()
}

fn unify_all(values: impl IntoIterator<Item = Value>) -> Result<Option<Value>> {
    let mut values = values.into_iter();
    let Some(mut unified) = values.next() else {
        return Ok(None);
    };
    for value in values {
        unified = unified.unify(&value).map_err(|error| anyhow!("{error}"))?;
    }
    Ok(Some(unified))
}

async fn read_cli_data_file(
    file: &Path,
    format: Option<OutputFormat>,
    load_options: &CliLoadOptions,
) -> Result<Value> {
    let file = resolve_cli_data_path(file, load_options).await?;
    read_data_file(&file, format, load_options.source_limits).await
}

async fn resolve_cli_data_path(file: &Path, load_options: &CliLoadOptions) -> Result<PathBuf> {
    if file == Path::new("-") {
        return Ok(PathBuf::from("-"));
    }
    if let Some(root) = &load_options.module_root {
        reject_path_traversal(file)?;
        let path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            std::env::current_dir()
                .context("failed to discover current directory")?
                .join(file)
        };
        let canonical = tokio::fs::canonicalize(&path)
            .await
            .with_context(|| format!("failed to canonicalize data file {}", file.display()))?;
        if !canonical.starts_with(root.as_std_path()) {
            return Err(anyhow!(
                "data file {} escapes module root {}",
                canonical.display(),
                root
            ));
        }
        return Ok(canonical);
    }
    Ok(file.to_path_buf())
}

fn reject_path_traversal(path: &Path) -> Result<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(anyhow!("path traversal is not allowed: {}", path.display()));
    }
    Ok(())
}

async fn read_data_file(
    file: &Path,
    format: Option<OutputFormat>,
    limits: cue_rust::SourceLimits,
) -> Result<Value> {
    let bytes = read_file_or_stdin(file, limits).await?;
    let output_format = format.unwrap_or_else(|| infer_format(file));
    match output_format {
        OutputFormat::Cue => {
            let ctx = cue_rust::Context::new();
            match ctx.compile_source_bytes(file.to_string_lossy().into_owned(), &bytes) {
                Ok(value) => Ok(value),
                Err(CueError::Diagnostics(report)) => {
                    write_diagnostics(&report)?;
                    Err(anyhow!("data file {} has diagnostics", file.display()))
                }
                Err(error) => Err(anyhow!("{error}")),
            }
        }
        OutputFormat::Json | OutputFormat::Yaml | OutputFormat::Toml => {
            decode_bytes(output_format.encoding(), &bytes, DecodeOptions::default())
                .map_err(|error| anyhow!("{error}"))
        }
    }
}

fn infer_format(file: &Path) -> OutputFormat {
    match file.extension().and_then(|extension| extension.to_str()) {
        Some("cue") => OutputFormat::Cue,
        Some("yaml" | "yml") => OutputFormat::Yaml,
        Some("toml") => OutputFormat::Toml,
        _ => OutputFormat::Json,
    }
}

fn write_encode_error(error: EncodeError) -> Result<bool> {
    match error {
        EncodeError::Eval(error) => {
            write_eval_error(error)?;
            Ok(true)
        }
        error => Err(anyhow!("{error}")),
    }
}

fn write_eval_error(error: EvalError) -> Result<()> {
    match error {
        EvalError::Diagnostics(report) => write_diagnostics(&report),
        EvalError::Adt(error) => Err(anyhow!("{error}")),
    }
}

fn write_diagnostics(report: &cue_rust::DiagnosticReport) -> Result<()> {
    for diagnostic in report.diagnostics() {
        let mut stderr = io::stderr().lock();
        writeln!(
            stderr,
            "{:?}: {}: {}",
            diagnostic.severity(),
            diagnostic.code(),
            diagnostic,
        )
        .context("failed to write diagnostic")?;
    }
    Ok(())
}

fn init_tracing(verbose: bool, quiet: bool) -> Result<()> {
    let level = match (quiet, verbose) {
        (true, _) => "error",
        (false, true) => "debug",
        (false, false) => "warn",
    };

    tracing_subscriber::fmt()
        .with_env_filter(level)
        .with_writer(io::stderr)
        .try_init()
        .map_err(|error| anyhow!("failed to initialize tracing subscriber: {error}"))
}
