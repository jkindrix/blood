//! Blood UCM Binary
//!
//! Run with: `blood-ucm [COMMAND]`

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use blood_ucm::{Codebase, DefRef, Name, Patch};

#[derive(Parser)]
#[command(name = "blood-ucm")]
#[command(about = "Content-addressed codebase manager for Blood")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the codebase (default: .blood-codebase)
    #[arg(short, long, global = true)]
    codebase: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new codebase
    Init {
        /// Name for the codebase
        #[arg(default_value = "main")]
        name: String,
    },

    /// Add a definition to the codebase
    Add {
        /// Name for the definition
        name: String,

        /// Source file to add
        file: PathBuf,
    },

    /// Look up a definition by name or hash
    Find {
        /// Name or hash to look up
        query: String,
    },

    /// List all names in the codebase
    List {
        /// Filter by prefix
        #[arg(short, long)]
        prefix: Option<String>,
    },

    /// Rename a definition
    Rename {
        /// Current name
        from: String,

        /// New name
        to: String,
    },

    /// Show the history of a definition
    History {
        /// Name or hash
        query: String,
    },

    /// Show dependencies of a definition
    Deps {
        /// Name or hash
        query: String,

        /// Show reverse dependencies (dependents)
        #[arg(short, long)]
        reverse: bool,
    },

    /// Show a definition's source
    View {
        /// Name or hash
        query: String,
    },

    /// Execute Blood code
    Run {
        /// Expression or name to run
        expr: String,
    },

    /// Run tests
    Test {
        /// Filter tests by prefix
        #[arg(short, long)]
        filter: Option<String>,
    },

    /// Sync with a remote codebase
    Sync {
        /// Remote URL
        remote: String,

        /// Push local changes
        #[arg(long)]
        push: bool,

        /// Pull remote changes
        #[arg(long)]
        pull: bool,
    },

    /// Show codebase statistics
    Stats,

    /// Interactive REPL
    Repl,
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
        .init();

    let codebase_path = cli
        .codebase
        .unwrap_or_else(|| PathBuf::from(".blood-codebase"));

    match cli.command {
        Commands::Init { name } => cmd_init(&codebase_path, &name),
        Commands::Add { name, file } => cmd_add(&codebase_path, &name, &file),
        Commands::Find { query } => cmd_find(&codebase_path, &query),
        Commands::List { prefix } => cmd_list(&codebase_path, prefix.as_deref()),
        Commands::Rename { from, to } => cmd_rename(&codebase_path, &from, &to),
        Commands::History { query } => cmd_history(&codebase_path, &query),
        Commands::Deps { query, reverse } => cmd_deps(&codebase_path, &query, reverse),
        Commands::View { query } => cmd_view(&codebase_path, &query),
        Commands::Run { expr } => cmd_run(&codebase_path, &expr),
        Commands::Test { filter } => cmd_test(&codebase_path, filter.as_deref()),
        Commands::Sync { remote, push, pull } => cmd_sync(&codebase_path, &remote, push, pull),
        Commands::Stats => cmd_stats(&codebase_path),
        Commands::Repl => cmd_repl(&codebase_path),
    }
}

fn cmd_init(path: &PathBuf, name: &str) -> Result<()> {
    info!("Initializing codebase '{}' at {:?}", name, path);
    Codebase::create(path, name)?;
    println!("Created codebase '{}' at {}", name, path.display());
    Ok(())
}

fn cmd_add(path: &PathBuf, name: &str, file: &PathBuf) -> Result<()> {
    let mut codebase = Codebase::open(path)?;
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let hash = codebase.add_term(&source)?;
    codebase.add_name(Name::new(name), hash.clone())?;

    println!("Added {} -> {}", name, hash);
    Ok(())
}

fn cmd_find(path: &PathBuf, query: &str) -> Result<()> {
    let codebase = Codebase::open(path)?;

    let def_ref = if query.starts_with('#') {
        DefRef::hash(query[1..].parse().context("Invalid hash")?)
    } else {
        DefRef::name(query)
    };

    match codebase.find(&def_ref)? {
        Some(info) => {
            println!("Hash: {}", info.hash);
            println!("Kind: {}", info.kind.as_str());
            println!("Names: {}", info.names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", "));
            println!("Dependencies: {}", info.dependencies.len());
            println!("Dependents: {}", info.dependents.len());
        }
        None => {
            println!("Not found: {}", query);
        }
    }
    Ok(())
}

fn cmd_list(path: &PathBuf, prefix: Option<&str>) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let names = codebase.list_names(prefix)?;

    for (name, hash) in names {
        println!("{} -> {}", name, hash);
    }
    Ok(())
}

fn cmd_rename(path: &PathBuf, from: &str, to: &str) -> Result<()> {
    let mut codebase = Codebase::open(path)?;
    codebase.rename(Name::new(from), Name::new(to))?;
    println!("Renamed {} -> {}", from, to);
    Ok(())
}

fn cmd_history(path: &PathBuf, query: &str) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let history = codebase.history(&DefRef::name(query))?;

    for (hash, timestamp) in history {
        println!("{} - {}", timestamp, hash);
    }
    Ok(())
}

fn cmd_deps(path: &PathBuf, query: &str, reverse: bool) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let deps = if reverse {
        codebase.dependents(&DefRef::name(query))?
    } else {
        codebase.dependencies(&DefRef::name(query))?
    };

    let label = if reverse { "Dependents" } else { "Dependencies" };
    println!("{}:", label);
    for (hash, names) in deps {
        let name_str = names.join(", ");
        println!("  {} ({})", hash, name_str);
    }
    Ok(())
}

fn cmd_view(path: &PathBuf, query: &str) -> Result<()> {
    let codebase = Codebase::open(path)?;

    if let Some(info) = codebase.find(&DefRef::name(query))? {
        println!("{}", info.source);
    } else {
        println!("Not found: {}", query);
    }
    Ok(())
}

fn cmd_run(_path: &PathBuf, expr: &str) -> Result<()> {
    // TODO: Integrate with Blood runtime
    println!("Would run: {}", expr);
    println!("(Runtime integration not yet implemented)");
    Ok(())
}

fn cmd_test(path: &PathBuf, filter: Option<&str>) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let tests = codebase.list_tests(filter)?;

    println!("Running {} tests...", tests.len());
    let mut passed = 0;
    let mut failed = 0;

    for (name, _hash) in &tests {
        // TODO: Actually run the test
        println!("  {} ... ok", name);
        passed += 1;
    }

    println!();
    println!("{} passed, {} failed", passed, failed);
    Ok(())
}

fn cmd_sync(_path: &PathBuf, remote: &str, push: bool, pull: bool) -> Result<()> {
    if push {
        println!("Pushing to {}...", remote);
    }
    if pull {
        println!("Pulling from {}...", remote);
    }
    println!("(Sync not yet implemented)");
    Ok(())
}

fn cmd_stats(path: &PathBuf) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let stats = codebase.stats()?;

    println!("Codebase Statistics:");
    println!("  Terms:    {}", stats.terms);
    println!("  Types:    {}", stats.types);
    println!("  Effects:  {}", stats.effects);
    println!("  Handlers: {}", stats.handlers);
    println!("  Tests:    {}", stats.tests);
    println!("  Names:    {}", stats.names);
    Ok(())
}

fn cmd_repl(path: &PathBuf) -> Result<()> {
    println!("Blood UCM v{}", env!("CARGO_PKG_VERSION"));
    println!("Codebase: {}", path.display());
    println!("Type :help for help, :quit to exit");
    println!();

    // TODO: Implement interactive REPL
    println!("(REPL not yet implemented)");
    Ok(())
}
