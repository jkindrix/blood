//! Blood Compiler CLI
//!
//! The main entry point for the Blood compiler.
//!
//! # Usage
//!
//! ```text
//! blood [OPTIONS] <COMMAND>
//!
//! Commands:
//!   new    Create a new Blood project
//!   init   Initialize Blood project in current directory
//!   lex    Tokenize source and display token stream
//!   parse  Parse source and display AST
//!   check  Type-check source file or project
//!   build  Compile to executable
//!   run    Compile and run source file
//!   test   Run tests in source file or project
//!
//! Options:
//!   -v, --verbose  Increase verbosity (can be repeated)
//!   -q, --quiet    Suppress non-error output
//!   --color <WHEN> Control color output [default: auto] [possible values: auto, always, never]
//!   -h, --help     Print help information
//!   -V, --version  Print version information
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::time::Instant;

use bloodc::attr::{AttrHelper, TestInfo, TestRegistry};
use bloodc::diagnostics::DiagnosticEmitter;
use bloodc::{Lexer, TokenKind};
use bloodc::codegen;
use bloodc::expand;
use bloodc::macro_expand;
use bloodc::mir;
use bloodc::project::{self, Manifest};
use bloodc::content::{ContentHash, BuildCache, hash_hir_item, extract_dependencies, VFT, VFTEntry};
use bloodc::content::hash::ContentHasher;
use bloodc::content::namespace::{NameRegistry, NameBinding, BindingKind};
use bloodc::content::distributed_cache::{DistributedCache, FetchResult};

/// The Blood Programming Language Compiler
///
/// Blood is a systems programming language designed for safety-critical domains.
/// It combines content-addressed code, generational memory safety, mutable value
/// semantics, algebraic effects, and multiple dispatch.
#[derive(Parser)]
#[command(name = "blood")]
#[command(author = "Blood Language Team")]
#[command(version)]
#[command(about = "The Blood programming language compiler", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Increase verbosity (can be repeated: -v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress non-error output
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Control when to use colored output
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    color: ColorChoice,

    /// Print per-phase compilation timing
    #[arg(long, global = true)]
    timings: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Blood project
    ///
    /// Creates a new directory with Blood.toml and a basic src/main.blood file.
    New(NewArgs),

    /// Initialize Blood project in current directory
    ///
    /// Creates a Blood.toml file in the current directory.
    Init(InitArgs),

    /// Tokenize source file and display token stream
    ///
    /// Runs the lexer on the input file and prints each token with its
    /// position, kind, and text (for tokens that carry text).
    Lex(FileArgs),

    /// Parse source file and display AST
    ///
    /// Parses the input file and prints the complete Abstract Syntax Tree
    /// in Rust debug format. Useful for debugging parser issues.
    Parse(FileArgs),

    /// Type-check source file or project
    ///
    /// Performs parsing and type checking. If no file is specified and
    /// a Blood.toml exists, type-checks the entire project.
    Check(BuildArgs),

    /// Compile source file or project to executable
    ///
    /// Compiles to a native executable using content-addressed incremental
    /// compilation with build caching. If no file is specified and a
    /// Blood.toml exists, builds the entire project.
    Build(BuildArgs),

    /// Compile and run source file
    ///
    /// Compiles the source file and immediately runs the resulting executable.
    Run(FileArgs),

    /// Run tests in source file or project
    ///
    /// Discovers functions marked with #[test] and runs them, reporting
    /// results. Supports #[ignore] and #[should_panic] attributes.
    Test(TestArgs),
}

#[derive(Args)]
struct FileArgs {
    /// Source file to process
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Dump additional debug information
    #[arg(short, long)]
    debug: bool,

    /// Disable automatic standard library import
    #[arg(long)]
    no_std: bool,

    /// Path to the standard library (defaults to built-in)
    #[arg(long, value_name = "PATH")]
    stdlib_path: Option<PathBuf>,

    /// Build in release mode with whole-module compilation for inlining
    #[arg(long)]
    release: bool,

    /// Emit intermediate representation(s) instead of building
    #[arg(long, value_enum, value_delimiter = ',')]
    emit: Vec<EmitKind>,

    /// Output file path (for executable or --emit output)
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    output: Option<PathBuf>,

    /// Disable the build cache (recompile all definitions)
    #[arg(long)]
    no_cache: bool,

    /// Directory for build output (default: build/)
    #[arg(long, value_name = "PATH")]
    build_dir: Option<PathBuf>,
}

#[derive(Args)]
struct NewArgs {
    /// Name of the project to create
    #[arg(value_name = "NAME")]
    name: String,

    /// Create a library project instead of a binary
    #[arg(long)]
    lib: bool,
}

#[derive(Args)]
struct InitArgs {
    /// Create a library project instead of a binary
    #[arg(long)]
    lib: bool,
}

#[derive(Args)]
struct BuildArgs {
    /// Source file to process (optional - if not provided, looks for Blood.toml)
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Dump additional debug information
    #[arg(short, long)]
    debug: bool,

    /// Disable automatic standard library import
    #[arg(long)]
    no_std: bool,

    /// Path to the standard library (defaults to built-in)
    #[arg(long, value_name = "PATH")]
    stdlib_path: Option<PathBuf>,

    /// Build in release mode with optimizations
    #[arg(long)]
    release: bool,

    /// Emit intermediate representation(s) instead of building
    #[arg(long, value_enum, value_delimiter = ',')]
    emit: Vec<EmitKind>,

    /// Output file path (for executable or --emit output)
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    output: Option<PathBuf>,

    /// Disable the build cache (recompile all definitions)
    #[arg(long)]
    no_cache: bool,

    /// Directory for build output (default: build/)
    #[arg(long, value_name = "PATH")]
    build_dir: Option<PathBuf>,
}

#[derive(Args)]
struct TestArgs {
    /// Source file or directory to test (optional - if not provided, looks for Blood.toml)
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Filter tests by name pattern (substring match)
    #[arg(long, short = 'F', value_name = "PATTERN")]
    filter: Option<String>,

    /// Run ignored tests as well
    #[arg(long)]
    include_ignored: bool,

    /// Only run ignored tests
    #[arg(long)]
    ignored: bool,

    /// Disable automatic standard library import
    #[arg(long)]
    no_std: bool,

    /// Path to the standard library (defaults to built-in)
    #[arg(long, value_name = "PATH")]
    stdlib_path: Option<PathBuf>,

    /// Show stdout/stderr from passing tests
    #[arg(long)]
    show_output: bool,

    /// Stop running tests after first failure
    #[arg(long)]
    fail_fast: bool,

    /// List tests without running them
    #[arg(long)]
    list: bool,

    /// Number of test threads (default: number of CPUs)
    #[arg(long, short = 'j', value_name = "N")]
    jobs: Option<usize>,
}

/// When to use colored output
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum ColorChoice {
    /// Automatically detect if terminal supports colors
    Auto,
    /// Always use colors
    Always,
    /// Never use colors
    Never,
}

impl ColorChoice {
    fn apply(self) {
        match self {
            ColorChoice::Auto => {} // ariadne auto-detects by default
            ColorChoice::Always => {
                // Force colors on
                std::env::set_var("CLICOLOR_FORCE", "1");
            }
            ColorChoice::Never => {
                // Force colors off
                std::env::set_var("NO_COLOR", "1");
            }
        }
    }
}

/// Intermediate representation to emit
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum EmitKind {
    /// Abstract Syntax Tree (stops after parsing)
    Ast,
    /// High-level IR (stops after type checking)
    Hir,
    /// Mid-level IR (stops after MIR lowering)
    Mir,
    /// Optimized LLVM IR (does not link)
    LlvmIr,
    /// Unoptimized LLVM IR (does not link)
    LlvmIrUnopt,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Apply color settings
    cli.color.apply();

    // Create verbosity level
    let verbosity = if cli.quiet { 0 } else { cli.verbose + 1 };

    let timings = cli.timings;

    match cli.command {
        Commands::New(args) => cmd_new(&args, verbosity),
        Commands::Init(args) => cmd_init(&args, verbosity),
        Commands::Lex(args) => cmd_lex(&args, verbosity),
        Commands::Parse(args) => cmd_parse(&args, verbosity),
        Commands::Check(args) => cmd_check_project(&args, verbosity),
        Commands::Build(args) => cmd_build_project(&args, verbosity, timings),
        Commands::Run(args) => cmd_run(&args, verbosity, timings),
        Commands::Test(args) => cmd_test(&args, verbosity),
    }
}

/// Read source file and return contents
fn read_source(path: &PathBuf) -> Result<String, ExitCode> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) => {
            eprintln!("Error reading file '{}': {}", path.display(), e);
            Err(ExitCode::from(1))
        }
    }
}

/// New command - create a new project
fn cmd_new(args: &NewArgs, verbosity: u8) -> ExitCode {
    let project_dir = PathBuf::from(&args.name);

    // Check if directory already exists
    if project_dir.exists() {
        eprintln!("Error: directory '{}' already exists", args.name);
        return ExitCode::from(1);
    }

    if verbosity > 0 {
        eprintln!("Creating new Blood project: {}", args.name);
    }

    // Create project directory
    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!("Error creating directory: {}", e);
        return ExitCode::from(1);
    }

    // Create src directory
    let src_dir = project_dir.join("src");
    if let Err(e) = fs::create_dir(&src_dir) {
        eprintln!("Error creating src directory: {}", e);
        return ExitCode::from(1);
    }

    // Create Blood.toml
    let manifest = Manifest::new(&args.name, "0.1.0");
    let toml_content = match manifest.to_toml() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error generating Blood.toml: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Err(e) = fs::write(project_dir.join("Blood.toml"), &toml_content) {
        eprintln!("Error writing Blood.toml: {}", e);
        return ExitCode::from(1);
    }

    // Create main entry file
    let entry_file = if args.lib {
        src_dir.join("lib.blood")
    } else {
        src_dir.join("main.blood")
    };

    let entry_content = if args.lib {
        "// Library entry point\n\npub fn hello() -> str {\n    \"Hello from Blood!\"\n}\n"
    } else {
        "// Blood application entry point\n\nfn main() {\n    println!(\"Hello, Blood!\");\n}\n"
    };

    if let Err(e) = fs::write(&entry_file, entry_content) {
        eprintln!("Error writing entry file: {}", e);
        return ExitCode::from(1);
    }

    println!("Created Blood project '{}' in {}", args.name, project_dir.display());
    if verbosity > 0 {
        eprintln!("  Blood.toml");
        eprintln!("  src/{}", if args.lib { "lib.blood" } else { "main.blood" });
    }

    ExitCode::SUCCESS
}

/// Init command - initialize project in current directory
fn cmd_init(args: &InitArgs, verbosity: u8) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error getting current directory: {}", e);
            return ExitCode::from(1);
        }
    };

    let manifest_path = cwd.join("Blood.toml");

    // Check if Blood.toml already exists
    if manifest_path.exists() {
        eprintln!("Error: Blood.toml already exists in this directory");
        return ExitCode::from(1);
    }

    // Use directory name as project name
    let project_name = cwd.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("blood_project")
        .to_string();

    if verbosity > 0 {
        eprintln!("Initializing Blood project: {}", project_name);
    }

    // Create Blood.toml
    let manifest = Manifest::new(&project_name, "0.1.0");
    let toml_content = match manifest.to_toml() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error generating Blood.toml: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Err(e) = fs::write(&manifest_path, &toml_content) {
        eprintln!("Error writing Blood.toml: {}", e);
        return ExitCode::from(1);
    }

    // Create src directory if it doesn't exist
    let src_dir = cwd.join("src");
    if !src_dir.exists() {
        if let Err(e) = fs::create_dir(&src_dir) {
            eprintln!("Error creating src directory: {}", e);
            return ExitCode::from(1);
        }
    }

    // Create entry file if it doesn't exist
    let entry_file = if args.lib {
        src_dir.join("lib.blood")
    } else {
        src_dir.join("main.blood")
    };

    if !entry_file.exists() {
        let entry_content = if args.lib {
            "// Library entry point\n\npub fn hello() -> str {\n    \"Hello from Blood!\"\n}\n"
        } else {
            "// Blood application entry point\n\nfn main() {\n    println!(\"Hello, Blood!\");\n}\n"
        };

        if let Err(e) = fs::write(&entry_file, entry_content) {
            eprintln!("Error writing entry file: {}", e);
            return ExitCode::from(1);
        }
    }

    println!("Initialized Blood project '{}' in {}", project_name, cwd.display());
    ExitCode::SUCCESS
}

/// Check command with project support - type-check source or project
fn cmd_check_project(args: &BuildArgs, verbosity: u8) -> ExitCode {
    // If a file is specified, use single-file mode
    if let Some(ref file) = args.file {
        let file_args = FileArgs {
            file: file.clone(),
            debug: args.debug,
            no_std: args.no_std,
            stdlib_path: args.stdlib_path.clone(),
            release: false, // Check command doesn't use release mode
            emit: Vec::new(),
            output: None,
            no_cache: false,
            build_dir: args.build_dir.clone(),
        };
        return cmd_check(&file_args, verbosity);
    }

    // Otherwise, try to find a project
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error getting current directory: {}", e);
            return ExitCode::from(1);
        }
    };

    match project::discover_project_root(&cwd) {
        Some(project_root) => {
            // Load manifest
            let manifest = match project::load_manifest(&project_root) {
                Ok(m) => m.with_defaults(&project_root),
                Err(e) => {
                    eprintln!("Error loading Blood.toml: {}", e);
                    return ExitCode::from(1);
                }
            };

            if verbosity > 0 {
                eprintln!("Checking project: {} v{}", manifest.package.name, manifest.package.version);
            }

            // Find the entry point to check
            let entry_file = if let Some(ref lib) = manifest.lib {
                project_root.join(&lib.path)
            } else if !manifest.bin.is_empty() {
                project_root.join(&manifest.bin[0].path)
            } else {
                eprintln!("Error: no lib or bin targets in Blood.toml");
                return ExitCode::from(1);
            };

            let file_args = FileArgs {
                file: entry_file,
                debug: args.debug,
                no_std: args.no_std,
                stdlib_path: args.stdlib_path.clone(),
                release: false, // Check command doesn't use release mode
                emit: Vec::new(),
                output: None,
                no_cache: false,
                build_dir: args.build_dir.clone(),
            };
            cmd_check(&file_args, verbosity)
        }
        None => {
            eprintln!("Error: no Blood.toml found. Specify a file or run from a project directory.");
            ExitCode::from(1)
        }
    }
}

/// Build command with project support - compile source or project
fn cmd_build_project(args: &BuildArgs, verbosity: u8, timings: bool) -> ExitCode {
    // If a file is specified, use single-file mode
    if let Some(ref file) = args.file {
        let file_args = FileArgs {
            file: file.clone(),
            debug: args.debug,
            no_std: args.no_std,
            stdlib_path: args.stdlib_path.clone(),
            release: args.release,
            emit: args.emit.clone(),
            output: args.output.clone(),
            no_cache: args.no_cache,
            build_dir: args.build_dir.clone(),
        };
        return cmd_build(&file_args, verbosity, timings);
    }

    // Otherwise, try to find a project
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error getting current directory: {}", e);
            return ExitCode::from(1);
        }
    };

    match project::discover_project_root(&cwd) {
        Some(project_root) => {
            // Load manifest
            let manifest = match project::load_manifest(&project_root) {
                Ok(m) => m.with_defaults(&project_root),
                Err(e) => {
                    eprintln!("Error loading Blood.toml: {}", e);
                    return ExitCode::from(1);
                }
            };

            if verbosity > 0 {
                eprintln!("Building project: {} v{}", manifest.package.name, manifest.package.version);
                if args.release {
                    eprintln!("  Mode: release");
                }
            }

            // Resolve build directory: CLI flag > Blood.toml [build] > env > default
            let effective_build_dir = args.build_dir.clone().or_else(|| {
                manifest.build.build_dir.as_ref().map(|d| project_root.join(d))
            });

            // Build all binary targets
            let mut success = true;
            for (bin_name, bin_path) in manifest.resolved_bins() {
                let entry_file = project_root.join(bin_path);

                if verbosity > 0 {
                    eprintln!("Building binary: {}", bin_name);
                }

                let file_args = FileArgs {
                    file: entry_file,
                    debug: args.debug,
                    no_std: args.no_std,
                    stdlib_path: args.stdlib_path.clone(),
                    release: args.release,
                    emit: args.emit.clone(),
                    output: args.output.clone(),
                    no_cache: args.no_cache,
                    build_dir: effective_build_dir.clone(),
                };

                let result = cmd_build(&file_args, verbosity, timings);
                if result != ExitCode::SUCCESS {
                    success = false;
                }
            }

            if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        None => {
            eprintln!("Error: no Blood.toml found. Specify a file or run from a project directory.");
            ExitCode::from(1)
        }
    }
}

/// Lexer command - tokenize and display token stream
fn cmd_lex(args: &FileArgs, verbosity: u8) -> ExitCode {
    let source = match read_source(&args.file) {
        Ok(s) => s,
        Err(code) => return code,
    };

    if verbosity > 1 {
        eprintln!("Lexing file: {}", args.file.display());
        eprintln!("Source length: {} bytes", source.len());
    }

    let lexer = Lexer::new(&source);
    let mut has_errors = false;
    let mut token_count = 0;

    for token in lexer {
        token_count += 1;
        match token.kind {
            TokenKind::Error => {
                has_errors = true;
                eprintln!(
                    "error[E0001]: unexpected character at {}:{}",
                    token.span.start_line, token.span.start_col
                );
            }
            TokenKind::UnclosedBlockComment => {
                has_errors = true;
                eprintln!(
                    "error[E0002]: unclosed block comment starting at {}:{}",
                    token.span.start_line, token.span.start_col
                );
            }
            TokenKind::Eof => {
                if args.debug {
                    println!("{:4}:{:<3} {:?}", token.span.start_line, token.span.start_col, token.kind);
                }
            }
            _ => {
                let text = if token.kind.has_text() {
                    format!(" '{}'", &source[token.span.start..token.span.end])
                } else {
                    String::new()
                };
                println!(
                    "{:4}:{:<3} {:?}{}",
                    token.span.start_line, token.span.start_col, token.kind, text
                );
            }
        }
    }

    if verbosity > 0 && !has_errors {
        eprintln!("Lexed {} tokens successfully.", token_count);
    }

    if has_errors {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Parse command - parse and display AST
fn cmd_parse(args: &FileArgs, verbosity: u8) -> ExitCode {
    let source = match read_source(&args.file) {
        Ok(s) => s,
        Err(code) => return code,
    };

    if verbosity > 1 {
        eprintln!("Parsing file: {}", args.file.display());
    }

    let mut parser = bloodc::Parser::new(&source);
    let result = parser.parse_program();

    let path_str = args.file.to_string_lossy();
    let emitter = DiagnosticEmitter::new(&path_str, &source);

    match result {
        Ok(program) => {
            println!("{:#?}", program);
            if verbosity > 0 {
                eprintln!(
                    "Parsed {} declarations successfully.",
                    program.declarations.len()
                );
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Parsing failed with {} error(s).", errors.len());
            ExitCode::from(1)
        }
    }
}

/// Check command - type-check source
fn cmd_check(args: &FileArgs, verbosity: u8) -> ExitCode {
    let source = match read_source(&args.file) {
        Ok(s) => s,
        Err(code) => return code,
    };

    if verbosity > 1 {
        eprintln!("Checking file: {}", args.file.display());
    }

    let mut parser = bloodc::Parser::new(&source);
    let result = parser.parse_program();

    let path_str = args.file.to_string_lossy();
    let emitter = DiagnosticEmitter::new(&path_str, &source);

    let mut program = match result {
        Ok(program) => {
            if verbosity > 0 {
                eprintln!("Parsed {} declarations.", program.declarations.len());
            }
            program
        }
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Parsing failed with {} error(s).", errors.len());
            return ExitCode::from(1);
        }
    };

    // Get the interner for symbol resolution
    let interner = parser.take_interner();

    // Expand user-defined macros before type checking
    let mut macro_expander = macro_expand::MacroExpander::with_source(interner, &source);
    let macro_errors = macro_expander.expand_program(&mut program);
    if !macro_errors.is_empty() {
        for error in &macro_errors {
            emitter.emit(error);
        }
        return ExitCode::from(1);
    }

    // Get the interner back from the expander (it may have new symbols from re-parsing)
    let interner = macro_expander.into_interner();

    if verbosity > 1 {
        eprintln!("Macro expansion passed.");
    }

    // Type check the program
    let ctx = bloodc::typeck::TypeContext::new(&source, interner)
        .with_source_path(&args.file)
        .with_no_std(args.no_std);

    // Set stdlib path if provided
    let mut ctx = if let Some(ref stdlib_path) = args.stdlib_path {
        ctx.with_stdlib_path(stdlib_path)
    } else {
        ctx
    };

    // Collect declarations and build type information
    if let Err(errors) = ctx.resolve_program(&program) {
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Type checking failed: {} error(s).", errors.len());
        return ExitCode::from(1);
    }

    // Expand derive macros after collection, before type checking bodies
    ctx.expand_derives();

    // Check for recursive types with infinite size
    if let Err(errors) = ctx.check_recursive_types() {
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Type checking failed: {} error(s).", errors.len());
        return ExitCode::from(1);
    }

    // Type-check all function bodies
    if let Err(errors) = ctx.check_all_bodies() {
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Type checking failed: {} error(s).", errors.len());
        return ExitCode::from(1);
    }

    // Generate HIR to verify everything is well-formed
    let hir_crate = ctx.into_hir();

    if verbosity > 0 {
        eprintln!("Type checking passed. {} items.", hir_crate.items.len());
    }

    // Check linearity (linear/affine type usage)
    let linearity_errors = bloodc::typeck::linearity::check_crate_linearity(&hir_crate);
    if !linearity_errors.is_empty() {
        for error in &linearity_errors {
            emitter.emit(&error.to_diagnostic());
        }
        eprintln!("Type checking failed: {} linearity error(s).", linearity_errors.len());
        return ExitCode::from(1);
    }

    println!("info: Type checking successful.");
    ExitCode::SUCCESS
}

/// Find the runtime libraries to link with.
///
/// Returns (c_runtime, rust_runtime):
/// - c_runtime: Required, provides main entry point and basic functions
/// - rust_runtime: Optional, provides advanced features (fibers, channels, effects)
///
/// Environment variables:
/// - BLOOD_RUNTIME: Override C runtime path
/// - BLOOD_RUST_RUNTIME: Override Rust runtime path
fn find_runtime_paths() -> (PathBuf, Option<PathBuf>) {
    let c_runtime = find_c_runtime();
    let rust_runtime = find_rust_runtime();
    (c_runtime, rust_runtime)
}

/// Find the C runtime (provides main entry point).
fn find_c_runtime() -> PathBuf {
    // Check environment variable first
    if let Ok(path) = std::env::var("BLOOD_RUNTIME") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }

    // Try relative to the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check in repo root runtime/ (for development: exe is at src/bootstrap/target/release/)
            let runtime_dev = exe_dir.join("../../../../runtime/runtime.o");
            if runtime_dev.exists() {
                return runtime_dev;
            }
            // Check alongside executable
            let runtime_sibling = exe_dir.join("runtime.o");
            if runtime_sibling.exists() {
                return runtime_sibling;
            }
        }
    }

    // Try current directory
    if let Ok(cwd) = std::env::current_dir() {
        let runtime_cwd = cwd.join("runtime/runtime.o");
        if runtime_cwd.exists() {
            return runtime_cwd;
        }
    }

    // Fallback - will fail at link time if not found
    PathBuf::from("runtime/runtime.o")
}

/// Find the Rust runtime (provides fibers, channels, effects).
fn find_rust_runtime() -> Option<PathBuf> {
    // Check environment variable first
    if let Ok(path) = std::env::var("BLOOD_RUST_RUNTIME") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    let rust_runtime_names = [
        "libblood_runtime.a",  // Unix static lib
        "blood_runtime.lib",   // Windows static lib
    ];

    // Try relative to the executable (works in cargo builds)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check in target/{debug,release} directory
            for name in &rust_runtime_names {
                let rust_runtime = exe_dir.join(name);
                if rust_runtime.exists() {
                    return Some(rust_runtime);
                }
            }

            // Check in parent target directory - prefer release over debug
            if let Some(target_dir) = exe_dir.parent() {
                for name in &rust_runtime_names {
                    // Prefer release runtime for smaller binary size
                    for profile in &["release", "debug"] {
                        let rust_runtime = target_dir.join(profile).join(name);
                        if rust_runtime.exists() {
                            return Some(rust_runtime);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Resolve the build output directory using the override hierarchy:
/// 1. CLI flag (--build-dir)
/// 2. Blood.toml [build] build-dir (project mode only)
/// 3. BLOOD_BUILD_DIR environment variable
/// 4. Default: build/ relative to source file parent
fn resolve_build_dir(cli_build_dir: Option<&PathBuf>, source_file: &std::path::Path) -> PathBuf {
    // 1. CLI flag (highest priority)
    if let Some(dir) = cli_build_dir {
        return dir.clone();
    }
    // 2. Blood.toml [build] build-dir (checked by caller in project mode,
    //    passed via cli_build_dir — see cmd_build_project)
    // 3. BLOOD_BUILD_DIR env var
    if let Ok(env_dir) = std::env::var("BLOOD_BUILD_DIR") {
        if !env_dir.is_empty() {
            return PathBuf::from(env_dir);
        }
    }
    // 4. Default: build/ relative to source file parent
    source_file
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("build")
}

/// Return the profile subdirectory name based on release mode.
fn profile_subdir(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

/// Build command - compile to executable
fn cmd_build(args: &FileArgs, verbosity: u8, timings: bool) -> ExitCode {
    let build_start = Instant::now();
    let mut phase_timings: Vec<(&str, std::time::Duration)> = Vec::new();
    let emit_set = &args.emit;

    let source = match read_source(&args.file) {
        Ok(s) => s,
        Err(code) => return code,
    };

    if verbosity > 1 {
        eprintln!("Building file: {}", args.file.display());
    }

    let path_str = args.file.to_string_lossy();
    let emitter = DiagnosticEmitter::new(&path_str, &source);

    // Parse
    let t = Instant::now();
    let mut parser = bloodc::Parser::new(&source);
    let mut program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Build failed: parsing errors.");
            return ExitCode::from(1);
        }
    };

    // Take interner from parser for type checking
    let interner = parser.take_interner();
    phase_timings.push(("Parse", t.elapsed()));

    if verbosity > 0 {
        eprintln!("Parsed {} declarations.", program.declarations.len());
    }

    // Emit AST if requested
    if emit_set.contains(&EmitKind::Ast) {
        println!("{:#?}", program);
        if !emit_set.iter().any(|e| !matches!(e, EmitKind::Ast)) {
            if timings { print_timings(&phase_timings, build_start.elapsed()); }
            return ExitCode::SUCCESS;
        }
    }

    // Expand user-defined macros before type checking
    let t = Instant::now();
    let mut macro_expander = macro_expand::MacroExpander::with_source(interner, &source);
    let macro_errors = macro_expander.expand_program(&mut program);
    if !macro_errors.is_empty() {
        for error in &macro_errors {
            emitter.emit(error);
        }
        return ExitCode::from(1);
    }

    // Get the interner back from the expander (it may have new symbols from re-parsing)
    let interner = macro_expander.into_interner();
    phase_timings.push(("Macro expand", t.elapsed()));

    if verbosity > 1 {
        eprintln!("User-defined macro expansion passed.");
    }

    // Type check and lower to HIR
    let t = Instant::now();
    let ctx = bloodc::typeck::TypeContext::new(&source, interner)
        .with_source_path(&args.file)
        .with_no_std(args.no_std);

    // Set stdlib path if provided
    let mut ctx = if let Some(ref stdlib_path) = args.stdlib_path {
        ctx.with_stdlib_path(stdlib_path)
    } else {
        ctx
    };

    let mut all_type_errors: Vec<bloodc::diagnostics::Diagnostic> = Vec::new();

    let resolve_ok = match ctx.resolve_program(&program) {
        Ok(()) => true,
        Err(errors) => {
            all_type_errors.extend(errors);
            false
        }
    };

    // Expand derive macros after collection, before type checking bodies
    ctx.expand_derives();

    // Check for recursive types with infinite size
    if let Err(errors) = ctx.check_recursive_types() {
        all_type_errors.extend(errors);
    }

    // Type-check all function bodies (only if resolution succeeded,
    // since body checking depends on resolved names)
    if resolve_ok {
        if let Err(errors) = ctx.check_all_bodies() {
            all_type_errors.extend(errors);
        }
    }

    if !all_type_errors.is_empty() {
        for error in &all_type_errors {
            emitter.emit(error);
        }
        eprintln!("Build failed: {} type error(s).", all_type_errors.len());
        return ExitCode::from(1);
    }
    phase_timings.push(("Type check", t.elapsed()));

    // Get builtin type DefIds before consuming the context
    let builtin_def_ids = ctx.get_builtin_def_ids();

    // Generate HIR
    let t = Instant::now();
    let mut hir_crate = ctx.into_hir();
    phase_timings.push(("HIR generation", t.elapsed()));

    if verbosity > 0 {
        eprintln!("Type checking passed. {} items.", hir_crate.items.len());
    }

    // Emit HIR if requested
    if emit_set.contains(&EmitKind::Hir) {
        println!("{:#?}", hir_crate);
        if !emit_set.iter().any(|e| matches!(e, EmitKind::Mir | EmitKind::LlvmIr | EmitKind::LlvmIrUnopt)) {
            if timings { print_timings(&phase_timings, build_start.elapsed()); }
            return ExitCode::SUCCESS;
        }
    }

    // Check linearity (linear/affine type usage)
    let t = Instant::now();
    let linearity_errors = bloodc::typeck::linearity::check_crate_linearity(&hir_crate);
    if !linearity_errors.is_empty() {
        for error in &linearity_errors {
            emitter.emit(&error.to_diagnostic());
        }
        eprintln!("Build failed: linearity errors.");
        return ExitCode::from(1);
    }
    phase_timings.push(("Linearity", t.elapsed()));

    // Expand macros in HIR before MIR lowering
    let t = Instant::now();
    if let Err(errors) = expand::expand_macros(&mut hir_crate) {
        let emitter = DiagnosticEmitter::new(&path_str, &source);
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Build failed: macro expansion errors.");
        return ExitCode::from(1);
    }
    phase_timings.push(("HIR macros", t.elapsed()));

    if verbosity > 1 {
        eprintln!("Macro expansion passed.");
    }

    // Content hashing
    let t = Instant::now();

    // Initialize build cache for incremental compilation
    let mut build_cache = BuildCache::new();
    match build_cache.init_and_load() {
        Ok(true) => {
            if verbosity > 0 {
                eprintln!("Loaded existing build cache.");
            }
        }
        Ok(false) => {
            if verbosity > 1 {
                eprintln!("Created fresh build cache.");
            }
        }
        Err(e) => {
            if verbosity > 0 {
                eprintln!("Warning: Failed to initialize build cache: {}", e);
            }
        }
    }

    // Wrap with distributed cache for remote caching support
    // Enabled via BLOOD_CACHE_REMOTES environment variable
    let mut distributed_cache = DistributedCache::from_env(build_cache);
    if distributed_cache.is_enabled() && verbosity > 0 {
        eprintln!("Distributed caching enabled.");
    }

    // Compute content hashes for all definitions
    let mut definition_hashes: std::collections::HashMap<bloodc::hir::DefId, ContentHash> = std::collections::HashMap::new();
    let mut cache_hits = 0usize;
    let mut cache_misses = 0usize;

    if verbosity > 1 {
        eprintln!("Computing content hashes...");
    }

    for (&def_id, item) in &hir_crate.items {
        let hash = hash_hir_item(def_id, item, &hir_crate.bodies, &hir_crate.items, Some(&args.file));
        definition_hashes.insert(def_id, hash);

        // Check if we have cached compiled code for this definition (local cache only for stats)
        let is_cached = distributed_cache.has_object(&hash);
        if is_cached {
            cache_hits += 1;
        } else {
            cache_misses += 1;
        }

        if verbosity > 1 {
            let cache_status = if is_cached { "[cached]" } else { "[new]" };
            eprintln!("  {:?}: {} ({}) {}", def_id, hash.short_display(), item.name, cache_status);
        }
    }

    if verbosity > 0 && !definition_hashes.is_empty() {
        eprintln!(
            "Content hashing: {} definitions ({} cached, {} to compile)",
            definition_hashes.len(),
            cache_hits,
            cache_misses
        );
    }

    // Extract and record dependencies for incremental invalidation
    for (&def_id, item) in &hir_crate.items {
        if let Some(&hash) = definition_hashes.get(&def_id) {
            let deps_set = extract_dependencies(item, &hir_crate.bodies);
            // Convert DefId dependencies to ContentHash dependencies
            let dep_hashes: Vec<ContentHash> = deps_set
                .iter()
                .filter_map(|dep_id| definition_hashes.get(dep_id).copied())
                .collect();
            if !dep_hashes.is_empty() {
                distributed_cache.local_mut().record_dependencies(hash, dep_hashes);
                if verbosity > 2 {
                    eprintln!("  {} depends on {} definitions", item.name, deps_set.len());
                }
            }
        }
    }

    // Populate namespace registry with content-addressed definitions
    let mut name_registry = NameRegistry::new();
    {
        let main_ns = name_registry.get_or_create("main");
        for (&def_id, item) in &hir_crate.items {
            if let Some(&hash) = definition_hashes.get(&def_id) {
                // Determine binding kind from item kind
                let binding = match &item.kind {
                    bloodc::hir::ItemKind::Fn(_) => NameBinding::value(hash),
                    bloodc::hir::ItemKind::Struct(_) => NameBinding::ty(hash),
                    bloodc::hir::ItemKind::Enum(_) => NameBinding::ty(hash),
                    bloodc::hir::ItemKind::Effect { .. } => NameBinding::effect(hash),
                    bloodc::hir::ItemKind::Trait { .. } => NameBinding::ty(hash),
                    bloodc::hir::ItemKind::Impl { .. } => continue, // Impls don't have names
                    bloodc::hir::ItemKind::Handler { .. } => NameBinding::value(hash),
                    bloodc::hir::ItemKind::TypeAlias { .. } => {
                        NameBinding {
                            hash,
                            kind: BindingKind::TypeAlias,
                            is_public: true,
                            doc: None,
                        }
                    }
                    bloodc::hir::ItemKind::Const { .. } => NameBinding::value(hash),
                    bloodc::hir::ItemKind::Static { .. } => NameBinding::value(hash),
                    bloodc::hir::ItemKind::ExternFn(_) => NameBinding::value(hash),
                    bloodc::hir::ItemKind::Bridge(_) => continue, // Bridges are namespaces, not standalone bindings
                    bloodc::hir::ItemKind::Module(_) => continue, // Modules are namespaces, not standalone bindings
                };
                main_ns.bind(&item.name, binding);
            }
        }
    }

    if verbosity > 1 {
        let main_ns = name_registry.get("main");
        if let Some(ns) = main_ns {
            eprintln!("Namespace registry: {} bindings in 'main'", ns.len());
        }
    }

    // Populate Virtual Function Table (VFT) with function metadata
    let mut vft = VFT::new();
    {
        use bloodc::content::vft::{CallingConvention, EffectMask};

        for (&def_id, item) in &hir_crate.items {
            if let Some(&hash) = definition_hashes.get(&def_id) {
                match &item.kind {
                    bloodc::hir::ItemKind::Fn(fn_def) => {
                        // Calculate arity from parameters
                        let arity = fn_def.sig.inputs.len().min(255) as u8;

                        // Determine calling convention
                        let convention = if fn_def.sig.is_async {
                            CallingConvention::Effect
                        } else {
                            CallingConvention::Blood
                        };

                        // Build effect mask from function properties
                        let effects = if fn_def.sig.is_async {
                            EffectMask::ASYNC
                        } else {
                            EffectMask::NONE
                        };

                        // Entry point is 0 for AOT compilation (resolved at link time)
                        let entry = VFTEntry::new(hash, 0, arity)
                            .with_convention(convention)
                            .with_effects(effects);

                        vft.register(entry);
                    }
                    bloodc::hir::ItemKind::Handler { .. } => {
                        // Handlers are callable - register with effect convention
                        let entry = VFTEntry::new(hash, 0, 0)
                            .with_convention(CallingConvention::Effect);
                        vft.register(entry);
                    }
                    _ => {
                        // Other item kinds don't need VFT entries
                    }
                }
            }
        }
    }

    if verbosity > 1 {
        eprintln!("VFT populated: {} function entries", vft.len());
    }
    phase_timings.push(("Content hash", t.elapsed()));

    // Lower to MIR (Phase 3 integration point)
    let t = Instant::now();
    let mut mir_lowering = mir::MirLowering::new(&hir_crate);
    let mir_result = match mir_lowering.lower_crate() {
        Ok((mir_bodies, inline_handler_bodies)) => {
            phase_timings.push(("MIR lowering", t.elapsed()));

            if verbosity > 1 {
                eprintln!("MIR lowering passed. {} function bodies, {} inline handlers.",
                    mir_bodies.len(), inline_handler_bodies.len());
            }

            // Emit MIR if requested
            if emit_set.contains(&EmitKind::Mir) {
                for (&def_id, body) in &mir_bodies {
                    println!("// MIR for {:?}", def_id);
                    println!("{:#?}", body);
                    println!();
                }
                if !emit_set.iter().any(|e| matches!(e, EmitKind::LlvmIr | EmitKind::LlvmIrUnopt)) {
                    if timings { print_timings(&phase_timings, build_start.elapsed()); }
                    return ExitCode::SUCCESS;
                }
            }

            // Validate MIR well-formedness
            let t = Instant::now();
            let validation = mir::validate::validate_mir_bodies(&mir_bodies);
            for warning in &validation.warnings {
                emitter.emit(warning);
            }
            if !validation.errors.is_empty() {
                for error in &validation.errors {
                    emitter.emit(error);
                }
                return ExitCode::from(1);
            }
            phase_timings.push(("MIR validation", t.elapsed()));

            // Run escape analysis on MIR bodies
            let t = Instant::now();
            // Create ADT field lookup closure for Copy detection
            let adt_fields = |def_id: bloodc::hir::DefId| -> Option<Vec<bloodc::hir::Type>> {
                let item = hir_crate.items.get(&def_id)?;
                if let bloodc::hir::ItemKind::Struct(struct_def) = &item.kind {
                    match &struct_def.kind {
                        bloodc::hir::item::StructKind::Record(fields) => {
                            Some(fields.iter().map(|f| f.ty.clone()).collect())
                        }
                        bloodc::hir::item::StructKind::Tuple(fields) => {
                            Some(fields.iter().map(|f| f.ty.clone()).collect())
                        }
                        bloodc::hir::item::StructKind::Unit => Some(Vec::new()),
                    }
                } else {
                    // Enums and other ADTs are not Copy by default
                    None
                }
            };

            let escape_results: std::collections::HashMap<_, _> = mir_bodies.iter()
                .map(|(&def_id, body)| {
                    let mut analyzer = mir::EscapeAnalyzer::new();
                    (def_id, analyzer.analyze_with_adt_lookup(body, &adt_fields))
                })
                .collect();

            // Compute aggregate escape analysis statistics (PERF-V-002)
            let mut escape_stats = mir::EscapeStatistics::new();
            for results in escape_results.values() {
                escape_stats.add_results(results);
            }

            if verbosity > 1 {
                eprintln!("Escape analysis complete. {}", escape_stats.format_summary());
                // Report tier recommendations per function
                for (&def_id, results) in &escape_results {
                    let stack_count = results.stack_promotable.len();
                    let effect_captured = results.effect_captured.len();
                    if verbosity > 2 {
                        eprintln!("  {:?}: {} stack-promotable, {} effect-captured",
                            def_id, stack_count, effect_captured);
                    }
                }
            }

            // At high verbosity, print full statistics report
            if verbosity > 2 {
                eprintln!("{}", escape_stats.format_report());
            }
            phase_timings.push(("Escape analysis", t.elapsed()));

            // Run closure analysis to identify inline optimization candidates
            let t = Instant::now();
            let closure_analysis = mir::ClosureAnalyzer::new().analyze_bodies(&mir_bodies);
            if verbosity > 1 {
                eprintln!("Closure analysis: {} closures, {} inline candidates ({:.0}%)",
                    closure_analysis.stats.total_closures,
                    closure_analysis.stats.inline_eligible,
                    if closure_analysis.stats.total_closures > 0 {
                        closure_analysis.stats.inline_eligible as f64 /
                        closure_analysis.stats.total_closures as f64 * 100.0
                    } else { 0.0 }
                );
                if verbosity > 2 && closure_analysis.stats.total_closures > 0 {
                    eprintln!("{}", closure_analysis.summary());
                }
            }
            phase_timings.push(("Closure analysis", t.elapsed()));

            // Return MIR bodies, escape analysis, inline handler bodies, and closure analysis for codegen
            Some((mir_bodies, escape_results, inline_handler_bodies, closure_analysis))
        }
        Err(errors) => {
            // MIR lowering is mandatory - errors are fatal
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Build failed: MIR lowering errors.");
            return ExitCode::from(1);
        }
    };

    // Compute crate hash for incremental compilation cache lookup
    let _crate_hash = {
        let mut crate_hasher = ContentHasher::new();
        let mut sorted_hashes: Vec<_> = definition_hashes.values().collect();
        sorted_hashes.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        for hash in sorted_hashes {
            crate_hasher.update_hash(hash);
        }
        crate_hasher.finalize()
    };

    // Determine output paths using build directory
    let build_dir = resolve_build_dir(args.build_dir.as_ref(), &args.file);
    let profile_dir = build_dir.join(profile_subdir(args.release));
    if let Err(e) = std::fs::create_dir_all(&profile_dir) {
        eprintln!("Failed to create build directory {}: {}", profile_dir.display(), e);
        return ExitCode::from(1);
    }
    let stem = args.file.file_stem().unwrap_or_default();
    let output_exe = args.output.clone().unwrap_or_else(|| profile_dir.join(stem));

    let (ref mir_bodies, ref escape_map, ref inline_handler_bodies, ref closure_analysis) = mir_result
        .expect("MIR result should be present (errors return early)");

    // Emit LLVM IR if requested (before object code generation)
    if emit_set.contains(&EmitKind::LlvmIr) || emit_set.contains(&EmitKind::LlvmIrUnopt) {
        // Default IR output goes to build/<profile>/<stem>.ll
        let default_ir_path = profile_dir.join(stem).with_extension("ll");
        // Helper: write IR to -o path, or build dir default
        let write_ir = |ir: &str, label: &str| -> Result<(), ExitCode> {
            let target = args.output.as_ref().unwrap_or(&default_ir_path);
            if let Err(e) = std::fs::write(target, ir) {
                eprintln!("Failed to write {} to {}: {}", label, target.display(), e);
                return Err(ExitCode::from(1));
            }
            if verbosity > 0 {
                eprintln!("Wrote {} to {}", label, target.display());
            }
            Ok(())
        };

        if emit_set.contains(&EmitKind::LlvmIr) {
            match codegen::compile_mir_to_ir(&hir_crate, mir_bodies, escape_map, builtin_def_ids, Some(inline_handler_bodies), Some(closure_analysis)) {
                Ok(ir) => {
                    if let Err(code) = write_ir(&ir, "optimized LLVM IR") {
                        return code;
                    }
                }
                Err(errors) => {
                    for error in &errors {
                        emitter.emit(error);
                    }
                    eprintln!("Failed to generate optimized LLVM IR.");
                    return ExitCode::from(1);
                }
            }
        }
        if emit_set.contains(&EmitKind::LlvmIrUnopt) {
            match codegen::compile_mir_to_ir_with_opt(
                &hir_crate, mir_bodies, escape_map,
                codegen::BloodOptLevel::None, builtin_def_ids,
                Some(inline_handler_bodies), Some(closure_analysis),
            ) {
                Ok(ir) => {
                    if let Err(code) = write_ir(&ir, "unoptimized LLVM IR") {
                        return code;
                    }
                }
                Err(errors) => {
                    for error in &errors {
                        emitter.emit(error);
                    }
                    eprintln!("Failed to generate unoptimized LLVM IR.");
                    return ExitCode::from(1);
                }
            }
        }
        // Don't continue to object generation and linking
        if timings { print_timings(&phase_timings, build_start.elapsed()); }
        return ExitCode::SUCCESS;
    }

    // Track object files for linking
    let t = Instant::now();
    let mut object_files: Vec<std::path::PathBuf> = Vec::new();
    let mut compile_errors: Vec<bloodc::diagnostics::Diagnostic> = Vec::new();

    // Create directory for per-definition object files inside build dir
    let obj_dir = build_dir.join("obj").join(stem);
    if !obj_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&obj_dir) {
            eprintln!("Failed to create object directory: {}", e);
            return ExitCode::from(1);
        }
    }

    if args.release {
        // Release mode: Whole-module compilation for cross-function inlining
        // All functions are compiled into a single LLVM module, enabling LLVM to
        // inline across function boundaries for maximum performance.
        if verbosity > 0 {
            eprintln!("Using whole-module compilation (release mode - enables inlining)");
        }

        let output_obj = obj_dir.join("whole_module.o");
        match codegen::compile_mir_to_object(&hir_crate, mir_bodies, escape_map, inline_handler_bodies, &output_obj, builtin_def_ids, Some(closure_analysis)) {
            Ok(()) => {
                object_files.push(output_obj);
                if verbosity > 0 {
                    eprintln!("Compiled {} functions into single module", mir_bodies.len());
                }
            }
            Err(errors) => {
                compile_errors.extend(errors);
            }
        }
    } else {
        // Debug mode: Per-definition incremental compilation
        // Each function gets its own object file, cached by content hash.
        // This enables fast incremental rebuilds but prevents cross-function inlining.
        if verbosity > 0 {
            eprintln!("Using per-definition incremental compilation");
        }

        let mut cached_count = 0;
        let mut compiled_count = 0;

        // For each function with a MIR body, check cache and compile if needed
        for (&def_id, mir_body) in mir_bodies {
            // Get the content hash for this definition
            let def_hash = if let Some(item) = hir_crate.items.get(&def_id) {
                hash_hir_item(def_id, item, &hir_crate.bodies, &hir_crate.items, Some(&args.file))
            } else {
                // Synthetic definition (closure) - hash based on MIR content and source file
                let mut hasher = ContentHasher::new();
                // Include source file path to prevent cross-file cache collisions
                if let Some(path_str) = args.file.to_str() {
                    hasher.update(path_str.as_bytes());
                }
                // Include DefId for symbol name consistency (deterministic after
                // sorting HIR items in lower_crate), plus MIR body characteristics
                // for content-addressability
                hasher.update(format!("closure_{}", def_id.index()).as_bytes());
                hasher.update(&mir_body.locals.len().to_le_bytes());
                hasher.update(&mir_body.basic_blocks.len().to_le_bytes());
                hasher.update(&mir_body.param_count.to_le_bytes());
                hasher.finalize()
            };

            // Check cache for this definition (local and remote)
            let obj_path = obj_dir.join(format!("def_{}.o", def_id.index()));

            if !args.no_cache {
                match distributed_cache.fetch(&def_hash) {
                    FetchResult::LocalHit(data) => {
                        // Local cache hit - write to obj file and use
                        if std::fs::write(&obj_path, &data).is_ok() {
                            object_files.push(obj_path);
                            cached_count += 1;
                            if verbosity > 2 {
                                eprintln!("  Cache hit (local): {:?} ({})", def_id, def_hash.short_display());
                            }
                            continue;
                        }
                        // If write fails, fall through to compile
                    }
                    FetchResult::RemoteHit { data, source } => {
                        // Remote cache hit - already stored locally by fetch(), write to obj file
                        if std::fs::write(&obj_path, &data).is_ok() {
                            object_files.push(obj_path);
                            cached_count += 1;
                            if verbosity > 1 {
                                eprintln!("  Cache hit (remote): {:?} ({}) from {}", def_id, def_hash.short_display(), source);
                            }
                            continue;
                        }
                        // If write fails, fall through to compile
                    }
                    FetchResult::NotFound | FetchResult::Error(_) => {
                        // Cache miss - need to compile
                    }
                }
            }

            // Compile this definition
            let escape_results = escape_map.get(&def_id);

            match codegen::compile_definition_to_object(
                def_id,
                &hir_crate,
                Some(mir_body),
                escape_results,
                Some(mir_bodies),
                Some(inline_handler_bodies),
                &obj_path,
                builtin_def_ids,
                Some(closure_analysis),
            ) {
                Ok(()) => {
                    compiled_count += 1;
                    if verbosity > 2 {
                        eprintln!("  Compiled: {:?} ({})", def_id, def_hash.short_display());
                    }

                    // Cache the compiled object (local and optionally remote)
                    if let Ok(obj_data) = std::fs::read(&obj_path) {
                        let publish_to_remote = distributed_cache.is_enabled();
                        if let Err(e) = distributed_cache.store(def_hash, &obj_data, publish_to_remote) {
                            if verbosity > 1 {
                                eprintln!("  Warning: Failed to cache {:?}: {}", def_id, e);
                            }
                        }
                    }

                    object_files.push(obj_path);
                }
                Err(errors) => {
                    compile_errors.extend(errors);
                }
            }
        }

        // Compile handler definitions (they don't have MIR bodies, so not in the loop above)
        for (&def_id, item) in &hir_crate.items {
            if !matches!(item.kind, bloodc::hir::ItemKind::Handler { .. }) {
                continue;
            }

            // Hash the handler item
            let def_hash = hash_hir_item(def_id, item, &hir_crate.bodies, &hir_crate.items, Some(&args.file));

            // Check cache for this handler (local and remote)
            let obj_path = obj_dir.join(format!("handler_{}.o", def_id.index()));

            if !args.no_cache {
                match distributed_cache.fetch(&def_hash) {
                    FetchResult::LocalHit(data) => {
                        if std::fs::write(&obj_path, &data).is_ok() {
                            object_files.push(obj_path);
                            cached_count += 1;
                            if verbosity > 2 {
                                eprintln!("  Cache hit (handler, local): {:?} ({})", def_id, def_hash.short_display());
                            }
                            continue;
                        }
                    }
                    FetchResult::RemoteHit { data, source } => {
                        if std::fs::write(&obj_path, &data).is_ok() {
                            object_files.push(obj_path);
                            cached_count += 1;
                            if verbosity > 1 {
                                eprintln!("  Cache hit (handler, remote): {:?} ({}) from {}", def_id, def_hash.short_display(), source);
                            }
                            continue;
                        }
                    }
                    FetchResult::NotFound | FetchResult::Error(_) => {}
                }
            }

            // Compile this handler
            match codegen::compile_definition_to_object(def_id, &hir_crate, None, None, Some(mir_bodies), Some(inline_handler_bodies), &obj_path, builtin_def_ids, Some(closure_analysis)) {
                Ok(()) => {
                    compiled_count += 1;
                    if verbosity > 2 {
                        eprintln!("  Compiled (handler): {:?} ({})", def_id, def_hash.short_display());
                    }

                    // Cache the compiled object (local and optionally remote)
                    if let Ok(obj_data) = std::fs::read(&obj_path) {
                        let publish_to_remote = distributed_cache.is_enabled();
                        if let Err(e) = distributed_cache.store(def_hash, &obj_data, publish_to_remote) {
                            if verbosity > 1 {
                                eprintln!("  Warning: Failed to cache {:?}: {}", def_id, e);
                            }
                        }
                    }

                    object_files.push(obj_path);
                }
                Err(errors) => {
                    compile_errors.extend(errors);
                }
            }
        }

        // Report compilation results
        if verbosity > 0 {
            eprintln!(
                "Compilation: {} cached, {} compiled, {} total",
                cached_count,
                compiled_count,
                cached_count + compiled_count
            );
        }
    }

    // Check for compile errors
    if !compile_errors.is_empty() {
        for error in &compile_errors {
            emitter.emit(error);
        }
        eprintln!("Build failed: {} codegen errors.", compile_errors.len());
        return ExitCode::from(1);
    }

    // If no object files, nothing to link
    if object_files.is_empty() {
        eprintln!("No functions to compile.");
        return ExitCode::from(1);
    }

    // Debug: show LLVM IR if requested
    if args.debug {
        let ir_result = codegen::compile_mir_to_ir(&hir_crate, mir_bodies, escape_map, builtin_def_ids, Some(inline_handler_bodies), Some(closure_analysis));
        if let Ok(ir) = ir_result {
            println!("{}", ir);
        }
    }

    // Generate handler registration object (needed for effect handlers in debug mode)
    // In release mode, handler registration is included in whole_module.o via
    // compile_mir_to_object, so we skip generating a separate object file.
    // In debug mode, we need a separate object for per-definition compilation.
    if !args.release {
        let handler_reg_path = obj_dir.join("handler_registration.o");
        if let Err(errors) = codegen::compile_handler_registration_to_object(&hir_crate, &handler_reg_path, builtin_def_ids) {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Build failed: handler registration codegen error.");
            return ExitCode::from(1);
        }
        object_files.push(handler_reg_path);
    }
    phase_timings.push(("Codegen", t.elapsed()));

    // Link with runtimes (for both cached and freshly compiled objects)
    let t = Instant::now();
    // C runtime is required (provides main entry point and string utilities)
    // Rust runtime provides all other FFI functions (memory, effects, dispatch, etc.)
    let (c_runtime, rust_runtime) = find_runtime_paths();

    // Collect FFI link specifications from all bridge blocks
    let mut ffi_link_specs: Vec<&bloodc::hir::item::LinkSpec> = Vec::new();
    for item in hir_crate.items.values() {
        if let bloodc::hir::ItemKind::Bridge(bridge_def) = &item.kind {
            for link_spec in &bridge_def.link_specs {
                ffi_link_specs.push(link_spec);
            }
        }
    }

    if verbosity > 1 && !ffi_link_specs.is_empty() {
        eprintln!("FFI link specifications: {} libraries", ffi_link_specs.len());
        for spec in &ffi_link_specs {
            eprintln!("  - {} ({:?})", spec.name, spec.kind);
        }
    }

    let mut link_cmd = std::process::Command::new("cc");

    // Enable dead code elimination, and strip symbols in release mode
    #[cfg(target_os = "linux")]
    {
        if args.release {
            link_cmd.arg("-s"); // Strip symbols in release mode only
        }
        link_cmd.arg("-Wl,--gc-sections"); // Remove unused sections
    }

    #[cfg(target_os = "macos")]
    {
        if args.release {
            link_cmd.arg("-Wl,-x"); // Strip local symbols in release mode
        }
        link_cmd.arg("-Wl,-dead_strip"); // macOS equivalent of gc-sections
    }

    // Add all per-definition object files
    for obj_file in &object_files {
        link_cmd.arg(obj_file);
    }

    link_cmd.arg(&c_runtime);

    // Add Rust runtime if available (required for full functionality)
    if let Some(rust_rt) = &rust_runtime {
        link_cmd.arg(rust_rt);
        if verbosity > 0 {
            eprintln!("Linking with Rust runtime: {}", rust_rt.display());
        }

        // Add system libraries required by the Rust runtime
        // These are needed for pthread support, dynamic loading, and math
        #[cfg(target_os = "linux")]
        {
            link_cmd.arg("-lpthread");
            link_cmd.arg("-ldl");
            link_cmd.arg("-lm");
        }

        #[cfg(target_os = "macos")]
        {
            link_cmd.arg("-lpthread");
            link_cmd.arg("-framework").arg("Security");
        }

        #[cfg(target_os = "windows")]
        {
            link_cmd.arg("-lws2_32");
            link_cmd.arg("-luserenv");
        }
    }

    // Add FFI library link flags from bridge blocks
    for link_spec in &ffi_link_specs {
        match link_spec.kind {
            bloodc::hir::item::LinkKind::Dylib => {
                // Dynamic library: -l<name>
                link_cmd.arg(format!("-l{}", link_spec.name));
            }
            bloodc::hir::item::LinkKind::Static => {
                // Static library: -l<name> with -static prefix or just -l<name>
                // Most linkers will search for lib<name>.a when -static is used
                // For simplicity, we use -l<name> which works for both
                link_cmd.arg(format!("-l{}", link_spec.name));
            }
            bloodc::hir::item::LinkKind::Framework => {
                // macOS framework: -framework <name>
                #[cfg(target_os = "macos")]
                {
                    link_cmd.arg("-framework").arg(&link_spec.name);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // On non-macOS, frameworks don't exist; warn and skip
                    if verbosity > 0 {
                        eprintln!("Warning: Framework '{}' ignored on non-macOS platform", link_spec.name);
                    }
                }
            }
        }
    }

    link_cmd.arg("-o").arg(&output_exe);

    let status = link_cmd.status();

    phase_timings.push(("Linking", t.elapsed()));

    match status {
        Ok(s) if s.success() => {
            // Save cache index on successful build for incremental compilation
            if let Err(e) = distributed_cache.local_mut().save_index() {
                if verbosity > 0 {
                    eprintln!("Warning: Failed to save build cache index: {}", e);
                }
            } else if verbosity > 1 {
                eprintln!("Saved build cache index.");
            }

            if verbosity > 0 {
                eprintln!("Linked executable: {}", output_exe.display());
            }
            if timings { print_timings(&phase_timings, build_start.elapsed()); }
            if verbosity > 0 {
                eprintln!("Build successful: {}", output_exe.display());
            }
            ExitCode::SUCCESS
        }
        Ok(_) => {
            if timings { print_timings(&phase_timings, build_start.elapsed()); }
            eprintln!("Linking failed.");
            ExitCode::from(1)
        }
        Err(e) => {
            if timings { print_timings(&phase_timings, build_start.elapsed()); }
            eprintln!("Failed to run linker: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Print a timing table to stderr.
fn print_timings(phases: &[(&str, std::time::Duration)], total: std::time::Duration) {
    eprintln!();
    eprintln!("Timings:");
    for (name, dur) in phases {
        let ms = dur.as_secs_f64() * 1000.0;
        eprintln!("  {:<20} {:>7.0}ms", name, ms);
    }
    let total_ms = total.as_secs_f64() * 1000.0;
    eprintln!("  {}", "\u{2500}".repeat(30));
    eprintln!("  {:<20} {:>7.0}ms", "Total", total_ms);
}

/// Run command - compile and execute
fn cmd_run(args: &FileArgs, verbosity: u8, timings: bool) -> ExitCode {
    // First build
    let build_result = cmd_build(args, verbosity, timings);
    if build_result != ExitCode::SUCCESS {
        return build_result;
    }

    // Determine where the build placed the executable
    let output_exe = if let Some(ref out) = args.output {
        out.clone()
    } else {
        let build_dir = resolve_build_dir(args.build_dir.as_ref(), &args.file);
        let profile_dir = build_dir.join(profile_subdir(args.release));
        let stem = args.file.file_stem().unwrap_or_default();
        profile_dir.join(stem)
    };
    let output_exe = if output_exe.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(&output_exe))
            .unwrap_or(output_exe)
    } else {
        output_exe
    };

    if verbosity > 0 {
        eprintln!("Running: {}", output_exe.display());
    }

    let status = std::process::Command::new(&output_exe)
        .status();

    match status {
        Ok(s) => {
            if s.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(s.code().unwrap_or(1) as u8)
            }
        }
        Err(e) => {
            eprintln!("Failed to run program: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Test command - discover and run tests
fn cmd_test(args: &TestArgs, verbosity: u8) -> ExitCode {
    // Determine file(s) to test
    let test_files = match determine_test_files(args) {
        Ok(files) => files,
        Err(code) => return code,
    };

    if test_files.is_empty() {
        eprintln!("No test files found.");
        return ExitCode::from(1);
    }

    if verbosity > 0 {
        eprintln!("Discovering tests in {} file(s)...", test_files.len());
    }

    // Discover tests in all files
    let mut all_tests = TestRegistry::new();
    let mut discovery_errors = false;

    for file_path in &test_files {
        match discover_tests_in_file(file_path, args, verbosity) {
            Ok(tests) => {
                for test in tests {
                    all_tests.register(test);
                }
            }
            Err(_) => {
                discovery_errors = true;
            }
        }
    }

    if discovery_errors && all_tests.is_empty() {
        eprintln!("Test discovery failed with errors.");
        return ExitCode::from(1);
    }

    // Apply filters
    let filtered_tests: Vec<&TestInfo> = all_tests
        .tests()
        .iter()
        .filter(|t| {
            // Apply name filter
            if let Some(ref pattern) = args.filter {
                if !t.full_path.contains(pattern) && !t.name.contains(pattern) {
                    return false;
                }
            }
            // Apply ignored filter
            if args.ignored {
                return t.config.ignore;
            }
            if !args.include_ignored && t.config.ignore {
                return false;
            }
            true
        })
        .collect();

    // Handle --list option
    if args.list {
        println!();
        for test in &filtered_tests {
            let suffix = if test.config.ignore {
                " (ignored)"
            } else if test.config.should_panic {
                " (should panic)"
            } else {
                ""
            };
            println!("  {}{}", test.full_path, suffix);
        }
        println!();
        println!("{} tests", filtered_tests.len());
        return ExitCode::SUCCESS;
    }

    if filtered_tests.is_empty() {
        if args.filter.is_some() {
            eprintln!("No tests matched the filter.");
        } else {
            eprintln!("No tests to run.");
        }
        return ExitCode::SUCCESS;
    }

    // Run tests
    let results = run_tests(&filtered_tests, &test_files, args, verbosity);

    // Print summary
    print_test_summary(&results);

    if results.failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Determine which files to test
fn determine_test_files(args: &TestArgs) -> Result<Vec<PathBuf>, ExitCode> {
    if let Some(ref file) = args.file {
        if file.is_file() {
            return Ok(vec![file.clone()]);
        } else if file.is_dir() {
            // Find all .blood files in directory
            return find_blood_files(file);
        } else {
            eprintln!("Error: '{}' is not a file or directory", file.display());
            return Err(ExitCode::from(1));
        }
    }

    // No file specified - look for project
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error getting current directory: {}", e);
            return Err(ExitCode::from(1));
        }
    };

    match project::discover_project_root(&cwd) {
        Some(project_root) => {
            // Find all .blood files in src/
            let src_dir = project_root.join("src");
            if src_dir.exists() {
                find_blood_files(&src_dir)
            } else {
                find_blood_files(&project_root)
            }
        }
        None => {
            eprintln!("Error: no Blood.toml found. Specify a file or run from a project directory.");
            Err(ExitCode::from(1))
        }
    }
}

/// Recursively find all .blood files in a directory
fn find_blood_files(dir: &PathBuf) -> Result<Vec<PathBuf>, ExitCode> {
    let mut files = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error reading directory '{}': {}", dir.display(), e);
            return Err(ExitCode::from(1));
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Error reading directory entry: {}", e);
                continue;
            }
        };

        let path = entry.path();
        if path.is_dir() {
            // Recurse into subdirectories
            match find_blood_files(&path) {
                Ok(mut subfiles) => files.append(&mut subfiles),
                Err(_) => continue, // Skip directories we can't read
            }
        } else if path.extension().map(|e| e == "blood").unwrap_or(false) {
            files.push(path);
        }
    }

    Ok(files)
}

/// Discover tests in a single file
fn discover_tests_in_file(
    file_path: &PathBuf,
    _args: &TestArgs,
    verbosity: u8,
) -> Result<Vec<TestInfo>, ExitCode> {
    let source = match read_source(file_path) {
        Ok(s) => s,
        Err(code) => return Err(code),
    };

    let path_str = file_path.to_string_lossy();
    let emitter = DiagnosticEmitter::new(&path_str, &source);

    // Parse the file
    let mut parser = bloodc::Parser::new(&source);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            if verbosity > 0 {
                eprintln!("Failed to parse '{}' for test discovery.", file_path.display());
            }
            return Err(ExitCode::from(1));
        }
    };

    // Get interner for attribute processing
    let interner = parser.take_interner();
    let attr_helper = AttrHelper::new(&interner);

    // Discover test functions
    let mut tests = Vec::new();

    // Calculate module path from file path
    let module_path = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    for decl in &program.declarations {
        if let bloodc::ast::Declaration::Function(fn_decl) = decl {
            let test_config = attr_helper.extract_test_config(&fn_decl.attrs);

            if test_config.is_test {
                // Get function name from interner
                let fn_name = interner
                    .resolve(fn_decl.name.node)
                    .unwrap_or("<unknown>")
                    .to_string();

                let test_info = TestInfo::new(
                    fn_name,
                    module_path.clone(),
                    test_config,
                    fn_decl.name.span,
                );

                tests.push(test_info);
            }
        }
    }

    if verbosity > 1 && !tests.is_empty() {
        eprintln!("  Found {} test(s) in '{}'", tests.len(), file_path.display());
    }

    Ok(tests)
}

/// Results from running tests
struct TestResults {
    passed: usize,
    failed: usize,
    ignored: usize,
    failures: Vec<TestFailure>,
}

/// Information about a failed test
struct TestFailure {
    name: String,
    message: String,
}

/// Run all discovered tests
fn run_tests(
    tests: &[&TestInfo],
    test_files: &[PathBuf],
    args: &TestArgs,
    verbosity: u8,
) -> TestResults {
    let mut results = TestResults {
        passed: 0,
        failed: 0,
        ignored: 0,
        failures: Vec::new(),
    };

    println!();
    println!("running {} test(s)", tests.len());

    for test in tests {
        // Handle ignored tests
        if test.config.ignore && !args.include_ignored && !args.ignored {
            results.ignored += 1;
            println!("test {} ... ignored", test.full_path);
            continue;
        }

        // Run the test
        let result = run_single_test(test, test_files, args, verbosity);

        match result {
            TestOutcome::Passed => {
                results.passed += 1;
                println!("test {} ... ok", test.full_path);
            }
            TestOutcome::Failed(msg) => {
                results.failed += 1;
                println!("test {} ... FAILED", test.full_path);
                results.failures.push(TestFailure {
                    name: test.full_path.clone(),
                    message: msg,
                });

                if args.fail_fast {
                    break;
                }
            }
            TestOutcome::Ignored => {
                results.ignored += 1;
                println!("test {} ... ignored", test.full_path);
            }
        }
    }

    results
}

/// Outcome of a single test
enum TestOutcome {
    Passed,
    Failed(String),
    #[allow(dead_code)]
    Ignored,
}

/// Run a single test
fn run_single_test(
    test: &TestInfo,
    test_files: &[PathBuf],
    args: &TestArgs,
    verbosity: u8,
) -> TestOutcome {
    // Find the file containing this test
    let test_file = test_files
        .iter()
        .find(|f| {
            f.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s == test.module_path || test.module_path.is_empty())
                .unwrap_or(false)
        })
        .or_else(|| test_files.first());

    let test_file = match test_file {
        Some(f) => f,
        None => {
            return TestOutcome::Failed("Could not find test file".to_string());
        }
    };

    // Generate a test harness that calls the test function
    let harness_source = generate_test_harness(test);

    // Create a temporary file for the harness
    let temp_dir = std::env::temp_dir();
    let harness_file = temp_dir.join(format!("blood_test_{}.blood", test.name));
    let harness_exe = temp_dir.join(format!("blood_test_{}", test.name));

    // Write harness - include original source and harness
    let original_source = match read_source(test_file) {
        Ok(s) => s,
        Err(_) => return TestOutcome::Failed("Could not read test file".to_string()),
    };

    let combined_source = format!(
        "// Original source\n{}\n\n// Test harness\n{}",
        original_source, harness_source
    );

    if let Err(e) = fs::write(&harness_file, &combined_source) {
        return TestOutcome::Failed(format!("Failed to write test harness: {}", e));
    }

    // Build the test harness
    let build_args = FileArgs {
        file: harness_file.clone(),
        debug: false,
        no_std: args.no_std,
        stdlib_path: args.stdlib_path.clone(),
        release: false,
        emit: Vec::new(),
        output: None,
        no_cache: false,
        build_dir: None,
    };

    // Suppress build output unless verbose
    let build_verbosity = if verbosity > 1 { verbosity } else { 0 };
    let build_result = cmd_build(&build_args, build_verbosity, false);

    // Clean up harness source
    let _ = fs::remove_file(&harness_file);

    if build_result != ExitCode::SUCCESS {
        // Clean up build artifacts
        let _ = fs::remove_file(&harness_exe);
        let _ = fs::remove_dir_all(harness_file.with_extension("blood_objs"));
        return TestOutcome::Failed("Test compilation failed".to_string());
    }

    // Run the test
    let output = std::process::Command::new(&harness_exe)
        .output();

    // Clean up executable and object directory
    let _ = fs::remove_file(&harness_exe);
    let _ = fs::remove_dir_all(harness_file.with_extension("blood_objs"));

    match output {
        Ok(output) => {
            let exited_successfully = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Show output if requested
            if args.show_output && (!stdout.is_empty() || !stderr.is_empty()) {
                if !stdout.is_empty() {
                    println!("---- {} stdout ----", test.name);
                    println!("{}", stdout);
                }
                if !stderr.is_empty() {
                    println!("---- {} stderr ----", test.name);
                    println!("{}", stderr);
                }
            }

            // Check for expected panic
            if test.config.should_panic {
                if exited_successfully {
                    return TestOutcome::Failed("Test should have panicked but didn't".to_string());
                }

                // If there's an expected message, check for it
                if let Some(ref expected) = test.config.expected_panic {
                    let combined_output = format!("{}{}", stdout, stderr);
                    if !combined_output.contains(expected) {
                        return TestOutcome::Failed(format!(
                            "Panic message '{}' not found in output",
                            expected
                        ));
                    }
                }

                return TestOutcome::Passed;
            }

            if exited_successfully {
                TestOutcome::Passed
            } else {
                let mut message = format!("Test exited with code {:?}", output.status.code());
                if !stderr.is_empty() {
                    message.push_str(&format!("\nstderr:\n{}", stderr));
                }
                TestOutcome::Failed(message)
            }
        }
        Err(e) => TestOutcome::Failed(format!("Failed to run test: {}", e)),
    }
}

/// Generate a test harness that calls the test function
fn generate_test_harness(test: &TestInfo) -> String {
    // The harness just needs to call the test function from main
    // The test function is already in scope from the original source
    format!(
        r#"
// Generated test harness for {}
fn main() {{
    {}();
}}
"#,
        test.name, test.name
    )
}

/// Print test summary
fn print_test_summary(results: &TestResults) {
    println!();

    // Print failures
    if !results.failures.is_empty() {
        println!("failures:");
        println!();
        for failure in &results.failures {
            println!("---- {} ----", failure.name);
            println!("{}", failure.message);
            println!();
        }
        println!("failures:");
        for failure in &results.failures {
            println!("    {}", failure.name);
        }
        println!();
    }

    // Print summary line
    let status = if results.failed > 0 {
        "FAILED"
    } else {
        "ok"
    };

    println!(
        "test result: {}. {} passed; {} failed; {} ignored",
        status, results.passed, results.failed, results.ignored
    );
}
