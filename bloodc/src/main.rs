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
//!   lex    Tokenize source and display token stream
//!   parse  Parse source and display AST
//!   check  Type-check source (parsing only in Phase 0)
//!   build  Compile to executable (not yet implemented)
//!   run    Compile and run (not yet implemented)
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

use bloodc::diagnostics::DiagnosticEmitter;
use bloodc::{Lexer, TokenKind};
use bloodc::codegen;
use bloodc::mir;
use bloodc::content::ContentHash;

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
}

#[derive(Subcommand)]
enum Commands {
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

    /// Type-check source file
    ///
    /// Currently only performs parsing (Phase 0). Type checking will be
    /// implemented in Phase 1.
    Check(FileArgs),

    /// Compile source file to executable
    ///
    /// Not yet implemented. Will be available in Phase 1.
    Build(FileArgs),

    /// Compile and run source file
    ///
    /// Not yet implemented. Will be available in Phase 1.
    Run(FileArgs),
}

#[derive(Args)]
struct FileArgs {
    /// Source file to process
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Dump additional debug information
    #[arg(short, long)]
    debug: bool,
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

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Apply color settings
    cli.color.apply();

    // Create verbosity level
    let verbosity = if cli.quiet { 0 } else { cli.verbose + 1 };

    match cli.command {
        Commands::Lex(args) => cmd_lex(&args, verbosity),
        Commands::Parse(args) => cmd_parse(&args, verbosity),
        Commands::Check(args) => cmd_check(&args, verbosity),
        Commands::Build(args) => cmd_build(&args, verbosity),
        Commands::Run(args) => cmd_run(&args, verbosity),
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

    match result {
        Ok(program) => {
            if verbosity > 0 {
                eprintln!(
                    "Parsed {} declarations.",
                    program.declarations.len()
                );
            }
            // Type checking not yet implemented
            println!("info: Parsing successful.");
            println!("info: Type checking not yet implemented (Phase 0).");
            println!("hint: See ROADMAP.md for Phase 1 implementation plans.");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Checking failed with {} error(s).", errors.len());
            ExitCode::from(1)
        }
    }
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
            // Check in target/release/../runtime (for development)
            let runtime_dev = exe_dir.join("../../runtime/runtime.o");
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

            // Check in parent target directory
            if let Some(target_dir) = exe_dir.parent() {
                for name in &rust_runtime_names {
                    for profile in &["debug", "release"] {
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

/// Build command - compile to executable
fn cmd_build(args: &FileArgs, verbosity: u8) -> ExitCode {
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
    let mut parser = bloodc::Parser::new(&source);
    let program = match parser.parse_program() {
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

    if verbosity > 0 {
        eprintln!("Parsed {} declarations.", program.declarations.len());
    }

    // Type check and lower to HIR
    let mut ctx = bloodc::typeck::TypeContext::new(&source, interner);
    if let Err(errors) = ctx.resolve_program(&program) {
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Build failed: type errors.");
        return ExitCode::from(1);
    }

    // Type-check all function bodies
    if let Err(errors) = ctx.check_all_bodies() {
        for error in &errors {
            emitter.emit(error);
        }
        eprintln!("Build failed: type errors.");
        return ExitCode::from(1);
    }

    // Generate HIR
    let hir_crate = ctx.into_hir();

    if verbosity > 0 {
        eprintln!("Type checking passed. {} items.", hir_crate.items.len());
    }

    // Compute content hashes for definitions (Phase 4 integration point)
    // This is a simplified implementation - full canonicalization would use
    // de Bruijn indices and proper AST normalization.
    if verbosity > 1 {
        eprintln!("Computing content hashes...");
        for (&def_id, item) in &hir_crate.items {
            // For now, hash the debug representation of the item
            // A proper implementation would use canonical AST serialization
            let item_repr = format!("{:?}", item);
            let hash = ContentHash::compute(item_repr.as_bytes());
            eprintln!("  {:?}: {} ({})", def_id, hash.short_display(), item.name);
        }
    }

    // Lower to MIR (Phase 3 integration point)
    let mut mir_lowering = mir::MirLowering::new(&hir_crate);
    match mir_lowering.lower_crate() {
        Ok(mir_bodies) => {
            if verbosity > 1 {
                eprintln!("MIR lowering passed. {} function bodies.", mir_bodies.len());
            }

            // Run escape analysis on MIR bodies
            let escape_results: std::collections::HashMap<_, _> = mir_bodies.iter()
                .map(|(&def_id, body)| {
                    let mut analyzer = mir::EscapeAnalyzer::new();
                    (def_id, analyzer.analyze(body))
                })
                .collect();

            if verbosity > 1 {
                eprintln!("Escape analysis complete. {} functions analyzed.", escape_results.len());
            }

            // Note: MIR bodies and escape_results are computed but not yet used by codegen.
            // Codegen still operates on HIR directly.
            // TODO: Wire MIR-based codegen in a future iteration.
            let _ = (mir_bodies, escape_results); // Suppress unused warnings
        }
        Err(errors) => {
            // MIR lowering errors are non-fatal for now - fall back to HIR codegen
            if verbosity > 0 {
                for error in &errors {
                    emitter.emit(error);
                }
                eprintln!("Warning: MIR lowering failed, using HIR codegen.");
            }
        }
    }

    // Generate LLVM IR
    match codegen::compile_to_ir(&hir_crate) {
        Ok(ir) => {
            if args.debug {
                println!("{}", ir);
            }

            // Determine output path
            let output_obj = args.file.with_extension("o");
            let output_exe = args.file.with_extension("");

            // Compile to object file
            if let Err(errors) = codegen::compile_to_object(&hir_crate, &output_obj) {
                for error in &errors {
                    emitter.emit(error);
                }
                eprintln!("Build failed: codegen errors.");
                return ExitCode::from(1);
            }

            if verbosity > 0 {
                eprintln!("Generated object file: {}", output_obj.display());
            }

            // Link with runtimes
            // C runtime is required (provides main entry point)
            // Rust runtime is optional (provides advanced features like fibers, channels)
            let (c_runtime, rust_runtime) = find_runtime_paths();

            let mut link_cmd = std::process::Command::new("cc");
            link_cmd.arg(&output_obj);
            link_cmd.arg(&c_runtime);

            // Add Rust runtime if available
            if let Some(rust_rt) = &rust_runtime {
                link_cmd.arg(rust_rt);
                if verbosity > 0 {
                    eprintln!("Linking with Rust runtime: {}", rust_rt.display());
                }
            }

            link_cmd.arg("-o").arg(&output_exe);

            let status = link_cmd.status();

            match status {
                Ok(s) if s.success() => {
                    if verbosity > 0 {
                        eprintln!("Linked executable: {}", output_exe.display());
                    }
                    println!("Build successful: {}", output_exe.display());
                    ExitCode::SUCCESS
                }
                Ok(_) => {
                    eprintln!("Linking failed.");
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("Failed to run linker: {}", e);
                    ExitCode::from(1)
                }
            }
        }
        Err(errors) => {
            for error in &errors {
                emitter.emit(error);
            }
            eprintln!("Build failed: codegen errors.");
            ExitCode::from(1)
        }
    }
}

/// Run command - compile and execute
fn cmd_run(args: &FileArgs, verbosity: u8) -> ExitCode {
    // First build
    let build_result = cmd_build(args, verbosity);
    if build_result != ExitCode::SUCCESS {
        return build_result;
    }

    // Then run
    let output_exe = args.file.with_extension("");

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
