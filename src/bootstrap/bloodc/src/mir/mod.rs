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
pub mod safepoint;
pub mod snapshot;
pub mod static_evidence;
pub mod types;
pub mod validate;
pub mod visitor;

pub use body::{LocalKind, MirBody, MirLocal};
pub use closure_analysis::{
    ClosureAnalysisConfig, ClosureAnalysisResults, ClosureAnalyzer, ClosureInfo, ClosureStats,
};
pub use escape::{
    EscapeAnalyzer, EscapeResults, EscapeState, EscapeStateBreakdown, EscapeStatistics,
};
pub use lowering::{InlineHandlerBodies, InlineHandlerBody, InlineHandlerCaptureInfo, MirLowering};
pub use ptr::{BloodPtr, MemoryTier, PtrFlags, PtrKind, PtrMetadata};
pub use snapshot::{GenerationSnapshot, LazySnapshot, LazyValidationStats, SnapshotEntry};
pub use static_evidence::{
    analyze_handler_allocation_tier, analyze_handler_deduplication, analyze_handler_state,
    handler_evidence_escapes, HandleAnalysis, HandlerDeduplicationResults, HandlerFingerprint,
};
pub use types::{
    AggregateKind, BasicBlock, BasicBlockData, BasicBlockId, BinOp, Constant, ConstantKind,
    InlineHandlerCapture, InlineHandlerOp, Operand, Place, PlaceElem, Projection, Rvalue,
    Statement, StatementKind, SwitchTargets, Terminator, TerminatorKind, UnOp,
};
pub use visitor::{
    collect_operand_locals, collect_rvalue_locals, walk_body, Location, PlaceContext, Visitor,
};
