//! # Mid-level Intermediate Representation (MIR)
//!
//! This module implements Blood's MIR, a control-flow graph based representation
//! designed for optimization and memory safety analysis.
//!
//! ## Design References
//!
//! - [Rust MIR Documentation](https://rustc-dev-guide.rust-lang.org/mir/index.html)
//! - [RFC 1211: MIR](https://rust-lang.github.io/rfcs/1211-mir.html)
//!
//! ## Key Properties
//!
//! MIR differs from HIR in several important ways:
//!
//! | Property | HIR | MIR |
//! |----------|-----|-----|
//! | Structure | Tree (nested expressions) | CFG (basic blocks) |
//! | Types | Partially inferred | All explicit |
//! | Control flow | Implicit (if/match/loop) | Explicit edges |
//! | Temporaries | Implicit | Explicit locals |
//!
//! ## Module Structure
//!
//! - [`types`] - Core MIR types (BasicBlock, Statement, Terminator)
//! - [`body`] - MIR function bodies
//! - [`ptr`] - 128-bit generational pointer representation
//! - [`lowering`] - HIR to MIR lowering pass
//! - [`escape`] - Escape analysis for tier promotion
//! - [`snapshot`] - Generation snapshots for effect handlers
//! - [`closure_analysis`] - Closure environment size analysis and optimization
//! - [`visitor`] - Visitor infrastructure for MIR traversal
//!
//! ## MIR Structure Overview
//!
//! ```text
//! MIR Body
//! ├── Locals (parameters, temporaries, return place)
//! └── Basic Blocks
//!     └── BasicBlock
//!         ├── Statements (assignments, storage operations)
//!         └── Terminator (goto, switch, call, return)
//! ```
//!
//! ## Phase 3 Implementation Status
//!
//! | Component | Status |
//! |-----------|--------|
//! | Basic types | Implemented |
//! | CFG structure | Implemented |
//! | 128-bit pointers | Implemented |
//! | HIR lowering | Pending |
//! | Escape analysis | Pending |
//! | Generation snapshots | Pending |

pub mod body;
pub mod closure_analysis;
pub mod escape;
pub mod lowering;
pub mod ptr;
pub mod snapshot;
pub mod static_evidence;
pub mod types;
pub mod visitor;

pub use body::{MirBody, MirLocal, LocalKind};
pub use closure_analysis::{ClosureAnalyzer, ClosureAnalysisConfig, ClosureAnalysisResults, ClosureInfo, ClosureStats};
pub use escape::{EscapeAnalyzer, EscapeState, EscapeResults, EscapeStatistics, EscapeStateBreakdown};
pub use static_evidence::{analyze_handler_state, HandleAnalysis};
pub use lowering::MirLowering;
pub use ptr::{BloodPtr, PtrMetadata, MemoryTier, PtrFlags};
pub use snapshot::{GenerationSnapshot, SnapshotEntry, LazySnapshot, LazyValidationStats};
pub use visitor::{Visitor, Location, PlaceContext, walk_body, collect_rvalue_locals, collect_operand_locals};
pub use types::{
    BasicBlock, BasicBlockData, BasicBlockId,
    Statement, StatementKind,
    Terminator, TerminatorKind,
    Place, PlaceElem, Projection,
    Operand, Rvalue,
    Constant, ConstantKind,
    SwitchTargets,
    BinOp, UnOp, AggregateKind,
};
