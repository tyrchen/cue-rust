//! Command-line interface for cue-rust.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    io::{self, Write},
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

/// Supported Phase 1 CLI commands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Print the cue-rust version.
    Version,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            let mut stderr = io::stderr().lock();
            let _ignored = writeln!(stderr, "error: {error:#}");
            ExitCode::from(3)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet)?;

    match cli.command {
        Command::Version => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", cue_rust::VERSION).context("failed to write version output")?;
            Ok(ExitCode::SUCCESS)
        }
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
