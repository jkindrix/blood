//! Identifier generation for fuzzing.
//!
//! Blood has two kinds of identifiers:
//! - Value identifiers: start with lowercase or underscore
//! - Type identifiers: start with uppercase

use arbitrary::{Arbitrary, Unstructured};

/// A valid Blood identifier (lowercase/underscore start).
#[derive(Debug, Clone)]
pub struct FuzzIdent(pub String);

impl FuzzIdent {
    pub fn to_source(&self) -> String {
        self.0.clone()
    }
}

impl<'a> Arbitrary<'a> for FuzzIdent {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Blood keywords to avoid
        const KEYWORDS: &[&str] = &[
            "as", "async", "await", "break", "const", "continue", "crate",
            "deep", "dyn", "effect", "else", "enum", "extends", "extern",
            "false", "fn", "for", "handler", "if", "impl", "in",
            "let", "linear", "loop", "match", "mod", "move", "mut",
            "perform", "pub", "ref", "region", "resume", "return",
            "self", "Self", "shallow", "static", "struct", "super",
            "trait", "true", "type", "use", "where", "while",
        ];

        // Simple identifier generation from a fixed pool + optional suffix
        const PREFIXES: &[&str] = &[
            "x", "y", "z", "a", "b", "c", "foo", "bar", "baz",
            "value", "result", "tmp", "arg", "param", "item",
            "_x", "_val", "_result",
        ];

        let prefix_idx: usize = u.arbitrary()?;
        let prefix = PREFIXES[prefix_idx % PREFIXES.len()];

        let add_suffix: bool = u.arbitrary()?;
        let ident = if add_suffix {
            let suffix: u8 = u.arbitrary()?;
            format!("{}{}", prefix, suffix % 100)
        } else {
            prefix.to_string()
        };

        // Ensure we didn't accidentally generate a keyword
        if KEYWORDS.contains(&ident.as_str()) {
            Ok(FuzzIdent(format!("{}_", ident)))
        } else {
            Ok(FuzzIdent(ident))
        }
    }
}

/// A valid Blood type identifier (uppercase start).
#[derive(Debug, Clone)]
pub struct FuzzTypeIdent(pub String);

impl FuzzTypeIdent {
    pub fn to_source(&self) -> String {
        self.0.clone()
    }
}

impl<'a> Arbitrary<'a> for FuzzTypeIdent {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Type identifiers must start with uppercase
        const PREFIXES: &[&str] = &[
            "T", "U", "V", "A", "B", "C",
            "Foo", "Bar", "Baz", "Type", "Item", "Node",
            "Result", "Option", "Vec", "Map", "Set",
            "Data", "State", "Effect", "Handler",
        ];

        let prefix_idx: usize = u.arbitrary()?;
        let prefix = PREFIXES[prefix_idx % PREFIXES.len()];

        let add_suffix: bool = u.arbitrary()?;
        let ident = if add_suffix {
            let suffix: u8 = u.arbitrary()?;
            format!("{}{}", prefix, suffix % 100)
        } else {
            prefix.to_string()
        };

        Ok(FuzzTypeIdent(ident))
    }
}

/// A lifetime identifier.
#[derive(Debug, Clone)]
pub struct FuzzLifetime(pub String);

impl FuzzLifetime {
    pub fn to_source(&self) -> String {
        format!("'{}", self.0)
    }
}

impl<'a> Arbitrary<'a> for FuzzLifetime {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        const LIFETIMES: &[&str] = &["a", "b", "c", "x", "y", "static", "_"];

        let idx: usize = u.arbitrary()?;
        Ok(FuzzLifetime(LIFETIMES[idx % LIFETIMES.len()].to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ident_generation() {
        let data = [0u8; 32];
        let mut u = Unstructured::new(&data);
        let ident = FuzzIdent::arbitrary(&mut u).unwrap();
        assert!(!ident.0.is_empty());
        // Should start with lowercase or underscore
        let first = ident.0.chars().next().unwrap();
        assert!(first.is_lowercase() || first == '_');
    }

    #[test]
    fn test_type_ident_generation() {
        let data = [0u8; 32];
        let mut u = Unstructured::new(&data);
        let ident = FuzzTypeIdent::arbitrary(&mut u).unwrap();
        assert!(!ident.0.is_empty());
        // Should start with uppercase
        assert!(ident.0.chars().next().unwrap().is_uppercase());
    }
}
