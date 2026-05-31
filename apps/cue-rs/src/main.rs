//! Command-line interface for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use cue_rust::{
    CueError, DecodeOptions, EncodeError, EncodeOptions, Encoding, EvalError, Value, decode_bytes,
    encode_value,
};

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
        /// CUE files to evaluate.
        files: Vec<PathBuf>,
    },
    /// Export concrete CUE values as external data.
    Export {
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        out: OutputFormat,
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

    match cli.command {
        Command::Parse { files } => scan_files(&files).await,
        Command::Eval { files } => eval_files(&files).await,
        Command::Export { out, files } => export_files(out, &files).await,
        Command::Vet {
            files,
            data,
            data_format,
        } => vet_files(&files, &data, data_format).await,
        Command::Version => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", cue_rust::VERSION).context("failed to write version output")?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn eval_files(files: &[PathBuf]) -> Result<ExitCode> {
    write_encoded_files(files, OutputFormat::Cue, false).await
}

async fn export_files(out: OutputFormat, files: &[PathBuf]) -> Result<ExitCode> {
    write_encoded_files(files, out, true).await
}

async fn write_encoded_files(
    files: &[PathBuf],
    output_format: OutputFormat,
    concrete: bool,
) -> Result<ExitCode> {
    if files.is_empty() {
        return Err(anyhow!("at least one CUE file is required"));
    }

    let ctx = cue_rust::Context::new();
    let mut saw_error = false;
    let loaded = compile_cue_args(&ctx, files).await?;
    saw_error |= loaded.saw_diagnostics;
    for value in loaded.values {
        let mut options = EncodeOptions::default();
        options.encoding = output_format.encoding();
        options.concrete = concrete;
        match encode_value(&value, options) {
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
    }

    if saw_error {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

async fn vet_files(
    files: &[PathBuf],
    data_files: &[PathBuf],
    data_format: Option<OutputFormat>,
) -> Result<ExitCode> {
    if files.is_empty() {
        return Err(anyhow!("at least one CUE file is required"));
    }

    let ctx = cue_rust::Context::new();
    let loaded = compile_cue_args(&ctx, files).await?;
    if loaded.saw_diagnostics {
        return Ok(ExitCode::from(1));
    }
    let Some(schema) = unify_all(loaded.values)? else {
        return Ok(ExitCode::from(1));
    };

    let mut saw_error = false;
    if data_files.is_empty() {
        if let Err(error) = schema.validate(cue_rust::ValidateOptions::default()) {
            write_eval_error(error)?;
            saw_error = true;
        }
    } else {
        for data_file in data_files {
            let data = read_data_file(data_file, data_format).await?;
            match schema.unify(&data).and_then(|value| {
                value.validate(cue_rust::ValidateOptions::default())?;
                Ok(value)
            }) {
                Ok(_) => {}
                Err(error) => {
                    write_eval_error(error)?;
                    saw_error = true;
                }
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
    values: Vec<Value>,
    saw_diagnostics: bool,
}

async fn compile_cue_args(ctx: &cue_rust::Context, files: &[PathBuf]) -> Result<LoadedValues> {
    let args = paths_to_utf8(files)?;
    let config = load_config_for(files)?;
    let instances = ctx.load(config, &args).await?;
    let mut values = Vec::new();
    let mut saw_diagnostics = false;
    for instance in instances {
        match ctx.build_instance(&instance) {
            Ok(value) => values.push(value),
            Err(CueError::Diagnostics(report)) => {
                write_diagnostics(&report)?;
                saw_diagnostics = true;
            }
            Err(error) => return Err(anyhow!("{error}")),
        }
    }
    Ok(LoadedValues {
        values,
        saw_diagnostics,
    })
}

fn load_config_for(files: &[PathBuf]) -> Result<cue_rust::LoadConfig> {
    let Some(first) = files.first() else {
        return Ok(cue_rust::LoadConfig::default());
    };
    let current_dir = if first.is_absolute() {
        let base = first.parent().unwrap_or_else(|| Path::new("."));
        Some(
            Utf8PathBuf::from_path_buf(base.to_path_buf())
                .map_err(|path| anyhow!("path is not valid UTF-8: {}", path.display()))?,
        )
    } else {
        None
    };
    Ok(cue_rust::LoadConfig::builder()
        .current_dir(current_dir)
        .build())
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

fn unify_all(values: Vec<Value>) -> Result<Option<Value>> {
    let mut values = values.into_iter();
    let Some(mut unified) = values.next() else {
        return Ok(None);
    };
    for value in values {
        unified = unified.unify(&value).map_err(|error| anyhow!("{error}"))?;
    }
    Ok(Some(unified))
}

async fn read_data_file(file: &Path, format: Option<OutputFormat>) -> Result<Value> {
    let bytes = tokio::fs::read(file)
        .await
        .with_context(|| format!("failed to read data file {}", file.display()))?;
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

async fn scan_files(files: &[PathBuf]) -> Result<ExitCode> {
    let ctx = cue_rust::Context::new();
    let mut saw_error = false;

    for file in files {
        let bytes = tokio::fs::read(file)
            .await
            .with_context(|| format!("failed to read input file {}", file.display()))?;
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
