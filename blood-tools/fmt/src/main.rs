//! Blood Formatter Binary
//!
//! Run with: `blood-fmt [OPTIONS] [FILES...]`

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use blood_fmt::{check_formatted_with_config, format_diff_with_config, format_source_with_config, Config};

#[derive(Parser)]
#[command(name = "blood-fmt")]
#[command(about = "Formatter for Blood source code")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Files to format (reads from stdin if none provided)
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Check if files are formatted without modifying them
    #[arg(short, long)]
    check: bool,

    /// Show diff instead of writing formatted output
    #[arg(short, long)]
    diff: bool,

    /// Format files in place (modifies files)
    #[arg(short = 'w', long)]
    write: bool,

    /// Maximum line width
    #[arg(long, default_value = "100")]
    max_width: usize,

    /// Indentation width (spaces)
    #[arg(long, default_value = "4")]
    indent_width: usize,

    /// Use tabs instead of spaces
    #[arg(long)]
    use_tabs: bool,

    /// Configuration file path
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the default configuration
    Config,
    /// Format stdin to stdout
    Stdin,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        "debug"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)))
        .with_writer(io::stderr)
        .init();

    match &cli.command {
        Some(Commands::Config) => {
            let config = Config::default();
            println!("{}", serde_json::to_string_pretty(&config)?);
            return Ok(());
        }
        Some(Commands::Stdin) => {
            return format_stdin(&cli);
        }
        None => {}
    }

    // Build configuration
    let config = build_config(&cli)?;

    // If no files provided, read from stdin
    if cli.files.is_empty() {
        return format_stdin(&cli);
    }

    let mut had_errors = false;
    let mut files_checked = 0;
    let mut files_changed = 0;

    for path in &cli.files {
        match process_file(path, &config, &cli) {
            Ok(changed) => {
                files_checked += 1;
                if changed {
                    files_changed += 1;
                }
            }
            Err(e) => {
                error!("Error processing {}: {}", path.display(), e);
                had_errors = true;
            }
        }
    }

    if cli.check {
        info!(
            "Checked {} files, {} would be reformatted",
            files_checked, files_changed
        );
        if files_changed > 0 {
            std::process::exit(1);
        }
    } else if cli.write {
        info!(
            "Formatted {} files, {} changed",
            files_checked, files_changed
        );
    }

    if had_errors {
        std::process::exit(1);
    }

    Ok(())
}

fn build_config(cli: &Cli) -> Result<Config> {
    let mut config = if let Some(config_path) = &cli.config {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?
    } else {
        Config::default()
    };

    // Override with CLI options
    config.max_width = cli.max_width;
    config.indent_width = cli.indent_width;
    config.use_tabs = cli.use_tabs;

    Ok(config)
}

fn format_stdin(cli: &Cli) -> Result<()> {
    let config = build_config(cli)?;

    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .context("Failed to read from stdin")?;

    if cli.check {
        let is_formatted = check_formatted_with_config(&source, &config)?;
        if !is_formatted {
            println!("stdin would be reformatted");
            std::process::exit(1);
        }
        return Ok(());
    }

    if cli.diff {
        if let Some(diff) = format_diff_with_config(&source, &config)? {
            println!("{}", diff);
        }
        return Ok(());
    }

    let formatted = format_source_with_config(&source, &config)?;
    print!("{}", formatted);

    Ok(())
}

fn process_file(path: &PathBuf, config: &Config, cli: &Cli) -> Result<bool> {
    debug!("Processing: {}", path.display());

    let source = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    if cli.check {
        let is_formatted = check_formatted_with_config(&source, config)?;
        if !is_formatted {
            println!("{}", path.display());
            return Ok(true);
        }
        return Ok(false);
    }

    if cli.diff {
        if let Some(diff) = format_diff_with_config(&source, config)? {
            println!("--- {}", path.display());
            println!("+++ {}", path.display());
            println!("{}", diff);
            return Ok(true);
        }
        return Ok(false);
    }

    let formatted = format_source_with_config(&source, config)?;

    if source == formatted {
        return Ok(false);
    }

    if cli.write {
        fs::write(path, &formatted)
            .with_context(|| format!("Failed to write file: {}", path.display()))?;
        info!("Formatted: {}", path.display());
    } else {
        print!("{}", formatted);
    }

    Ok(true)
}
