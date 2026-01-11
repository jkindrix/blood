//! Codebase Synchronization
//!
//! Handles syncing codebases between local and remote locations.

use crate::hash::Hash;
use crate::names::Name;
use crate::{Codebase, DefKind, UcmError, UcmResult};

/// A remote codebase location.
#[derive(Debug, Clone)]
pub struct Remote {
    /// Remote name (e.g., "origin")
    pub name: String,
    /// Remote URL
    pub url: String,
}

impl Remote {
    /// Creates a new remote.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
        }
    }
}

/// A sync operation to perform.
#[derive(Debug, Clone)]
pub enum SyncOp {
    /// Add a definition locally
    AddLocal(Name, String, DefKind),
    /// Add a definition remotely
    AddRemote(Name, String, DefKind),
    /// Remove a definition locally
    RemoveLocal(Hash),
    /// Remove a definition remotely
    RemoveRemote(Hash),
    /// Conflict that needs resolution
    Conflict {
        name: Name,
        local_hash: Hash,
        remote_hash: Hash,
    },
}

/// Result of comparing two codebases.
#[derive(Debug, Clone, Default)]
pub struct SyncPlan {
    /// Operations to perform
    pub operations: Vec<SyncOp>,
    /// Whether there are conflicts
    pub has_conflicts: bool,
}

impl SyncPlan {
    /// Creates an empty sync plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an operation to the plan.
    pub fn add(&mut self, op: SyncOp) {
        if matches!(op, SyncOp::Conflict { .. }) {
            self.has_conflicts = true;
        }
        self.operations.push(op);
    }

    /// Returns true if the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns the number of operations.
    pub fn len(&self) -> usize {
        self.operations.len()
    }
}

/// Synchronization engine.
pub struct SyncEngine<'a> {
    _local: &'a mut Codebase,
}

impl<'a> SyncEngine<'a> {
    /// Creates a new sync engine for the given codebase.
    pub fn new(local: &'a mut Codebase) -> Self {
        Self { _local: local }
    }

    /// Computes the sync plan between local and remote.
    pub fn plan(&self, _remote: &Remote) -> UcmResult<SyncPlan> {
        // TODO: Fetch remote codebase state and compare
        // For now, return an empty plan
        Ok(SyncPlan::new())
    }

    /// Executes a sync plan.
    pub fn execute(&mut self, _plan: &SyncPlan) -> UcmResult<()> {
        // TODO: Apply sync operations
        Ok(())
    }

    /// Pushes local changes to remote.
    pub fn push(&mut self, remote: &Remote) -> UcmResult<PushResult> {
        // TODO: Implement push
        Ok(PushResult {
            pushed: 0,
            remote: remote.clone(),
        })
    }

    /// Pulls remote changes to local.
    pub fn pull(&mut self, remote: &Remote) -> UcmResult<PullResult> {
        // TODO: Implement pull
        Ok(PullResult {
            pulled: 0,
            remote: remote.clone(),
        })
    }
}

/// Result of a push operation.
#[derive(Debug)]
pub struct PushResult {
    /// Number of definitions pushed
    pub pushed: usize,
    /// Remote that was pushed to
    pub remote: Remote,
}

/// Result of a pull operation.
#[derive(Debug)]
pub struct PullResult {
    /// Number of definitions pulled
    pub pulled: usize,
    /// Remote that was pulled from
    pub remote: Remote,
}

/// Conflict resolution strategies.
#[derive(Debug, Clone, Copy)]
pub enum ConflictResolution {
    /// Keep the local version
    KeepLocal,
    /// Keep the remote version
    KeepRemote,
    /// Keep both (with different names)
    KeepBoth,
    /// Skip this conflict
    Skip,
}

/// Resolves a conflict using the given strategy.
pub fn resolve_conflict(
    _name: &Name,
    _local_hash: &Hash,
    _remote_hash: &Hash,
    resolution: ConflictResolution,
) -> Option<SyncOp> {
    match resolution {
        ConflictResolution::KeepLocal => None, // No-op, already have local
        ConflictResolution::KeepRemote => {
            // TODO: Return operation to update local with remote
            None
        }
        ConflictResolution::KeepBoth => {
            // TODO: Rename one and keep both
            None
        }
        ConflictResolution::Skip => None,
    }
}

/// Format for exporting/importing codebase data.
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    /// JSON format
    Json,
    /// Binary format (more compact)
    Binary,
}

/// Exported codebase data for sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportData {
    /// Version of the export format
    pub version: u32,
    /// Definitions: (hash_hex, kind_str, source)
    pub definitions: Vec<(String, String, String)>,
    /// Names: (name, hash_hex)
    pub names: Vec<(String, String)>,
}

impl ExportData {
    /// Create a new export data container.
    pub fn new() -> Self {
        Self {
            version: 1,
            definitions: Vec::new(),
            names: Vec::new(),
        }
    }
}

impl Default for ExportData {
    fn default() -> Self {
        Self::new()
    }
}

/// Exports a codebase to a file.
pub fn export_codebase(
    codebase: &Codebase,
    path: &std::path::Path,
    format: ExportFormat,
) -> UcmResult<()> {
    // Collect all definitions and names
    let mut export_data = ExportData::new();

    // Get all names and their hashes
    let names = codebase.list_names(None)?;
    for (name, hash) in &names {
        export_data.names.push((name.to_string(), hash.to_string()));

        // Get the definition for this hash
        if let Some(info) = codebase.find(&crate::DefRef::Hash(hash.clone()))? {
            // Check if we already have this definition
            if !export_data.definitions.iter().any(|(h, _, _)| h == &hash.to_string()) {
                export_data.definitions.push((
                    hash.to_string(),
                    info.kind.as_str().to_string(),
                    info.source.clone(),
                ));
            }
        }
    }

    // Write to file
    let file = std::fs::File::create(path)?;
    match format {
        ExportFormat::Json => {
            serde_json::to_writer_pretty(file, &export_data)
                .map_err(|e| UcmError::Storage(crate::storage::StorageError::Other(e.to_string())))?;
        }
        ExportFormat::Binary => {
            bincode::serialize_into(file, &export_data)
                .map_err(|e| UcmError::Storage(crate::storage::StorageError::Other(e.to_string())))?;
        }
    }

    Ok(())
}

/// Imports a codebase from a file.
pub fn import_codebase(
    codebase: &mut Codebase,
    path: &std::path::Path,
    format: ExportFormat,
) -> UcmResult<usize> {
    // Read from file
    let file = std::fs::File::open(path)?;
    let export_data: ExportData = match format {
        ExportFormat::Json => {
            serde_json::from_reader(file)
                .map_err(|e| UcmError::Storage(crate::storage::StorageError::Other(e.to_string())))?
        }
        ExportFormat::Binary => {
            bincode::deserialize_from(file)
                .map_err(|e| UcmError::Storage(crate::storage::StorageError::Other(e.to_string())))?
        }
    };

    let mut imported = 0;

    // Import definitions
    for (_hash_str, kind_str, source) in &export_data.definitions {
        let _hash = match kind_str.as_str() {
            "term" => codebase.add_term(source)?,
            "type" => codebase.add_type(source)?,
            "effect" => codebase.add_effect(source)?,
            "handler" => codebase.add_handler(source)?,
            "test" => codebase.add_test(source)?,
            _ => continue, // Skip unknown kinds
        };
        imported += 1;
    }

    // Import names
    for (name_str, hash_str) in &export_data.names {
        let name = Name::new(name_str);
        let hash: Hash = hash_str.parse()
            .map_err(|_| UcmError::ParseError(format!("Invalid hash: {}", hash_str)))?;
        // Only add if name doesn't already exist
        if codebase.resolve(&name)?.is_none() {
            codebase.add_name(name, hash)?;
        }
    }

    Ok(imported)
}
