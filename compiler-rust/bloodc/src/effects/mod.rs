//! # Effects System
//!
//! This module implements Blood's algebraic effects system based on evidence passing.
//!
//! ## Design Overview
//!
//! Blood's effect system is based on research from:
//! - [Type Directed Compilation of Row-Typed Algebraic Effects](https://dl.acm.org/doi/10.1145/3093333.3009872) (POPL'17)
//! - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
//! - [Lexical Effect Handlers, Directly](https://dl.acm.org/doi/10.1145/3689770) (OOPSLA'24)
//! - [Zero-Overhead Lexical Effect Handlers](https://doi.org/10.1145/3763177) (OOPSLA'25)
//! - [Parallel Algebraic Effect Handlers](https://icfp24.sigplan.org/details/icfp-2024-papers/27/Parallel-Algebraic-Effect-Handlers) (ICFP'24)
//! - [Optimize Effect Handling for Tail-resumption with Stack Unwinding](https://dl.acm.org/doi/10.1145/3732771.3742721) (SLE'25)
//!
//! ## Key Optimizations
//!
//! ### Tail-Resumptive Handlers
//!
//! Most handlers in practice are "tail-resumptive" (resume exactly once in tail position).
//! These are optimized to avoid continuation capture, achieving up to 150M ops/sec
//! (per SLE'25 research). Common examples: State, Reader, Writer effects.
//!
//! ### Lexical Scoping
//!
//! Blood uses lexically scoped handlers (per OOPSLA'24/25) which enable:
//! - Local reasoning about handler behavior
//! - Zero-overhead implementation when no effects raised
//! - Type-directed stack walking for handler lookup
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
//! - [`infer`] - Effect inference for automatic effect signatures
//! - [`std_effects`] - Standard effects (State, Error, IO)
//!
//! ## Implementation Status
//!
//! | Phase | Description | Status |
//! |-------|-------------|--------|
//! | 2.1 | Basic evidence passing | Complete |
//! | 2.2 | Tail-resumptive optimization | Complete |
//! | 2.3 | Segmented stack continuations | Deferred — bootstrap compiler uses synchronous dispatch; full continuation capture requires self-hosted compiler |
//! | 2.4 | Evidence fusion optimization | Deferred — self-hosted compiler optimization |
//! | 2.5 | Zero-overhead lexical handlers | Deferred — self-hosted compiler optimization |
//! | 2.6 | Parallel effect handlers | Deferred — self-hosted compiler feature |

pub mod evidence;
pub mod handler;
pub mod infer;
pub mod lowering;
pub mod row;
pub mod std_effects;

pub use evidence::{Evidence, EvidenceVector, EvidenceEntry, EvidenceContext, TranslatedOp};
pub use evidence::{EvidenceCache, HandlerPattern, CacheStats};
pub use handler::{Handler, HandlerKind, Continuation, ResumeMode};
pub use handler::{analyze_tail_resumptive, analyze_handler_tail_resumptive, analyze_resume_mode};
pub use infer::{EffectInferencer, DetailedEffectInferencer, InferenceResult, infer_effects, verify_effects_subset};
pub use lowering::{EffectLowering, EffectInfo, OperationInfo, EvidenceRequirement, HandlerInfo};
pub use row::{EffectRow, RowVar, EffectRef};
