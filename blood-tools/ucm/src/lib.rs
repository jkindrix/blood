//! Blood Unison-style Codebase Manager
//!
//! A content-addressed codebase manager for Blood that stores code by its hash,
//! enabling refactoring, renaming, and code sharing without breaking references.
//!
//! # Concepts
//!
//! ## Content-Addressed Code
//!
//! Code is stored by its hash, not by name. This means:
//! - Renaming is free and doesn't break anything
//! - Code can be shared across projects trivially
//! - History is preserved forever
//! - Dependencies are exact and reproducible
//!
//! ## Names as Metadata
//!
//! Names are just pointers to hashes. Multiple names can point to the same code.
//! Names can be changed without affecting the code itself.
//!
//! ## The Codebase
//!
//! The codebase is a database of:
//! - Terms (functions, values)
//! - Types (structs, enums, type aliases)
//! - Effects (effect declarations)
//! - Handlers (effect handler implementations)
//!
//! Each is identified by a unique hash derived from its content.
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_ucm::{Codebase, Name};
//!
//! let mut codebase = Codebase::open("my-project")?;
//!
//! // Add a new term
//! let hash = codebase.add_term("fn double(x: i32) -> i32 { x * 2 }")?;
//!
//! // Give it a name
//! codebase.add_name(Name::new("math.double"), hash)?;
//!
//! // Later, rename it (the code doesn't change, just the name)
//! codebase.rename(Name::new("math.double"), Name::new("utils.double"))?;
//! ```

pub mod codebase;
pub mod hash;
pub mod names;
pub mod storage;
pub mod sync;
pub mod test_runner;

pub use codebase::{Codebase, DiffResult, Difference};
pub use hash::Hash;
pub use names::Name;
pub use test_runner::{TestRunner, TestRunOptions, TestResult, TestOutcome, TestSummary, TestDiscovery};

use thiserror::Error;

/// Errors that can occur in the codebase manager.
#[derive(Debug, Error)]
pub enum UcmError {
    #[error("Storage error: {0}")]
    Storage(#[from] storage::StorageError),

    #[error("Name not found: {0}")]
    NameNotFound(String),

    #[error("Hash not found: {0}")]
    HashNotFound(String),

    #[error("Ambiguous hash prefix '{prefix}' matches {count} definitions")]
    AmbiguousHash { prefix: String, count: usize },

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for UCM operations.
pub type UcmResult<T> = Result<T, UcmError>;

/// The kind of definition stored in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// A function or value
    Term,
    /// A type definition (struct, enum, type alias)
    Type,
    /// An effect declaration
    Effect,
    /// An effect handler
    Handler,
    /// A test
    Test,
    /// Documentation
    Doc,
}

impl DefKind {
    /// Returns the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            DefKind::Term => "term",
            DefKind::Type => "type",
            DefKind::Effect => "effect",
            DefKind::Handler => "handler",
            DefKind::Test => "test",
            DefKind::Doc => "doc",
        }
    }

    /// Parses a DefKind from a string.
    pub fn from_str(s: &str) -> Self {
        match s {
            "term" => DefKind::Term,
            "type" => DefKind::Type,
            "effect" => DefKind::Effect,
            "handler" => DefKind::Handler,
            "test" => DefKind::Test,
            "doc" => DefKind::Doc,
            _ => DefKind::Term, // Default fallback
        }
    }
}

/// A reference to a definition, either by name or hash.
#[derive(Debug, Clone)]
pub enum DefRef {
    /// Reference by name
    Name(Name),
    /// Reference by hash
    Hash(Hash),
}

impl DefRef {
    /// Creates a name reference.
    pub fn name(s: impl Into<String>) -> Self {
        DefRef::Name(Name::new(s))
    }

    /// Creates a hash reference.
    pub fn hash(h: Hash) -> Self {
        DefRef::Hash(h)
    }
}

/// Information about a definition.
#[derive(Debug, Clone)]
pub struct DefInfo {
    /// The content hash
    pub hash: Hash,
    /// The definition kind
    pub kind: DefKind,
    /// Names pointing to this definition
    pub names: Vec<Name>,
    /// The source code
    pub source: String,
    /// Dependencies (hashes of referenced definitions)
    pub dependencies: Vec<Hash>,
    /// Dependents (hashes of definitions that reference this)
    pub dependents: Vec<Hash>,
}

/// A patch representing changes to the codebase.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    /// Definitions to add
    pub additions: Vec<(Name, String)>,
    /// Definitions to remove
    pub removals: Vec<Hash>,
    /// Renames to perform
    pub renames: Vec<(Name, Name)>,
    /// Updates (old hash -> new source)
    pub updates: Vec<(Hash, String)>,
}

impl Patch {
    /// Creates an empty patch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a definition.
    pub fn add(&mut self, name: Name, source: String) -> &mut Self {
        self.additions.push((name, source));
        self
    }

    /// Removes a definition by hash.
    pub fn remove(&mut self, hash: Hash) -> &mut Self {
        self.removals.push(hash);
        self
    }

    /// Renames a definition.
    pub fn rename(&mut self, from: Name, to: Name) -> &mut Self {
        self.renames.push((from, to));
        self
    }

    /// Updates a definition.
    pub fn update(&mut self, hash: Hash, new_source: String) -> &mut Self {
        self.updates.push((hash, new_source));
        self
    }

    /// Returns true if the patch is empty.
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty()
            && self.removals.is_empty()
            && self.renames.is_empty()
            && self.updates.is_empty()
    }
}
