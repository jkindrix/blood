//! # Effects System
//!
//! This module implements Blood's algebraic effects system based on evidence passing.
//!
//! ## Design Overview
//!
//! Blood's effect system is based on research from:
//! - [Type Directed Compilation of Row-Typed Algebraic Effects](https://dl.acm.org/doi/10.1145/3093333.3009872) (POPL'17)
//! - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
//!
//! ## Implementation Strategy
//!
//! Effects are compiled using **evidence passing**:
//!
//! ```text
//! // Source (with effects)
//! fn increment() / {State<i32>} {
//!     let x = get()
//!     put(x + 1)
//! }
//!
//! // After evidence translation
//! fn increment(ev: Evidence<State<i32>>) {
//!     let x = ev.get()
//!     ev.put(x + 1)
//! }
//! ```
//!
//! ## Module Structure
//!
//! - [`row`] - Effect row types and row polymorphism
//! - [`evidence`] - Evidence vectors and evidence passing
//! - [`handler`] - Handler compilation and continuation capture
//! - [`lowering`] - HIR to effect-free translation
//!
//! ## Phase 2 Implementation Plan
//!
//! | Phase | Description | Status |
//! |-------|-------------|--------|
//! | 2.1 | Basic evidence passing | Pending |
//! | 2.2 | Tail-resumptive optimization | Pending |
//! | 2.3 | Segmented stack continuations | Pending |
//! | 2.4 | Evidence fusion optimization | Pending |

pub mod evidence;
pub mod handler;
pub mod lowering;
pub mod row;

pub use evidence::{Evidence, EvidenceVector};
pub use handler::{Handler, HandlerKind, Continuation};
pub use lowering::EffectLowering;
pub use row::{EffectRow, RowVar};
