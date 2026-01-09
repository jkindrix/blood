//! # Blood Compiler Library
//!
//! The Blood programming language compiler core library.
//!
//! Blood is a systems programming language designed for environments where
//! failure is not an option. It synthesizes five cutting-edge innovations:
//!
//! 1. **Content-Addressed Code** - Code identity via BLAKE3-256 hashes
//! 2. **Generational Memory Safety** - 128-bit fat pointers, no GC
//! 3. **Mutable Value Semantics** - Simple ownership without borrow checker complexity
//! 4. **Algebraic Effects** - All side effects typed and composable
//! 5. **Multiple Dispatch** - Type-stable open extensibility
//!
//! ## Compiler Pipeline
//!
//! ```text
//! Source -> Lexer -> Parser -> AST -> HIR -> Type Check -> MIR -> Codegen -> LLVM
//! ```
//!
//! ## Quick Start
//!
//! ### Lexing Source Code
//!
//! ```rust
//! use bloodc::Lexer;
//!
//! let source = "fn main() { 42 }";
//! let lexer = Lexer::new(source);
//!
//! for token in lexer {
//!     println!("{:?}", token.kind);
//! }
//! ```
//!
//! ### Parsing Source Code
//!
//! ```rust
//! use bloodc::Parser;
//!
//! let source = r#"
//! fn hello(name: String) -> () {
//!     print("Hello, " + name);
//! }
//! "#;
//!
//! let mut parser = Parser::new(source);
//! match parser.parse_program() {
//!     Ok(program) => {
//!         println!("Parsed {} declarations", program.declarations.len());
//!     }
//!     Err(errors) => {
//!         for error in errors {
//!             eprintln!("Error: {}", error.message);
//!         }
//!     }
//! }
//! ```
//!
//! ### Error Handling
//!
//! The compiler provides structured diagnostics with error codes:
//!
//! ```rust
//! use bloodc::{Parser, diagnostics::DiagnosticEmitter};
//!
//! let source = "fn foo( {}";  // Malformed input
//! let mut parser = Parser::new(source);
//!
//! if let Err(errors) = parser.parse_program() {
//!     let emitter = DiagnosticEmitter::new("example.blood", source);
//!     for error in &errors {
//!         emitter.emit(error);
//!     }
//! }
//! ```
//!
//! ## Module Overview
//!
//! - [`arena`] - Arena allocation for efficient memory management
//! - [`ast`] - Abstract Syntax Tree types
//! - [`diagnostics`] - Error reporting infrastructure
//! - [`hir`] - High-level Intermediate Representation (Phase 1+)
//! - [`lexer`] - Tokenization (lexical analysis)
//! - [`parser`] - Parsing (syntax analysis)
//! - [`span`] - Source location tracking
//! - [`syntax_kind`] - Syntax kinds for future CST support
//! - [`trivia`] - Trivia preservation for lossless syntax trees
//! - [`typeck`] - Type checking and inference (Phase 1+)
//! - [`codegen`] - LLVM code generation (Phase 1+)
//! - [`effects`] - Algebraic effects system with evidence passing (Phase 2+)

pub mod arena;
pub mod ast;
pub mod codegen;
pub mod diagnostics;
pub mod effects;
pub mod hir;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod syntax_kind;
pub mod trivia;
pub mod typeck;

// Re-export commonly used types
pub use diagnostics::{Diagnostic, DiagnosticEmitter, DiagnosticKind, ErrorCode};
pub use lexer::{Lexer, Token, TokenKind};
pub use parser::Parser;
pub use span::{Span, Spanned};
pub use syntax_kind::SyntaxKind;
