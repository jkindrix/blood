//! Blood Standard Library - Rust Tooling Support
//!
//! This crate provides Rust tooling support for the Blood standard library.
//! The actual standard library is written in Blood syntax and located in
//! the `std/` directory.
//!
//! # Standard Library Structure
//!
//! ```text
//! std/
//! ├── prelude.blood      # Automatically imported
//! ├── primitive/         # Primitive type methods
//! ├── core/              # Option, Result, Box, etc.
//! ├── collections/       # Vec, HashMap, etc.
//! ├── traits/            # Core trait definitions
//! ├── effects/           # Effect definitions
//! ├── handlers/          # Standard handlers
//! ├── io/                # IO operations
//! ├── mem/               # Memory operations
//! ├── sync/              # Synchronization primitives
//! ├── iter/              # Iterator traits and adapters
//! └── ops/               # Operator traits
//! ```
//!
//! # Effect System
//!
//! Blood's standard library is built around algebraic effects:
//! - `Error<E>`: Recoverable error handling
//! - `State<S>`: Mutable state
//! - `IO`: Input/output operations
//! - `Async`: Asynchronous operations
//! - `Yield<T>`: Generator/iterator effects
//!
//! # Memory Model
//!
//! Uses 128-bit generational pointers with compile-time linearity checking
//! and runtime generation validation for memory safety without garbage collection.

/// Standard library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Standard library modules (for tooling introspection)
pub const MODULES: &[&str] = &[
    "prelude",
    "primitive",
    "core",
    "collections",
    "traits",
    "effects",
    "handlers",
    "io",
    "mem",
    "sync",
    "iter",
    "ops",
];

/// Effect types defined in the standard library
pub const EFFECTS: &[&str] = &[
    "Error",
    "State",
    "IO",
    "Async",
    "Yield",
    "Panic",
    "StaleReference",
    "NonDet",
    "Resource",
    "Random",
];

/// Core types provided by the standard library
pub const CORE_TYPES: &[&str] = &[
    "Option",
    "Result",
    "Box",
    "String",
    "Frozen",
    "Vec",
    "HashMap",
    "HashSet",
    "VecDeque",
    "BTreeMap",
    "BTreeSet",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_modules() {
        assert!(MODULES.contains(&"prelude"));
        assert!(MODULES.contains(&"core"));
        assert!(MODULES.contains(&"effects"));
    }

    #[test]
    fn test_effects() {
        assert!(EFFECTS.contains(&"Error"));
        assert!(EFFECTS.contains(&"IO"));
        assert!(EFFECTS.contains(&"State"));
    }
}
