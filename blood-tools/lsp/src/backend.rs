//! LSP Backend Implementation
//!
//! The main language server that handles LSP requests and notifications.

use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{debug, info};

use crate::analysis::{DefinitionProvider, HoverProvider};
use crate::capabilities;
use crate::diagnostics::DiagnosticEngine;
use crate::document::Document;
use crate::semantic_tokens::SemanticTokensProvider;

/// The Blood language server backend.
pub struct BloodLanguageServer {
    /// The LSP client for sending notifications and requests.
    client: Client,
    /// Open documents indexed by URI.
    documents: DashMap<Url, Document>,
    /// Diagnostic engine for error reporting.
    diagnostics: DiagnosticEngine,
    /// Semantic tokens provider.
    semantic_tokens: SemanticTokensProvider,
    /// Hover information provider.
    hover_provider: HoverProvider,
    /// Go-to-definition provider.
    definition_provider: DefinitionProvider,
}

impl BloodLanguageServer {
    /// Creates a new language server instance.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: DashMap::new(),
            diagnostics: DiagnosticEngine::new(),
            semantic_tokens: SemanticTokensProvider::new(),
            hover_provider: HoverProvider::new(),
            definition_provider: DefinitionProvider::new(),
        }
    }

    /// Validates a document and publishes diagnostics.
    async fn validate_document(&self, uri: &Url) {
        let Some(doc) = self.documents.get(uri) else {
            return;
        };

        let diagnostics = self.diagnostics.check(&doc);

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, Some(doc.version()))
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for BloodLanguageServer {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        info!("Initializing Blood language server");

        Ok(InitializeResult {
            capabilities: capabilities::server_capabilities(),
            server_info: Some(ServerInfo {
                name: "blood-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        info!("Blood language server initialized");

        self.client
            .log_message(MessageType::INFO, "Blood language server ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down Blood language server");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let text = params.text_document.text;

        debug!("Document opened: {}", uri);

        let doc = Document::new(uri.clone(), version, text);
        self.documents.insert(uri.clone(), doc);

        self.validate_document(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        debug!("Document changed: {}", uri);

        if let Some(mut doc) = self.documents.get_mut(&uri) {
            for change in params.content_changes {
                doc.apply_change(version, change);
            }
        }

        self.validate_document(&uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        debug!("Document saved: {}", uri);

        self.validate_document(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        debug!("Document closed: {}", uri);

        self.documents.remove(&uri);

        // Clear diagnostics
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Hover request at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        // Use the hover provider for real type information
        Ok(self.hover_provider.hover(&doc, position))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Go to definition at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        // Use the definition provider for real symbol navigation
        if let Some(location) = self.definition_provider.definition(&doc, position) {
            Ok(Some(GotoDefinitionResponse::Scalar(location)))
        } else {
            Ok(None)
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        debug!("Find references at {} line {} char {}", uri, position.line, position.character);

        let Some(_doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        // TODO: Integrate with bloodc for actual reference lookup

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        debug!("Completion request at {} line {} char {}", uri, position.line, position.character);

        let Some(_doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        // TODO: Integrate with bloodc for actual completions
        // Return some basic keyword completions for now

        let items = vec![
            CompletionItem::new_simple("fn".to_string(), "Function declaration".to_string()),
            CompletionItem::new_simple("let".to_string(), "Variable binding".to_string()),
            CompletionItem::new_simple("effect".to_string(), "Effect declaration".to_string()),
            CompletionItem::new_simple("handler".to_string(), "Effect handler".to_string()),
            CompletionItem::new_simple("perform".to_string(), "Perform effect operation".to_string()),
            CompletionItem::new_simple("resume".to_string(), "Resume continuation".to_string()),
            CompletionItem::new_simple("match".to_string(), "Pattern matching".to_string()),
            CompletionItem::new_simple("if".to_string(), "Conditional".to_string()),
            CompletionItem::new_simple("struct".to_string(), "Struct declaration".to_string()),
            CompletionItem::new_simple("enum".to_string(), "Enum declaration".to_string()),
            CompletionItem::new_simple("trait".to_string(), "Trait declaration".to_string()),
            CompletionItem::new_simple("impl".to_string(), "Implementation block".to_string()),
            CompletionItem::new_simple("pub".to_string(), "Public visibility".to_string()),
            CompletionItem::new_simple("pure".to_string(), "Pure effect annotation".to_string()),
        ];

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;

        debug!("Semantic tokens request for {}", uri);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let tokens = self.semantic_tokens.provide(&doc);

        Ok(Some(SemanticTokensResult::Tokens(tokens)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;

        debug!("Format request for {}", uri);

        let Some(_doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        // TODO: Integrate with blood-fmt for actual formatting

        Ok(None)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;

        debug!("Code action request for {}", uri);

        // TODO: Implement code actions

        Ok(None)
    }
}
