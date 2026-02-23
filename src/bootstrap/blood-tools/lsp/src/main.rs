//! Blood Language Server Binary
//!
//! Run with: `blood-lsp`
//!
//! The server communicates via stdin/stdout using the Language Server Protocol.

use blood_lsp::run_server;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    run_server().await
}
