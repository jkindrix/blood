//! Blood Language Server Protocol Implementation
//!
//! This crate provides an LSP server for the Blood programming language,
//! enabling IDE features like:
//!
//! - Syntax highlighting (semantic tokens)
//! - Error diagnostics
//! - Go to definition
//! - Find references
//! - Hover information
//! - Completion suggestions
//! - Code actions and refactoring
//! - Effect signature display
//!
//! # Architecture
//!
//! The LSP server uses tower-lsp for the protocol handling and integrates
//! with bloodc for parsing, type checking, and semantic analysis.
//!
//! ```text
//! ┌─────────┐    ┌──────────────┐    ┌─────────┐
//! │  IDE    │◄──►│  blood-lsp   │◄──►│ bloodc  │
//! │ Client  │    │   Server     │    │Compiler │
//! └─────────┘    └──────────────┘    └─────────┘
//! ```

pub mod analysis;
pub mod backend;
pub mod capabilities;
pub mod diagnostics;
pub mod document;
pub mod handlers;
pub mod semantic_tokens;

use tower_lsp::{LspService, Server};
use tracing::info;

pub use backend::BloodLanguageServer;

/// Runs the Blood language server.
///
/// This is the main entry point for the LSP binary.
pub async fn run_server() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| BloodLanguageServer::new(client));

    info!("Starting Blood language server");
    Server::new(stdin, stdout, socket).serve(service).await;
    info!("Blood language server stopped");

    Ok(())
}
