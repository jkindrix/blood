//! Blood REPL - Interactive exploration environment for Blood.
//!
//! # Usage
//!
//! ```text
//! blood-repl [OPTIONS]
//!
//! Options:
//!   -v, --verbose  Increase verbosity
//!   -h, --help     Print help information
//!   -V, --version  Print version information
//! ```

use std::process::ExitCode;

use blood_repl::ReplSession;
use colored::Colorize;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Editor, Result as RlResult};

/// Blood REPL prompt.
fn prompt(session: &ReplSession) -> String {
    format!("blood[{}]> ", session.line_count())
}

/// Print welcome banner.
fn print_banner() {
    println!("{}", "╔════════════════════════════════════════════════════╗".bright_blue());
    println!("{}", "║         Blood Interactive REPL v0.1.0              ║".bright_blue());
    println!("{}", "║  Type :help for commands, :quit to exit           ║".bright_blue());
    println!("{}", "╚════════════════════════════════════════════════════╝".bright_blue());
    println!();
}

/// Run the REPL.
fn run_repl() -> RlResult<ExitCode> {
    print_banner();

    // Configure rustyline
    let config = Config::builder()
        .history_ignore_space(true)
        .history_ignore_dups(true)?
        .build();

    let mut rl: Editor<(), DefaultHistory> = Editor::with_config(config)?;

    // Load history if it exists
    let history_path = dirs::data_dir()
        .map(|p| p.join("blood").join("repl_history"))
        .unwrap_or_else(|| std::path::PathBuf::from(".blood_repl_history"));

    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.load_history(&history_path);

    let mut session = ReplSession::new();
    let mut multiline_buffer = String::new();
    let mut in_multiline = false;

    loop {
        let prompt_str = if in_multiline {
            "...> ".to_string()
        } else {
            prompt(&session)
        };

        match rl.readline(&prompt_str) {
            Ok(line) => {
                // Handle multiline input
                let trimmed = line.trim();

                // Check for multiline continuations
                if trimmed.ends_with('{') || trimmed.ends_with('\\') ||
                   (in_multiline && !is_complete(&multiline_buffer, &line)) {
                    multiline_buffer.push_str(&line);
                    multiline_buffer.push('\n');
                    in_multiline = true;
                    continue;
                }

                // Complete the input
                let input = if in_multiline {
                    multiline_buffer.push_str(&line);
                    let complete = multiline_buffer.clone();
                    multiline_buffer.clear();
                    in_multiline = false;
                    complete
                } else {
                    line.clone()
                };

                // Skip empty lines
                if input.trim().is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(&input);

                // Execute command
                match session.execute(&input) {
                    Ok(result) => {
                        if result == "quit" {
                            break;
                        }
                        if !result.is_empty() {
                            println!("{}", result);
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {}", "Error".red().bold(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C - clear current input
                if in_multiline {
                    multiline_buffer.clear();
                    in_multiline = false;
                    println!("{}", "Input cancelled.".yellow());
                } else {
                    println!("{}", "Use :quit or Ctrl-D to exit.".yellow());
                }
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D - exit
                println!("{}", "Goodbye!".green());
                break;
            }
            Err(err) => {
                eprintln!("{}: {:?}", "Error".red().bold(), err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_path);

    Ok(ExitCode::SUCCESS)
}

/// Check if a multiline input is complete.
fn is_complete(buffer: &str, current_line: &str) -> bool {
    let combined = format!("{}{}", buffer, current_line);

    // Count braces
    let open_braces = combined.matches('{').count();
    let close_braces = combined.matches('}').count();

    // Count parentheses
    let open_parens = combined.matches('(').count();
    let close_parens = combined.matches(')').count();

    // Count brackets
    let open_brackets = combined.matches('[').count();
    let close_brackets = combined.matches(']').count();

    // Input is complete if all brackets are balanced
    open_braces == close_braces &&
    open_parens == close_parens &&
    open_brackets == close_brackets &&
    !current_line.trim().ends_with('\\')
}

fn main() -> ExitCode {
    match run_repl() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("REPL error: {}", e);
            ExitCode::from(1)
        }
    }
}
