//! Grammar-based fuzzing generators for Blood.
//!
//! This module provides types that implement `Arbitrary` to generate
//! syntactically plausible Blood programs for fuzzing. The generators
//! follow the grammar defined in `docs/spec/GRAMMAR.md`.
//!
//! # Architecture
//!
//! The generators are organized hierarchically:
//! - `FuzzProgram` - Complete programs with declarations
//! - `FuzzDeclaration` - Individual declarations (fn, struct, enum, effect, handler)
//! - `FuzzExpr` - Expressions with proper precedence
//! - `FuzzType` - Type expressions
//! - `FuzzPattern` - Match patterns
//!
//! Each type implements:
//! - `Arbitrary` for random generation
//! - `to_source()` for converting to valid Blood source code

pub mod grammar;
pub mod ident;

pub use grammar::*;
pub use ident::*;
