//! Syntax kinds for the concrete syntax tree (CST).
//!
//! The `SyntaxKind` enum represents all possible node types in the Blood
//! concrete syntax tree. This includes:
//!
//! - **Tokens**: Terminal nodes from the lexer
//! - **Nodes**: Non-terminal syntax tree nodes
//! - **Trivia**: Comments and whitespace
//!
//! This design follows the approach used by rust-analyzer and provides
//! a unified type system for both the lexer output and parser output.
//!
//! # Architecture
//!
//! In a full CST implementation, the tree would be structured as:
//!
//! ```text
//! SyntaxNode (SyntaxKind, TextRange)
//!   └── children: Vec<SyntaxNode | SyntaxToken>
//!
//! SyntaxToken (SyntaxKind, TextRange, trivia)
//! ```
//!
//! This module provides the kind enumeration only; the actual tree
//! implementation is planned for future development.

/// A kind of syntax element in the Blood language.
///
/// This enum is designed to be:
/// - **Exhaustive**: Covers all possible syntax elements
/// - **Efficient**: Fits in a single byte when possible
/// - **Stable**: Allows for lossless syntax tree operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // ============================================================
    // Tokens - Literals
    // ============================================================
    /// Integer literal: `42`, `0xFF`
    IntLit = 0,
    /// Float literal: `3.14`
    FloatLit,
    /// String literal: `"hello"`
    StringLit,
    /// Character literal: `'a'`
    CharLit,
    /// Boolean literal: `true`, `false`
    BoolLit,

    // ============================================================
    // Tokens - Identifiers
    // ============================================================
    /// Lowercase identifier: `foo`, `bar`
    Ident,
    /// Uppercase identifier (type names): `Foo`, `Bar`
    TypeIdent,
    /// Lifetime: `'a`, `'static`
    Lifetime,

    // ============================================================
    // Tokens - Keywords
    // ============================================================
    /// `fn`
    KwFn,
    /// `let`
    KwLet,
    /// `mut`
    KwMut,
    /// `const`
    KwConst,
    /// `static`
    KwStatic,
    /// `if`
    KwIf,
    /// `else`
    KwElse,
    /// `match`
    KwMatch,
    /// `while`
    KwWhile,
    /// `for`
    KwFor,
    /// `loop`
    KwLoop,
    /// `break`
    KwBreak,
    /// `continue`
    KwContinue,
    /// `return`
    KwReturn,
    /// `struct`
    KwStruct,
    /// `enum`
    KwEnum,
    /// `trait`
    KwTrait,
    /// `impl`
    KwImpl,
    /// `type`
    KwType,
    /// `where`
    KwWhere,
    /// `use`
    KwUse,
    /// `pub`
    KwPub,
    /// `as`
    KwAs,
    /// `in`
    KwIn,
    /// `ref`
    KwRef,
    /// `self`
    KwSelf,
    /// `Self`
    KwSelfUpper,
    /// `crate`
    KwCrate,
    /// `super`
    KwSuper,
    /// `module`
    KwModule,

    // Effect system keywords
    /// `effect`
    KwEffect,
    /// `handler`
    KwHandler,
    /// `perform`
    KwPerform,
    /// `resume`
    KwResume,
    /// `with`
    KwWith,
    /// `handle`
    KwHandle,
    /// `pure`
    KwPure,
    /// `deep`
    KwDeep,
    /// `shallow`
    KwShallow,

    // Capability keywords
    /// `async`
    KwAsync,
    /// `await`
    KwAwait,
    /// `unsafe`
    KwUnsafe,
    /// `move`
    KwMove,

    // Ownership keywords
    /// `linear`
    KwLinear,
    /// `affine`
    KwAffine,

    // ============================================================
    // Tokens - Punctuation
    // ============================================================
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `,`
    Comma,
    /// `;`
    Semi,
    /// `:`
    Colon,
    /// `::`
    ColonColon,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `..=`
    DotDotEq,
    /// `=>`
    FatArrow,
    /// `->`
    Arrow,
    /// `<-`
    LeftArrow,
    /// `@`
    At,
    /// `#`
    Hash,
    /// `?`
    Question,

    // ============================================================
    // Tokens - Operators
    // ============================================================
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `=`
    Eq,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,
    /// `!`
    Not,
    /// `&`
    And,
    /// `|`
    Or,
    /// `^`
    Caret,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `|>`
    Pipe,

    // Compound assignment
    /// `+=`
    PlusEq,
    /// `-=`
    MinusEq,
    /// `*=`
    StarEq,
    /// `/=`
    SlashEq,
    /// `%=`
    PercentEq,
    /// `&=`
    AndEq,
    /// `|=`
    OrEq,
    /// `^=`
    CaretEq,
    /// `<<=`
    ShlEq,
    /// `>>=`
    ShrEq,

    // ============================================================
    // Tokens - Special
    // ============================================================
    /// End of file
    Eof,
    /// Lexer error
    Error,

    // ============================================================
    // Trivia
    // ============================================================
    /// Whitespace (spaces, tabs, newlines)
    Whitespace,
    /// Line comment: `// ...`
    LineComment,
    /// Block comment: `/* ... */`
    BlockComment,
    /// Doc comment: `/// ...` or `//! ...`
    DocComment,

    // ============================================================
    // Syntax Nodes - Program Structure
    // ============================================================
    /// Root node of the syntax tree
    SourceFile,
    /// Module declaration: `module foo.bar;`
    ModuleDecl,
    /// Import declaration: `use foo::bar;`
    ImportDecl,
    /// Import path segment
    ImportPath,
    /// Import tree for complex imports
    ImportTree,

    // ============================================================
    // Syntax Nodes - Items (Declarations)
    // ============================================================
    /// Function declaration
    FnDecl,
    /// Struct declaration
    StructDecl,
    /// Enum declaration
    EnumDecl,
    /// Enum variant
    EnumVariant,
    /// Trait declaration
    TraitDecl,
    /// Impl block
    ImplBlock,
    /// Type alias declaration
    TypeAlias,
    /// Effect declaration
    EffectDecl,
    /// Handler declaration
    HandlerDecl,
    /// Operation declaration (in effect)
    OpDecl,
    /// Const declaration
    ConstDecl,
    /// Static declaration
    StaticDecl,

    // ============================================================
    // Syntax Nodes - Type Expressions
    // ============================================================
    /// Named type: `Foo`, `Bar<T>`
    TypePath,
    /// Type path segment
    TypePathSegment,
    /// Type arguments: `<T, U>`
    TypeArgs,
    /// Tuple type: `(A, B, C)`
    TupleType,
    /// Array type: `[T; N]`
    ArrayType,
    /// Slice type: `[T]`
    SliceType,
    /// Reference type: `&T`, `&mut T`
    RefType,
    /// Pointer type: `*T`, `*mut T`
    PtrType,
    /// Function type: `fn(A) -> B`
    FnType,
    /// Never type: `!`
    NeverType,
    /// Inferred type: `_`
    InferredType,
    /// Record type: `{ x: T, y: U }`
    RecordType,
    /// Record type field
    RecordTypeField,
    /// Ownership qualified type: `linear T`
    OwnershipType,
    /// Effect row: `{E1, E2 | R}`
    EffectRow,

    // ============================================================
    // Syntax Nodes - Patterns
    // ============================================================
    /// Wildcard pattern: `_`
    WildcardPat,
    /// Binding pattern: `x`, `mut x`
    BindingPat,
    /// Literal pattern: `42`, `"hello"`
    LitPat,
    /// Tuple pattern: `(a, b)`
    TuplePat,
    /// Struct pattern: `Foo { x, y }`
    StructPat,
    /// Struct pattern field
    StructPatField,
    /// Enum variant pattern: `Some(x)`
    EnumPat,
    /// Array pattern: `[a, b, c]`
    ArrayPat,
    /// Slice pattern: `[head, .., tail]`
    SlicePat,
    /// Reference pattern: `&x`
    RefPat,
    /// Or pattern: `A | B`
    OrPat,
    /// Range pattern: `1..10`
    RangePat,
    /// Guard pattern: `x if x > 0`
    GuardPat,
    /// Rest pattern: `..`
    RestPat,

    // ============================================================
    // Syntax Nodes - Expressions
    // ============================================================
    /// Literal expression
    LitExpr,
    /// Path expression: `foo::bar`
    PathExpr,
    /// Unary expression: `!x`, `-x`
    UnaryExpr,
    /// Binary expression: `a + b`
    BinaryExpr,
    /// Grouped expression: `(expr)`
    ParenExpr,
    /// Tuple expression: `(a, b, c)`
    TupleExpr,
    /// Array expression: `[a, b, c]`
    ArrayExpr,
    /// Array repeat: `[x; n]`
    ArrayRepeatExpr,
    /// Index expression: `arr[i]`
    IndexExpr,
    /// Field access: `obj.field`
    FieldExpr,
    /// Method call: `obj.method(args)`
    MethodCallExpr,
    /// Function call: `func(args)`
    CallExpr,
    /// Call argument list
    ArgList,
    /// Struct literal: `Foo { x: 1 }`
    StructExpr,
    /// Struct literal field
    StructExprField,
    /// If expression: `if cond { } else { }`
    IfExpr,
    /// Match expression: `match x { ... }`
    MatchExpr,
    /// Match arm
    MatchArm,
    /// Loop expression: `loop { }`
    LoopExpr,
    /// While expression: `while cond { }`
    WhileExpr,
    /// For expression: `for x in iter { }`
    ForExpr,
    /// Block expression: `{ ... }`
    BlockExpr,
    /// Return expression: `return x`
    ReturnExpr,
    /// Break expression: `break x`
    BreakExpr,
    /// Continue expression: `continue`
    ContinueExpr,
    /// Closure expression: `|x| x + 1`
    ClosureExpr,
    /// Closure parameter list
    ClosureParams,
    /// Range expression: `a..b`
    RangeExpr,
    /// Cast expression: `x as T`
    CastExpr,
    /// Reference expression: `&x`, `&mut x`
    RefExpr,
    /// Dereference expression: `*x`
    DerefExpr,
    /// Assignment expression: `x = y`
    AssignExpr,
    /// Compound assignment: `x += y`
    CompoundAssignExpr,
    /// Let expression: `let x = y`
    LetExpr,
    /// Await expression: `x.await`
    AwaitExpr,
    /// Try expression: `x?`
    TryExpr,
    /// Perform expression: `perform op()`
    PerformExpr,
    /// Handle expression: `with handler handle expr`
    HandleExpr,
    /// Resume expression: `resume(x)`
    ResumeExpr,
    /// Unsafe block: `unsafe { }`
    UnsafeExpr,
    /// Async block: `async { }`
    AsyncExpr,

    // ============================================================
    // Syntax Nodes - Statements
    // ============================================================
    /// Expression statement
    ExprStmt,
    /// Let statement: `let x = y;`
    LetStmt,
    /// Item statement (declaration in block)
    ItemStmt,

    // ============================================================
    // Syntax Nodes - Components
    // ============================================================
    /// Function parameter
    Param,
    /// Parameter list
    ParamList,
    /// Generic parameter: `T`, `T: Trait`
    GenericParam,
    /// Generic parameter list
    GenericParamList,
    /// Where clause
    WhereClause,
    /// Where predicate
    WherePredicate,
    /// Trait bound: `T: Foo + Bar`
    TraitBound,
    /// Struct field
    StructField,
    /// Visibility modifier: `pub`
    Visibility,
    /// Attribute: `#[...]`
    Attribute,
    /// Attribute argument
    AttrArg,
    /// Handler operation clause
    HandlerOp,
    /// Return clause in handler
    ReturnClause,
}

impl SyntaxKind {
    /// Returns true if this kind represents a token (terminal).
    #[inline]
    pub const fn is_token(self) -> bool {
        (self as u16) < (SyntaxKind::Whitespace as u16)
    }

    /// Returns true if this kind represents trivia.
    #[inline]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Whitespace
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
                | SyntaxKind::DocComment
        )
    }

    /// Returns true if this kind represents a keyword.
    #[inline]
    pub const fn is_keyword(self) -> bool {
        matches!(
            self,
            SyntaxKind::KwFn
                | SyntaxKind::KwLet
                | SyntaxKind::KwMut
                | SyntaxKind::KwConst
                | SyntaxKind::KwStatic
                | SyntaxKind::KwIf
                | SyntaxKind::KwElse
                | SyntaxKind::KwMatch
                | SyntaxKind::KwWhile
                | SyntaxKind::KwFor
                | SyntaxKind::KwLoop
                | SyntaxKind::KwBreak
                | SyntaxKind::KwContinue
                | SyntaxKind::KwReturn
                | SyntaxKind::KwStruct
                | SyntaxKind::KwEnum
                | SyntaxKind::KwTrait
                | SyntaxKind::KwImpl
                | SyntaxKind::KwType
                | SyntaxKind::KwWhere
                | SyntaxKind::KwUse
                | SyntaxKind::KwPub
                | SyntaxKind::KwAs
                | SyntaxKind::KwIn
                | SyntaxKind::KwRef
                | SyntaxKind::KwSelf
                | SyntaxKind::KwSelfUpper
                | SyntaxKind::KwCrate
                | SyntaxKind::KwSuper
                | SyntaxKind::KwModule
                | SyntaxKind::KwEffect
                | SyntaxKind::KwHandler
                | SyntaxKind::KwPerform
                | SyntaxKind::KwResume
                | SyntaxKind::KwWith
                | SyntaxKind::KwHandle
                | SyntaxKind::KwPure
                | SyntaxKind::KwDeep
                | SyntaxKind::KwShallow
                | SyntaxKind::KwAsync
                | SyntaxKind::KwAwait
                | SyntaxKind::KwUnsafe
                | SyntaxKind::KwMove
                | SyntaxKind::KwLinear
                | SyntaxKind::KwAffine
        )
    }

    /// Returns true if this kind represents a literal.
    #[inline]
    pub const fn is_literal(self) -> bool {
        matches!(
            self,
            SyntaxKind::IntLit
                | SyntaxKind::FloatLit
                | SyntaxKind::StringLit
                | SyntaxKind::CharLit
                | SyntaxKind::BoolLit
        )
    }

    /// Returns true if this kind represents a punctuation token.
    #[inline]
    pub const fn is_punct(self) -> bool {
        matches!(
            self,
            SyntaxKind::LParen
                | SyntaxKind::RParen
                | SyntaxKind::LBrace
                | SyntaxKind::RBrace
                | SyntaxKind::LBracket
                | SyntaxKind::RBracket
                | SyntaxKind::Lt
                | SyntaxKind::Gt
                | SyntaxKind::Comma
                | SyntaxKind::Semi
                | SyntaxKind::Colon
                | SyntaxKind::ColonColon
                | SyntaxKind::Dot
                | SyntaxKind::DotDot
                | SyntaxKind::DotDotEq
                | SyntaxKind::FatArrow
                | SyntaxKind::Arrow
                | SyntaxKind::LeftArrow
                | SyntaxKind::At
                | SyntaxKind::Hash
                | SyntaxKind::Question
        )
    }

    /// Returns true if this kind represents an operator.
    #[inline]
    pub const fn is_operator(self) -> bool {
        matches!(
            self,
            SyntaxKind::Plus
                | SyntaxKind::Minus
                | SyntaxKind::Star
                | SyntaxKind::Slash
                | SyntaxKind::Percent
                | SyntaxKind::Eq
                | SyntaxKind::EqEq
                | SyntaxKind::NotEq
                | SyntaxKind::LtEq
                | SyntaxKind::GtEq
                | SyntaxKind::Not
                | SyntaxKind::And
                | SyntaxKind::Or
                | SyntaxKind::Caret
                | SyntaxKind::AndAnd
                | SyntaxKind::OrOr
                | SyntaxKind::Shl
                | SyntaxKind::Shr
                | SyntaxKind::Pipe
                | SyntaxKind::PlusEq
                | SyntaxKind::MinusEq
                | SyntaxKind::StarEq
                | SyntaxKind::SlashEq
                | SyntaxKind::PercentEq
                | SyntaxKind::AndEq
                | SyntaxKind::OrEq
                | SyntaxKind::CaretEq
                | SyntaxKind::ShlEq
                | SyntaxKind::ShrEq
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_token() {
        assert!(SyntaxKind::IntLit.is_token());
        assert!(SyntaxKind::Ident.is_token());
        assert!(SyntaxKind::KwFn.is_token());
        assert!(!SyntaxKind::Whitespace.is_token());
        assert!(!SyntaxKind::SourceFile.is_token());
    }

    #[test]
    fn test_is_trivia() {
        assert!(SyntaxKind::Whitespace.is_trivia());
        assert!(SyntaxKind::LineComment.is_trivia());
        assert!(SyntaxKind::BlockComment.is_trivia());
        assert!(SyntaxKind::DocComment.is_trivia());
        assert!(!SyntaxKind::IntLit.is_trivia());
    }

    #[test]
    fn test_is_keyword() {
        assert!(SyntaxKind::KwFn.is_keyword());
        assert!(SyntaxKind::KwLet.is_keyword());
        assert!(SyntaxKind::KwEffect.is_keyword());
        assert!(!SyntaxKind::Ident.is_keyword());
        assert!(!SyntaxKind::IntLit.is_keyword());
    }

    #[test]
    fn test_is_literal() {
        assert!(SyntaxKind::IntLit.is_literal());
        assert!(SyntaxKind::FloatLit.is_literal());
        assert!(SyntaxKind::StringLit.is_literal());
        assert!(!SyntaxKind::Ident.is_literal());
    }

    #[test]
    fn test_syntax_kind_size() {
        // Ensure the enum fits in a reasonable size
        assert!(std::mem::size_of::<SyntaxKind>() <= 2);
    }
}
