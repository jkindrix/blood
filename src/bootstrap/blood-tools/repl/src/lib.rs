//! Blood REPL - Interactive exploration environment for Blood.
//!
//! The REPL provides an interactive way to explore the Blood language,
//! test expressions, and learn about types and effects.
//!
//! # Design Decision: Parse-Only Mode
//!
//! The REPL operates in **parse-only mode** by design. Expressions are parsed
//! and validated but not evaluated. Full evaluation requires JIT pipeline
//! integration, which is a self-hosted compiler concern. Use `blood run <file>`
//! to execute Blood programs.
//!
//! # Features
//!
//! - Expression parsing and validation
//! - Variable binding tracking (parse-only)
//! - Special commands (`:help`, `:type`, `:clear`, `:quit`)
//! - Type information display
//! - Parse and type error feedback

use std::collections::HashMap;
use std::io;

use bloodc::Parser;
use colored::Colorize;
use thiserror::Error;

/// REPL errors.
#[derive(Error, Debug)]
pub enum ReplError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Evaluation error: {0}")]
    EvalError(String),

    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
}

/// Result type for REPL operations.
pub type ReplResult<T> = Result<T, ReplError>;

/// REPL command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Display help.
    Help,
    /// Show type of expression.
    Type(String),
    /// Clear all bindings.
    Clear,
    /// Quit the REPL.
    Quit,
    /// Load a file.
    Load(String),
    /// Show current bindings.
    Env,
    /// Regular Blood code.
    Code(String),
}

impl Command {
    /// Parse a command from input.
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();

        if trimmed.starts_with(':') {
            let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
            let cmd = parts[0].to_lowercase();
            let arg = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

            match cmd.as_str() {
                "help" | "h" | "?" => Command::Help,
                "type" | "t" => Command::Type(arg),
                "clear" | "c" => Command::Clear,
                "quit" | "q" | "exit" => Command::Quit,
                "load" | "l" => Command::Load(arg),
                "env" | "e" => Command::Env,
                _ => {
                    // Unknown command, treat as code (might be a label)
                    Command::Code(input.to_string())
                }
            }
        } else {
            Command::Code(input.to_string())
        }
    }
}

/// Variable binding in the REPL environment.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Name of the binding.
    pub name: String,
    /// Type of the binding (as string).
    pub type_str: String,
    /// The source code that defined this binding.
    pub source: String,
}

/// Definition in the REPL environment.
#[derive(Debug, Clone)]
pub struct Definition {
    /// Name of the definition.
    pub name: String,
    /// Kind of definition (function, struct, enum, effect, handler).
    pub kind: String,
    /// The source code of the definition.
    pub source: String,
}

/// REPL session state.
pub struct ReplSession {
    /// Variable bindings.
    bindings: HashMap<String, Binding>,
    /// Definitions (types, effects, functions, handlers).
    definitions: HashMap<String, Definition>,
    /// Line counter.
    line_count: usize,
    /// Whether to show types after evaluation.
    show_types: bool,
}

impl ReplSession {
    /// Create a new REPL session.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            definitions: HashMap::new(),
            line_count: 0,
            show_types: true,
        }
    }

    /// Execute a command and return the output.
    pub fn execute(&mut self, input: &str) -> ReplResult<String> {
        let command = Command::parse(input);

        match command {
            Command::Help => Ok(self.help_text()),
            Command::Type(expr) => self.show_type(&expr),
            Command::Clear => {
                self.clear();
                Ok("Environment cleared.".to_string())
            }
            Command::Quit => Ok("quit".to_string()),
            Command::Load(path) => self.load_file(&path),
            Command::Env => Ok(self.show_env()),
            Command::Code(code) => self.eval_code(&code),
        }
    }

    /// Generate help text.
    fn help_text(&self) -> String {
        let mut help = String::new();
        help.push_str(&"Blood REPL Commands:\n".bold().to_string());
        help.push_str(&format!("  {}  - Show this help\n", ":help, :h, :?".cyan()));
        help.push_str(&format!("  {}  - Show type of expression\n", ":type <expr>".cyan()));
        help.push_str(&format!("  {}  - Load a Blood file\n", ":load <file>".cyan()));
        help.push_str(&format!("  {}  - Show current environment\n", ":env".cyan()));
        help.push_str(&format!("  {}  - Clear all bindings\n", ":clear".cyan()));
        help.push_str(&format!("  {}  - Exit the REPL\n", ":quit, :q".cyan()));
        help.push_str("\n");
        help.push_str(&"Examples:\n".bold().to_string());
        help.push_str("  let x = 42           // Bind a variable\n");
        help.push_str("  x + 1                // Evaluate an expression\n");
        help.push_str("  fn add(a, b) = a + b // Define a function\n");
        help.push_str("  :type x              // Show type of x\n");
        help
    }

    /// Show the type of an expression.
    fn show_type(&self, expr: &str) -> ReplResult<String> {
        if expr.is_empty() {
            return Err(ReplError::ParseError("No expression provided".to_string()));
        }

        // Wrap the expression to make it parseable
        let source = self.wrap_for_type_check(expr);

        let mut parser = Parser::new(&source);
        match parser.parse_program() {
            Ok(_ast) => {
                // For now, just acknowledge the expression is valid
                // Full type inference would require running the type checker
                Ok(format!("{}: {}", expr, "inferred".dimmed()))
            }
            Err(e) => Err(ReplError::ParseError(format!("{:?}", e))),
        }
    }

    /// Load a file into the REPL environment.
    fn load_file(&mut self, path: &str) -> ReplResult<String> {
        if path.is_empty() {
            return Err(ReplError::IoError(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No file path provided",
            )));
        }

        let content = std::fs::read_to_string(path)?;

        // Parse the file
        let mut parser = Parser::new(&content);
        match parser.parse_program() {
            Ok(ast) => {
                // Count items
                let item_count = ast.declarations.len();

                // Store the whole file content
                for line in content.lines() {
                    let trimmed = line.trim();

                    // Extract definitions
                    if let Some(name) = extract_definition_name(trimmed) {
                        let kind = if trimmed.starts_with("fn ") {
                            "function"
                        } else if trimmed.starts_with("struct ") {
                            "struct"
                        } else if trimmed.starts_with("enum ") {
                            "enum"
                        } else if trimmed.starts_with("effect ") {
                            "effect"
                        } else if trimmed.starts_with("handler ") {
                            "handler"
                        } else if trimmed.starts_with("trait ") {
                            "trait"
                        } else {
                            continue;
                        };

                        self.definitions.insert(name.clone(), Definition {
                            name: name.clone(),
                            kind: kind.to_string(),
                            source: trimmed.to_string(),
                        });
                    }
                }

                Ok(format!("Loaded {} items from {}", item_count, path.green()))
            }
            Err(e) => Err(ReplError::ParseError(format!("Failed to parse {}: {:?}", path, e))),
        }
    }

    /// Show current environment.
    fn show_env(&self) -> String {
        let mut output = String::new();

        if self.bindings.is_empty() && self.definitions.is_empty() {
            return "Environment is empty.".dimmed().to_string();
        }

        if !self.bindings.is_empty() {
            output.push_str(&"Variables:\n".bold().to_string());
            for (name, binding) in &self.bindings {
                output.push_str(&format!("  {}: {}\n", name.cyan(), binding.type_str));
            }
        }

        // Group definitions by kind
        let mut functions = Vec::new();
        let mut types = Vec::new();
        let mut effects = Vec::new();

        for (name, def) in &self.definitions {
            match def.kind.as_str() {
                "function" => functions.push(name.as_str()),
                "struct" | "enum" | "trait" => types.push(name.as_str()),
                "effect" | "handler" => effects.push(name.as_str()),
                _ => {}
            }
        }

        if !functions.is_empty() {
            output.push_str(&"Functions:\n".bold().to_string());
            for name in functions {
                output.push_str(&format!("  {}\n", name.cyan()));
            }
        }

        if !types.is_empty() {
            output.push_str(&"Types:\n".bold().to_string());
            for name in types {
                output.push_str(&format!("  {}\n", name.cyan()));
            }
        }

        if !effects.is_empty() {
            output.push_str(&"Effects:\n".bold().to_string());
            for name in effects {
                output.push_str(&format!("  {}\n", name.cyan()));
            }
        }

        output
    }

    /// Clear all bindings.
    fn clear(&mut self) {
        self.bindings.clear();
        self.definitions.clear();
        self.line_count = 0;
    }

    /// Evaluate Blood code.
    fn eval_code(&mut self, code: &str) -> ReplResult<String> {
        if code.trim().is_empty() {
            return Ok(String::new());
        }

        self.line_count += 1;

        // Try to parse as different kinds of input
        // First, try as a complete item (function, struct, etc.)
        if self.try_parse_definition(code)? {
            return Ok("Defined.".dimmed().to_string());
        }

        // Then try as a statement or expression
        self.eval_expression(code)
    }

    /// Try to parse input as a definition (function, struct, enum, effect).
    fn try_parse_definition(&mut self, code: &str) -> ReplResult<bool> {
        let trimmed = code.trim();

        // Check if this looks like a definition
        if !trimmed.starts_with("fn ") &&
           !trimmed.starts_with("struct ") &&
           !trimmed.starts_with("enum ") &&
           !trimmed.starts_with("effect ") &&
           !trimmed.starts_with("handler ") &&
           !trimmed.starts_with("trait ") &&
           !trimmed.starts_with("impl ") &&
           !trimmed.starts_with("pub ") {
            return Ok(false);
        }

        let mut parser = Parser::new(code);
        match parser.parse_program() {
            Ok(ast) => {
                if ast.declarations.is_empty() {
                    return Ok(false);
                }

                // Extract definition name
                if let Some(name) = extract_definition_name(trimmed) {
                    let kind = if trimmed.starts_with("fn ") {
                        "function"
                    } else if trimmed.starts_with("struct ") {
                        "struct"
                    } else if trimmed.starts_with("enum ") {
                        "enum"
                    } else if trimmed.starts_with("effect ") {
                        "effect"
                    } else if trimmed.starts_with("handler ") {
                        "handler"
                    } else if trimmed.starts_with("trait ") {
                        "trait"
                    } else {
                        "unknown"
                    };

                    self.definitions.insert(name.clone(), Definition {
                        name,
                        kind: kind.to_string(),
                        source: code.to_string(),
                    });
                }

                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Evaluate an expression or statement.
    fn eval_expression(&mut self, code: &str) -> ReplResult<String> {
        // Wrap the code in a function for parsing
        let source = self.wrap_for_evaluation(code);

        let mut parser = Parser::new(&source);
        match parser.parse_program() {
            Ok(_ast) => {
                // Check if this is a let binding
                let trimmed = code.trim();
                if trimmed.starts_with("let ") {
                    // Extract variable name and store binding
                    if let Some(eq_pos) = trimmed.find('=') {
                        let name_part = &trimmed[4..eq_pos].trim();
                        // Handle pattern: let name: Type = value or let name = value
                        let name = name_part.split(':').next().unwrap_or(name_part).trim();
                        self.bindings.insert(name.to_string(), Binding {
                            name: name.to_string(),
                            type_str: "inferred".to_string(),
                            source: code.to_string(),
                        });
                        return Ok(format!("{} = ...", name.cyan()));
                    }
                }

                // Design decision: REPL operates in parse-only mode (see module doc)
                Ok(format!("{}", "(parse mode: expression is valid. Use 'blood run <file>' to execute.)".dimmed()))
            }
            Err(e) => {
                Err(ReplError::ParseError(format!("{:?}", e)))
            }
        }
    }

    /// Wrap code for type checking.
    fn wrap_for_type_check(&self, expr: &str) -> String {
        let mut source = String::new();

        // Add existing definitions
        for (_, def) in &self.definitions {
            source.push_str(&def.source);
            source.push('\n');
        }

        // Wrap expression in a function
        source.push_str("fn __repl_type_check__() {\n");

        // Add bindings as let statements
        for (_, binding) in &self.bindings {
            source.push_str(&format!("    {};\n", binding.source));
        }

        source.push_str(&format!("    {}\n", expr));
        source.push_str("}\n");

        source
    }

    /// Wrap code for evaluation.
    fn wrap_for_evaluation(&self, code: &str) -> String {
        let mut source = String::new();

        // Add existing definitions
        for (_, def) in &self.definitions {
            source.push_str(&def.source);
            source.push('\n');
        }

        // Wrap in a main function
        source.push_str("fn __repl_eval__() {\n");

        // Add existing bindings
        for (_, binding) in &self.bindings {
            source.push_str(&format!("    {};\n", binding.source));
        }

        // Add the code with appropriate termination
        let code_trimmed = code.trim();
        if code_trimmed.ends_with(';') || code_trimmed.ends_with('}') {
            source.push_str(&format!("    {}\n", code));
        } else {
            source.push_str(&format!("    {};\n", code));
        }
        source.push_str("}\n");

        source
    }

    /// Get the line count.
    pub fn line_count(&self) -> usize {
        self.line_count
    }
}

impl Default for ReplSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the name from a definition line.
fn extract_definition_name(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // Remove visibility modifier
    let trimmed = if trimmed.starts_with("pub ") {
        &trimmed[4..]
    } else {
        trimmed
    };

    // Extract name based on definition type
    let prefix_len = if trimmed.starts_with("fn ") {
        3
    } else if trimmed.starts_with("struct ") {
        7
    } else if trimmed.starts_with("enum ") {
        5
    } else if trimmed.starts_with("effect ") {
        7
    } else if trimmed.starts_with("handler ") {
        8
    } else if trimmed.starts_with("trait ") {
        6
    } else if trimmed.starts_with("impl ") {
        5
    } else {
        return None;
    };

    let rest = &trimmed[prefix_len..];

    // Find where the name ends (at '<', '(', '{', ':' or whitespace)
    let name_end = rest
        .find(|c: char| c == '<' || c == '(' || c == '{' || c == ':' || c.is_whitespace())
        .unwrap_or(rest.len());

    let name = rest[..name_end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_parse_help() {
        assert_eq!(Command::parse(":help"), Command::Help);
        assert_eq!(Command::parse(":h"), Command::Help);
        assert_eq!(Command::parse(":?"), Command::Help);
    }

    #[test]
    fn test_command_parse_quit() {
        assert_eq!(Command::parse(":quit"), Command::Quit);
        assert_eq!(Command::parse(":q"), Command::Quit);
        assert_eq!(Command::parse(":exit"), Command::Quit);
    }

    #[test]
    fn test_command_parse_type() {
        assert_eq!(Command::parse(":type x"), Command::Type("x".to_string()));
        assert_eq!(Command::parse(":t 1 + 2"), Command::Type("1 + 2".to_string()));
    }

    #[test]
    fn test_command_parse_code() {
        assert_eq!(Command::parse("let x = 42"), Command::Code("let x = 42".to_string()));
        assert_eq!(Command::parse("x + 1"), Command::Code("x + 1".to_string()));
    }

    #[test]
    fn test_session_help() {
        let mut session = ReplSession::new();
        let result = session.execute(":help").unwrap();
        assert!(result.contains("Blood REPL Commands"));
        assert!(result.contains(":quit"));
    }

    #[test]
    fn test_session_clear() {
        let mut session = ReplSession::new();
        session.bindings.insert("x".to_string(), Binding {
            name: "x".to_string(),
            type_str: "i32".to_string(),
            source: "let x = 42".to_string(),
        });

        let result = session.execute(":clear").unwrap();
        assert!(result.contains("cleared"));
        assert!(session.bindings.is_empty());
    }

    #[test]
    fn test_session_env_empty() {
        let session = ReplSession::new();
        let result = session.show_env();
        assert!(result.contains("empty"));
    }

    #[test]
    fn test_extract_definition_name() {
        assert_eq!(extract_definition_name("fn foo() {}"), Some("foo".to_string()));
        assert_eq!(extract_definition_name("fn bar(x: i32) -> i32 {}"), Some("bar".to_string()));
        assert_eq!(extract_definition_name("struct Point { x: i32 }"), Some("Point".to_string()));
        assert_eq!(extract_definition_name("enum Option<T> { Some(T), None }"), Some("Option".to_string()));
        assert_eq!(extract_definition_name("effect State<T> {}"), Some("State".to_string()));
        assert_eq!(extract_definition_name("handler MyHandler for State<i32> {}"), Some("MyHandler".to_string()));
        assert_eq!(extract_definition_name("pub fn public_fn() {}"), Some("public_fn".to_string()));
    }

    #[test]
    fn test_let_binding() {
        let mut session = ReplSession::new();
        let result = session.eval_code("let x = 42");
        assert!(result.is_ok(), "let binding should parse successfully");
        assert!(session.bindings.contains_key("x"), "x should be in bindings");
    }
}
