//! LSP Server Capabilities
//!
//! Defines what features the Blood language server supports.

use tower_lsp::lsp_types::*;

use crate::semantic_tokens;

/// Returns the server capabilities for the Blood language server.
///
/// NOTE: Only capabilities that are actually implemented are advertised.
/// Features marked as "not yet implemented" have been disabled to avoid
/// misleading IDE users.
pub fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Text document sync - IMPLEMENTED
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                will_save: Some(false),
                will_save_wait_until: Some(false),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
            },
        )),

        // Hover provider - IMPLEMENTED (basic keyword and type hovering)
        hover_provider: Some(HoverProviderCapability::Simple(true)),

        // Completion provider - IMPLEMENTED (basic keyword completions)
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(false), // resolve not implemented
            trigger_characters: Some(vec![
                ".".to_string(),
                ":".to_string(),
            ]),
            all_commit_characters: None,
            work_done_progress_options: WorkDoneProgressOptions::default(),
            completion_item: None,
        }),

        // Signature help - NOT YET IMPLEMENTED
        signature_help_provider: None,

        // Go to definition - IMPLEMENTED (partial, integrated with bloodc)
        definition_provider: Some(OneOf::Left(true)),

        // Go to type definition - NOT YET IMPLEMENTED
        type_definition_provider: None,

        // Go to implementation - NOT YET IMPLEMENTED
        implementation_provider: None,

        // Find references - NOT YET IMPLEMENTED
        references_provider: None,

        // Document highlight - NOT YET IMPLEMENTED
        document_highlight_provider: None,

        // Document symbols (outline) - IMPLEMENTED
        document_symbol_provider: Some(OneOf::Left(true)),

        // Workspace symbols - NOT YET IMPLEMENTED
        workspace_symbol_provider: None,

        // Code actions - NOT YET IMPLEMENTED
        code_action_provider: None,

        // Code lens - IMPLEMENTED (main, test, handler annotations)
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(true),
        }),

        // Document formatting - NOT YET IMPLEMENTED
        // (blood-fmt exists but LSP integration not complete)
        document_formatting_provider: None,

        // Document range formatting - NOT YET IMPLEMENTED
        document_range_formatting_provider: None,

        // Rename - NOT YET IMPLEMENTED
        rename_provider: None,

        // Folding ranges - IMPLEMENTED
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),

        // Selection ranges - NOT YET IMPLEMENTED
        selection_range_provider: None,

        // Semantic tokens - IMPLEMENTED (full Blood syntax highlighting)
        semantic_tokens_provider: Some(
            SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: semantic_tokens::legend(),
                range: Some(true),
                full: Some(SemanticTokensFullOptions::Bool(true)),
            }),
        ),

        // Inlay hints - IMPLEMENTED (let binding types, effect annotations)
        inlay_hint_provider: Some(OneOf::Left(true)),

        // Workspace capabilities
        workspace: Some(WorkspaceServerCapabilities {
            workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                supported: Some(true),
                change_notifications: Some(OneOf::Left(true)),
            }),
            file_operations: None,
        }),

        // Diagnostic provider - IMPLEMENTED (parse + type errors)
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: Some("blood".to_string()),
            inter_file_dependencies: true,
            workspace_diagnostics: true,
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),

        ..Default::default()
    }
}

/// Blood-specific trigger characters for completions.
pub mod triggers {
    /// Triggers that start member access completions.
    pub const MEMBER_ACCESS: &[&str] = &[".", "::"];

    /// Triggers for effect annotations.
    pub const EFFECT_ANNOTATION: &[&str] = &["/"];

    /// Triggers for generic type parameters.
    pub const TYPE_PARAMS: &[&str] = &["<"];

    /// Triggers for function arguments.
    pub const FUNCTION_ARGS: &[&str] = &["(", ","];
}

/// File patterns the Blood LSP handles.
pub mod file_patterns {
    /// Blood source file extension.
    pub const BLOOD_EXT: &str = ".blood";

    /// Blood manifest file.
    pub const MANIFEST: &str = "Blood.toml";

    /// Check if a file path is a Blood source file.
    pub fn is_blood_file(path: &str) -> bool {
        path.ends_with(BLOOD_EXT)
    }

    /// Check if a file path is a Blood manifest.
    pub fn is_manifest(path: &str) -> bool {
        path.ends_with(MANIFEST)
    }
}
