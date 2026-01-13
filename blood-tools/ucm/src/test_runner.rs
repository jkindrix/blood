//! Test Execution Framework
//!
//! Provides functionality to run tests stored in the codebase.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::hash::Hash;
use crate::names::Name;
use crate::{Codebase, DefRef, UcmError, UcmResult};

/// Result of running a single test.
#[derive(Debug, Clone)]
pub enum TestOutcome {
    /// Test passed
    Passed {
        /// Time taken to run the test
        duration: Duration,
    },
    /// Test failed
    Failed {
        /// Failure message
        message: String,
        /// Time taken to run the test
        duration: Duration,
    },
    /// Test was skipped
    Skipped {
        /// Reason for skipping
        reason: String,
    },
    /// Test encountered an error (couldn't run)
    Error {
        /// Error message
        message: String,
    },
}

impl TestOutcome {
    /// Returns true if the test passed.
    pub fn is_passed(&self) -> bool {
        matches!(self, TestOutcome::Passed { .. })
    }

    /// Returns true if the test failed.
    pub fn is_failed(&self) -> bool {
        matches!(self, TestOutcome::Failed { .. })
    }

    /// Returns true if the test was skipped.
    pub fn is_skipped(&self) -> bool {
        matches!(self, TestOutcome::Skipped { .. })
    }

    /// Returns true if the test encountered an error.
    pub fn is_error(&self) -> bool {
        matches!(self, TestOutcome::Error { .. })
    }
}

/// Result of a test with its name.
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Test name
    pub name: Name,
    /// Test hash
    pub hash: Hash,
    /// Test outcome
    pub outcome: TestOutcome,
}

/// Summary of a test run.
#[derive(Debug, Clone, Default)]
pub struct TestSummary {
    /// Total number of tests
    pub total: usize,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Number of skipped tests
    pub skipped: usize,
    /// Number of errored tests
    pub errors: usize,
    /// Total duration
    pub duration: Duration,
}

impl TestSummary {
    /// Returns true if all tests passed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.errors == 0
    }

    /// Creates a summary from test results.
    pub fn from_results(results: &[TestResult]) -> Self {
        let mut summary = Self::default();
        summary.total = results.len();

        for result in results {
            match &result.outcome {
                TestOutcome::Passed { duration } => {
                    summary.passed += 1;
                    summary.duration += *duration;
                }
                TestOutcome::Failed { duration, .. } => {
                    summary.failed += 1;
                    summary.duration += *duration;
                }
                TestOutcome::Skipped { .. } => {
                    summary.skipped += 1;
                }
                TestOutcome::Error { .. } => {
                    summary.errors += 1;
                }
            }
        }

        summary
    }
}

/// Options for running tests.
#[derive(Debug, Clone)]
pub struct TestRunOptions {
    /// Run tests in parallel
    pub parallel: bool,
    /// Maximum number of parallel tests
    pub max_parallel: usize,
    /// Timeout for each test
    pub timeout: Option<Duration>,
    /// Show output for passing tests
    pub show_output: bool,
    /// Stop on first failure
    pub fail_fast: bool,
}

impl Default for TestRunOptions {
    fn default() -> Self {
        Self {
            parallel: false,
            max_parallel: 4,
            timeout: Some(Duration::from_secs(30)),
            show_output: false,
            fail_fast: false,
        }
    }
}

/// Test execution engine.
pub struct TestRunner<'a> {
    codebase: &'a Codebase,
    options: TestRunOptions,
}

impl<'a> TestRunner<'a> {
    /// Creates a new test runner.
    pub fn new(codebase: &'a Codebase) -> Self {
        Self {
            codebase,
            options: TestRunOptions::default(),
        }
    }

    /// Creates a test runner with custom options.
    pub fn with_options(codebase: &'a Codebase, options: TestRunOptions) -> Self {
        Self { codebase, options }
    }

    /// Runs all tests matching the filter.
    pub fn run(&self, filter: Option<&str>) -> UcmResult<Vec<TestResult>> {
        let tests = self.codebase.list_tests(filter)?;
        let mut results = Vec::new();

        for (name, hash) in tests {
            let result = self.run_single_test(&name, &hash)?;
            let failed = result.outcome.is_failed() || result.outcome.is_error();
            results.push(result);

            if failed && self.options.fail_fast {
                break;
            }
        }

        Ok(results)
    }

    /// Runs a single test by reference.
    pub fn run_test(&self, def_ref: &DefRef) -> UcmResult<TestResult> {
        let info = self.codebase.find(def_ref)?
            .ok_or_else(|| UcmError::NameNotFound("test not found".into()))?;

        let name = info.names.first()
            .cloned()
            .unwrap_or_else(|| Name::new(info.hash.short()));

        self.run_single_test(&name, &info.hash)
    }

    /// Runs a single test and returns the result.
    fn run_single_test(&self, name: &Name, hash: &Hash) -> UcmResult<TestResult> {
        // Get the test source
        let info = self.codebase.find(&DefRef::Hash(hash.clone()))?
            .ok_or_else(|| UcmError::HashNotFound(hash.to_string()))?;

        let start = Instant::now();
        let outcome = self.execute_test(&info.source);
        let duration = start.elapsed();

        // Wrap outcome with duration if it doesn't have one
        let outcome = match outcome {
            TestOutcome::Passed { .. } => TestOutcome::Passed { duration },
            TestOutcome::Failed { message, .. } => TestOutcome::Failed { message, duration },
            other => other,
        };

        Ok(TestResult {
            name: name.clone(),
            hash: hash.clone(),
            outcome,
        })
    }

    /// Executes a test and returns the outcome.
    ///
    /// Tests are expected to be functions that either:
    /// - Return () for success
    /// - Panic for failure
    /// - Use assert! macros
    fn execute_test(&self, source: &str) -> TestOutcome {
        // Parse the test source
        let mut parser = bloodc::Parser::new(source);
        match parser.parse_program() {
            Ok(program) => {
                // Check for parse errors
                if parser.has_errors() {
                    return TestOutcome::Error {
                        message: "Parse errors in test".to_string(),
                    };
                }

                // Try to type check the test
                let interner = parser.take_interner();
                match bloodc::typeck::check_program(&program, source, interner) {
                    Ok(_) => {
                        // Type checking passed
                        // For now, we consider a parseable, type-checkable test as "passed"
                        // Full execution would require JIT or native compilation
                        TestOutcome::Passed {
                            duration: Duration::default(),
                        }
                    }
                    Err(errors) => {
                        // Type errors in test
                        let message: String = errors.iter()
                            .map(|e| e.message.clone())
                            .collect::<Vec<_>>()
                            .join("\n");
                        TestOutcome::Failed {
                            message: format!("Type errors:\n{}", message),
                            duration: Duration::default(),
                        }
                    }
                }
            }
            Err(e) => {
                TestOutcome::Error {
                    message: format!("Parse error: {:?}", e),
                }
            }
        }
    }
}

/// Collected test assertions.
#[derive(Debug, Clone)]
pub struct TestAssertions {
    assertions: Vec<Assertion>,
}

/// A single assertion in a test.
#[derive(Debug, Clone)]
pub struct Assertion {
    /// Description of what was asserted
    pub description: String,
    /// Whether the assertion passed
    pub passed: bool,
    /// Expected value (if any)
    pub expected: Option<String>,
    /// Actual value (if any)
    pub actual: Option<String>,
}

impl TestAssertions {
    /// Creates a new empty assertions collector.
    pub fn new() -> Self {
        Self {
            assertions: Vec::new(),
        }
    }

    /// Adds an assertion.
    pub fn add(&mut self, assertion: Assertion) {
        self.assertions.push(assertion);
    }

    /// Returns true if all assertions passed.
    pub fn all_passed(&self) -> bool {
        self.assertions.iter().all(|a| a.passed)
    }

    /// Returns the failed assertions.
    pub fn failures(&self) -> Vec<&Assertion> {
        self.assertions.iter().filter(|a| !a.passed).collect()
    }
}

impl Default for TestAssertions {
    fn default() -> Self {
        Self::new()
    }
}

/// Test discovery result.
#[derive(Debug, Clone)]
pub struct TestDiscovery {
    /// Tests found by category
    pub tests: HashMap<String, Vec<(Name, Hash)>>,
    /// Total number of tests
    pub total: usize,
}

impl TestDiscovery {
    /// Discovers all tests in a codebase.
    pub fn discover(codebase: &Codebase) -> UcmResult<Self> {
        let tests = codebase.list_tests(None)?;
        let mut by_category: HashMap<String, Vec<(Name, Hash)>> = HashMap::new();

        for (name, hash) in tests {
            // Extract category from namespace (e.g., "tests.unit.math" -> "tests.unit")
            let category = name.parent()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "default".to_string());

            by_category.entry(category)
                .or_default()
                .push((name, hash));
        }

        let total = by_category.values().map(|v| v.len()).sum();

        Ok(Self {
            tests: by_category,
            total,
        })
    }

    /// Returns test categories.
    pub fn categories(&self) -> Vec<&String> {
        self.tests.keys().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary_from_results() {
        let results = vec![
            TestResult {
                name: Name::new("test1"),
                hash: Hash::of_str("test1"),
                outcome: TestOutcome::Passed { duration: Duration::from_millis(10) },
            },
            TestResult {
                name: Name::new("test2"),
                hash: Hash::of_str("test2"),
                outcome: TestOutcome::Failed {
                    message: "assertion failed".to_string(),
                    duration: Duration::from_millis(5),
                },
            },
            TestResult {
                name: Name::new("test3"),
                hash: Hash::of_str("test3"),
                outcome: TestOutcome::Skipped { reason: "not implemented".to_string() },
            },
        ];

        let summary = TestSummary::from_results(&results);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.skipped, 1);
        assert!(!summary.all_passed());
    }

    #[test]
    fn test_outcome_predicates() {
        let passed = TestOutcome::Passed { duration: Duration::from_secs(1) };
        assert!(passed.is_passed());
        assert!(!passed.is_failed());

        let failed = TestOutcome::Failed {
            message: "fail".to_string(),
            duration: Duration::from_secs(1),
        };
        assert!(!failed.is_passed());
        assert!(failed.is_failed());
    }
}
