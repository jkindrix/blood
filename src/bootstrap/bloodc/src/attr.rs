//! Attribute Processing
//!
//! This module provides utilities for working with Blood attributes.
//!
//! # Supported Attributes
//!
//! ## Testing
//! - `#[test]` - Marks a function as a test
//! - `#[ignore]` - Marks a test as ignored
//! - `#[should_panic]` - Test expects a panic
//!
//! ## FFI
//! - `#[link_name = "..."]` - External symbol name
//! - `#[repr(...)]` - Type representation
//!
//! ## Derive
//! - `#[derive(...)]` - Derive implementations
//!
//! ## Documentation
//! - `#[doc = "..."]` - Documentation

use crate::ast::{Attribute, AttributeArgs, AttributeArg, LiteralKind};
use crate::span::Span;
use string_interner::DefaultStringInterner;

/// Known attribute names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownAttr {
    /// `#[test]`
    Test,
    /// `#[ignore]`
    Ignore,
    /// `#[should_panic]`
    ShouldPanic,
    /// `#[bench]`
    Bench,
    /// `#[link_name = "..."]`
    LinkName,
    /// `#[repr(...)]`
    Repr,
    /// `#[derive(...)]`
    Derive,
    /// `#[doc = "..."]`
    Doc,
    /// `#[inline]`
    Inline,
    /// `#[cold]`
    Cold,
    /// `#[deprecated]`
    Deprecated,
    /// `#[allow(...)]`
    Allow,
    /// `#[warn(...)]`
    Warn,
    /// `#[deny(...)]`
    Deny,
    /// `#[forbid(...)]`
    Forbid,
    /// `#[cfg(...)]`
    Cfg,
    /// `#[cfg_attr(...)]`
    CfgAttr,
    /// `#[must_use]`
    MustUse,
    /// `#[no_mangle]`
    NoMangle,
    /// Unknown attribute
    Unknown,
}

impl KnownAttr {
    /// Parse an attribute name.
    pub fn from_str(s: &str) -> Self {
        match s {
            "test" => KnownAttr::Test,
            "ignore" => KnownAttr::Ignore,
            "should_panic" => KnownAttr::ShouldPanic,
            "bench" => KnownAttr::Bench,
            "link_name" => KnownAttr::LinkName,
            "repr" => KnownAttr::Repr,
            "derive" => KnownAttr::Derive,
            "doc" => KnownAttr::Doc,
            "inline" => KnownAttr::Inline,
            "cold" => KnownAttr::Cold,
            "deprecated" => KnownAttr::Deprecated,
            "allow" => KnownAttr::Allow,
            "warn" => KnownAttr::Warn,
            "deny" => KnownAttr::Deny,
            "forbid" => KnownAttr::Forbid,
            "cfg" => KnownAttr::Cfg,
            "cfg_attr" => KnownAttr::CfgAttr,
            "must_use" => KnownAttr::MustUse,
            "no_mangle" => KnownAttr::NoMangle,
            _ => KnownAttr::Unknown,
        }
    }

    /// Get the attribute name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            KnownAttr::Test => "test",
            KnownAttr::Ignore => "ignore",
            KnownAttr::ShouldPanic => "should_panic",
            KnownAttr::Bench => "bench",
            KnownAttr::LinkName => "link_name",
            KnownAttr::Repr => "repr",
            KnownAttr::Derive => "derive",
            KnownAttr::Doc => "doc",
            KnownAttr::Inline => "inline",
            KnownAttr::Cold => "cold",
            KnownAttr::Deprecated => "deprecated",
            KnownAttr::Allow => "allow",
            KnownAttr::Warn => "warn",
            KnownAttr::Deny => "deny",
            KnownAttr::Forbid => "forbid",
            KnownAttr::Cfg => "cfg",
            KnownAttr::CfgAttr => "cfg_attr",
            KnownAttr::MustUse => "must_use",
            KnownAttr::NoMangle => "no_mangle",
            KnownAttr::Unknown => "<unknown>",
        }
    }
}

/// Test configuration extracted from attributes.
#[derive(Debug, Clone, Default)]
pub struct TestConfig {
    /// Whether this is a test function.
    pub is_test: bool,
    /// Whether this test should be ignored.
    pub ignore: bool,
    /// Optional ignore reason.
    pub ignore_reason: Option<String>,
    /// Whether this test should panic.
    pub should_panic: bool,
    /// Expected panic message (if should_panic).
    pub expected_panic: Option<String>,
}

/// Attribute helper for extracting information.
pub struct AttrHelper<'a> {
    interner: &'a DefaultStringInterner,
}

impl<'a> AttrHelper<'a> {
    /// Create a new attribute helper.
    pub fn new(interner: &'a DefaultStringInterner) -> Self {
        Self { interner }
    }

    /// Get the name of an attribute.
    pub fn attr_name(&self, attr: &Attribute) -> Option<String> {
        if attr.path.len() == 1 {
            Some(self.interner.resolve(attr.path[0].node)?.to_string())
        } else {
            None
        }
    }

    /// Get the known attribute type.
    pub fn known_attr(&self, attr: &Attribute) -> KnownAttr {
        match self.attr_name(attr) {
            Some(name) => KnownAttr::from_str(&name),
            None => KnownAttr::Unknown,
        }
    }

    /// Check if an attribute has a specific name.
    pub fn is_attr(&self, attr: &Attribute, name: &str) -> bool {
        self.attr_name(attr).map(|n| n == name).unwrap_or(false)
    }

    /// Check if attributes contain a specific attribute.
    pub fn has_attr(&self, attrs: &[Attribute], name: &str) -> bool {
        attrs.iter().any(|attr| self.is_attr(attr, name))
    }

    /// Find an attribute by name.
    pub fn find_attr<'b>(&self, attrs: &'b [Attribute], name: &str) -> Option<&'b Attribute> {
        attrs.iter().find(|attr| self.is_attr(attr, name))
    }

    /// Check if a function has the #[test] attribute.
    pub fn is_test(&self, attrs: &[Attribute]) -> bool {
        self.has_attr(attrs, "test")
    }

    /// Check if a function has the #[ignore] attribute.
    pub fn is_ignored(&self, attrs: &[Attribute]) -> bool {
        self.has_attr(attrs, "ignore")
    }

    /// Check if a function has the #[should_panic] attribute.
    pub fn should_panic(&self, attrs: &[Attribute]) -> bool {
        self.has_attr(attrs, "should_panic")
    }

    /// Extract test configuration from attributes.
    pub fn extract_test_config(&self, attrs: &[Attribute]) -> TestConfig {
        let mut config = TestConfig::default();

        for attr in attrs {
            match self.known_attr(attr) {
                KnownAttr::Test => {
                    config.is_test = true;
                }
                KnownAttr::Ignore => {
                    config.ignore = true;
                    // Check for reason
                    if let Some(AttributeArgs::Eq(lit)) = &attr.args {
                        if let LiteralKind::String(s) = &lit.kind {
                            config.ignore_reason = Some(s.clone());
                        }
                    }
                }
                KnownAttr::ShouldPanic => {
                    config.should_panic = true;
                    // Check for expected message
                    if let Some(AttributeArgs::List(args)) = &attr.args {
                        for arg in args {
                            if let AttributeArg::KeyValue(key, val) = arg {
                                if let Some(key_name) = self.interner.resolve(key.node) {
                                    if key_name == "expected" {
                                        if let LiteralKind::String(s) = &val.kind {
                                            config.expected_panic = Some(s.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        config
    }

    /// Extract a string value from an attribute like `#[name = "value"]`.
    pub fn extract_string_value(&self, attr: &Attribute) -> Option<String> {
        if let Some(AttributeArgs::Eq(lit)) = &attr.args {
            if let LiteralKind::String(s) = &lit.kind {
                return Some(s.clone());
            }
        }
        None
    }
}

/// Information about a discovered test function.
#[derive(Debug, Clone)]
pub struct TestInfo {
    /// Function name.
    pub name: String,
    /// Module path (e.g., "foo::bar").
    pub module_path: String,
    /// Full path including function name.
    pub full_path: String,
    /// Test configuration.
    pub config: TestConfig,
    /// Span of the test function.
    pub span: Span,
}

impl TestInfo {
    /// Create a new test info.
    pub fn new(name: String, module_path: String, config: TestConfig, span: Span) -> Self {
        let full_path = if module_path.is_empty() {
            name.clone()
        } else {
            format!("{}::{}", module_path, name)
        };
        Self {
            name,
            module_path,
            full_path,
            config,
            span,
        }
    }
}

/// Registry for discovered tests.
#[derive(Debug, Default)]
pub struct TestRegistry {
    /// All discovered tests.
    tests: Vec<TestInfo>,
}

impl TestRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a test.
    pub fn register(&mut self, test: TestInfo) {
        self.tests.push(test);
    }

    /// Get all tests.
    pub fn tests(&self) -> &[TestInfo] {
        &self.tests
    }

    /// Get the number of tests.
    pub fn len(&self) -> usize {
        self.tests.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    /// Get tests that should run (not ignored).
    pub fn runnable_tests(&self) -> impl Iterator<Item = &TestInfo> {
        self.tests.iter().filter(|t| !t.config.ignore)
    }

    /// Get ignored tests.
    pub fn ignored_tests(&self) -> impl Iterator<Item = &TestInfo> {
        self.tests.iter().filter(|t| t.config.ignore)
    }

    /// Get test count summary.
    pub fn summary(&self) -> TestSummary {
        let total = self.tests.len();
        let ignored = self.tests.iter().filter(|t| t.config.ignore).count();
        let runnable = total - ignored;
        TestSummary {
            total,
            runnable,
            ignored,
        }
    }
}

/// Summary of test counts.
#[derive(Debug, Clone, Copy)]
pub struct TestSummary {
    /// Total test count.
    pub total: usize,
    /// Runnable test count.
    pub runnable: usize,
    /// Ignored test count.
    pub ignored: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_attr_from_str() {
        assert_eq!(KnownAttr::from_str("test"), KnownAttr::Test);
        assert_eq!(KnownAttr::from_str("ignore"), KnownAttr::Ignore);
        assert_eq!(KnownAttr::from_str("should_panic"), KnownAttr::ShouldPanic);
        assert_eq!(KnownAttr::from_str("repr"), KnownAttr::Repr);
        assert_eq!(KnownAttr::from_str("unknown_attr"), KnownAttr::Unknown);
    }

    #[test]
    fn test_known_attr_as_str() {
        assert_eq!(KnownAttr::Test.as_str(), "test");
        assert_eq!(KnownAttr::Ignore.as_str(), "ignore");
        assert_eq!(KnownAttr::Unknown.as_str(), "<unknown>");
    }

    #[test]
    fn test_test_config_default() {
        let config = TestConfig::default();
        assert!(!config.is_test);
        assert!(!config.ignore);
        assert!(!config.should_panic);
        assert!(config.ignore_reason.is_none());
        assert!(config.expected_panic.is_none());
    }

    #[test]
    fn test_test_info_new() {
        let config = TestConfig { is_test: true, ..Default::default() };
        let info = TestInfo::new(
            "my_test".to_string(),
            "foo::bar".to_string(),
            config,
            Span::dummy(),
        );
        assert_eq!(info.name, "my_test");
        assert_eq!(info.module_path, "foo::bar");
        assert_eq!(info.full_path, "foo::bar::my_test");
    }

    #[test]
    fn test_test_info_empty_module() {
        let config = TestConfig::default();
        let info = TestInfo::new(
            "my_test".to_string(),
            String::new(),
            config,
            Span::dummy(),
        );
        assert_eq!(info.full_path, "my_test");
    }

    #[test]
    fn test_test_registry() {
        let mut registry = TestRegistry::new();
        assert!(registry.is_empty());

        let config1 = TestConfig { is_test: true, ..Default::default() };
        let config2 = TestConfig { is_test: true, ignore: true, ..Default::default() };

        registry.register(TestInfo::new("test1".into(), "".into(), config1, Span::dummy()));
        registry.register(TestInfo::new("test2".into(), "".into(), config2, Span::dummy()));

        assert_eq!(registry.len(), 2);

        let summary = registry.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.runnable, 1);
        assert_eq!(summary.ignored, 1);
    }
}
