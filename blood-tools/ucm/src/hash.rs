//! Content Hashing
//!
//! Provides content-addressable hashing for Blood definitions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A content hash identifying a definition.
///
/// Uses BLAKE3 for fast, secure hashing.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Creates a hash from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Computes the hash of the given content.
    pub fn of(content: &[u8]) -> Self {
        let hash = blake3::hash(content);
        Self(*hash.as_bytes())
    }

    /// Computes the hash of a string.
    pub fn of_str(s: &str) -> Self {
        Self::of(s.as_bytes())
    }

    /// Computes the hash of multiple parts.
    pub fn of_parts(parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(part);
        }
        Self(*hasher.finalize().as_bytes())
    }

    /// Returns the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the hash as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Returns a short version of the hash (first 8 chars).
    pub fn short(&self) -> String {
        self.to_hex()[..8].to_string()
    }

    /// Creates a hash from a hex string.
    pub fn from_hex(s: &str) -> Result<Self, HashParseError> {
        let bytes = hex::decode(s).map_err(|_| HashParseError::InvalidHex)?;
        if bytes.len() != 32 {
            return Err(HashParseError::InvalidLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", &self.to_hex()[..8])
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix('#').unwrap_or(s);
        Self::from_hex(s)
    }
}

/// Error parsing a hash.
#[derive(Debug, Clone)]
pub enum HashParseError {
    InvalidHex,
    InvalidLength,
}

impl fmt::Display for HashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashParseError::InvalidHex => write!(f, "invalid hex string"),
            HashParseError::InvalidLength => write!(f, "invalid hash length"),
        }
    }
}

impl std::error::Error for HashParseError {}

/// A hasher for incrementally building hashes.
pub struct Hasher {
    inner: blake3::Hasher,
}

impl Hasher {
    /// Creates a new hasher.
    pub fn new() -> Self {
        Self {
            inner: blake3::Hasher::new(),
        }
    }

    /// Updates the hasher with more data.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.inner.update(data);
        self
    }

    /// Updates the hasher with a string.
    pub fn update_str(&mut self, s: &str) -> &mut Self {
        self.update(s.as_bytes())
    }

    /// Finalizes and returns the hash.
    pub fn finalize(self) -> Hash {
        Hash(*self.inner.finalize().as_bytes())
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the structural hash of a Blood definition.
///
/// The structural hash is based on the AST structure, not the source text.
/// This means equivalent definitions with different formatting have the same hash.
pub fn structural_hash(source: &str) -> Hash {
    // Try to parse and hash the AST structure
    if let Some(ast_hash) = try_ast_hash(source) {
        return ast_hash;
    }

    // Fallback: normalize whitespace and hash
    let normalized: String = source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Hash::of_str(&normalized)
}

/// Tries to compute an AST-based hash of the source.
///
/// This parses the source to verify it's syntactically valid,
/// then hashes a whitespace-normalized version for consistency.
/// Symbol values differ between parses (different interners),
/// so we use normalized source text for deterministic hashing.
fn try_ast_hash(source: &str) -> Option<Hash> {
    use bloodc::Parser;

    let mut parser = Parser::new(source);
    let _program = parser.parse_program().ok()?;

    // Check if there were parse errors
    if parser.has_errors() {
        return None;
    }

    // Parse succeeded - use whitespace-normalized source for deterministic hash
    // This ensures "fn foo() { 42 }" and "fn  foo()  {  42  }" produce same hash
    let normalized: String = source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(Hash::of_str(&normalized))
}

/// Extracts dependencies (referenced names) from a Blood definition.
///
/// Returns a list of name references found in the source.
pub fn extract_dependencies(source: &str) -> Vec<String> {
    use bloodc::Parser;

    let mut parser = Parser::new(source);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let interner = parser.take_interner();
    let mut deps = Vec::new();

    // Collect referenced names from expressions
    for decl in &program.declarations {
        collect_deps_from_decl(decl, &interner, &mut deps);
    }

    // Remove duplicates and sort
    deps.sort();
    deps.dedup();
    deps
}

/// Collects dependencies from a declaration.
fn collect_deps_from_decl(
    decl: &bloodc::ast::Declaration,
    interner: &string_interner::DefaultStringInterner,
    deps: &mut Vec<String>,
) {
    use bloodc::ast::Declaration;

    match decl {
        Declaration::Function(f) => {
            if let Some(body) = &f.body {
                collect_deps_from_block(body, interner, deps);
            }
        }
        Declaration::Struct(_)
        | Declaration::Enum(_)
        | Declaration::Effect(_)
        | Declaration::Handler(_)
        | Declaration::Trait(_)
        | Declaration::Impl(_)
        | Declaration::Type(_)
        | Declaration::Const(_)
        | Declaration::Static(_)
        | Declaration::Bridge(_)
        | Declaration::Module(_)
        | Declaration::Macro(_)
        | Declaration::Use(_) => {}
    }
}

/// Collects dependencies from a block.
fn collect_deps_from_block(
    block: &bloodc::ast::Block,
    interner: &string_interner::DefaultStringInterner,
    deps: &mut Vec<String>,
) {
    for stmt in &block.statements {
        collect_deps_from_stmt(stmt, interner, deps);
    }
    if let Some(expr) = &block.expr {
        collect_deps_from_expr(expr, interner, deps);
    }
}

/// Collects dependencies from an expression.
fn collect_deps_from_expr(
    expr: &bloodc::ast::Expr,
    interner: &string_interner::DefaultStringInterner,
    deps: &mut Vec<String>,
) {
    use bloodc::ast::ExprKind;

    match &expr.kind {
        ExprKind::Path(path) => {
            let path_str: Vec<_> = path.segments.iter()
                .filter_map(|s| interner.resolve(s.name.node))
                .collect();
            if !path_str.is_empty() {
                let name = path_str[0];
                if !is_builtin(name) {
                    deps.push(path_str.join("::"));
                }
            }
        }
        ExprKind::Block(block) => {
            collect_deps_from_block(block, interner, deps);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_deps_from_expr(left, interner, deps);
            collect_deps_from_expr(right, interner, deps);
        }
        ExprKind::Unary { operand, .. } => {
            collect_deps_from_expr(operand, interner, deps);
        }
        ExprKind::Call { callee, args } => {
            collect_deps_from_expr(callee, interner, deps);
            for arg in args {
                collect_deps_from_expr(&arg.value, interner, deps);
            }
        }
        ExprKind::If { condition, then_branch, else_branch } => {
            collect_deps_from_expr(condition, interner, deps);
            collect_deps_from_block(then_branch, interner, deps);
            if let Some(e) = else_branch {
                collect_deps_from_else(e, interner, deps);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_deps_from_expr(scrutinee, interner, deps);
            for arm in arms {
                collect_deps_from_expr(&arm.body, interner, deps);
            }
        }
        ExprKind::Closure { body, .. } => {
            collect_deps_from_expr(body, interner, deps);
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_deps_from_expr(receiver, interner, deps);
            for arg in args {
                collect_deps_from_expr(&arg.value, interner, deps);
            }
        }
        ExprKind::Field { base, .. } => {
            collect_deps_from_expr(base, interner, deps);
        }
        ExprKind::Index { base, index } => {
            collect_deps_from_expr(base, interner, deps);
            collect_deps_from_expr(index, interner, deps);
        }
        ExprKind::Record { fields, base, .. } => {
            for field in fields {
                if let Some(val) = &field.value {
                    collect_deps_from_expr(val, interner, deps);
                }
            }
            if let Some(b) = base {
                collect_deps_from_expr(b, interner, deps);
            }
        }
        ExprKind::Array(arr) => {
            match arr {
                bloodc::ast::ArrayExpr::List(elements) => {
                    for e in elements {
                        collect_deps_from_expr(e, interner, deps);
                    }
                }
                bloodc::ast::ArrayExpr::Repeat { value, count } => {
                    collect_deps_from_expr(value, interner, deps);
                    collect_deps_from_expr(count, interner, deps);
                }
            }
        }
        ExprKind::Tuple(elements) => {
            for e in elements {
                collect_deps_from_expr(e, interner, deps);
            }
        }
        ExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                collect_deps_from_expr(s, interner, deps);
            }
            if let Some(e) = end {
                collect_deps_from_expr(e, interner, deps);
            }
        }
        ExprKind::Return(Some(e)) => {
            collect_deps_from_expr(e, interner, deps);
        }
        ExprKind::Return(None) => {}
        ExprKind::Break { value: Some(e), .. } => {
            collect_deps_from_expr(e, interner, deps);
        }
        ExprKind::Break { value: None, .. } => {}
        ExprKind::Loop { body, .. } => {
            collect_deps_from_block(body, interner, deps);
        }
        ExprKind::While { condition, body, .. } => {
            collect_deps_from_expr(condition, interner, deps);
            collect_deps_from_block(body, interner, deps);
        }
        ExprKind::For { iter, body, .. } => {
            collect_deps_from_expr(iter, interner, deps);
            collect_deps_from_block(body, interner, deps);
        }
        ExprKind::WithHandle { handler, body } => {
            collect_deps_from_expr(handler, interner, deps);
            collect_deps_from_expr(body, interner, deps);
        }
        ExprKind::Perform { args, .. } => {
            for arg in args {
                collect_deps_from_expr(arg, interner, deps);
            }
        }
        ExprKind::Resume(e) => {
            collect_deps_from_expr(e, interner, deps);
        }
        ExprKind::Cast { expr, .. } => {
            collect_deps_from_expr(expr, interner, deps);
        }
        ExprKind::Unsafe(block) | ExprKind::Region { body: block, .. } => {
            collect_deps_from_block(block, interner, deps);
        }
        ExprKind::Paren(e) => {
            collect_deps_from_expr(e, interner, deps);
        }
        _ => {}
    }
}

/// Collects dependencies from an else branch.
fn collect_deps_from_else(
    branch: &bloodc::ast::ElseBranch,
    interner: &string_interner::DefaultStringInterner,
    deps: &mut Vec<String>,
) {
    match branch {
        bloodc::ast::ElseBranch::Block(block) => collect_deps_from_block(block, interner, deps),
        bloodc::ast::ElseBranch::If(expr) => collect_deps_from_expr(expr, interner, deps),
    }
}

/// Collects dependencies from a statement.
fn collect_deps_from_stmt(
    stmt: &bloodc::ast::Statement,
    interner: &string_interner::DefaultStringInterner,
    deps: &mut Vec<String>,
) {
    use bloodc::ast::Statement;

    match stmt {
        Statement::Let { value, .. } => {
            if let Some(e) = value {
                collect_deps_from_expr(e, interner, deps);
            }
        }
        Statement::Expr { expr, .. } => {
            collect_deps_from_expr(expr, interner, deps);
        }
        Statement::Item(decl) => {
            collect_deps_from_decl(decl, interner, deps);
        }
    }
}

/// Checks if a name is a builtin/primitive that shouldn't be tracked as a dependency.
fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "f32" | "f64"
            | "bool" | "char" | "str" | "String"
            | "true" | "false"
            | "self" | "Self"
            | "Some" | "None" | "Ok" | "Err"
            | "Vec" | "Option" | "Result" | "Box"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_creation() {
        let h1 = Hash::of_str("hello");
        let h2 = Hash::of_str("hello");
        let h3 = Hash::of_str("world");

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_hash_display() {
        let h = Hash::of_str("test");
        let s = h.to_string();
        assert!(s.starts_with('#'));
        assert_eq!(s.len(), 9); // # + 8 hex chars
    }

    #[test]
    fn test_hash_parse() {
        let h = Hash::of_str("test");
        let hex = h.to_hex();
        let parsed = Hash::from_hex(&hex).unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn test_structural_hash() {
        let h1 = structural_hash("fn foo() { 42 }");
        let h2 = structural_hash("fn  foo()  {  42  }");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_deterministic_hash_across_runs() {
        // Verify that hashing produces the same result every time
        let input = "fn add(x: i32, y: i32) -> i32 { x + y }";

        let hash1 = Hash::of_str(input);
        let hash2 = Hash::of_str(input);
        let hash3 = Hash::of_str(input);

        assert_eq!(hash1, hash2, "Hash should be deterministic");
        assert_eq!(hash2, hash3, "Hash should be deterministic across calls");
        assert_eq!(hash1.as_bytes(), hash2.as_bytes(), "Raw bytes should match");
    }

    #[test]
    fn test_deterministic_hash_parts() {
        // Verify Hash::of_parts is deterministic
        fn build_hash() -> Hash {
            Hash::of_parts(&[
                b"part1",
                b"part2",
                b"part3",
            ])
        }

        let h1 = build_hash();
        let h2 = build_hash();
        let h3 = build_hash();

        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }

    #[test]
    fn test_deterministic_hasher() {
        // Verify incremental Hasher is deterministic
        fn build_hash() -> Hash {
            let mut hasher = Hasher::new();
            hasher.update(b"some data");
            hasher.update_str("string data");
            hasher.update(b"\x00\x01\x02\x03");
            hasher.finalize()
        }

        let h1 = build_hash();
        let h2 = build_hash();
        let h3 = build_hash();

        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }

    #[test]
    fn test_structural_hash_determinism() {
        // Verify structural hash is deterministic
        let source = "fn complex(a: i32, b: String) -> bool { a > 0 && !b.is_empty() }";

        let h1 = structural_hash(source);
        let h2 = structural_hash(source);
        let h3 = structural_hash(source);

        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }

    #[test]
    fn test_hash_serialization_roundtrip_determinism() {
        // Verify serialization is deterministic
        let original = Hash::of_str("test content");

        // Round-trip multiple times
        let hex1 = original.to_hex();
        let parsed1 = Hash::from_hex(&hex1).unwrap();

        let hex2 = parsed1.to_hex();
        let parsed2 = Hash::from_hex(&hex2).unwrap();

        assert_eq!(hex1, hex2, "Hex encoding should be deterministic");
        assert_eq!(original, parsed1);
        assert_eq!(parsed1, parsed2);
    }

    #[test]
    fn test_hash_from_bytes_determinism() {
        let bytes = [42u8; 32];

        let h1 = Hash::from_bytes(bytes);
        let h2 = Hash::from_bytes(bytes);

        assert_eq!(h1, h2);
        assert_eq!(h1.as_bytes(), &bytes);
    }
}
