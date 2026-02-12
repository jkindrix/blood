//! # Standard Library Core
//!
//! Core types and effects for the Blood standard library.
//!
//! ## Design
//!
//! Provides runtime support for:
//! - Core effects (State, Error, IO)
//! - Basic types (Option, Result, List)
//! - String operations
//! - Numeric operations
//!
//! ## Note
//!
//! Most of the standard library will be written in Blood itself.
//! This module provides the minimal Rust support required for bootstrap.

use std::fmt;
use std::io::{self, Write};

/// Effect identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectId(pub u64);

impl EffectId {
    /// Create a new effect ID.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Effect({})", self.0)
    }
}

/// Built-in effect IDs.
pub mod effects {
    use super::EffectId;

    /// Pure (no effects).
    pub const PURE: EffectId = EffectId::new(0);
    /// State effect.
    pub const STATE: EffectId = EffectId::new(1);
    /// Error/Exception effect.
    pub const ERROR: EffectId = EffectId::new(2);
    /// I/O effect.
    pub const IO: EffectId = EffectId::new(3);
    /// Async effect.
    pub const ASYNC: EffectId = EffectId::new(4);
    /// Generator/yield effect.
    pub const GENERATOR: EffectId = EffectId::new(5);
    /// Non-determinism effect.
    pub const NONDET: EffectId = EffectId::new(6);
    /// Logging effect.
    pub const LOG: EffectId = EffectId::new(7);
    /// Stale reference effect.
    pub const STALE_REF: EffectId = EffectId::new(8);
}

/// Blood value representation (for runtime interpretation).
#[derive(Debug, Clone)]
pub enum Value {
    /// Unit value.
    Unit,
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit floating point.
    Float(f64),
    /// String.
    String(String),
    /// List of values.
    List(Vec<Value>),
    /// Record (struct).
    Record(Vec<(String, Value)>),
    /// Variant (enum).
    Variant {
        /// Variant name.
        tag: String,
        /// Variant payload.
        value: Box<Value>,
    },
    /// Function closure.
    Closure {
        /// Function ID.
        func_id: u64,
        /// Captured environment.
        env: Vec<Value>,
    },
    /// Effect handler.
    Handler {
        /// Effect ID.
        effect_id: EffectId,
        /// Handler operations.
        operations: Vec<HandlerOp>,
    },
    /// Continuation.
    Continuation {
        /// Continuation ID.
        id: u64,
    },
    /// Generational reference.
    Reference {
        /// Address.
        address: usize,
        /// Generation.
        generation: u32,
    },
}

impl Value {
    /// Check if this is a unit value.
    pub fn is_unit(&self) -> bool {
        matches!(self, Value::Unit)
    }

    /// Try to get as bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as int.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get as float.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Try to get as string.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as list.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
}

/// Handler operation for an effect.
#[derive(Debug, Clone)]
pub struct HandlerOp {
    /// Operation name.
    pub name: String,
    /// Operation implementation (closure ID).
    pub impl_id: u64,
}

/// Standard library error.
#[derive(Debug, Clone)]
pub struct StdError {
    /// Error kind.
    pub kind: StdErrorKind,
    /// Error message.
    pub message: String,
}

impl StdError {
    /// Create a new error.
    pub fn new(kind: StdErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for StdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for StdError {}

/// Standard library error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdErrorKind {
    /// Type error.
    TypeError,
    /// Value error.
    ValueError,
    /// Index out of bounds.
    IndexError,
    /// Key not found.
    KeyError,
    /// I/O error.
    IoError,
    /// Division by zero.
    DivisionByZero,
    /// Overflow.
    Overflow,
    /// Stale reference.
    StaleReference,
    /// Effect not handled.
    UnhandledEffect,
    /// Other error.
    Other,
}

/// Result type for stdlib operations.
pub type StdResult<T> = Result<T, StdError>;

// ============================================================================
// Core Operations
// ============================================================================

/// Integer operations.
pub mod int {
    use super::*;

    /// Add two integers.
    pub fn add(a: i64, b: i64) -> StdResult<i64> {
        a.checked_add(b).ok_or_else(|| {
            StdError::new(StdErrorKind::Overflow, "integer overflow in addition")
        })
    }

    /// Subtract two integers.
    pub fn sub(a: i64, b: i64) -> StdResult<i64> {
        a.checked_sub(b).ok_or_else(|| {
            StdError::new(StdErrorKind::Overflow, "integer overflow in subtraction")
        })
    }

    /// Multiply two integers.
    pub fn mul(a: i64, b: i64) -> StdResult<i64> {
        a.checked_mul(b).ok_or_else(|| {
            StdError::new(StdErrorKind::Overflow, "integer overflow in multiplication")
        })
    }

    /// Divide two integers.
    pub fn div(a: i64, b: i64) -> StdResult<i64> {
        if b == 0 {
            Err(StdError::new(StdErrorKind::DivisionByZero, "division by zero"))
        } else {
            a.checked_div(b).ok_or_else(|| {
                StdError::new(StdErrorKind::Overflow, "integer overflow in division")
            })
        }
    }

    /// Modulo of two integers.
    pub fn modulo(a: i64, b: i64) -> StdResult<i64> {
        if b == 0 {
            Err(StdError::new(StdErrorKind::DivisionByZero, "modulo by zero"))
        } else {
            Ok(a % b)
        }
    }

    /// Negate an integer.
    pub fn neg(a: i64) -> StdResult<i64> {
        a.checked_neg().ok_or_else(|| {
            StdError::new(StdErrorKind::Overflow, "integer overflow in negation")
        })
    }

    /// Absolute value.
    pub fn abs(a: i64) -> StdResult<i64> {
        a.checked_abs().ok_or_else(|| {
            StdError::new(StdErrorKind::Overflow, "integer overflow in abs")
        })
    }

    /// Parse an integer from a string.
    pub fn parse(s: &str) -> StdResult<i64> {
        s.trim().parse().map_err(|_| {
            StdError::new(StdErrorKind::ValueError, format!("invalid integer: {}", s))
        })
    }
}

/// Float operations.
pub mod float {
    use super::*;

    /// Add two floats.
    pub fn add(a: f64, b: f64) -> f64 {
        a + b
    }

    /// Subtract two floats.
    pub fn sub(a: f64, b: f64) -> f64 {
        a - b
    }

    /// Multiply two floats.
    pub fn mul(a: f64, b: f64) -> f64 {
        a * b
    }

    /// Divide two floats.
    pub fn div(a: f64, b: f64) -> StdResult<f64> {
        if b == 0.0 {
            Err(StdError::new(StdErrorKind::DivisionByZero, "division by zero"))
        } else {
            Ok(a / b)
        }
    }

    /// Negate a float.
    pub fn neg(a: f64) -> f64 {
        -a
    }

    /// Absolute value.
    pub fn abs(a: f64) -> f64 {
        a.abs()
    }

    /// Floor.
    pub fn floor(a: f64) -> f64 {
        a.floor()
    }

    /// Ceiling.
    pub fn ceil(a: f64) -> f64 {
        a.ceil()
    }

    /// Round.
    pub fn round(a: f64) -> f64 {
        a.round()
    }

    /// Parse a float from a string.
    pub fn parse(s: &str) -> StdResult<f64> {
        s.trim().parse().map_err(|_| {
            StdError::new(StdErrorKind::ValueError, format!("invalid float: {}", s))
        })
    }
}

/// String operations.
pub mod string {
    use super::*;

    /// Get string length.
    pub fn len(s: &str) -> usize {
        s.len()
    }

    /// Get character count (Unicode).
    pub fn char_count(s: &str) -> usize {
        s.chars().count()
    }

    /// Check if empty.
    pub fn is_empty(s: &str) -> bool {
        s.is_empty()
    }

    /// Concatenate two strings.
    pub fn concat(a: &str, b: &str) -> String {
        format!("{}{}", a, b)
    }

    /// Get a substring.
    pub fn substring(s: &str, start: usize, end: usize) -> StdResult<String> {
        if start > end || end > s.len() {
            Err(StdError::new(
                StdErrorKind::IndexError,
                format!("substring indices out of bounds: {}..{} for len {}", start, end, s.len()),
            ))
        } else {
            Ok(s[start..end].to_string())
        }
    }

    /// Convert to uppercase.
    pub fn to_uppercase(s: &str) -> String {
        s.to_uppercase()
    }

    /// Convert to lowercase.
    pub fn to_lowercase(s: &str) -> String {
        s.to_lowercase()
    }

    /// Trim whitespace.
    pub fn trim(s: &str) -> String {
        s.trim().to_string()
    }

    /// Split by delimiter.
    pub fn split(s: &str, delimiter: &str) -> Vec<String> {
        s.split(delimiter).map(|p| p.to_string()).collect()
    }

    /// Join strings.
    pub fn join(parts: &[String], separator: &str) -> String {
        parts.join(separator)
    }

    /// Check if contains substring.
    pub fn contains(s: &str, needle: &str) -> bool {
        s.contains(needle)
    }

    /// Check if starts with prefix.
    pub fn starts_with(s: &str, prefix: &str) -> bool {
        s.starts_with(prefix)
    }

    /// Check if ends with suffix.
    pub fn ends_with(s: &str, suffix: &str) -> bool {
        s.ends_with(suffix)
    }

    /// Replace all occurrences.
    pub fn replace(s: &str, from: &str, to: &str) -> String {
        s.replace(from, to)
    }
}

/// List operations.
pub mod list {
    use super::*;

    /// Get list length.
    pub fn len<T>(list: &[T]) -> usize {
        list.len()
    }

    /// Check if empty.
    pub fn is_empty<T>(list: &[T]) -> bool {
        list.is_empty()
    }

    /// Get element at index.
    pub fn get<T: Clone>(list: &[T], index: usize) -> StdResult<T> {
        list.get(index).cloned().ok_or_else(|| {
            StdError::new(
                StdErrorKind::IndexError,
                format!("index {} out of bounds for list of len {}", index, list.len()),
            )
        })
    }

    /// Get first element.
    pub fn first<T: Clone>(list: &[T]) -> StdResult<T> {
        list.first().cloned().ok_or_else(|| {
            StdError::new(StdErrorKind::IndexError, "list is empty")
        })
    }

    /// Get last element.
    pub fn last<T: Clone>(list: &[T]) -> StdResult<T> {
        list.last().cloned().ok_or_else(|| {
            StdError::new(StdErrorKind::IndexError, "list is empty")
        })
    }

    /// Append an element.
    pub fn append<T>(list: &mut Vec<T>, elem: T) {
        list.push(elem);
    }

    /// Concatenate two lists.
    pub fn concat<T: Clone>(a: &[T], b: &[T]) -> Vec<T> {
        let mut result = a.to_vec();
        result.extend_from_slice(b);
        result
    }

    /// Reverse a list.
    pub fn reverse<T: Clone>(list: &[T]) -> Vec<T> {
        list.iter().rev().cloned().collect()
    }

    /// Take first n elements.
    pub fn take<T: Clone>(list: &[T], n: usize) -> Vec<T> {
        list.iter().take(n).cloned().collect()
    }

    /// Drop first n elements.
    pub fn drop<T: Clone>(list: &[T], n: usize) -> Vec<T> {
        list.iter().skip(n).cloned().collect()
    }
}

/// I/O operations.
pub mod io_ops {
    use super::*;

    /// Print to stdout.
    pub fn print(s: &str) -> StdResult<()> {
        print!("{}", s);
        io::stdout().flush().map_err(|e| {
            StdError::new(StdErrorKind::IoError, e.to_string())
        })
    }

    /// Print line to stdout.
    pub fn println(s: &str) -> StdResult<()> {
        println!("{}", s);
        Ok(())
    }

    /// Print to stderr.
    pub fn eprint(s: &str) -> StdResult<()> {
        eprint!("{}", s);
        io::stderr().flush().map_err(|e| {
            StdError::new(StdErrorKind::IoError, e.to_string())
        })
    }

    /// Print line to stderr.
    pub fn eprintln(s: &str) -> StdResult<()> {
        eprintln!("{}", s);
        Ok(())
    }

    /// Read line from stdin.
    pub fn read_line() -> StdResult<String> {
        let mut line = String::new();
        io::stdin().read_line(&mut line).map_err(|e| {
            StdError::new(StdErrorKind::IoError, e.to_string())
        })?;
        Ok(line.trim_end_matches('\n').to_string())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_id() {
        assert_eq!(effects::PURE.as_u64(), 0);
        assert_eq!(effects::STATE.as_u64(), 1);
        assert_eq!(effects::IO.as_u64(), 3);
    }

    #[test]
    fn test_value_types() {
        assert!(Value::Unit.is_unit());
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Int(42).as_int(), Some(42));
        assert_eq!(Value::Float(2.5).as_float(), Some(2.5));
        assert_eq!(Value::String("hello".into()).as_string(), Some("hello"));
    }

    #[test]
    fn test_int_operations() {
        assert_eq!(int::add(2, 3).unwrap(), 5);
        assert_eq!(int::sub(5, 3).unwrap(), 2);
        assert_eq!(int::mul(4, 5).unwrap(), 20);
        assert_eq!(int::div(10, 2).unwrap(), 5);
        assert_eq!(int::modulo(10, 3).unwrap(), 1);
        assert_eq!(int::neg(-5).unwrap(), 5);
        assert_eq!(int::abs(-7).unwrap(), 7);
    }

    #[test]
    fn test_int_errors() {
        assert!(int::div(1, 0).is_err());
        assert!(int::modulo(1, 0).is_err());
        assert!(int::add(i64::MAX, 1).is_err());
    }

    #[test]
    fn test_float_operations() {
        assert_eq!(float::add(2.0, 3.0), 5.0);
        assert_eq!(float::sub(5.0, 3.0), 2.0);
        assert_eq!(float::mul(4.0, 5.0), 20.0);
        assert_eq!(float::div(10.0, 2.0).unwrap(), 5.0);
        assert_eq!(float::neg(-5.0), 5.0);
        assert_eq!(float::abs(-7.0), 7.0);
        assert_eq!(float::floor(3.7), 3.0);
        assert_eq!(float::ceil(3.2), 4.0);
        assert_eq!(float::round(3.5), 4.0);
    }

    #[test]
    fn test_string_operations() {
        assert_eq!(string::len("hello"), 5);
        assert_eq!(string::concat("hello", " world"), "hello world");
        assert_eq!(string::to_uppercase("hello"), "HELLO");
        assert_eq!(string::to_lowercase("HELLO"), "hello");
        assert_eq!(string::trim("  hello  "), "hello");
        assert!(string::contains("hello world", "world"));
        assert!(string::starts_with("hello world", "hello"));
        assert!(string::ends_with("hello world", "world"));
        assert_eq!(string::replace("hello world", "world", "rust"), "hello rust");
    }

    #[test]
    fn test_list_operations() {
        let list = vec![1, 2, 3, 4, 5];
        assert_eq!(list::len(&list), 5);
        assert!(!list::is_empty(&list));
        assert_eq!(list::get(&list, 2).unwrap(), 3);
        assert_eq!(list::first(&list).unwrap(), 1);
        assert_eq!(list::last(&list).unwrap(), 5);
        assert_eq!(list::reverse(&list), vec![5, 4, 3, 2, 1]);
        assert_eq!(list::take(&list, 3), vec![1, 2, 3]);
        assert_eq!(list::drop(&list, 2), vec![3, 4, 5]);
    }

    #[test]
    fn test_list_errors() {
        let list: Vec<i32> = vec![];
        assert!(list::get(&list, 0).is_err());
        assert!(list::first(&list).is_err());
        assert!(list::last(&list).is_err());
    }

    #[test]
    fn test_parse() {
        assert_eq!(int::parse("42").unwrap(), 42);
        assert_eq!(int::parse(" -123 ").unwrap(), -123);
        assert!(int::parse("abc").is_err());

        assert_eq!(float::parse("2.5").unwrap(), 2.5);
        assert_eq!(float::parse(" -2.5 ").unwrap(), -2.5);
        assert!(float::parse("abc").is_err());
    }

    #[test]
    fn test_std_error() {
        let err = StdError::new(StdErrorKind::TypeError, "test error");
        assert_eq!(err.kind, StdErrorKind::TypeError);
        assert!(err.to_string().contains("test error"));
    }
}
