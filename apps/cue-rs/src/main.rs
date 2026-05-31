//! Command-line interface for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use clap::{Parser, Subcommand};

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
    /// Print the cue-rust version.
    Version,
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
        Command::Version => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", cue_rust::VERSION).context("failed to write version output")?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn scan_files(files: &[PathBuf]) -> Result<ExitCode> {
    let ctx = cue_rust::Context::new();
    let mut saw_error = false;

    for file in files {
        let bytes = tokio::fs::read(file)
            .await
            .with_context(|| format!("failed to read input file {}", file.display()))?;
        let name = file.to_string_lossy().into_owned();
        let result = ctx.scan_source_bytes(name, &bytes);
        for diagnostic in result.diagnostics().diagnostics() {
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
