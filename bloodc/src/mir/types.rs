//! # Core MIR Types
//!
//! This module defines the fundamental types for Blood's MIR representation.
//!
//! ## Design Based On
//!
//! - [Rust MIR](https://rustc-dev-guide.rust-lang.org/mir/index.html)
//! - [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html)
//!
//! ## Type Hierarchy
//!
//! ```text
//! MirBody
//! └── BasicBlockData
//!     ├── Vec<Statement>
//!     │   └── StatementKind
//!     │       ├── Assign(Place, Rvalue)
//!     │       ├── StorageLive(Local)
//!     │       ├── StorageDead(Local)
//!     │       └── ...
//!     └── Terminator
//!         └── TerminatorKind
//!             ├── Goto { target }
//!             ├── SwitchInt { discr, targets }
//!             ├── Return
//!             ├── Call { func, args, destination }
//!             └── ...
//! ```

use std::fmt;
use crate::effects::evidence::HandlerStateKind;
use crate::hir::{DefId, LocalId, Type};
use crate::span::Span;
use super::ptr::MemoryTier;
use super::static_evidence::InlineEvidenceMode;

// ============================================================================
// Basic Blocks
// ============================================================================

/// A unique identifier for a basic block within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicBlockId(pub u32);

impl BasicBlockId {
    /// The entry block ID (always 0).
    pub const ENTRY: BasicBlockId = BasicBlockId(0);

    /// Create a new BasicBlockId.
    pub const fn new(id: u32) -> Self {
        BasicBlockId(id)
    }

    /// Get the index value.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for BasicBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// A basic block in the control-flow graph.
///
/// From [Rust MIR](https://rustc-dev-guide.rust-lang.org/mir/index.html):
/// "A basic block is a sequence of statements, followed by a single terminator."
#[derive(Debug, Clone)]
pub struct BasicBlockData {
    /// Statements executed sequentially.
    pub statements: Vec<Statement>,
    /// The terminator that ends this block.
    pub terminator: Option<Terminator>,
}

impl BasicBlockData {
    /// Create a new empty basic block.
    pub fn new() -> Self {
        Self {
            statements: Vec::new(),
            terminator: None,
        }
    }

    /// Check if this block has a terminator.
    pub fn is_terminated(&self) -> bool {
        self.terminator.is_some()
    }

    /// Get successor blocks.
    pub fn successors(&self) -> Vec<BasicBlockId> {
        match &self.terminator {
            Some(term) => term.successors(),
            None => vec![],
        }
    }
}

impl Default for BasicBlockData {
    fn default() -> Self {
        Self::new()
    }
}

/// A basic block reference (used for graph traversal).
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// The block ID.
    pub id: BasicBlockId,
    /// The block data.
    pub data: BasicBlockData,
}

// ============================================================================
// Statements
// ============================================================================

/// A statement in a basic block.
///
/// Statements have exactly one successor (the next statement or terminator).
#[derive(Debug, Clone)]
pub struct Statement {
    /// The kind of statement.
    pub kind: StatementKind,
    /// Source location for error reporting.
    pub span: Span,
}

impl Statement {
    /// Create a new statement.
    pub fn new(kind: StatementKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// An inline handler operation for `try { } with { }` expressions.
///
/// This represents a single operation handler defined inline in a `with` block.
#[derive(Debug, Clone)]
pub struct InlineHandlerOp {
    /// The operation name (e.g., "emit" for Emit.emit).
    pub op_name: String,
    /// The operation index within the effect.
    pub op_index: u32,
    /// Synthetic DefId for the generated handler function.
    pub synthetic_fn_def_id: DefId,
    /// Parameter types for the operation.
    pub param_types: Vec<Type>,
    /// Return type of the operation.
    pub return_type: Type,
}

/// The kind of a statement.
///
/// Based on [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html):
/// "Statements are the bread and butter of the MIR. They are relatively
/// simple and represent actions that have exactly one successor."
#[derive(Debug, Clone)]
pub enum StatementKind {
    /// Assignment: `place = rvalue`
    Assign(Place, Rvalue),

    /// Mark storage as live (for borrow checking/LLVM).
    StorageLive(LocalId),

    /// Mark storage as dead.
    StorageDead(LocalId),

    /// Drop a value (run destructor if applicable).
    Drop(Place),

    /// Increment generation counter for a slot.
    IncrementGeneration(Place),

    /// Capture generation snapshot for effect handler.
    CaptureSnapshot(LocalId),

    /// Validate generation (runtime check).
    ValidateGeneration {
        /// The pointer to validate.
        ptr: Place,
        /// The expected generation.
        expected_gen: Operand,
    },

    /// Push an effect handler onto the evidence vector.
    ///
    /// This installs the handler for the specified effect, making it
    /// available for `perform` operations in the enclosed computation.
    /// The effect_id is looked up from the handler definition during codegen.
    ///
    /// ## Static Evidence Optimization (EFF-OPT-001)
    ///
    /// When `state_kind` indicates the handler state is static (Stateless, Constant,
    /// or ZeroInit), the codegen can avoid runtime evidence allocation and instead
    /// use a pre-allocated static evidence structure.
    ///
    /// ## Stack Allocation Optimization (EFF-OPT-005/006)
    ///
    /// When `allocation_tier` is `Stack`, the handler evidence is lexically scoped
    /// and can use stack allocation instead of heap allocation, avoiding the
    /// overhead of `blood_evidence_create()`.
    ///
    /// ## Inline Evidence Optimization (EFF-OPT-003/004)
    ///
    /// When `inline_mode` is `Inline` or `SpecializedPair`, the handler evidence
    /// can be passed directly without going through the evidence vector, avoiding
    /// heap allocation and FFI overhead for shallow handler stacks.
    PushHandler {
        /// The handler definition ID.
        handler_id: DefId,
        /// The handler state place (pointer to handler state struct).
        state_place: Place,
        /// Classification of handler state for static evidence optimization.
        ///
        /// - `Stateless`: Handler has no state (unit type). Optimal case.
        /// - `Constant`: Handler state is a compile-time constant.
        /// - `ZeroInit`: Handler state is default/zero-initialized.
        /// - `Dynamic`: Handler state is computed at runtime. No optimization.
        state_kind: HandlerStateKind,
        /// Memory tier for handler evidence allocation (EFF-OPT-005/006).
        ///
        /// - `Stack`: Handler is lexically scoped, can use stack allocation.
        /// - `Region`: Handler evidence may escape, needs heap allocation.
        allocation_tier: MemoryTier,
        /// Inline evidence mode for optimized evidence passing (EFF-OPT-003/004).
        ///
        /// - `Inline`: Single handler, can pass entry directly without vector.
        /// - `SpecializedPair`: Two handlers, can use fixed-size pair structure.
        /// - `Vector`: Three or more handlers, must use heap-allocated vector.
        inline_mode: InlineEvidenceMode,
    },

    /// Pop an effect handler from the evidence vector.
    ///
    /// This removes the most recently pushed handler, restoring the
    /// previous handler state.
    PopHandler,

    /// Push an inline effect handler onto the evidence vector.
    ///
    /// This is used for `try { } with { }` inline handlers. Unlike `PushHandler`,
    /// inline handlers don't reference a pre-declared handler definition. Instead,
    /// they carry references to synthetic functions generated during MIR lowering.
    PushInlineHandler {
        /// The effect being handled.
        effect_id: DefId,
        /// The inline handler operations. Each entry is:
        /// (op_name, op_index, synthetic_fn_def_id, param_types, return_type)
        operations: Vec<InlineHandlerOp>,
        /// Memory tier for handler evidence allocation.
        allocation_tier: MemoryTier,
        /// Inline evidence mode for optimized evidence passing.
        inline_mode: InlineEvidenceMode,
    },

    /// Call a handler's return clause to transform the body result.
    ///
    /// This calls the handler's return clause function: `{handler_name}_return(result, state)`.
    /// The result is stored in the destination place.
    CallReturnClause {
        /// The handler DefId (used for handler metadata lookup).
        handler_id: DefId,
        /// The handler name (used for content-based function naming).
        /// This must be included in MIR for proper content hashing.
        handler_name: String,
        /// The body result value to pass to the return clause.
        body_result: Operand,
        /// The handler state place (pointer to state struct).
        state_place: Place,
        /// Where to store the return clause result.
        destination: Place,
    },

    /// No-op (placeholder for removed statements).
    Nop,
}

// ============================================================================
// Terminators
// ============================================================================

/// A terminator ends a basic block.
///
/// Terminators can have multiple successors (unlike statements).
#[derive(Debug, Clone)]
pub struct Terminator {
    /// The kind of terminator.
    pub kind: TerminatorKind,
    /// Source location for error reporting.
    pub span: Span,
}

impl Terminator {
    /// Create a new terminator.
    pub fn new(kind: TerminatorKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Get all successor blocks.
    pub fn successors(&self) -> Vec<BasicBlockId> {
        self.kind.successors()
    }
}

/// The kind of a terminator.
///
/// Based on [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html):
/// "Terminators represent actions that can have multiple outcomes."
#[derive(Debug, Clone)]
pub enum TerminatorKind {
    /// Unconditional jump.
    Goto { target: BasicBlockId },

    /// Conditional branch based on integer discriminant.
    SwitchInt {
        /// The value being switched on.
        discr: Operand,
        /// Switch targets.
        targets: SwitchTargets,
    },

    /// Return from the function.
    Return,

    /// Unreachable code (e.g., after diverging expression).
    Unreachable,

    /// Function call.
    Call {
        /// The function to call.
        func: Operand,
        /// Arguments to the function.
        args: Vec<Operand>,
        /// Where to store the result.
        destination: Place,
        /// Block to jump to after call completes.
        target: Option<BasicBlockId>,
        /// Block to jump to if call unwinds (panics).
        unwind: Option<BasicBlockId>,
    },

    /// Assert condition is true, otherwise panic.
    Assert {
        /// The condition to check.
        cond: Operand,
        /// Whether we expect true or false.
        expected: bool,
        /// Message for assertion failure.
        msg: String,
        /// Block to jump to on success.
        target: BasicBlockId,
        /// Block to jump to on failure (cleanup).
        unwind: Option<BasicBlockId>,
    },

    /// Drop a value and continue.
    DropAndReplace {
        /// Place to drop.
        place: Place,
        /// New value to assign.
        value: Operand,
        /// Next block.
        target: BasicBlockId,
        /// Cleanup on unwind.
        unwind: Option<BasicBlockId>,
    },

    /// Effect operation: perform.
    Perform {
        /// Effect being performed.
        effect_id: DefId,
        /// Operation index within the effect.
        op_index: u32,
        /// Arguments to the operation.
        args: Vec<Operand>,
        /// Where to store the result.
        destination: Place,
        /// Block to continue to (after handler resumes).
        target: BasicBlockId,
        /// Whether this effect operation is tail-resumptive.
        ///
        /// Tail-resumptive operations (like State.get, State.put) always
        /// resume immediately, so they don't need continuation capture.
        /// Non-tail-resumptive operations (like Error.throw) may suspend
        /// indefinitely and require capturing the continuation.
        is_tail_resumptive: bool,
    },

    /// Resume from an effect handler.
    Resume {
        /// Value to resume with.
        value: Option<Operand>,
    },

    /// Stale reference detected (effect).
    StaleReference {
        /// The pointer that was stale.
        ptr: Place,
        /// Expected generation.
        expected: u32,
        /// Actual generation found.
        actual: u32,
    },
}

impl TerminatorKind {
    /// Get all successor blocks for this terminator.
    pub fn successors(&self) -> Vec<BasicBlockId> {
        match self {
            TerminatorKind::Goto { target } => vec![*target],
            TerminatorKind::SwitchInt { targets, .. } => targets.all_targets(),
            TerminatorKind::Return => vec![],
            TerminatorKind::Unreachable => vec![],
            TerminatorKind::Call { target, unwind, .. } => {
                let mut succs = Vec::new();
                if let Some(t) = target {
                    succs.push(*t);
                }
                if let Some(u) = unwind {
                    succs.push(*u);
                }
                succs
            }
            TerminatorKind::Assert { target, unwind, .. } => {
                let mut succs = vec![*target];
                if let Some(u) = unwind {
                    succs.push(*u);
                }
                succs
            }
            TerminatorKind::DropAndReplace { target, unwind, .. } => {
                let mut succs = vec![*target];
                if let Some(u) = unwind {
                    succs.push(*u);
                }
                succs
            }
            TerminatorKind::Perform { target, .. } => vec![*target],
            TerminatorKind::Resume { .. } => vec![],
            TerminatorKind::StaleReference { .. } => vec![],
        }
    }

    /// Check if this terminator is a return.
    pub fn is_return(&self) -> bool {
        matches!(self, TerminatorKind::Return)
    }

    /// Check if this terminator diverges (no successors).
    pub fn is_diverging(&self) -> bool {
        self.successors().is_empty()
    }
}

/// Switch targets for SwitchInt terminator.
#[derive(Debug, Clone)]
pub struct SwitchTargets {
    /// Value -> target block mappings.
    pub branches: Vec<(u128, BasicBlockId)>,
    /// Default/otherwise block.
    pub otherwise: BasicBlockId,
}

impl SwitchTargets {
    /// Create new switch targets.
    pub fn new(branches: Vec<(u128, BasicBlockId)>, otherwise: BasicBlockId) -> Self {
        Self { branches, otherwise }
    }

    /// Get all target blocks.
    pub fn all_targets(&self) -> Vec<BasicBlockId> {
        let mut targets: Vec<_> = self.branches.iter().map(|(_, t)| *t).collect();
        targets.push(self.otherwise);
        targets
    }

    /// Find target for a specific value.
    pub fn target_for_value(&self, value: u128) -> BasicBlockId {
        for (v, target) in &self.branches {
            if *v == value {
                return *target;
            }
        }
        self.otherwise
    }
}

// ============================================================================
// Places and Operands
// ============================================================================

/// A place (memory location) that can be read or written.
///
/// Based on [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html):
/// "Places represent paths to memory locations."
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    /// The base local variable.
    pub local: LocalId,
    /// Projections applied to the base.
    pub projection: Vec<PlaceElem>,
}

impl Place {
    /// Create a place from just a local.
    pub fn local(id: LocalId) -> Self {
        Self {
            local: id,
            projection: vec![],
        }
    }

    /// Create a place with projections.
    pub fn projected(local: LocalId, projection: Vec<PlaceElem>) -> Self {
        Self { local, projection }
    }

    /// Add a projection element.
    pub fn project(&self, elem: PlaceElem) -> Self {
        let mut proj = self.projection.clone();
        proj.push(elem);
        Self {
            local: self.local,
            projection: proj,
        }
    }

    /// Check if this is just a local (no projections).
    pub fn is_local(&self) -> bool {
        self.projection.is_empty()
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.local.index)?;
        for elem in &self.projection {
            match elem {
                PlaceElem::Deref => write!(f, ".*")?,
                PlaceElem::Field(idx) => write!(f, ".{}", idx)?,
                PlaceElem::Index(local) => write!(f, "[_{}]", local.index)?,
                PlaceElem::ConstantIndex { offset, from_end, .. } => {
                    if *from_end {
                        write!(f, "[-{}]", offset)?;
                    } else {
                        write!(f, "[{}]", offset)?;
                    }
                }
                PlaceElem::Subslice { from, to, from_end } => {
                    write!(f, "[{}..{}{}]", from, if *from_end { "-" } else { "" }, to)?;
                }
                PlaceElem::Downcast(variant) => write!(f, " as variant#{}", variant)?,
            }
        }
        Ok(())
    }
}

/// A projection element for places.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceElem {
    /// Dereference: `*place`
    Deref,
    /// Field access: `place.field`
    Field(u32),
    /// Array/slice indexing with local: `place[local]`
    Index(LocalId),
    /// Constant index into array/slice.
    ConstantIndex {
        /// Offset from start or end.
        offset: u64,
        /// Minimum length required.
        min_length: u64,
        /// If true, offset is from end.
        from_end: bool,
    },
    /// Subslice: `place[from..to]`
    Subslice {
        from: u64,
        to: u64,
        from_end: bool,
    },
    /// Enum variant downcast.
    Downcast(u32),
}

/// A projection path (alias for clarity).
pub type Projection = Vec<PlaceElem>;

/// An operand in an expression.
///
/// Operands are the "read-only" inputs to operations.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Copy from a place (for Copy types).
    Copy(Place),
    /// Move from a place (transfers ownership).
    Move(Place),
    /// A constant value.
    Constant(Constant),
}

impl Operand {
    /// Create a copy operand.
    pub fn copy(place: Place) -> Self {
        Operand::Copy(place)
    }

    /// Create a move operand.
    pub fn r#move(place: Place) -> Self {
        Operand::Move(place)
    }

    /// Create a constant operand.
    pub fn constant(c: Constant) -> Self {
        Operand::Constant(c)
    }

    /// Get the place if this is a Copy or Move.
    pub fn place(&self) -> Option<&Place> {
        match self {
            Operand::Copy(p) | Operand::Move(p) => Some(p),
            Operand::Constant(_) => None,
        }
    }
}

// ============================================================================
// Rvalues
// ============================================================================

/// An rvalue (right-hand side of an assignment).
///
/// Rvalues produce values that can be assigned to places.
#[derive(Debug, Clone)]
pub enum Rvalue {
    /// Use an operand directly.
    Use(Operand),

    /// Create a reference to a place.
    Ref {
        place: Place,
        mutable: bool,
    },

    /// Create a raw pointer to a place.
    AddressOf {
        place: Place,
        mutable: bool,
    },

    /// Binary operation.
    BinaryOp {
        op: BinOp,
        left: Operand,
        right: Operand,
    },

    /// Checked binary operation (returns (result, overflow)).
    CheckedBinaryOp {
        op: BinOp,
        left: Operand,
        right: Operand,
    },

    /// Unary operation.
    UnaryOp {
        op: UnOp,
        operand: Operand,
    },

    /// Type cast.
    Cast {
        operand: Operand,
        target_ty: Type,
    },

    /// Get discriminant of an enum.
    Discriminant(Place),

    /// Compute length of an array/slice.
    Len(Place),

    /// Create an aggregate value (struct, tuple, array).
    Aggregate {
        kind: AggregateKind,
        operands: Vec<Operand>,
    },

    /// Null check: returns true if pointer is valid.
    NullCheck(Operand),

    /// Read generation from a generational pointer.
    ReadGeneration(Place),

    /// Create a generational pointer.
    MakeGenPtr {
        address: Operand,
        generation: Operand,
        metadata: Operand,
    },

    /// Zero-initialize a value of the given type.
    /// Used for `default` expressions and uninitialized struct fields.
    ZeroInit(Type),
}

/// Binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem,
    BitAnd, BitOr, BitXor,
    Shl, Shr,
    Eq, Ne, Lt, Le, Gt, Ge,
    Offset, // Pointer offset
}

/// Unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

/// Kind of aggregate value being constructed.
#[derive(Debug, Clone)]
pub enum AggregateKind {
    /// Tuple.
    Tuple,
    /// Array.
    Array(Type),
    /// Struct or enum.
    Adt {
        def_id: DefId,
        variant_idx: Option<u32>,
        /// Type arguments for generic ADTs (e.g., `Option<i32>` has `[i32]`).
        type_args: Vec<Type>,
    },
    /// Anonymous record.
    Record,
    /// Closure (captures).
    Closure {
        def_id: DefId,
    },
    /// Range (start, end, [exhausted]).
    Range {
        element: Type,
        inclusive: bool,
    },
}

// ============================================================================
// Constants
// ============================================================================

/// A constant value in MIR.
#[derive(Debug, Clone)]
pub struct Constant {
    /// The type of the constant.
    pub ty: Type,
    /// The constant value.
    pub kind: ConstantKind,
}

impl Constant {
    /// Create a new constant.
    pub fn new(ty: Type, kind: ConstantKind) -> Self {
        Self { ty, kind }
    }

    /// Create an integer constant.
    pub fn int(value: i128, ty: Type) -> Self {
        Self::new(ty, ConstantKind::Int(value))
    }

    /// Create a boolean constant.
    pub fn bool(value: bool) -> Self {
        Self::new(Type::bool(), ConstantKind::Bool(value))
    }

    /// Create a unit constant.
    pub fn unit() -> Self {
        Self::new(Type::unit(), ConstantKind::Unit)
    }
}

/// The kind of a constant.
#[derive(Debug, Clone)]
pub enum ConstantKind {
    /// Integer constant.
    Int(i128),
    /// Unsigned integer constant.
    Uint(u128),
    /// Floating-point constant.
    Float(f64),
    /// Boolean constant.
    Bool(bool),
    /// Character constant.
    Char(char),
    /// String constant (interned).
    String(String),
    /// Byte string constant.
    ByteString(Vec<u8>),
    /// Unit value.
    Unit,
    /// Function reference.
    FnDef(DefId),
    /// Const item reference.
    ConstDef(DefId),
    /// Static item reference.
    StaticDef(DefId),
    /// Zero-sized type value.
    ZeroSized,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_block_id_display() {
        let bb = BasicBlockId::new(5);
        assert_eq!(format!("{}", bb), "bb5");
    }

    #[test]
    fn test_basic_block_id_entry() {
        assert_eq!(BasicBlockId::ENTRY.0, 0);
    }

    #[test]
    fn test_basic_block_data_default() {
        let bb = BasicBlockData::new();
        assert!(bb.statements.is_empty());
        assert!(bb.terminator.is_none());
        assert!(!bb.is_terminated());
    }

    #[test]
    fn test_place_local() {
        let place = Place::local(LocalId::new(5));
        assert!(place.is_local());
        assert_eq!(place.local.index, 5);
    }

    #[test]
    fn test_place_projection() {
        let place = Place::local(LocalId::new(0))
            .project(PlaceElem::Field(1))
            .project(PlaceElem::Deref);
        assert!(!place.is_local());
        assert_eq!(place.projection.len(), 2);
    }

    #[test]
    fn test_place_display() {
        let place = Place::local(LocalId::new(0))
            .project(PlaceElem::Field(1));
        assert_eq!(format!("{}", place), "_0.1");
    }

    #[test]
    fn test_switch_targets() {
        let targets = SwitchTargets::new(
            vec![(0, BasicBlockId::new(1)), (1, BasicBlockId::new(2))],
            BasicBlockId::new(3),
        );
        assert_eq!(targets.target_for_value(0), BasicBlockId::new(1));
        assert_eq!(targets.target_for_value(1), BasicBlockId::new(2));
        assert_eq!(targets.target_for_value(99), BasicBlockId::new(3));
        assert_eq!(targets.all_targets().len(), 3);
    }

    #[test]
    fn test_terminator_successors_goto() {
        let term = Terminator::new(
            TerminatorKind::Goto { target: BasicBlockId::new(1) },
            Span::dummy(),
        );
        assert_eq!(term.successors(), vec![BasicBlockId::new(1)]);
    }

    #[test]
    fn test_terminator_successors_return() {
        let term = Terminator::new(TerminatorKind::Return, Span::dummy());
        assert!(term.successors().is_empty());
    }

    #[test]
    fn test_terminator_is_diverging() {
        assert!(TerminatorKind::Return.is_diverging());
        assert!(TerminatorKind::Unreachable.is_diverging());
        assert!(!TerminatorKind::Goto { target: BasicBlockId::new(0) }.is_diverging());
    }

    #[test]
    fn test_operand_place() {
        let place = Place::local(LocalId::new(0));
        let copy_op = Operand::Copy(place.clone());
        let move_op = Operand::Move(place.clone());
        let const_op = Operand::Constant(Constant::int(42, Type::i32()));

        assert!(copy_op.place().is_some());
        assert!(move_op.place().is_some());
        assert!(const_op.place().is_none());
    }

    #[test]
    fn test_constant_creation() {
        let c = Constant::int(42, Type::i32());
        assert!(matches!(c.kind, ConstantKind::Int(42)));

        let b = Constant::bool(true);
        assert!(matches!(b.kind, ConstantKind::Bool(true)));

        let u = Constant::unit();
        assert!(matches!(u.kind, ConstantKind::Unit));
    }
}
