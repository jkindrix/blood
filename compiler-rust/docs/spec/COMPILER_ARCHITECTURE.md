# Blood Compiler Architecture

Internal architecture documentation for contributors working on the Blood compiler.

## Table of Contents

1. [Overview](#1-overview)
2. [Compilation Pipeline](#2-compilation-pipeline)
3. [Lexer](#3-lexer)
4. [Parser](#4-parser)
5. [HIR (High-level IR)](#5-hir-high-level-ir)
6. [Type Checking](#6-type-checking)
7. [MIR (Mid-level IR)](#7-mir-mid-level-ir)
8. [Code Generation](#8-code-generation)
9. [Effect System](#9-effect-system)
10. [Key Design Decisions](#10-key-design-decisions)

---

## 1. Overview

The Blood compiler (`bloodc`) transforms Blood source code into machine code via LLVM. It uses a multi-pass architecture with several intermediate representations.

### High-Level Flow

```
Source (.blood) → Lexer → Parser → HIR → TypeChecker → MIR → Codegen → LLVM IR → Object Code
```

### Design Principles

1. **Explicit over implicit**: No hidden control flow, explicit error handling
2. **Incremental compilation**: Content-addressed caching for fast rebuilds
3. **Rich diagnostics**: Helpful error messages with source locations
4. **Zero shortcuts**: Per CLAUDE.md, no silent failures or catch-all patterns

---

## 2. Compilation Pipeline

### Entry Point

The compiler entry point is in `main.rs`:

```rust
// bloodc/src/main.rs
fn main() {
    let args = Args::parse();
    match args.command {
        Command::Check { file } => check_file(&file),
        Command::Build { file } => build_file(&file),
        Command::Run { file } => run_file(&file),
    }
}
```

### Phase Execution

```rust
// Simplified compilation flow
fn compile(source: &str) -> Result<CompiledModule, Vec<Diagnostic>> {
    // Phase 1: Lexing
    let tokens = lexer::lex(source)?;

    // Phase 2: Parsing
    let ast = parser::parse(&tokens)?;

    // Phase 3: HIR lowering
    let hir = hir::lower(ast)?;

    // Phase 4: Type checking
    let typed_hir = typeck::check(hir)?;

    // Phase 5: MIR lowering
    let mir = mir::lower(typed_hir)?;

    // Phase 6: Code generation
    let llvm_module = codegen::generate(mir)?;

    Ok(llvm_module)
}
```

---

## 3. Lexer

**Location**: `bloodc/src/lexer.rs`

The lexer uses the `logos` crate for fast tokenization.

### Token Types

```rust
#[derive(Logos, Debug, Clone)]
pub enum Token {
    // Keywords
    #[token("fn")]
    Fn,
    #[token("let")]
    Let,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("effect")]
    Effect,
    #[token("handler")]
    Handler,
    // ...

    // Literals
    #[regex(r"[0-9]+")]
    IntLit,
    #[regex(r"[0-9]+\.[0-9]+")]
    FloatLit,
    #[regex(r#""[^"]*""#)]
    StringLit,

    // Identifiers
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,
}
```

### Span Tracking

Every token carries source location information:

```rust
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub file_id: FileId,
}
```

---

## 4. Parser

**Location**: `bloodc/src/parser/`

The parser is a hand-written recursive descent parser with Pratt parsing for expressions.

### Module Structure

```
parser/
├── mod.rs      # Main parser, declarations
├── expr.rs     # Expression parsing (Pratt parser)
├── item.rs     # Top-level items (fn, struct, enum, effect)
├── pattern.rs  # Pattern parsing
└── types.rs    # Type annotation parsing
```

### AST Types

```rust
// bloodc/src/parser/mod.rs
pub struct Ast {
    pub items: Vec<Item>,
}

pub enum Item {
    Function(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    Effect(EffectDef),
    Handler(HandlerDef),
    Impl(ImplBlock),
    // ...
}

pub struct FnDef {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_ty: Option<Type>,
    pub effects: EffectRow,
    pub body: Option<Block>,
    pub span: Span,
}
```

### Expression Parsing (Pratt)

The expression parser uses precedence climbing for operators:

```rust
// bloodc/src/parser/expr.rs
fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
    let mut lhs = self.parse_atom()?;

    loop {
        let Some(op) = self.peek_operator() else { break };
        let (l_bp, r_bp) = op.binding_power();

        if l_bp < min_bp { break }

        self.advance();
        let rhs = self.parse_expr(r_bp)?;
        lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
    }

    Ok(lhs)
}
```

---

## 5. HIR (High-level IR)

**Location**: `bloodc/src/hir/`

HIR is a desugared representation with resolved names and explicit types.

### Key Differences from AST

| AST | HIR |
|-----|-----|
| String identifiers | DefIds (interned) |
| Optional types | All types present |
| Surface syntax sugar | Desugared forms |
| No type information | Types on all expressions |

### HIR Types

```rust
// bloodc/src/hir/mod.rs
pub struct Module {
    pub items: IndexMap<DefId, Item>,
}

pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

pub enum ExprKind {
    Literal(Literal),
    Local(LocalId),
    Def(DefId),
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    MethodCall { receiver: Box<Expr>, method: Symbol, args: Vec<Expr> },
    Field { base: Box<Expr>, field: Symbol },
    Index { base: Box<Expr>, index: Box<Expr> },
    If { condition: Box<Expr>, then_branch: Box<Expr>, else_branch: Option<Box<Expr>> },
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },
    Block { stmts: Vec<Stmt>, expr: Option<Box<Expr>> },
    Closure { params: Vec<Param>, body: Box<Body>, captures: Vec<LocalId> },
    // Effect operations
    Perform { effect: DefId, op_index: u32, args: Vec<Expr> },
    Handle { body: Box<Expr>, handler: DefId, handler_instance: Box<Expr> },
    Resume { value: Option<Box<Expr>> },
    // ...
}
```

### DefId System

DefIds uniquely identify all definitions:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId {
    pub index: u32,
}

// Usage
let fn_id: DefId = context.define_function(...);
let struct_id: DefId = context.define_struct(...);
```

---

## 6. Type Checking

**Location**: `bloodc/src/typeck/`

Type checking performs inference, checking, and effect inference.

### Module Structure

```
typeck/
├── context/
│   ├── mod.rs        # TypeckContext - main state
│   ├── builtins.rs   # Built-in types and functions
│   ├── check.rs      # Type checking entry point
│   ├── collect.rs    # Item collection phase
│   ├── expr.rs       # Expression type inference
│   ├── patterns.rs   # Pattern checking
│   ├── closure.rs    # Closure type inference
│   └── traits.rs     # Trait resolution
├── dispatch.rs       # Multiple dispatch resolution
├── exhaustiveness.rs # Pattern exhaustiveness checking
├── ffi.rs           # FFI type validation
└── effects/
    └── infer.rs     # Effect inference
```

### Type Representation

```rust
// bloodc/src/hir/mod.rs
pub enum Type {
    // Primitives
    Int(IntTy),       // i8, i16, i32, i64
    Uint(UintTy),     // u8, u16, u32, u64
    Float(FloatTy),   // f32, f64
    Bool,
    Str,
    Unit,
    Never,            // Bottom type (no values)

    // Compound
    Tuple(Vec<Type>),
    Array(Box<Type>, usize),
    Slice(Box<Type>),

    // References
    Ref(Box<Type>, Mutability),
    Ptr(Box<Type>, Mutability),

    // User-defined
    Named(DefId, Vec<Type>),  // struct/enum with type args

    // Functions
    Function(FnType),

    // Type variables (during inference)
    Var(TypeVarId),

    // Error recovery
    Error,

    // Linear/affine modifiers
    Linear(Box<Type>),
    Affine(Box<Type>),
}
```

### Type Unification

```rust
// Unification: finds substitutions making two types equal
fn unify(&mut self, t1: &Type, t2: &Type) -> Result<(), TypeError> {
    match (t1, t2) {
        // Same concrete types unify
        (Type::Int(a), Type::Int(b)) if a == b => Ok(()),
        (Type::Bool, Type::Bool) => Ok(()),

        // Type variables
        (Type::Var(v), t) | (t, Type::Var(v)) => {
            self.bind_var(*v, t.clone())
        }

        // Recursive cases
        (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => {
            for (t1, t2) in a.iter().zip(b.iter()) {
                self.unify(t1, t2)?;
            }
            Ok(())
        }

        // Type mismatch
        _ => Err(TypeError::Mismatch { expected: t1.clone(), found: t2.clone() }),
    }
}
```

### Effect Inference

```rust
// Effect inference determines which effects a function may perform
fn infer_effects(&mut self, expr: &Expr) -> EffectSet {
    match &expr.kind {
        ExprKind::Perform { effect, .. } => {
            EffectSet::singleton(*effect)
        }
        ExprKind::Call { callee, .. } => {
            // Include callee's effects
            let callee_effects = self.get_fn_effects(callee);
            callee_effects
        }
        ExprKind::Handle { body, handler, .. } => {
            // Handler masks certain effects
            let body_effects = self.infer_effects(body);
            let handled = self.get_handler_effect(*handler);
            body_effects.minus(handled)
        }
        // ...
    }
}
```

---

## 7. MIR (Mid-level IR)

**Location**: `bloodc/src/mir/`

MIR is a control-flow graph representation suitable for analysis and optimization.

### Module Structure

```
mir/
├── mod.rs           # MIR types
├── lowering/
│   ├── mod.rs       # Main lowering entry
│   ├── function.rs  # Function lowering
│   ├── closure.rs   # Closure lowering
│   └── util.rs      # Lowering utilities
└── types.rs         # MIR type definitions
```

### MIR Structure

```rust
// Control-flow graph representation
pub struct MirBody {
    pub basic_blocks: Vec<BasicBlock>,
    pub locals: Vec<Local>,
    pub arg_count: usize,
}

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

pub enum Statement {
    Assign(Place, RValue),
    StorageLive(LocalId),
    StorageDead(LocalId),
}

pub enum Terminator {
    Return(Option<Operand>),
    Goto(BlockId),
    If { cond: Operand, then_block: BlockId, else_block: BlockId },
    Switch { value: Operand, targets: Vec<(Constant, BlockId)>, default: BlockId },
    Call { func: Operand, args: Vec<Operand>, dest: Place, next: BlockId },
    // Effect operations
    Perform { effect: DefId, op: u32, args: Vec<Operand>, dest: Place, next: BlockId },
    Resume { value: Option<Operand> },
}
```

### HIR to MIR Lowering

```rust
// bloodc/src/mir/lowering/function.rs
fn lower_expr(&mut self, expr: &hir::Expr) -> Operand {
    match &expr.kind {
        hir::ExprKind::Binary { op, left, right } => {
            let l = self.lower_expr(left);
            let r = self.lower_expr(right);
            let result = self.new_temp(expr.ty.clone());
            self.emit(Statement::Assign(
                Place::local(result),
                RValue::BinaryOp(*op, l, r)
            ));
            Operand::Move(Place::local(result))
        }

        hir::ExprKind::If { condition, then_branch, else_branch } => {
            let cond = self.lower_expr(condition);
            let then_block = self.new_block();
            let else_block = self.new_block();
            let join_block = self.new_block();

            self.terminate(Terminator::If {
                cond,
                then_block,
                else_block,
            });

            // Lower then branch
            self.switch_to(then_block);
            let then_val = self.lower_expr(then_branch);
            self.terminate(Terminator::Goto(join_block));

            // Lower else branch
            self.switch_to(else_block);
            let else_val = else_branch.as_ref()
                .map(|e| self.lower_expr(e))
                .unwrap_or(Operand::Constant(Constant::Unit));
            self.terminate(Terminator::Goto(join_block));

            // Join
            self.switch_to(join_block);
            // Phi node to merge values
            self.create_phi(then_val, else_val)
        }
        // ...
    }
}
```

---

## 8. Code Generation

**Location**: `bloodc/src/codegen/`

Code generation translates MIR to LLVM IR using the inkwell crate.

### Module Structure

```
codegen/
├── mod.rs              # Entry point, module setup
├── context/
│   ├── mod.rs          # CodegenContext - main state
│   ├── expr.rs         # Expression codegen
│   ├── stmt.rs         # Statement codegen
│   ├── effects.rs      # Effect operation codegen
│   └── tests.rs        # Codegen tests
└── mir_codegen/
    ├── mod.rs          # MIR to LLVM lowering
    ├── rvalue.rs       # RValue codegen
    └── terminator.rs   # Terminator codegen
```

### CodegenContext

```rust
// bloodc/src/codegen/context/mod.rs
pub struct CodegenContext<'ctx, 'a> {
    pub context: &'ctx Context,
    pub module: &'a Module<'ctx>,
    pub builder: Builder<'ctx>,

    // Current function state
    pub current_fn: Option<FunctionValue<'ctx>>,
    pub locals: HashMap<LocalId, PointerValue<'ctx>>,

    // Effect state
    pub current_continuation: Option<PointerValue<'ctx>>,
    pub is_multishot_handler: bool,

    // Type cache
    pub type_cache: HashMap<Type, BasicTypeEnum<'ctx>>,
}
```

### Type Lowering

```rust
fn lower_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
    match ty {
        Type::Int(IntTy::I32) => self.context.i32_type().into(),
        Type::Int(IntTy::I64) => self.context.i64_type().into(),
        Type::Float(FloatTy::F32) => self.context.f32_type().into(),
        Type::Float(FloatTy::F64) => self.context.f64_type().into(),
        Type::Bool => self.context.bool_type().into(),

        Type::Tuple(elems) => {
            let elem_types: Vec<_> = elems.iter()
                .map(|t| self.lower_type(t))
                .collect();
            self.context.struct_type(&elem_types, false).into()
        }

        Type::Ref(inner, _) | Type::Ptr(inner, _) => {
            // 128-bit fat pointer
            let addr = self.context.i64_type();
            let gen = self.context.i32_type();
            let meta = self.context.i32_type();
            self.context.struct_type(&[addr.into(), gen.into(), meta.into()], false)
                .ptr_type(AddressSpace::default())
                .into()
        }
        // ...
    }
}
```

### MIR Codegen

```rust
// bloodc/src/codegen/mir_codegen/mod.rs
fn generate_basic_block(&mut self, block: &BasicBlock) {
    for stmt in &block.statements {
        self.generate_statement(stmt);
    }
    self.generate_terminator(&block.terminator);
}

fn generate_terminator(&mut self, term: &Terminator) {
    match term {
        Terminator::Return(val) => {
            match val {
                Some(op) => {
                    let v = self.load_operand(op);
                    self.builder.build_return(Some(&v));
                }
                None => {
                    self.builder.build_return(None);
                }
            }
        }

        Terminator::If { cond, then_block, else_block } => {
            let cond_val = self.load_operand(cond);
            let then_bb = self.get_block(*then_block);
            let else_bb = self.get_block(*else_block);
            self.builder.build_conditional_branch(cond_val, then_bb, else_bb);
        }
        // ...
    }
}
```

---

## 9. Effect System

**Location**: `bloodc/src/effects/` and `bloodc/src/codegen/context/effects.rs`

### Effect Declarations

```blood
effect State<S> {
    op get() -> S
    op put(s: S) -> ()
}
```

Represented as:

```rust
pub struct EffectDef {
    pub name: Symbol,
    pub generics: Vec<GenericParam>,
    pub operations: Vec<EffectOperation>,
}

pub struct EffectOperation {
    pub name: Symbol,
    pub params: Vec<Type>,
    pub return_ty: Type,
    pub is_resumptive: bool,  // true unless return is `never`
}
```

### Handler Compilation

Handlers compile to:
1. **Operation functions** - One per operation
2. **Return clause** - Transforms final result
3. **Evidence vector entry** - For runtime dispatch

```rust
// Simplified handler codegen
fn compile_handler(&mut self, handler: &HandlerDef) -> Result<(), Diagnostic> {
    // Generate operation functions
    for (i, op) in handler.operations.iter().enumerate() {
        let fn_name = format!("handler_{}_{}", handler.id.index, op.name);
        self.compile_operation_fn(&fn_name, op)?;
    }

    // Generate return clause
    let return_fn_name = format!("handler_{}_return", handler.id.index);
    self.compile_return_clause(&return_fn_name, &handler.return_clause)?;

    Ok(())
}
```

### Evidence Passing

Effects use the evidence passing model from ICFP'21:

```rust
// Runtime: evidence vector management
pub struct EvidenceVector {
    entries: Vec<EvidenceEntry>,
}

pub struct EvidenceEntry {
    registry_index: u64,  // Handler in global registry
    state: *mut c_void,   // Handler instance state
}

// Codegen: perform expression
fn compile_perform(&mut self, effect: DefId, op_index: u32, args: &[Expr]) {
    // 1. Look up handler in evidence vector
    // 2. Get operation function pointer
    // 3. Create continuation for non-tail-resumptive
    // 4. Call operation with state and continuation
}
```

---

## 10. Key Design Decisions

### ADR-022: Slot Registry for Generation Tracking

Generation validation uses a global hash table mapping addresses to generations:

```rust
struct SlotRegistry {
    slots: HashMap<u64, SlotEntry>,
}

struct SlotEntry {
    generation: u32,
    size: u32,
    in_use: bool,
}
```

**Rationale**: O(1) lookup, allows efficient validation without scanning memory.

### ADR-023: MIR as Intermediate Representation

Blood uses MIR between HIR and LLVM IR:

**Benefits**:
- Explicit control flow (easier to analyze)
- Uniform representation (pattern match → switch)
- Good target for optimizations

**Tradeoffs**:
- Additional compilation phase
- More code to maintain

### ADR-024: Closure Capture by LocalId Comparison

Closure capture analysis compares LocalIds:

```rust
fn detect_captures(&self, closure_body: &Body) -> Vec<LocalId> {
    let defined_in_closure = self.collect_defined_locals(closure_body);
    let used_locals = self.collect_used_locals(closure_body);

    used_locals.into_iter()
        .filter(|id| !defined_in_closure.contains(id))
        .collect()
}
```

**Rationale**: Simple, correct, works with nested closures.

---

## Related Documentation

- [CONTRIBUTING.md](../../CONTRIBUTING.md) - Contributor guide
- [DECISIONS.md](./DECISIONS.md) - Architecture Decision Records
- [SPECIFICATION.md](./SPECIFICATION.md) - Language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) - Memory model details

---

*Last updated: 2026-01-13*
