//! LSP Backend Implementation
//!
//! The main language server that handles LSP requests and notifications.

use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{debug, info};

use crate::analysis::{
    CompletionProvider, DefinitionProvider, DocumentHighlightProvider, HoverProvider,
    ImplementationProvider, ReferencesProvider, RenameProvider, SignatureHelpProvider,
    TypeDefinitionProvider, WorkspaceSymbolProvider,
};
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
    /// Find references provider.
    references_provider: ReferencesProvider,
    /// Code completion provider.
    completion_provider: CompletionProvider,
    /// Document highlight provider.
    highlight_provider: DocumentHighlightProvider,
    /// Go-to-type-definition provider.
    type_definition_provider: TypeDefinitionProvider,
    /// Signature help provider.
    signature_help_provider: SignatureHelpProvider,
    /// Rename provider.
    rename_provider: RenameProvider,
    /// Go-to-implementation provider.
    implementation_provider: ImplementationProvider,
    /// Workspace symbol provider.
    workspace_symbol_provider: WorkspaceSymbolProvider,
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
            references_provider: ReferencesProvider::new(),
            completion_provider: CompletionProvider::new(),
            highlight_provider: DocumentHighlightProvider::new(),
            type_definition_provider: TypeDefinitionProvider::new(),
            signature_help_provider: SignatureHelpProvider::new(),
            rename_provider: RenameProvider::new(),
            implementation_provider: ImplementationProvider::new(),
            workspace_symbol_provider: WorkspaceSymbolProvider::new(),
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
        let include_declaration = params.context.include_declaration;

        debug!("Find references at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        Ok(self.references_provider.references(&doc, position, include_declaration))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        debug!("Completion request at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let items = self.completion_provider.completions(&doc, position);

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

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let source = doc.text();

        // Use blood-fmt to format the source
        match blood_fmt::format_source(&source) {
            Ok(formatted) => {
                if formatted == source {
                    // No changes needed
                    return Ok(None);
                }

                // Calculate the range to replace (entire document)
                let lines: Vec<&str> = source.lines().collect();
                let last_line = lines.len().saturating_sub(1);
                let last_col = lines.last().map(|l| l.len()).unwrap_or(0);

                let edit = TextEdit {
                    range: Range {
                        start: Position { line: 0, character: 0 },
                        end: Position {
                            line: last_line as u32,
                            character: last_col as u32,
                        },
                    },
                    new_text: formatted,
                };

                Ok(Some(vec![edit]))
            }
            Err(e) => {
                debug!("Format error: {}", e);
                Ok(None)
            }
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let range = params.range;

        debug!("Code action request for {}", uri);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let mut actions = Vec::new();
        let text = doc.text();

        // Get the line at the cursor
        let lines: Vec<&str> = text.lines().collect();
        let line_idx = range.start.line as usize;
        let current_line = lines.get(line_idx).unwrap_or(&"");
        let trimmed = current_line.trim();

        // Code action: Add effect annotation for functions
        if (trimmed.starts_with("fn ") || trimmed.starts_with("pub fn "))
            && !trimmed.contains(" / ")
            && trimmed.contains("->")
        {
            if let Some(brace_pos) = current_line.find('{') {
                let action = CodeAction {
                    title: "Add effect annotation".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut changes = std::collections::HashMap::new();
                            changes.insert(uri.clone(), vec![TextEdit {
                                range: Range {
                                    start: Position {
                                        line: range.start.line,
                                        character: brace_pos as u32,
                                    },
                                    end: Position {
                                        line: range.start.line,
                                        character: brace_pos as u32,
                                    },
                                },
                                new_text: "/ pure ".to_string(),
                            }]);
                            changes
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                actions.push(CodeActionOrCommand::CodeAction(action));
            }
        }

        // Code action: Add type annotation for let bindings
        if trimmed.starts_with("let ") && !trimmed.contains(':') && trimmed.contains('=') {
            // Find the variable name and = position
            if let Some(eq_pos) = current_line.find('=') {
                let action = CodeAction {
                    title: "Add type annotation".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut changes = std::collections::HashMap::new();
                            changes.insert(uri.clone(), vec![TextEdit {
                                range: Range {
                                    start: Position {
                                        line: range.start.line,
                                        character: (eq_pos.saturating_sub(1)) as u32,
                                    },
                                    end: Position {
                                        line: range.start.line,
                                        character: eq_pos as u32,
                                    },
                                },
                                new_text: ": Type ".to_string(),
                            }]);
                            changes
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                actions.push(CodeActionOrCommand::CodeAction(action));
            }
        }

        // Code action: Format document
        let format_action = CodeAction {
            title: "Format document".to_string(),
            kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
            command: Some(Command {
                title: "Format".to_string(),
                command: "blood.formatDocument".to_string(),
                arguments: None,
            }),
            ..Default::default()
        };
        actions.push(CodeActionOrCommand::CodeAction(format_action));

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Document highlight at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        Ok(self.highlight_provider.highlights(&doc, position))
    }

    async fn goto_type_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Go to type definition at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        if let Some(location) = self.type_definition_provider.type_definition(&doc, position) {
            Ok(Some(GotoDefinitionResponse::Scalar(location)))
        } else {
            Ok(None)
        }
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Signature help at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        Ok(self.signature_help_provider.signature_help(&doc, position))
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = &params.new_name;

        debug!("Rename at {} line {} char {} -> {}", uri, position.line, position.character, new_name);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        Ok(self.rename_provider.rename(&doc, position, new_name))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let position = params.position;

        debug!("Prepare rename at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        Ok(self.rename_provider.prepare_rename(&doc, position))
    }

    async fn goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Go to implementation at {} line {} char {}", uri, position.line, position.character);

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        if let Some(locations) = self.implementation_provider.implementations(&doc, position) {
            Ok(Some(GotoDefinitionResponse::Array(locations)))
        } else {
            Ok(None)
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = &params.query;

        debug!("Workspace symbol search: '{}'", query);

        // Collect all open documents
        let docs: Vec<(Url, Document)> = self.documents.iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        let results = self.workspace_symbol_provider.workspace_symbols(query, &docs);

        if results.is_empty() {
            Ok(None)
        } else {
            Ok(Some(results))
        }
    }
}
