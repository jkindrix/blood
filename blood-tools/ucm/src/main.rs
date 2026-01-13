//! Blood UCM Binary
//!
//! Run with: `blood-ucm [COMMAND]`

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use blood_ucm::{Codebase, DefRef, Name};

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

    /// Garbage collect unreferenced definitions
    Gc {
        /// Show what would be collected without actually deleting
        #[arg(long)]
        dry_run: bool,
    },

    /// Compare two definitions
    Diff {
        /// First hash (or name)
        query1: String,

        /// Second hash (or name)
        query2: String,
    },

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
        Commands::Gc { dry_run } => cmd_gc(&codebase_path, dry_run),
        Commands::Diff { query1, query2 } => cmd_diff(&codebase_path, &query1, &query2),
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

    if let Some(hash_str) = query.strip_prefix('#') {
        // Hash query - try prefix lookup first
        // Try to find by prefix (allows short hashes like #a7f2)
        let matches = codebase.find_by_hash_prefix(hash_str)?;

        match matches.len() {
            0 => {
                println!("Not found: {}", query);
            }
            1 => {
                let info = &matches[0];
                print_def_info(info);
            }
            n => {
                println!("Ambiguous hash prefix '{}' matches {} definitions:", hash_str, n);
                for info in &matches {
                    let names_str = if info.names.is_empty() {
                        "(unnamed)".to_string()
                    } else {
                        info.names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
                    };
                    println!("  {} ({}) - {}", info.hash, info.kind.as_str(), names_str);
                }
                println!("\nUse a longer prefix to disambiguate.");
            }
        }
    } else {
        // Name query
        match codebase.find(&DefRef::name(query))? {
            Some(info) => {
                print_def_info(&info);
            }
            None => {
                println!("Not found: {}", query);
            }
        }
    }
    Ok(())
}

fn print_def_info(info: &blood_ucm::DefInfo) {
    println!("Hash: {}", info.hash);
    println!("Kind: {}", info.kind.as_str());
    let names_str = if info.names.is_empty() {
        "(unnamed)".to_string()
    } else {
        info.names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
    };
    println!("Names: {}", names_str);
    println!("Dependencies: {}", info.dependencies.len());
    println!("Dependents: {}", info.dependents.len());
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

fn cmd_run(path: &PathBuf, expr: &str) -> Result<()> {
    let codebase = Codebase::open(path)?;

    // Check if expr is a name in the codebase
    let source = if let Some(info) = codebase.find(&DefRef::name(expr))? {
        info.source
    } else {
        // Treat as inline expression
        expr.to_string()
    };

    // Parse using bloodc parser
    use bloodc::Parser;
    let mut parser = Parser::new(&source);

    match parser.parse_program() {
        Ok(program) => {
            println!("Parsed successfully.");
            println!("Declarations: {}", program.declarations.len());
            // Type checking would require full module context with HIR lowering
            println!("Type checking: (requires full module context)");
            println!("Execution: (runtime integration pending)");
        }
        Err(errors) => {
            for error in errors {
                eprintln!("Error: {}", error.message);
            }
        }
    }
    Ok(())
}

fn cmd_test(path: &PathBuf, filter: Option<&str>) -> Result<()> {
    use blood_ucm::test_runner::{TestRunner, TestOutcome, TestSummary};

    let codebase = Codebase::open(path)?;
    let runner = TestRunner::new(&codebase);

    println!("Running tests...");
    let results = runner.run(filter)?;

    for result in &results {
        let status = match &result.outcome {
            TestOutcome::Passed { duration } => {
                format!("ok ({:.2}ms)", duration.as_secs_f64() * 1000.0)
            }
            TestOutcome::Failed { message, duration } => {
                format!("FAILED ({:.2}ms)\n    {}", duration.as_secs_f64() * 1000.0, message)
            }
            TestOutcome::Skipped { reason } => {
                format!("skipped: {}", reason)
            }
            TestOutcome::Error { message } => {
                format!("ERROR: {}", message)
            }
        };
        println!("  {} ... {}", result.name, status);
    }

    let summary = TestSummary::from_results(&results);
    println!();
    println!(
        "{} passed, {} failed, {} skipped, {} errors (total: {})",
        summary.passed,
        summary.failed,
        summary.skipped,
        summary.errors,
        summary.total
    );

    if !summary.all_passed() {
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_sync(path: &PathBuf, remote: &str, push: bool, pull: bool) -> Result<()> {
    use blood_ucm::sync::{export_codebase, import_codebase, ExportFormat};
    use std::path::Path;

    // For now, treat "remote" as a file path for file-based sync
    let remote_path = Path::new(remote);

    if push {
        println!("Exporting to {}...", remote);
        let codebase = Codebase::open(path)?;
        export_codebase(&codebase, remote_path, ExportFormat::Json)
            .context("Failed to export codebase")?;
        println!("Exported successfully.");
    }

    if pull {
        println!("Importing from {}...", remote);
        let mut codebase = Codebase::open(path)?;
        let count = import_codebase(&mut codebase, remote_path, ExportFormat::Json)
            .context("Failed to import codebase")?;
        println!("Imported {} definitions.", count);
    }

    if !push && !pull {
        println!("Specify --push or --pull (or both) to sync.");
    }

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

fn cmd_gc(path: &PathBuf, dry_run: bool) -> Result<()> {
    let codebase = Codebase::open(path)?;
    let garbage = codebase.find_garbage()?;

    if garbage.is_empty() {
        println!("No unreferenced definitions found.");
        return Ok(());
    }

    println!("Found {} unreferenced definition(s):", garbage.len());
    for hash in &garbage {
        println!("  {}", hash);
    }

    if dry_run {
        println!("\nDry run - no definitions were deleted.");
        println!("Run without --dry-run to actually delete.");
    } else {
        let count = codebase.garbage_collect()?;
        println!("\nDeleted {} definition(s).", count);
    }

    Ok(())
}

fn cmd_diff(path: &PathBuf, query1: &str, query2: &str) -> Result<()> {
    use blood_ucm::Difference;

    let codebase = Codebase::open(path)?;

    // Resolve both queries to hashes
    let hash1 = resolve_query(&codebase, query1)?;
    let hash2 = resolve_query(&codebase, query2)?;

    let result = codebase.diff(&hash1, &hash2)?;

    if result.is_identical() {
        println!("Definitions are identical.");
        return Ok(());
    }

    println!("Comparing {} vs {}", hash1, hash2);
    println!();

    for diff in &result.differences {
        match diff {
            Difference::KindChange { old, new } => {
                println!("Kind changed: {} -> {}", old.as_str(), new.as_str());
            }
            Difference::LineAdded { line_num, content } => {
                println!("\x1b[32m+ {:>4}: {}\x1b[0m", line_num, content);
            }
            Difference::LineRemoved { line_num, content } => {
                println!("\x1b[31m- {:>4}: {}\x1b[0m", line_num, content);
            }
            Difference::LineChanged { line_num, old, new } => {
                println!("\x1b[31m- {:>4}: {}\x1b[0m", line_num, old);
                println!("\x1b[32m+ {:>4}: {}\x1b[0m", line_num, new);
            }
        }
    }

    Ok(())
}

/// Resolves a query (name or hash) to a hash.
fn resolve_query(codebase: &Codebase, query: &str) -> Result<blood_ucm::Hash> {
    if let Some(hash_str) = query.strip_prefix('#') {
        // Hash query
        codebase
            .resolve_hash_prefix(hash_str)?
            .ok_or_else(|| anyhow::anyhow!("Hash not found: {}", query))
    } else {
        // Name query
        codebase
            .resolve(&Name::new(query))?
            .ok_or_else(|| anyhow::anyhow!("Name not found: {}", query))
    }
}

fn cmd_repl(path: &PathBuf) -> Result<()> {
    use std::io::{self, Write};

    println!("Blood UCM v{}", env!("CARGO_PKG_VERSION"));
    println!("Codebase: {}", path.display());
    println!("Type :help for help, :quit to exit");
    println!();

    let mut codebase = Codebase::open(path)?;

    loop {
        print!("blood> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            // EOF
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if let Some(rest) = input.strip_prefix(':') {
            // REPL command
            let parts: Vec<&str> = rest.split_whitespace().collect();
            match parts.first().copied() {
                Some("quit") | Some("q") | Some("exit") => {
                    println!("Goodbye!");
                    break;
                }
                Some("help") | Some("h") | Some("?") => {
                    println!("REPL Commands:");
                    println!("  :help, :h, :?     Show this help");
                    println!("  :quit, :q, :exit  Exit the REPL");
                    println!("  :list [prefix]    List definitions");
                    println!("  :find <name>      Find a definition");
                    println!("  :view <name>      View source of a definition");
                    println!("  :add <name>       Add a definition (enter source, then blank line)");
                    println!("  :stats            Show codebase statistics");
                    println!();
                    println!("Or type an expression to evaluate.");
                }
                Some("list") | Some("ls") => {
                    let prefix = parts.get(1).copied();
                    let names = codebase.list_names(prefix)?;
                    for (name, hash) in names {
                        println!("  {} -> {}", name, hash);
                    }
                }
                Some("find") => {
                    if let Some(query) = parts.get(1) {
                        match codebase.find(&DefRef::name(*query))? {
                            Some(info) => {
                                println!("Hash: {}", info.hash);
                                println!("Kind: {}", info.kind.as_str());
                            }
                            None => println!("Not found: {}", query),
                        }
                    } else {
                        println!("Usage: :find <name>");
                    }
                }
                Some("view") => {
                    if let Some(query) = parts.get(1) {
                        match codebase.find(&DefRef::name(*query))? {
                            Some(info) => println!("{}", info.source),
                            None => println!("Not found: {}", query),
                        }
                    } else {
                        println!("Usage: :view <name>");
                    }
                }
                Some("add") => {
                    if let Some(name) = parts.get(1) {
                        println!("Enter source (blank line to finish):");
                        let mut source = String::new();
                        loop {
                            let mut line = String::new();
                            io::stdin().read_line(&mut line)?;
                            if line.trim().is_empty() {
                                break;
                            }
                            source.push_str(&line);
                        }
                        if !source.is_empty() {
                            let hash = codebase.add_term(&source)?;
                            codebase.add_name(Name::new(*name), hash.clone())?;
                            println!("Added {} -> {}", name, hash);
                        }
                    } else {
                        println!("Usage: :add <name>");
                    }
                }
                Some("stats") => {
                    let stats = codebase.stats()?;
                    println!("Terms: {}, Types: {}, Effects: {}, Handlers: {}, Tests: {}",
                        stats.terms, stats.types, stats.effects, stats.handlers, stats.tests);
                }
                Some(cmd) => {
                    println!("Unknown command: :{}", cmd);
                    println!("Type :help for available commands.");
                }
                None => {}
            }
        } else {
            // Treat as expression to evaluate
            evaluate_expression(input, &codebase);
        }
    }

    Ok(())
}

/// Evaluate an expression in the REPL.
fn evaluate_expression(input: &str, codebase: &Codebase) {
    use bloodc::Parser;
    use bloodc::typeck::const_eval;

    // First, check if the input is a name in the codebase
    if let Ok(Some(info)) = codebase.find(&DefRef::name(input.trim())) {
        println!("= {}", info.source.trim());
        return;
    }

    // Try to parse as an expression
    let mut parser = Parser::new(input);
    let expr = parser.parse_expr();

    // Check for parse errors
    let errors = parser.take_errors();
    if !errors.is_empty() {
        for err in errors {
            eprintln!("Parse error: {}", err.message);
        }
        return;
    }

    // Try compile-time constant evaluation
    match const_eval::eval_const_expr(&expr) {
        Ok(result) => {
            match result {
                const_eval::ConstResult::Int(v) => println!("= {} : Int", v),
                const_eval::ConstResult::Uint(v) => println!("= {} : UInt", v),
                const_eval::ConstResult::Bool(v) => println!("= {} : Bool", v),
            }
        }
        Err(_) => {
            // Non-constant expression - provide helpful message
            println!("Parsed: {:?}", expr.kind);
            println!("(Runtime evaluation requires compilation to native code)");
            println!("Hint: Use 'blood run <file.blood>' to execute Blood programs");
        }
    }
}
