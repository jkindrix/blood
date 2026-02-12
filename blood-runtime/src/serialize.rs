//! Serialization Infrastructure
//!
//! This module provides serialization and deserialization support for Blood types.
//! It includes both trait definitions and a built-in JSON implementation.
//!
//! # Design
//!
//! Serialization in Blood is type-safe and extensible:
//! - `Serialize` trait for converting values to bytes/strings
//! - `Deserialize` trait for parsing bytes/strings to values
//! - Multiple format support (JSON built-in, others via traits)
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::serialize::{Serialize, Deserialize, json};
//!
//! #[derive(Serialize, Deserialize)]
//! struct User {
//!     name: String,
//!     age: u32,
//! }
//!
//! let user = User { name: "Alice".into(), age: 30 };
//! let json = json::to_string(&user)?;
//! let parsed: User = json::from_str(&json)?;
//! ```

use std::collections::HashMap;
use std::fmt;

// ============================================================================
// SERIALIZATION VALUE TYPE
// ============================================================================

/// A generic value type that can represent any serializable data.
///
/// This is used as an intermediate representation for serialization.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Null/None value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value (signed 64-bit).
    Int(i64),
    /// Unsigned integer value (64-bit).
    Uint(u64),
    /// Floating point value (64-bit).
    Float(f64),
    /// String value.
    String(String),
    /// Array of values.
    Array(Vec<Value>),
    /// Object/map of string keys to values.
    Object(HashMap<String, Value>),
    /// Binary data.
    Bytes(Vec<u8>),
}

impl Value {
    /// Check if the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Check if the value is a boolean.
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    /// Check if the value is a number (int, uint, or float).
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Uint(_) | Value::Float(_))
    }

    /// Check if the value is a string.
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// Check if the value is an array.
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// Check if the value is an object.
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    /// Try to get the value as a bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get the value as an i64.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Uint(n) => i64::try_from(*n).ok(),
            Value::Float(f) => {
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to get the value as a u64.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Uint(n) => Some(*n),
            Value::Int(n) => u64::try_from(*n).ok(),
            Value::Float(f) => {
                if f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64 {
                    Some(*f as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to get the value as an f64.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(n) => Some(*n as f64),
            Value::Uint(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// Try to get the value as a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get the value as an array.
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Try to get the value as a mutable array.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Try to get the value as an object.
    pub fn as_object(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Try to get the value as a mutable object.
    pub fn as_object_mut(&mut self) -> Option<&mut HashMap<String, Value>> {
        match self {
            Value::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get a value from an object by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    /// Get a value from an array by index.
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        self.as_array()?.get(index)
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::Null
    }
}

// ============================================================================
// SERIALIZATION ERROR
// ============================================================================

/// Error that can occur during serialization or deserialization.
#[derive(Debug, Clone)]
pub enum SerializeError {
    /// Error message.
    Message(String),
    /// Unexpected end of input.
    UnexpectedEof,
    /// Invalid character in input.
    InvalidChar {
        /// The invalid character.
        char: char,
        /// Position in input.
        position: usize,
    },
    /// Expected a different type.
    TypeMismatch {
        /// Expected type.
        expected: &'static str,
        /// Actual type found.
        found: &'static str,
    },
    /// Missing required field.
    MissingField {
        /// Field name.
        field: String,
    },
    /// Unknown field in input.
    UnknownField {
        /// Field name.
        field: String,
    },
    /// Invalid value for a type.
    InvalidValue {
        /// Description of the issue.
        message: String,
    },
    /// Number out of range.
    OutOfRange {
        /// Description of the issue.
        message: String,
    },
    /// UTF-8 encoding error.
    Utf8Error(String),
    /// Custom error from serialization format.
    Custom(String),
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerializeError::Message(msg) => write!(f, "{}", msg),
            SerializeError::UnexpectedEof => write!(f, "unexpected end of input"),
            SerializeError::InvalidChar { char, position } => {
                write!(f, "invalid character '{}' at position {}", char, position)
            }
            SerializeError::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {}, found {}", expected, found)
            }
            SerializeError::MissingField { field } => {
                write!(f, "missing required field: {}", field)
            }
            SerializeError::UnknownField { field } => {
                write!(f, "unknown field: {}", field)
            }
            SerializeError::InvalidValue { message } => {
                write!(f, "invalid value: {}", message)
            }
            SerializeError::OutOfRange { message } => {
                write!(f, "value out of range: {}", message)
            }
            SerializeError::Utf8Error(msg) => {
                write!(f, "UTF-8 error: {}", msg)
            }
            SerializeError::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for SerializeError {}

/// Result type for serialization operations.
pub type Result<T> = std::result::Result<T, SerializeError>;

// ============================================================================
// SERIALIZE TRAIT
// ============================================================================

/// Trait for types that can be serialized to a Value.
pub trait Serialize {
    /// Serialize this value to a generic Value.
    fn serialize(&self) -> Result<Value>;
}

/// Trait for types that can be deserialized from a Value.
pub trait Deserialize: Sized {
    /// Deserialize a Value into this type.
    fn deserialize(value: &Value) -> Result<Self>;
}

// ============================================================================
// PRIMITIVE IMPLEMENTATIONS
// ============================================================================

impl Serialize for () {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::Null)
    }
}

impl Deserialize for () {
    fn deserialize(value: &Value) -> Result<Self> {
        match value {
            Value::Null => Ok(()),
            _ => Err(SerializeError::TypeMismatch {
                expected: "null",
                found: value_type_name(value),
            }),
        }
    }
}

impl Serialize for bool {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::Bool(*self))
    }
}

impl Deserialize for bool {
    fn deserialize(value: &Value) -> Result<Self> {
        value.as_bool().ok_or_else(|| SerializeError::TypeMismatch {
            expected: "bool",
            found: value_type_name(value),
        })
    }
}

macro_rules! impl_serialize_int {
    ($($ty:ty),*) => {
        $(
            impl Serialize for $ty {
                fn serialize(&self) -> Result<Value> {
                    Ok(Value::Int(*self as i64))
                }
            }

            impl Deserialize for $ty {
                fn deserialize(value: &Value) -> Result<Self> {
                    let n = value.as_i64().ok_or_else(|| SerializeError::TypeMismatch {
                        expected: stringify!($ty),
                        found: value_type_name(value),
                    })?;
                    <$ty>::try_from(n).map_err(|_| SerializeError::OutOfRange {
                        message: format!("value {} out of range for {}", n, stringify!($ty)),
                    })
                }
            }
        )*
    };
}

impl_serialize_int!(i8, i16, i32, i64, isize);

macro_rules! impl_serialize_uint {
    ($($ty:ty),*) => {
        $(
            impl Serialize for $ty {
                fn serialize(&self) -> Result<Value> {
                    Ok(Value::Uint(*self as u64))
                }
            }

            impl Deserialize for $ty {
                fn deserialize(value: &Value) -> Result<Self> {
                    let n = value.as_u64().ok_or_else(|| SerializeError::TypeMismatch {
                        expected: stringify!($ty),
                        found: value_type_name(value),
                    })?;
                    <$ty>::try_from(n).map_err(|_| SerializeError::OutOfRange {
                        message: format!("value {} out of range for {}", n, stringify!($ty)),
                    })
                }
            }
        )*
    };
}

impl_serialize_uint!(u8, u16, u32, u64, usize);

impl Serialize for f32 {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::Float(*self as f64))
    }
}

impl Deserialize for f32 {
    fn deserialize(value: &Value) -> Result<Self> {
        let f = value.as_f64().ok_or_else(|| SerializeError::TypeMismatch {
            expected: "f32",
            found: value_type_name(value),
        })?;
        Ok(f as f32)
    }
}

impl Serialize for f64 {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::Float(*self))
    }
}

impl Deserialize for f64 {
    fn deserialize(value: &Value) -> Result<Self> {
        value.as_f64().ok_or_else(|| SerializeError::TypeMismatch {
            expected: "f64",
            found: value_type_name(value),
        })
    }
}

impl Serialize for String {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::String(self.clone()))
    }
}

impl Deserialize for String {
    fn deserialize(value: &Value) -> Result<Self> {
        value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| SerializeError::TypeMismatch {
                expected: "string",
                found: value_type_name(value),
            })
    }
}

impl Serialize for &str {
    fn serialize(&self) -> Result<Value> {
        Ok(Value::String((*self).to_string()))
    }
}

impl<T: Serialize> Serialize for Vec<T> {
    fn serialize(&self) -> Result<Value> {
        let values: Result<Vec<Value>> = self.iter().map(|v| v.serialize()).collect();
        Ok(Value::Array(values?))
    }
}

impl<T: Deserialize> Deserialize for Vec<T> {
    fn deserialize(value: &Value) -> Result<Self> {
        let arr = value.as_array().ok_or_else(|| SerializeError::TypeMismatch {
            expected: "array",
            found: value_type_name(value),
        })?;
        arr.iter().map(|v| T::deserialize(v)).collect()
    }
}

impl<T: Serialize> Serialize for Option<T> {
    fn serialize(&self) -> Result<Value> {
        match self {
            Some(v) => v.serialize(),
            None => Ok(Value::Null),
        }
    }
}

impl<T: Deserialize> Deserialize for Option<T> {
    fn deserialize(value: &Value) -> Result<Self> {
        if value.is_null() {
            Ok(None)
        } else {
            T::deserialize(value).map(Some)
        }
    }
}

impl<K: AsRef<str>, V: Serialize> Serialize for HashMap<K, V> {
    fn serialize(&self) -> Result<Value> {
        let mut obj = HashMap::new();
        for (k, v) in self.iter() {
            obj.insert(k.as_ref().to_string(), v.serialize()?);
        }
        Ok(Value::Object(obj))
    }
}

impl<V: Deserialize> Deserialize for HashMap<String, V> {
    fn deserialize(value: &Value) -> Result<Self> {
        let obj = value.as_object().ok_or_else(|| SerializeError::TypeMismatch {
            expected: "object",
            found: value_type_name(value),
        })?;
        let mut result = HashMap::new();
        for (k, v) in obj.iter() {
            result.insert(k.clone(), V::deserialize(v)?);
        }
        Ok(result)
    }
}

/// Get the type name of a Value for error messages.
fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Uint(_) => "uint",
        Value::Float(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Bytes(_) => "bytes",
    }
}

// ============================================================================
// JSON MODULE
// ============================================================================

/// JSON serialization and deserialization.
pub mod json {
    use super::*;

    /// Serialize a value to a JSON string.
    pub fn to_string<T: Serialize>(value: &T) -> Result<String> {
        let v = value.serialize()?;
        value_to_json(&v)
    }

    /// Serialize a value to a pretty-printed JSON string.
    pub fn to_string_pretty<T: Serialize>(value: &T) -> Result<String> {
        let v = value.serialize()?;
        value_to_json_pretty(&v, 0)
    }

    /// Deserialize a JSON string to a value.
    pub fn from_str<T: Deserialize>(s: &str) -> Result<T> {
        let value = parse_json(s)?;
        T::deserialize(&value)
    }

    /// Parse a JSON string into a Value.
    pub fn parse(s: &str) -> Result<Value> {
        parse_json(s)
    }

    /// Convert a Value to a JSON string.
    fn value_to_json(value: &Value) -> Result<String> {
        Ok(match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Uint(n) => n.to_string(),
            Value::Float(f) => {
                if f.is_infinite() {
                    return Err(SerializeError::InvalidValue {
                        message: "cannot serialize infinity to JSON".to_string(),
                    });
                }
                if f.is_nan() {
                    return Err(SerializeError::InvalidValue {
                        message: "cannot serialize NaN to JSON".to_string(),
                    });
                }
                format!("{}", f)
            }
            Value::String(s) => escape_json_string(s),
            Value::Array(arr) => {
                let items: Result<Vec<String>> = arr.iter().map(value_to_json).collect();
                format!("[{}]", items?.join(","))
            }
            Value::Object(obj) => {
                let items: Result<Vec<String>> = obj
                    .iter()
                    .map(|(k, v)| Ok(format!("{}:{}", escape_json_string(k), value_to_json(v)?)))
                    .collect();
                format!("{{{}}}", items?.join(","))
            }
            Value::Bytes(b) => escape_json_string(&base64_encode(b)),
        })
    }

    /// Convert a Value to a pretty-printed JSON string.
    fn value_to_json_pretty(value: &Value, indent: usize) -> Result<String> {
        let indent_str = "  ".repeat(indent);
        let inner_indent = "  ".repeat(indent + 1);

        Ok(match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Uint(n) => n.to_string(),
            Value::Float(f) => {
                if f.is_infinite() {
                    return Err(SerializeError::InvalidValue {
                        message: "cannot serialize infinity to JSON".to_string(),
                    });
                }
                if f.is_nan() {
                    return Err(SerializeError::InvalidValue {
                        message: "cannot serialize NaN to JSON".to_string(),
                    });
                }
                format!("{}", f)
            }
            Value::String(s) => escape_json_string(s),
            Value::Array(arr) => {
                if arr.is_empty() {
                    "[]".to_string()
                } else {
                    let items: Result<Vec<String>> = arr
                        .iter()
                        .map(|v| value_to_json_pretty(v, indent + 1))
                        .collect();
                    format!(
                        "[\n{}{}\n{}]",
                        inner_indent,
                        items?.join(&format!(",\n{}", inner_indent)),
                        indent_str
                    )
                }
            }
            Value::Object(obj) => {
                if obj.is_empty() {
                    "{}".to_string()
                } else {
                    let mut items: Vec<_> = obj.iter().collect();
                    items.sort_by_key(|(k, _)| *k);
                    let items: Result<Vec<String>> = items
                        .iter()
                        .map(|(k, v)| {
                            Ok(format!(
                                "{}: {}",
                                escape_json_string(k),
                                value_to_json_pretty(v, indent + 1)?
                            ))
                        })
                        .collect();
                    format!(
                        "{{\n{}{}\n{}}}",
                        inner_indent,
                        items?.join(&format!(",\n{}", inner_indent)),
                        indent_str
                    )
                }
            }
            Value::Bytes(b) => escape_json_string(&base64_encode(b)),
        })
    }

    /// Escape a string for JSON.
    fn escape_json_string(s: &str) -> String {
        let mut result = String::with_capacity(s.len() + 2);
        result.push('"');
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if c.is_control() => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => result.push(c),
            }
        }
        result.push('"');
        result
    }

    /// Parse a JSON string.
    fn parse_json(s: &str) -> Result<Value> {
        let mut parser = JsonParser::new(s);
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos < parser.input.len() {
            return Err(SerializeError::InvalidValue {
                message: "unexpected characters after JSON value".to_string(),
            });
        }
        Ok(value)
    }

    /// Simple JSON parser.
    struct JsonParser<'a> {
        input: &'a str,
        pos: usize,
    }

    impl<'a> JsonParser<'a> {
        fn new(input: &'a str) -> Self {
            Self { input, pos: 0 }
        }

        fn peek(&self) -> Option<char> {
            self.input[self.pos..].chars().next()
        }

        fn advance(&mut self) {
            if let Some(c) = self.peek() {
                self.pos += c.len_utf8();
            }
        }

        fn skip_whitespace(&mut self) {
            while let Some(c) = self.peek() {
                if c.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        fn expect(&mut self, expected: char) -> Result<()> {
            self.skip_whitespace();
            match self.peek() {
                Some(c) if c == expected => {
                    self.advance();
                    Ok(())
                }
                Some(c) => Err(SerializeError::InvalidChar {
                    char: c,
                    position: self.pos,
                }),
                None => Err(SerializeError::UnexpectedEof),
            }
        }

        fn parse_value(&mut self) -> Result<Value> {
            self.skip_whitespace();
            match self.peek() {
                Some('n') => self.parse_null(),
                Some('t') | Some('f') => self.parse_bool(),
                Some('"') => self.parse_string(),
                Some('[') => self.parse_array(),
                Some('{') => self.parse_object(),
                Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
                Some(c) => Err(SerializeError::InvalidChar {
                    char: c,
                    position: self.pos,
                }),
                None => Err(SerializeError::UnexpectedEof),
            }
        }

        fn parse_null(&mut self) -> Result<Value> {
            if self.input[self.pos..].starts_with("null") {
                self.pos += 4;
                Ok(Value::Null)
            } else {
                Err(SerializeError::InvalidValue {
                    message: "expected 'null'".to_string(),
                })
            }
        }

        fn parse_bool(&mut self) -> Result<Value> {
            if self.input[self.pos..].starts_with("true") {
                self.pos += 4;
                Ok(Value::Bool(true))
            } else if self.input[self.pos..].starts_with("false") {
                self.pos += 5;
                Ok(Value::Bool(false))
            } else {
                Err(SerializeError::InvalidValue {
                    message: "expected 'true' or 'false'".to_string(),
                })
            }
        }

        fn parse_string(&mut self) -> Result<Value> {
            self.expect('"')?;
            let mut result = String::new();
            loop {
                match self.peek() {
                    Some('"') => {
                        self.advance();
                        return Ok(Value::String(result));
                    }
                    Some('\\') => {
                        self.advance();
                        match self.peek() {
                            Some('"') => {
                                result.push('"');
                                self.advance();
                            }
                            Some('\\') => {
                                result.push('\\');
                                self.advance();
                            }
                            Some('/') => {
                                result.push('/');
                                self.advance();
                            }
                            Some('n') => {
                                result.push('\n');
                                self.advance();
                            }
                            Some('r') => {
                                result.push('\r');
                                self.advance();
                            }
                            Some('t') => {
                                result.push('\t');
                                self.advance();
                            }
                            Some('b') => {
                                result.push('\x08');
                                self.advance();
                            }
                            Some('f') => {
                                result.push('\x0c');
                                self.advance();
                            }
                            Some('u') => {
                                self.advance();
                                let hex: String = (0..4)
                                    .filter_map(|_| {
                                        let c = self.peek()?;
                                        self.advance();
                                        Some(c)
                                    })
                                    .collect();
                                if hex.len() != 4 {
                                    return Err(SerializeError::InvalidValue {
                                        message: "invalid unicode escape".to_string(),
                                    });
                                }
                                let code = u32::from_str_radix(&hex, 16).map_err(|_| {
                                    SerializeError::InvalidValue {
                                        message: "invalid unicode escape".to_string(),
                                    }
                                })?;
                                let c = char::from_u32(code).ok_or_else(|| {
                                    SerializeError::InvalidValue {
                                        message: "invalid unicode code point".to_string(),
                                    }
                                })?;
                                result.push(c);
                            }
                            Some(c) => {
                                return Err(SerializeError::InvalidValue {
                                    message: format!("invalid escape sequence: \\{}", c),
                                });
                            }
                            None => return Err(SerializeError::UnexpectedEof),
                        }
                    }
                    Some(c) if c.is_control() => {
                        return Err(SerializeError::InvalidValue {
                            message: "control character in string".to_string(),
                        });
                    }
                    Some(c) => {
                        result.push(c);
                        self.advance();
                    }
                    None => return Err(SerializeError::UnexpectedEof),
                }
            }
        }

        fn parse_number(&mut self) -> Result<Value> {
            let start = self.pos;
            let mut is_float = false;

            // Optional negative sign
            if self.peek() == Some('-') {
                self.advance();
            }

            // Integer part
            match self.peek() {
                Some('0') => self.advance(),
                Some(c) if c.is_ascii_digit() => {
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                _ => {
                    return Err(SerializeError::InvalidValue {
                        message: "invalid number".to_string(),
                    });
                }
            }

            // Fractional part
            if self.peek() == Some('.') {
                is_float = true;
                self.advance();
                let mut has_digits = false;
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        self.advance();
                        has_digits = true;
                    } else {
                        break;
                    }
                }
                if !has_digits {
                    return Err(SerializeError::InvalidValue {
                        message: "invalid number: expected digits after decimal point".to_string(),
                    });
                }
            }

            // Exponent
            if let Some('e') | Some('E') = self.peek() {
                is_float = true;
                self.advance();
                if let Some('+') | Some('-') = self.peek() {
                    self.advance();
                }
                let mut has_digits = false;
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        self.advance();
                        has_digits = true;
                    } else {
                        break;
                    }
                }
                if !has_digits {
                    return Err(SerializeError::InvalidValue {
                        message: "invalid number: expected digits in exponent".to_string(),
                    });
                }
            }

            let num_str = &self.input[start..self.pos];
            if is_float {
                let f: f64 = num_str.parse().map_err(|_| SerializeError::InvalidValue {
                    message: format!("invalid float: {}", num_str),
                })?;
                Ok(Value::Float(f))
            } else if num_str.starts_with('-') {
                let n: i64 = num_str.parse().map_err(|_| SerializeError::InvalidValue {
                    message: format!("invalid integer: {}", num_str),
                })?;
                Ok(Value::Int(n))
            } else {
                // Try u64 first for large positive numbers
                if let Ok(n) = num_str.parse::<u64>() {
                    if n <= i64::MAX as u64 {
                        Ok(Value::Int(n as i64))
                    } else {
                        Ok(Value::Uint(n))
                    }
                } else {
                    let n: i64 = num_str.parse().map_err(|_| SerializeError::InvalidValue {
                        message: format!("invalid integer: {}", num_str),
                    })?;
                    Ok(Value::Int(n))
                }
            }
        }

        fn parse_array(&mut self) -> Result<Value> {
            self.expect('[')?;
            self.skip_whitespace();

            let mut items = Vec::new();
            if self.peek() == Some(']') {
                self.advance();
                return Ok(Value::Array(items));
            }

            loop {
                items.push(self.parse_value()?);
                self.skip_whitespace();
                match self.peek() {
                    Some(',') => {
                        self.advance();
                        self.skip_whitespace();
                    }
                    Some(']') => {
                        self.advance();
                        return Ok(Value::Array(items));
                    }
                    Some(c) => {
                        return Err(SerializeError::InvalidChar {
                            char: c,
                            position: self.pos,
                        });
                    }
                    None => return Err(SerializeError::UnexpectedEof),
                }
            }
        }

        fn parse_object(&mut self) -> Result<Value> {
            self.expect('{')?;
            self.skip_whitespace();

            let mut obj = HashMap::new();
            if self.peek() == Some('}') {
                self.advance();
                return Ok(Value::Object(obj));
            }

            loop {
                self.skip_whitespace();
                let key = match self.parse_value()? {
                    Value::String(s) => s,
                    _ => {
                        return Err(SerializeError::InvalidValue {
                            message: "object key must be a string".to_string(),
                        });
                    }
                };
                self.skip_whitespace();
                self.expect(':')?;
                let value = self.parse_value()?;
                obj.insert(key, value);

                self.skip_whitespace();
                match self.peek() {
                    Some(',') => {
                        self.advance();
                    }
                    Some('}') => {
                        self.advance();
                        return Ok(Value::Object(obj));
                    }
                    Some(c) => {
                        return Err(SerializeError::InvalidChar {
                            char: c,
                            position: self.pos,
                        });
                    }
                    None => return Err(SerializeError::UnexpectedEof),
                }
            }
        }
    }
}

// ============================================================================
// BASE64 ENCODING/DECODING
// ============================================================================

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes to base64.
fn base64_encode(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);

        result.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);

        if i + 1 < data.len() {
            result.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < data.len() {
            result.push(BASE64_CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}

/// Decode base64 to bytes.
fn base64_decode(s: &str) -> std::result::Result<Vec<u8>, &'static str> {
    let mut result = Vec::new();
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();

    if chars.len() % 4 != 0 {
        return Err("invalid base64 length");
    }

    let mut i = 0;
    while i < chars.len() {
        let c0 = base64_char_value(chars[i])?;
        let c1 = base64_char_value(chars[i + 1])?;
        let c2 = if chars[i + 2] == '=' {
            0
        } else {
            base64_char_value(chars[i + 2])?
        };
        let c3 = if chars[i + 3] == '=' {
            0
        } else {
            base64_char_value(chars[i + 3])?
        };

        let n = (c0 << 18) | (c1 << 12) | (c2 << 6) | c3;

        result.push((n >> 16) as u8);
        if chars[i + 2] != '=' {
            result.push((n >> 8) as u8);
        }
        if chars[i + 3] != '=' {
            result.push(n as u8);
        }

        i += 4;
    }

    Ok(result)
}

fn base64_char_value(c: char) -> std::result::Result<u32, &'static str> {
    match c {
        'A'..='Z' => Ok((c as u32) - ('A' as u32)),
        'a'..='z' => Ok((c as u32) - ('a' as u32) + 26),
        '0'..='9' => Ok((c as u32) - ('0' as u32) + 52),
        '+' => Ok(62),
        '/' => Ok(63),
        _ => Err("invalid base64 character"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod value_tests {
        use super::*;

        #[test]
        fn test_value_types() {
            assert!(Value::Null.is_null());
            assert!(Value::Bool(true).is_bool());
            assert!(Value::Int(42).is_number());
            assert!(Value::Float(3.14).is_number());
            assert!(Value::String("hello".into()).is_string());
            assert!(Value::Array(vec![]).is_array());
            assert!(Value::Object(HashMap::new()).is_object());
        }

        #[test]
        fn test_value_accessors() {
            assert_eq!(Value::Bool(true).as_bool(), Some(true));
            assert_eq!(Value::Int(42).as_i64(), Some(42));
            assert_eq!(Value::Uint(100).as_u64(), Some(100));
            assert_eq!(Value::Float(3.14).as_f64(), Some(3.14));
            assert_eq!(Value::String("test".into()).as_str(), Some("test"));
        }

        #[test]
        fn test_value_conversion() {
            // Int to u64
            assert_eq!(Value::Int(42).as_u64(), Some(42));
            // Uint to i64
            assert_eq!(Value::Uint(100).as_i64(), Some(100));
            // Float to int
            assert_eq!(Value::Float(42.0).as_i64(), Some(42));
        }
    }

    mod primitive_tests {
        use super::*;

        #[test]
        fn test_bool_serialize() {
            assert_eq!(true.serialize().unwrap(), Value::Bool(true));
            assert_eq!(false.serialize().unwrap(), Value::Bool(false));
        }

        #[test]
        fn test_bool_deserialize() {
            assert_eq!(bool::deserialize(&Value::Bool(true)).unwrap(), true);
            assert_eq!(bool::deserialize(&Value::Bool(false)).unwrap(), false);
        }

        #[test]
        fn test_int_serialize() {
            assert_eq!(42i32.serialize().unwrap(), Value::Int(42));
            assert_eq!((-10i64).serialize().unwrap(), Value::Int(-10));
        }

        #[test]
        fn test_int_deserialize() {
            assert_eq!(i32::deserialize(&Value::Int(42)).unwrap(), 42);
            assert_eq!(i64::deserialize(&Value::Int(-100)).unwrap(), -100);
        }

        #[test]
        fn test_uint_serialize() {
            assert_eq!(42u32.serialize().unwrap(), Value::Uint(42));
            assert_eq!(100u64.serialize().unwrap(), Value::Uint(100));
        }

        #[test]
        fn test_float_serialize() {
            assert_eq!(3.14f64.serialize().unwrap(), Value::Float(3.14));
        }

        #[test]
        fn test_string_serialize() {
            assert_eq!(
                "hello".serialize().unwrap(),
                Value::String("hello".into())
            );
            assert_eq!(
                String::from("world").serialize().unwrap(),
                Value::String("world".into())
            );
        }

        #[test]
        fn test_vec_serialize() {
            let v = vec![1i32, 2, 3];
            let expected = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
            assert_eq!(v.serialize().unwrap(), expected);
        }

        #[test]
        fn test_vec_deserialize() {
            let value = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
            let result: Vec<i32> = Vec::deserialize(&value).unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn test_option_serialize() {
            assert_eq!(Some(42i32).serialize().unwrap(), Value::Int(42));
            assert_eq!(None::<i32>.serialize().unwrap(), Value::Null);
        }

        #[test]
        fn test_option_deserialize() {
            assert_eq!(
                Option::<i32>::deserialize(&Value::Int(42)).unwrap(),
                Some(42)
            );
            assert_eq!(
                Option::<i32>::deserialize(&Value::Null).unwrap(),
                None
            );
        }

        #[test]
        fn test_hashmap_serialize() {
            let mut map = HashMap::new();
            map.insert("key".to_string(), 42i32);
            let value = map.serialize().unwrap();
            assert!(value.is_object());
            assert_eq!(value.get("key").unwrap().as_i64(), Some(42));
        }
    }

    mod json_tests {
        use super::json;
        use super::*;

        #[test]
        fn test_json_null() {
            let result = json::to_string(&()).unwrap();
            assert_eq!(result, "null");
            let parsed: () = json::from_str(&result).unwrap();
            assert_eq!(parsed, ());
        }

        #[test]
        fn test_json_bool() {
            assert_eq!(json::to_string(&true).unwrap(), "true");
            assert_eq!(json::to_string(&false).unwrap(), "false");
            assert_eq!(json::from_str::<bool>("true").unwrap(), true);
            assert_eq!(json::from_str::<bool>("false").unwrap(), false);
        }

        #[test]
        fn test_json_numbers() {
            assert_eq!(json::to_string(&42i32).unwrap(), "42");
            assert_eq!(json::to_string(&(-10i64)).unwrap(), "-10");
            assert_eq!(json::from_str::<i32>("42").unwrap(), 42);
            assert_eq!(json::from_str::<i64>("-100").unwrap(), -100);
        }

        #[test]
        fn test_json_float() {
            let json_str = json::to_string(&3.14f64).unwrap();
            assert!(json_str.starts_with("3.14"));
            let parsed: f64 = json::from_str("3.14").unwrap();
            assert!((parsed - 3.14).abs() < 0.001);
        }

        #[test]
        fn test_json_string() {
            assert_eq!(json::to_string(&"hello").unwrap(), "\"hello\"");
            assert_eq!(json::from_str::<String>("\"world\"").unwrap(), "world");
        }

        #[test]
        fn test_json_string_escapes() {
            let s = "hello\nworld\t\"quoted\"";
            let json_str = json::to_string(&s).unwrap();
            assert!(json_str.contains("\\n"));
            assert!(json_str.contains("\\t"));
            assert!(json_str.contains("\\\""));
            let parsed: String = json::from_str(&json_str).unwrap();
            assert_eq!(parsed, s);
        }

        #[test]
        fn test_json_array() {
            let arr = vec![1i32, 2, 3];
            let json_str = json::to_string(&arr).unwrap();
            assert_eq!(json_str, "[1,2,3]");
            let parsed: Vec<i32> = json::from_str(&json_str).unwrap();
            assert_eq!(parsed, arr);
        }

        #[test]
        fn test_json_object() {
            let mut obj = HashMap::new();
            obj.insert("name".to_string(), "Alice".to_string());
            let json_str = json::to_string(&obj).unwrap();
            assert!(json_str.contains("\"name\""));
            assert!(json_str.contains("\"Alice\""));
            let parsed: HashMap<String, String> = json::from_str(&json_str).unwrap();
            assert_eq!(parsed.get("name"), Some(&"Alice".to_string()));
        }

        #[test]
        fn test_json_nested() {
            let mut inner = HashMap::new();
            inner.insert("value".to_string(), 42i32);
            let mut outer = HashMap::new();
            outer.insert("inner".to_string(), inner);
            let json_str = json::to_string(&outer).unwrap();
            let parsed: HashMap<String, HashMap<String, i32>> = json::from_str(&json_str).unwrap();
            assert_eq!(parsed.get("inner").unwrap().get("value"), Some(&42));
        }

        #[test]
        fn test_json_pretty() {
            let arr = vec![1i32, 2, 3];
            let pretty = json::to_string_pretty(&arr).unwrap();
            assert!(pretty.contains('\n'));
            assert!(pretty.contains("  "));
        }

        #[test]
        fn test_json_parse_whitespace() {
            let result: i32 = json::from_str("  42  ").unwrap();
            assert_eq!(result, 42);
        }

        #[test]
        fn test_json_parse_unicode() {
            let result: String = json::from_str("\"hello\\u0020world\"").unwrap();
            assert_eq!(result, "hello world");
        }
    }

    mod base64_tests {
        use super::*;

        #[test]
        fn test_base64_encode() {
            assert_eq!(base64_encode(b""), "");
            assert_eq!(base64_encode(b"f"), "Zg==");
            assert_eq!(base64_encode(b"fo"), "Zm8=");
            assert_eq!(base64_encode(b"foo"), "Zm9v");
            assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
            assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
            assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        }

        #[test]
        fn test_base64_decode() {
            assert_eq!(base64_decode("").unwrap(), b"");
            assert_eq!(base64_decode("Zg==").unwrap(), b"f");
            assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
            assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
            assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        }

        #[test]
        fn test_base64_roundtrip() {
            let data = b"Hello, World!";
            let encoded = base64_encode(data);
            let decoded = base64_decode(&encoded).unwrap();
            assert_eq!(decoded, data);
        }
    }

    mod error_tests {
        use super::*;

        #[test]
        fn test_type_mismatch() {
            let result = bool::deserialize(&Value::Int(42));
            assert!(result.is_err());
        }

        #[test]
        fn test_out_of_range() {
            let result = u8::deserialize(&Value::Int(1000));
            assert!(result.is_err());
        }

        #[test]
        fn test_json_parse_error() {
            let result: Result<i32> = json::from_str("invalid");
            assert!(result.is_err());
        }

        #[test]
        fn test_error_display() {
            let err = SerializeError::TypeMismatch {
                expected: "bool",
                found: "int",
            };
            assert!(err.to_string().contains("bool"));
            assert!(err.to_string().contains("int"));
        }
    }
}
