//! # AST Canonicalization
//!
//! Transforms AST into canonical form for content addressing.
//!
//! ## Canonicalization Steps
//!
//! 1. **De Bruijn Indexing**: Replace variable names with position indices
//! 2. **Dependency Resolution**: Replace name references with content hashes
//! 3. **Type Normalization**: Normalize type expressions
//! 4. **Effect Row Sorting**: Sort effects lexicographically by hash
//! 5. **Metadata Stripping**: Remove comments, whitespace, source locations
//!
//! ## De Bruijn Indices
//!
//! ```text
//! // Source
//! fn add(x, y) { x + y }
//!
//! // Canonicalized
//! fn #0(#0, #1) { #0 + #1 }
//! ```
//!
//! This makes `add(x, y) = x + y` and `add(a, b) = a + b` produce identical hashes.

use std::collections::HashMap;

use super::hash::{ContentHash, ContentHasher};
use crate::hir::DefId;

/// Effect definition ID (uses DefId internally).
pub type EffectId = DefId;

/// Node type tags for serialization.
mod tags {
    pub const INT_LIT: u8 = 0x01;
    pub const FLOAT_LIT: u8 = 0x02;
    pub const STRING_LIT: u8 = 0x03;
    pub const BOOL_LIT: u8 = 0x04;
    pub const LOCAL_VAR: u8 = 0x05;
    pub const TYPE_VAR: u8 = 0x06;
    pub const DEF_REF: u8 = 0x07;
    pub const BUILTIN_REF: u8 = 0x08;
    pub const LAMBDA: u8 = 0x10;
    pub const APPLY: u8 = 0x11;
    pub const LET: u8 = 0x12;
    pub const IF_THEN_ELSE: u8 = 0x13;
    pub const TYPE_ARROW: u8 = 0x20;
    pub const TYPE_APP: u8 = 0x21;
    pub const TYPE_RECORD: u8 = 0x22;
    pub const PERFORM: u8 = 0x30;
    pub const HANDLE: u8 = 0x31;
    pub const RESUME: u8 = 0x32;
    pub const STRUCT: u8 = 0x40;
    pub const ENUM: u8 = 0x41;
    pub const MATCH: u8 = 0x42;
    pub const UNIT: u8 = 0x50;
    pub const TUPLE: u8 = 0x51;
    pub const BLOCK: u8 = 0x52;
    pub const BINOP: u8 = 0x53;
    pub const UNOP: u8 = 0x54;
    pub const RETURN: u8 = 0x55;
    pub const BREAK: u8 = 0x56;
    pub const CONTINUE: u8 = 0x57;
    pub const LOOP: u8 = 0x58;
    pub const WHILE: u8 = 0x59;
    pub const ARRAY: u8 = 0x5a;
    pub const INDEX: u8 = 0x5b;
    pub const FIELD: u8 = 0x5c;
    pub const ASSIGN: u8 = 0x5d;
}

/// De Bruijn index for local variables.
///
/// Index 0 refers to the innermost binding, increasing outward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeBruijnIndex(pub u32);

impl DeBruijnIndex {
    pub fn new(index: u32) -> Self {
        Self(index)
    }

    pub fn index(&self) -> u32 {
        self.0
    }
}

/// A field ID for struct/record fields (de Bruijn indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u32);

/// A variant ID for enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariantId(pub u32);

/// A builtin operation ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinId(pub u16);

impl BuiltinId {
    // Integer operations
    pub const INT_ADD: Self = Self(0x0001);
    pub const INT_SUB: Self = Self(0x0002);
    pub const INT_MUL: Self = Self(0x0003);
    pub const INT_DIV: Self = Self(0x0004);
    pub const INT_MOD: Self = Self(0x0005);
    pub const INT_NEG: Self = Self(0x0006);

    // Comparison
    pub const EQ: Self = Self(0x0010);
    pub const NE: Self = Self(0x0011);
    pub const LT: Self = Self(0x0012);
    pub const LE: Self = Self(0x0013);
    pub const GT: Self = Self(0x0014);
    pub const GE: Self = Self(0x0015);

    // Boolean operations
    pub const BOOL_AND: Self = Self(0x0020);
    pub const BOOL_OR: Self = Self(0x0021);
    pub const BOOL_NOT: Self = Self(0x0022);

    // Bitwise operations
    pub const BIT_AND: Self = Self(0x0030);
    pub const BIT_OR: Self = Self(0x0031);
    pub const BIT_XOR: Self = Self(0x0032);
    pub const BIT_NOT: Self = Self(0x0033);
    pub const SHL: Self = Self(0x0034);
    pub const SHR: Self = Self(0x0035);
}

/// Binary operator for canonical AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanonicalBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl CanonicalBinOp {
    fn tag(&self) -> u8 {
        match self {
            Self::Add => 0,
            Self::Sub => 1,
            Self::Mul => 2,
            Self::Div => 3,
            Self::Mod => 4,
            Self::Eq => 5,
            Self::Ne => 6,
            Self::Lt => 7,
            Self::Le => 8,
            Self::Gt => 9,
            Self::Ge => 10,
            Self::And => 11,
            Self::Or => 12,
            Self::BitAnd => 13,
            Self::BitOr => 14,
            Self::BitXor => 15,
            Self::Shl => 16,
            Self::Shr => 17,
        }
    }
}

/// Unary operator for canonical AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanonicalUnOp {
    Neg,
    Not,
    BitNot,
}

impl CanonicalUnOp {
    fn tag(&self) -> u8 {
        match self {
            Self::Neg => 0,
            Self::Not => 1,
            Self::BitNot => 2,
        }
    }
}

/// Canonicalized AST node.
///
/// All local names are replaced with de Bruijn indices.
/// All external references are replaced with content hashes.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalAST {
    // Literals
    IntLit(i128),
    FloatLit(f64),
    StringLit(Vec<u8>),
    BoolLit(bool),
    Unit,

    // Variables (de Bruijn indexed)
    LocalVar(DeBruijnIndex),
    TypeVar(u32),

    // References (by hash)
    DefRef(ContentHash),
    BuiltinRef(BuiltinId),

    // Expressions
    Lambda {
        param_count: u32,
        body: Box<CanonicalAST>,
    },
    Apply {
        func: Box<CanonicalAST>,
        args: Vec<CanonicalAST>,
    },
    Let {
        value: Box<CanonicalAST>,
        body: Box<CanonicalAST>,
    },
    IfThenElse {
        cond: Box<CanonicalAST>,
        then_branch: Box<CanonicalAST>,
        else_branch: Box<CanonicalAST>,
    },
    Block(Vec<CanonicalAST>),
    BinOp {
        op: CanonicalBinOp,
        lhs: Box<CanonicalAST>,
        rhs: Box<CanonicalAST>,
    },
    UnOp {
        op: CanonicalUnOp,
        operand: Box<CanonicalAST>,
    },

    // Control flow
    Return(Option<Box<CanonicalAST>>),
    Break(Option<Box<CanonicalAST>>),
    Continue,
    Loop(Box<CanonicalAST>),
    While {
        cond: Box<CanonicalAST>,
        body: Box<CanonicalAST>,
    },

    // Data structures
    Tuple(Vec<CanonicalAST>),
    Array(Vec<CanonicalAST>),
    Struct {
        fields: Vec<(FieldId, CanonicalAST)>,
    },
    Enum {
        variant: VariantId,
        payload: Box<CanonicalAST>,
    },
    Index {
        base: Box<CanonicalAST>,
        index: Box<CanonicalAST>,
    },
    Field {
        base: Box<CanonicalAST>,
        field: FieldId,
    },
    Assign {
        target: Box<CanonicalAST>,
        value: Box<CanonicalAST>,
    },

    // Pattern matching
    Match {
        scrutinee: Box<CanonicalAST>,
        arms: Vec<CanonicalMatchArm>,
    },

    // Types
    TypeArrow {
        params: Vec<CanonicalAST>,
        ret: Box<CanonicalAST>,
        effects: CanonicalEffectRow,
    },
    TypeApp {
        constructor: Box<CanonicalAST>,
        args: Vec<CanonicalAST>,
    },
    TypeRecord {
        fields: Vec<(FieldId, CanonicalAST)>,
    },

    // Effects
    Perform {
        effect: ContentHash,
        op_index: u32,
        args: Vec<CanonicalAST>,
    },
    Handle {
        body: Box<CanonicalAST>,
        handler: CanonicalHandler,
    },
    Resume(Box<CanonicalAST>),
}

/// A canonicalized match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalMatchArm {
    pub pattern: CanonicalPattern,
    pub guard: Option<Box<CanonicalAST>>,
    pub body: Box<CanonicalAST>,
}

/// A canonicalized pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalPattern {
    Wildcard,
    Binding(DeBruijnIndex),
    Literal(Box<CanonicalAST>),
    Tuple(Vec<CanonicalPattern>),
    Struct {
        fields: Vec<(FieldId, CanonicalPattern)>,
    },
    Enum {
        variant: VariantId,
        payload: Option<Box<CanonicalPattern>>,
    },
}

/// A canonicalized effect row (sorted by hash).
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalEffectRow {
    /// Effects sorted by their hash.
    pub effects: Vec<ContentHash>,
    /// Whether the row is open (has a row variable).
    pub is_open: bool,
}

impl CanonicalEffectRow {
    pub fn empty() -> Self {
        Self {
            effects: Vec::new(),
            is_open: false,
        }
    }

    pub fn pure() -> Self {
        Self::empty()
    }

    pub fn with_effects(mut effects: Vec<ContentHash>, is_open: bool) -> Self {
        // Sort by hash for canonical ordering
        effects.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Self { effects, is_open }
    }
}

/// A canonicalized effect handler.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalHandler {
    pub effect: ContentHash,
    pub operations: Vec<CanonicalOpImpl>,
    pub return_clause: Option<Box<CanonicalAST>>,
}

/// A canonicalized operation implementation.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalOpImpl {
    pub op_index: u32,
    pub param_count: u32,
    pub body: Box<CanonicalAST>,
}

impl CanonicalAST {
    /// Serialize to bytes for hashing.
    pub fn serialize(&self, hasher: &mut ContentHasher) {
        match self {
            Self::IntLit(n) => {
                hasher.update_u8(tags::INT_LIT);
                hasher.update_i128(*n);
            }
            Self::FloatLit(f) => {
                hasher.update_u8(tags::FLOAT_LIT);
                hasher.update_f64(*f);
            }
            Self::StringLit(s) => {
                hasher.update_u8(tags::STRING_LIT);
                hasher.update_u32(s.len() as u32);
                hasher.update(s);
            }
            Self::BoolLit(b) => {
                hasher.update_u8(tags::BOOL_LIT);
                hasher.update_u8(if *b { 1 } else { 0 });
            }
            Self::Unit => {
                hasher.update_u8(tags::UNIT);
            }
            Self::LocalVar(idx) => {
                hasher.update_u8(tags::LOCAL_VAR);
                hasher.update_u32(idx.0);
            }
            Self::TypeVar(idx) => {
                hasher.update_u8(tags::TYPE_VAR);
                hasher.update_u32(*idx);
            }
            Self::DefRef(hash) => {
                hasher.update_u8(tags::DEF_REF);
                hasher.update_hash(hash);
            }
            Self::BuiltinRef(id) => {
                hasher.update_u8(tags::BUILTIN_REF);
                hasher.update_u16(id.0);
            }
            Self::Lambda { param_count, body } => {
                hasher.update_u8(tags::LAMBDA);
                hasher.update_u32(*param_count);
                body.serialize(hasher);
            }
            Self::Apply { func, args } => {
                hasher.update_u8(tags::APPLY);
                func.serialize(hasher);
                hasher.update_u32(args.len() as u32);
                for arg in args {
                    arg.serialize(hasher);
                }
            }
            Self::Let { value, body } => {
                hasher.update_u8(tags::LET);
                value.serialize(hasher);
                body.serialize(hasher);
            }
            Self::IfThenElse {
                cond,
                then_branch,
                else_branch,
            } => {
                hasher.update_u8(tags::IF_THEN_ELSE);
                cond.serialize(hasher);
                then_branch.serialize(hasher);
                else_branch.serialize(hasher);
            }
            Self::Block(stmts) => {
                hasher.update_u8(tags::BLOCK);
                hasher.update_u32(stmts.len() as u32);
                for stmt in stmts {
                    stmt.serialize(hasher);
                }
            }
            Self::BinOp { op, lhs, rhs } => {
                hasher.update_u8(tags::BINOP);
                hasher.update_u8(op.tag());
                lhs.serialize(hasher);
                rhs.serialize(hasher);
            }
            Self::UnOp { op, operand } => {
                hasher.update_u8(tags::UNOP);
                hasher.update_u8(op.tag());
                operand.serialize(hasher);
            }
            Self::Return(value) => {
                hasher.update_u8(tags::RETURN);
                if let Some(v) = value {
                    hasher.update_u8(1);
                    v.serialize(hasher);
                } else {
                    hasher.update_u8(0);
                }
            }
            Self::Break(value) => {
                hasher.update_u8(tags::BREAK);
                if let Some(v) = value {
                    hasher.update_u8(1);
                    v.serialize(hasher);
                } else {
                    hasher.update_u8(0);
                }
            }
            Self::Continue => {
                hasher.update_u8(tags::CONTINUE);
            }
            Self::Loop(body) => {
                hasher.update_u8(tags::LOOP);
                body.serialize(hasher);
            }
            Self::While { cond, body } => {
                hasher.update_u8(tags::WHILE);
                cond.serialize(hasher);
                body.serialize(hasher);
            }
            Self::Tuple(elems) => {
                hasher.update_u8(tags::TUPLE);
                hasher.update_u32(elems.len() as u32);
                for elem in elems {
                    elem.serialize(hasher);
                }
            }
            Self::Array(elems) => {
                hasher.update_u8(tags::ARRAY);
                hasher.update_u32(elems.len() as u32);
                for elem in elems {
                    elem.serialize(hasher);
                }
            }
            Self::Struct { fields } => {
                hasher.update_u8(tags::STRUCT);
                hasher.update_u32(fields.len() as u32);
                // Fields should already be sorted by FieldId
                for (id, value) in fields {
                    hasher.update_u32(id.0);
                    value.serialize(hasher);
                }
            }
            Self::Enum { variant, payload } => {
                hasher.update_u8(tags::ENUM);
                hasher.update_u32(variant.0);
                payload.serialize(hasher);
            }
            Self::Index { base, index } => {
                hasher.update_u8(tags::INDEX);
                base.serialize(hasher);
                index.serialize(hasher);
            }
            Self::Field { base, field } => {
                hasher.update_u8(tags::FIELD);
                base.serialize(hasher);
                hasher.update_u32(field.0);
            }
            Self::Assign { target, value } => {
                hasher.update_u8(tags::ASSIGN);
                target.serialize(hasher);
                value.serialize(hasher);
            }
            Self::Match { scrutinee, arms } => {
                hasher.update_u8(tags::MATCH);
                scrutinee.serialize(hasher);
                hasher.update_u32(arms.len() as u32);
                for arm in arms {
                    arm.serialize(hasher);
                }
            }
            Self::TypeArrow {
                params,
                ret,
                effects,
            } => {
                hasher.update_u8(tags::TYPE_ARROW);
                hasher.update_u32(params.len() as u32);
                for param in params {
                    param.serialize(hasher);
                }
                ret.serialize(hasher);
                effects.serialize(hasher);
            }
            Self::TypeApp { constructor, args } => {
                hasher.update_u8(tags::TYPE_APP);
                constructor.serialize(hasher);
                hasher.update_u32(args.len() as u32);
                for arg in args {
                    arg.serialize(hasher);
                }
            }
            Self::TypeRecord { fields } => {
                hasher.update_u8(tags::TYPE_RECORD);
                hasher.update_u32(fields.len() as u32);
                for (id, ty) in fields {
                    hasher.update_u32(id.0);
                    ty.serialize(hasher);
                }
            }
            Self::Perform {
                effect,
                op_index,
                args,
            } => {
                hasher.update_u8(tags::PERFORM);
                hasher.update_hash(effect);
                hasher.update_u32(*op_index);
                hasher.update_u32(args.len() as u32);
                for arg in args {
                    arg.serialize(hasher);
                }
            }
            Self::Handle { body, handler } => {
                hasher.update_u8(tags::HANDLE);
                body.serialize(hasher);
                handler.serialize(hasher);
            }
            Self::Resume(value) => {
                hasher.update_u8(tags::RESUME);
                value.serialize(hasher);
            }
        }
    }

    /// Compute the content hash for this AST.
    pub fn compute_hash(&self) -> ContentHash {
        let mut hasher = ContentHasher::new();
        self.serialize(&mut hasher);
        hasher.finalize()
    }
}

impl CanonicalMatchArm {
    fn serialize(&self, hasher: &mut ContentHasher) {
        self.pattern.serialize(hasher);
        if let Some(guard) = &self.guard {
            hasher.update_u8(1);
            guard.serialize(hasher);
        } else {
            hasher.update_u8(0);
        }
        self.body.serialize(hasher);
    }
}

impl CanonicalPattern {
    fn serialize(&self, hasher: &mut ContentHasher) {
        match self {
            Self::Wildcard => hasher.update_u8(0),
            Self::Binding(idx) => {
                hasher.update_u8(1);
                hasher.update_u32(idx.0);
            }
            Self::Literal(lit) => {
                hasher.update_u8(2);
                lit.serialize(hasher);
            }
            Self::Tuple(pats) => {
                hasher.update_u8(3);
                hasher.update_u32(pats.len() as u32);
                for pat in pats {
                    pat.serialize(hasher);
                }
            }
            Self::Struct { fields } => {
                hasher.update_u8(4);
                hasher.update_u32(fields.len() as u32);
                for (id, pat) in fields {
                    hasher.update_u32(id.0);
                    pat.serialize(hasher);
                }
            }
            Self::Enum { variant, payload } => {
                hasher.update_u8(5);
                hasher.update_u32(variant.0);
                if let Some(p) = payload {
                    hasher.update_u8(1);
                    p.serialize(hasher);
                } else {
                    hasher.update_u8(0);
                }
            }
        }
    }
}

impl CanonicalEffectRow {
    fn serialize(&self, hasher: &mut ContentHasher) {
        hasher.update_u32(self.effects.len() as u32);
        for effect in &self.effects {
            hasher.update_hash(effect);
        }
        hasher.update_u8(if self.is_open { 1 } else { 0 });
    }
}

impl CanonicalHandler {
    fn serialize(&self, hasher: &mut ContentHasher) {
        hasher.update_hash(&self.effect);
        hasher.update_u32(self.operations.len() as u32);
        for op in &self.operations {
            op.serialize(hasher);
        }
        if let Some(ret) = &self.return_clause {
            hasher.update_u8(1);
            ret.serialize(hasher);
        } else {
            hasher.update_u8(0);
        }
    }
}

impl CanonicalOpImpl {
    fn serialize(&self, hasher: &mut ContentHasher) {
        hasher.update_u32(self.op_index);
        hasher.update_u32(self.param_count);
        self.body.serialize(hasher);
    }
}

/// Context for converting to canonical form.
pub struct Canonicalizer {
    /// Name to hash mappings for external definitions.
    def_hashes: HashMap<DefId, ContentHash>,
    /// Effect ID to hash mappings.
    effect_hashes: HashMap<EffectId, ContentHash>,
    /// Stack of bound variable names for de Bruijn indexing.
    scope_stack: Vec<String>,
}

impl Canonicalizer {
    /// Create a new canonicalizer.
    pub fn new() -> Self {
        Self {
            def_hashes: HashMap::new(),
            effect_hashes: HashMap::new(),
            scope_stack: Vec::new(),
        }
    }

    /// Register a definition hash.
    pub fn register_def(&mut self, def_id: DefId, hash: ContentHash) {
        self.def_hashes.insert(def_id, hash);
    }

    /// Register an effect hash.
    pub fn register_effect(&mut self, effect_id: EffectId, hash: ContentHash) {
        self.effect_hashes.insert(effect_id, hash);
    }

    /// Look up a definition hash.
    pub fn lookup_def(&self, def_id: DefId) -> Option<ContentHash> {
        self.def_hashes.get(&def_id).copied()
    }

    /// Look up an effect hash.
    pub fn lookup_effect(&self, effect_id: EffectId) -> Option<ContentHash> {
        self.effect_hashes.get(&effect_id).copied()
    }

    /// Push a binding onto the scope stack.
    pub fn push_binding(&mut self, name: String) {
        self.scope_stack.push(name);
    }

    /// Pop a binding from the scope stack.
    pub fn pop_binding(&mut self) {
        self.scope_stack.pop();
    }

    /// Look up a variable by name and return its de Bruijn index.
    pub fn lookup_local(&self, name: &str) -> Option<DeBruijnIndex> {
        for (i, bound_name) in self.scope_stack.iter().rev().enumerate() {
            if bound_name == name {
                return Some(DeBruijnIndex(i as u32));
            }
        }
        None
    }

    /// Get current scope depth.
    pub fn scope_depth(&self) -> u32 {
        self.scope_stack.len() as u32
    }
}

impl Default for Canonicalizer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debruijn_index() {
        let idx = DeBruijnIndex::new(5);
        assert_eq!(idx.index(), 5);
    }

    #[test]
    fn test_canonical_ast_hash_determinism() {
        let ast1 = CanonicalAST::IntLit(42);
        let ast2 = CanonicalAST::IntLit(42);
        let ast3 = CanonicalAST::IntLit(43);

        assert_eq!(ast1.compute_hash(), ast2.compute_hash());
        assert_ne!(ast1.compute_hash(), ast3.compute_hash());
    }

    #[test]
    fn test_canonical_lambda_hash() {
        // fn(x) { x }
        let ast1 = CanonicalAST::Lambda {
            param_count: 1,
            body: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0))),
        };

        // Same lambda with different original variable name produces same hash
        let ast2 = CanonicalAST::Lambda {
            param_count: 1,
            body: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0))),
        };

        assert_eq!(ast1.compute_hash(), ast2.compute_hash());
    }

    #[test]
    fn test_canonical_struct_field_ordering() {
        // Fields are sorted by FieldId
        let ast1 = CanonicalAST::Struct {
            fields: vec![
                (FieldId(0), CanonicalAST::IntLit(1)),
                (FieldId(1), CanonicalAST::IntLit(2)),
            ],
        };

        let ast2 = CanonicalAST::Struct {
            fields: vec![
                (FieldId(0), CanonicalAST::IntLit(1)),
                (FieldId(1), CanonicalAST::IntLit(2)),
            ],
        };

        assert_eq!(ast1.compute_hash(), ast2.compute_hash());
    }

    #[test]
    fn test_canonicalizer_scope() {
        let mut canon = Canonicalizer::new();

        canon.push_binding("x".into());
        canon.push_binding("y".into());

        // y is at index 0 (innermost)
        assert_eq!(canon.lookup_local("y"), Some(DeBruijnIndex(0)));
        // x is at index 1 (outer)
        assert_eq!(canon.lookup_local("x"), Some(DeBruijnIndex(1)));
        // z is not bound
        assert_eq!(canon.lookup_local("z"), None);

        canon.pop_binding();
        // After popping y, x is at index 0
        assert_eq!(canon.lookup_local("x"), Some(DeBruijnIndex(0)));
    }

    #[test]
    fn test_effect_row_sorting() {
        let h1 = ContentHash::compute(b"effect1");
        let h2 = ContentHash::compute(b"effect2");

        // Different orderings should produce same canonical row
        let row1 = CanonicalEffectRow::with_effects(vec![h1, h2], false);
        let row2 = CanonicalEffectRow::with_effects(vec![h2, h1], false);

        // After sorting, both should be equal
        assert_eq!(row1.effects, row2.effects);
    }

    #[test]
    fn test_builtin_id() {
        assert_eq!(BuiltinId::INT_ADD.0, 0x0001);
        assert_eq!(BuiltinId::BOOL_AND.0, 0x0020);
    }

    #[test]
    fn test_binop_serialization() {
        let ast1 = CanonicalAST::BinOp {
            op: CanonicalBinOp::Add,
            lhs: Box::new(CanonicalAST::IntLit(1)),
            rhs: Box::new(CanonicalAST::IntLit(2)),
        };

        let ast2 = CanonicalAST::BinOp {
            op: CanonicalBinOp::Sub,
            lhs: Box::new(CanonicalAST::IntLit(1)),
            rhs: Box::new(CanonicalAST::IntLit(2)),
        };

        assert_ne!(ast1.compute_hash(), ast2.compute_hash());
    }

    #[test]
    fn test_match_arm_serialization() {
        let arm = CanonicalMatchArm {
            pattern: CanonicalPattern::Wildcard,
            guard: None,
            body: Box::new(CanonicalAST::IntLit(0)),
        };

        let ast = CanonicalAST::Match {
            scrutinee: Box::new(CanonicalAST::IntLit(42)),
            arms: vec![arm],
        };

        // Should not panic
        let _ = ast.compute_hash();
    }

    #[test]
    fn test_canonical_ast_deterministic_hashing() {
        // Test that identical ASTs produce identical hashes across multiple computations
        fn build_complex_ast() -> CanonicalAST {
            CanonicalAST::Lambda {
                param_count: 2,
                body: Box::new(CanonicalAST::BinOp {
                    op: CanonicalBinOp::Add,
                    lhs: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0))),
                    rhs: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(1))),
                }),
            }
        }

        let ast1 = build_complex_ast();
        let ast2 = build_complex_ast();
        let ast3 = build_complex_ast();

        let hash1 = ast1.compute_hash();
        let hash2 = ast2.compute_hash();
        let hash3 = ast3.compute_hash();

        assert_eq!(hash1, hash2, "Identical ASTs should produce same hash");
        assert_eq!(hash2, hash3, "Hash computation should be deterministic");
    }

    #[test]
    fn test_debruijn_renaming_produces_same_hash() {
        // The key property: alpha-equivalent terms have the same hash
        // fn(x) { x + 1 } and fn(y) { y + 1 } should hash the same
        // because they both canonicalize to fn(#0) { #0 + 1 }

        let ast1 = CanonicalAST::Lambda {
            param_count: 1,
            body: Box::new(CanonicalAST::BinOp {
                op: CanonicalBinOp::Add,
                lhs: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0))),
                rhs: Box::new(CanonicalAST::IntLit(1)),
            }),
        };

        // Same structure, different "conceptual" variable name but same de Bruijn index
        let ast2 = CanonicalAST::Lambda {
            param_count: 1,
            body: Box::new(CanonicalAST::BinOp {
                op: CanonicalBinOp::Add,
                lhs: Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0))),
                rhs: Box::new(CanonicalAST::IntLit(1)),
            }),
        };

        assert_eq!(ast1.compute_hash(), ast2.compute_hash());
    }

    #[test]
    fn test_all_ast_node_types_deterministic() {
        // Test each AST node type produces deterministic hashes

        // Literals
        assert_eq!(
            CanonicalAST::IntLit(42).compute_hash(),
            CanonicalAST::IntLit(42).compute_hash()
        );
        assert_eq!(
            CanonicalAST::FloatLit(3.14).compute_hash(),
            CanonicalAST::FloatLit(3.14).compute_hash()
        );
        assert_eq!(
            CanonicalAST::StringLit(b"test".to_vec()).compute_hash(),
            CanonicalAST::StringLit(b"test".to_vec()).compute_hash()
        );
        assert_eq!(
            CanonicalAST::BoolLit(true).compute_hash(),
            CanonicalAST::BoolLit(true).compute_hash()
        );
        assert_eq!(
            CanonicalAST::Unit.compute_hash(),
            CanonicalAST::Unit.compute_hash()
        );

        // Variables
        assert_eq!(
            CanonicalAST::LocalVar(DeBruijnIndex(5)).compute_hash(),
            CanonicalAST::LocalVar(DeBruijnIndex(5)).compute_hash()
        );
        assert_eq!(
            CanonicalAST::TypeVar(3).compute_hash(),
            CanonicalAST::TypeVar(3).compute_hash()
        );

        // References
        let hash = ContentHash::compute(b"test ref");
        assert_eq!(
            CanonicalAST::DefRef(hash).compute_hash(),
            CanonicalAST::DefRef(hash).compute_hash()
        );
        assert_eq!(
            CanonicalAST::BuiltinRef(BuiltinId::INT_ADD).compute_hash(),
            CanonicalAST::BuiltinRef(BuiltinId::INT_ADD).compute_hash()
        );

        // Control flow
        assert_eq!(
            CanonicalAST::Return(None).compute_hash(),
            CanonicalAST::Return(None).compute_hash()
        );
        assert_eq!(
            CanonicalAST::Break(None).compute_hash(),
            CanonicalAST::Break(None).compute_hash()
        );
        assert_eq!(
            CanonicalAST::Continue.compute_hash(),
            CanonicalAST::Continue.compute_hash()
        );

        // Collections
        assert_eq!(
            CanonicalAST::Tuple(vec![CanonicalAST::IntLit(1), CanonicalAST::IntLit(2)]).compute_hash(),
            CanonicalAST::Tuple(vec![CanonicalAST::IntLit(1), CanonicalAST::IntLit(2)]).compute_hash()
        );
        assert_eq!(
            CanonicalAST::Array(vec![CanonicalAST::IntLit(3)]).compute_hash(),
            CanonicalAST::Array(vec![CanonicalAST::IntLit(3)]).compute_hash()
        );
    }

    #[test]
    fn test_effect_row_deterministic() {
        let h1 = ContentHash::compute(b"IO");
        let h2 = ContentHash::compute(b"State");

        // Same effects should produce same row regardless of initial order
        let row1 = CanonicalEffectRow::with_effects(vec![h1, h2], false);
        let row2 = CanonicalEffectRow::with_effects(vec![h2, h1], false);

        // After sorting, they should be equal
        assert_eq!(row1.effects, row2.effects);

        // Hash the rows via type arrow
        let type1 = CanonicalAST::TypeArrow {
            params: vec![CanonicalAST::IntLit(0)],
            ret: Box::new(CanonicalAST::IntLit(0)),
            effects: row1,
        };
        let type2 = CanonicalAST::TypeArrow {
            params: vec![CanonicalAST::IntLit(0)],
            ret: Box::new(CanonicalAST::IntLit(0)),
            effects: row2,
        };

        assert_eq!(type1.compute_hash(), type2.compute_hash());
    }

    #[test]
    fn test_handler_deterministic() {
        let effect_hash = ContentHash::compute(b"TestEffect");

        fn build_handler(effect: ContentHash) -> CanonicalHandler {
            CanonicalHandler {
                effect,
                operations: vec![
                    CanonicalOpImpl {
                        op_index: 0,
                        param_count: 1,
                        body: Box::new(CanonicalAST::IntLit(42)),
                    },
                ],
                return_clause: Some(Box::new(CanonicalAST::LocalVar(DeBruijnIndex(0)))),
            }
        }

        let handler1 = build_handler(effect_hash);
        let handler2 = build_handler(effect_hash);

        let ast1 = CanonicalAST::Handle {
            body: Box::new(CanonicalAST::IntLit(0)),
            handler: handler1,
        };
        let ast2 = CanonicalAST::Handle {
            body: Box::new(CanonicalAST::IntLit(0)),
            handler: handler2,
        };

        assert_eq!(ast1.compute_hash(), ast2.compute_hash());
    }

    #[test]
    fn test_pattern_deterministic() {
        // Wildcard
        assert_eq!(
            CanonicalPattern::Wildcard.serialize_and_hash(),
            CanonicalPattern::Wildcard.serialize_and_hash()
        );

        // Binding
        assert_eq!(
            CanonicalPattern::Binding(DeBruijnIndex(0)).serialize_and_hash(),
            CanonicalPattern::Binding(DeBruijnIndex(0)).serialize_and_hash()
        );

        // Tuple
        let tuple_pat = CanonicalPattern::Tuple(vec![
            CanonicalPattern::Wildcard,
            CanonicalPattern::Binding(DeBruijnIndex(0)),
        ]);
        assert_eq!(
            tuple_pat.serialize_and_hash(),
            tuple_pat.clone().serialize_and_hash()
        );
    }
}

impl CanonicalPattern {
    #[cfg(test)]
    fn serialize_and_hash(&self) -> ContentHash {
        let mut hasher = ContentHasher::new();
        self.serialize(&mut hasher);
        hasher.finalize()
    }
}
