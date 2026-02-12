//! # Closure Environment Analysis
//!
//! This module analyzes closure environments to identify optimization opportunities.
//!
//! ## Purpose
//!
//! The analysis determines:
//! - Size of closure environments (total bytes of captured values)
//! - Whether closures are candidates for inline environment optimization
//! - Statistics about closure usage patterns
//!
//! ## Optimization Strategy
//!
//! Closures have two possible environment representations:
//!
//! 1. **Pointer-based (current)**: `{ fn_ptr: i8*, env_ptr: i8* }`
//!    - Environment is stack-allocated separately
//!    - env_ptr points to the captures struct
//!    - 16 bytes total for closure value
//!
//! 2. **Inline (optimized)**: `{ fn_ptr: i8*, env: [captures...] }`
//!    - Captures stored directly in closure struct
//!    - No pointer indirection for access
//!    - Better cache locality for small environments
//!
//! ## Inline Threshold
//!
//! The default inline threshold is 16 bytes. Closures with environments
//! at or below this size are candidates for inline optimization:
//!
//! | Env Size | Representation | Rationale |
//! |----------|----------------|-----------|
//! | 0 bytes  | Inline (null)  | No captures, env_ptr is null |
//! | 1-16 bytes | Inline eligible | Fits in 2 registers, no alloca needed |
//! | 17+ bytes | Pointer-based | Larger than register pair |
//!
//! ## Usage
//!
//! ```ignore
//! let analyzer = ClosureAnalyzer::new();
//! let results = analyzer.analyze_bodies(&mir_bodies);
//!
//! for (def_id, info) in &results.closures {
//!     if info.is_inline_candidate {
//!         println!("Closure {:?} can be inlined ({} bytes)", def_id, info.env_size);
//!     }
//! }
//! ```

use std::collections::HashMap;

use crate::hir::{DefId, Type};
use crate::codegen::types::type_size;
use super::body::MirBody;
use super::types::{
    Rvalue, StatementKind, AggregateKind,
};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for closure analysis.
#[derive(Debug, Clone)]
pub struct ClosureAnalysisConfig {
    /// Maximum environment size (in bytes) for inline optimization.
    /// Default: 16 bytes (fits in 2 64-bit registers).
    pub inline_threshold: usize,

    /// Whether to report statistics.
    pub report_stats: bool,
}

impl Default for ClosureAnalysisConfig {
    fn default() -> Self {
        Self {
            inline_threshold: 16,
            report_stats: false,
        }
    }
}

// ============================================================================
// Analysis Results
// ============================================================================

/// Information about a single closure.
#[derive(Debug, Clone)]
pub struct ClosureInfo {
    /// DefId of the closure function.
    pub def_id: DefId,

    /// Number of captured values.
    pub capture_count: usize,

    /// Total size of the environment in bytes.
    pub env_size: usize,

    /// Types of captured values.
    pub capture_types: Vec<Type>,

    /// Whether this closure is a candidate for inline optimization.
    pub is_inline_candidate: bool,

    /// Whether the closure has zero captures (can use null env).
    pub is_zero_capture: bool,

    /// DefId of the containing function.
    pub parent_fn: DefId,
}

/// Aggregated statistics about closures.
#[derive(Debug, Clone, Default)]
pub struct ClosureStats {
    /// Total number of closures analyzed.
    pub total_closures: usize,

    /// Number of zero-capture closures.
    pub zero_capture: usize,

    /// Number of closures eligible for inline optimization.
    pub inline_eligible: usize,

    /// Number of closures requiring pointer-based environments.
    pub pointer_based: usize,

    /// Total environment bytes across all closures.
    pub total_env_bytes: usize,

    /// Average environment size.
    pub avg_env_size: f64,

    /// Maximum environment size seen.
    pub max_env_size: usize,

    /// Distribution of capture counts.
    pub capture_count_distribution: HashMap<usize, usize>,
}

/// Results of closure analysis.
#[derive(Debug, Clone)]
pub struct ClosureAnalysisResults {
    /// Information about each closure.
    pub closures: HashMap<DefId, ClosureInfo>,

    /// Aggregated statistics.
    pub stats: ClosureStats,

    /// Configuration used for analysis.
    pub config: ClosureAnalysisConfig,
}

impl ClosureAnalysisResults {
    /// Create empty results.
    pub fn new(config: ClosureAnalysisConfig) -> Self {
        Self {
            closures: HashMap::new(),
            stats: ClosureStats::default(),
            config,
        }
    }

    /// Get information about a specific closure.
    pub fn get(&self, def_id: DefId) -> Option<&ClosureInfo> {
        self.closures.get(&def_id)
    }

    /// Check if a closure is eligible for inline optimization.
    pub fn is_inline_candidate(&self, def_id: DefId) -> bool {
        self.closures.get(&def_id)
            .map(|info| info.is_inline_candidate)
            .unwrap_or(false)
    }

    /// Get all inline-eligible closures.
    pub fn inline_candidates(&self) -> impl Iterator<Item = &ClosureInfo> {
        self.closures.values().filter(|info| info.is_inline_candidate)
    }

    /// Generate a summary report.
    pub fn summary(&self) -> String {
        let mut report = String::new();
        report.push_str("=== Closure Analysis Summary ===\n\n");

        report.push_str(&format!("Total closures: {}\n", self.stats.total_closures));
        report.push_str(&format!("Zero-capture: {} ({:.1}%)\n",
            self.stats.zero_capture,
            if self.stats.total_closures > 0 {
                self.stats.zero_capture as f64 / self.stats.total_closures as f64 * 100.0
            } else { 0.0 }
        ));
        report.push_str(&format!("Inline eligible (≤{} bytes): {} ({:.1}%)\n",
            self.config.inline_threshold,
            self.stats.inline_eligible,
            if self.stats.total_closures > 0 {
                self.stats.inline_eligible as f64 / self.stats.total_closures as f64 * 100.0
            } else { 0.0 }
        ));
        report.push_str(&format!("Pointer-based: {} ({:.1}%)\n",
            self.stats.pointer_based,
            if self.stats.total_closures > 0 {
                self.stats.pointer_based as f64 / self.stats.total_closures as f64 * 100.0
            } else { 0.0 }
        ));

        report.push_str("\nEnvironment sizes:\n");
        report.push_str(&format!("  Total: {} bytes\n", self.stats.total_env_bytes));
        report.push_str(&format!("  Average: {:.1} bytes\n", self.stats.avg_env_size));
        report.push_str(&format!("  Maximum: {} bytes\n", self.stats.max_env_size));

        report.push_str("\nCapture count distribution:\n");
        let mut counts: Vec<_> = self.stats.capture_count_distribution.iter().collect();
        counts.sort_by_key(|(k, _)| *k);
        for (count, occurrences) in counts {
            report.push_str(&format!("  {} captures: {} closures\n", count, occurrences));
        }

        report
    }
}

// ============================================================================
// Analyzer
// ============================================================================

/// Analyzes closure environments in MIR bodies.
pub struct ClosureAnalyzer {
    config: ClosureAnalysisConfig,
}

impl ClosureAnalyzer {
    /// Create a new analyzer with default configuration.
    pub fn new() -> Self {
        Self {
            config: ClosureAnalysisConfig::default(),
        }
    }

    /// Create an analyzer with custom configuration.
    pub fn with_config(config: ClosureAnalysisConfig) -> Self {
        Self { config }
    }

    /// Analyze all MIR bodies.
    pub fn analyze_bodies(&self, bodies: &HashMap<DefId, MirBody>) -> ClosureAnalysisResults {
        let mut results = ClosureAnalysisResults::new(self.config.clone());

        for (&def_id, body) in bodies {
            self.analyze_body(def_id, body, &mut results);
        }

        self.compute_statistics(&mut results);
        results
    }

    /// Analyze a single MIR body for closure creations.
    fn analyze_body(&self, fn_def_id: DefId, body: &MirBody, results: &mut ClosureAnalysisResults) {
        for block in &body.basic_blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(_, rvalue) = &stmt.kind {
                    self.analyze_rvalue(fn_def_id, rvalue, body, results);
                }
            }
        }
    }

    /// Analyze an rvalue for closure creation.
    fn analyze_rvalue(
        &self,
        parent_fn: DefId,
        rvalue: &Rvalue,
        body: &MirBody,
        results: &mut ClosureAnalysisResults,
    ) {
        if let Rvalue::Aggregate { kind: AggregateKind::Closure { def_id }, operands } = rvalue {
            // Collect capture types
            let mut capture_types = Vec::with_capacity(operands.len());
            let mut env_size = 0usize;

            for operand in operands {
                // Get the type of the captured value
                let ty = match operand {
                    super::types::Operand::Copy(place) | super::types::Operand::Move(place) => {
                        body.locals.get(place.local_unchecked().index as usize)
                            .map(|local| local.ty.clone())
                            .unwrap_or_else(Type::error)
                    }
                    super::types::Operand::Constant(c) => c.ty.clone(),
                };

                let size = type_size(&ty);
                env_size += size;
                capture_types.push(ty);
            }

            let is_zero_capture = operands.is_empty();
            let is_inline_candidate = env_size <= self.config.inline_threshold;

            let info = ClosureInfo {
                def_id: *def_id,
                capture_count: operands.len(),
                env_size,
                capture_types,
                is_inline_candidate,
                is_zero_capture,
                parent_fn,
            };

            results.closures.insert(*def_id, info);
        }
    }

    /// Compute aggregate statistics.
    fn compute_statistics(&self, results: &mut ClosureAnalysisResults) {
        let stats = &mut results.stats;

        stats.total_closures = results.closures.len();

        for info in results.closures.values() {
            // Count by category
            if info.is_zero_capture {
                stats.zero_capture += 1;
            }
            if info.is_inline_candidate {
                stats.inline_eligible += 1;
            }
            if !info.is_inline_candidate {
                stats.pointer_based += 1;
            }

            // Accumulate sizes
            stats.total_env_bytes += info.env_size;
            if info.env_size > stats.max_env_size {
                stats.max_env_size = info.env_size;
            }

            // Distribution
            *stats.capture_count_distribution.entry(info.capture_count).or_insert(0) += 1;
        }

        // Average
        stats.avg_env_size = if stats.total_closures > 0 {
            stats.total_env_bytes as f64 / stats.total_closures as f64
        } else {
            0.0
        };
    }
}

impl Default for ClosureAnalyzer {
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
    fn test_config_defaults() {
        let config = ClosureAnalysisConfig::default();
        assert_eq!(config.inline_threshold, 16);
        assert!(!config.report_stats);
    }

    #[test]
    fn test_analyzer_creation() {
        let analyzer = ClosureAnalyzer::new();
        assert_eq!(analyzer.config.inline_threshold, 16);
    }

    #[test]
    fn test_empty_analysis() {
        let analyzer = ClosureAnalyzer::new();
        let bodies = HashMap::new();
        let results = analyzer.analyze_bodies(&bodies);

        assert_eq!(results.stats.total_closures, 0);
        assert_eq!(results.stats.zero_capture, 0);
        assert_eq!(results.stats.inline_eligible, 0);
    }

    #[test]
    fn test_closure_info_inline_threshold() {
        // Small closure should be inline candidate
        let info = ClosureInfo {
            def_id: DefId::new(1),
            capture_count: 1,
            env_size: 8,
            capture_types: vec![Type::i64()],
            is_inline_candidate: true,
            is_zero_capture: false,
            parent_fn: DefId::new(0),
        };
        assert!(info.is_inline_candidate);

        // Large closure should not be inline candidate
        let info_large = ClosureInfo {
            def_id: DefId::new(2),
            capture_count: 3,
            env_size: 24,
            capture_types: vec![Type::i64(), Type::i64(), Type::i64()],
            is_inline_candidate: false,
            is_zero_capture: false,
            parent_fn: DefId::new(0),
        };
        assert!(!info_large.is_inline_candidate);
    }

    #[test]
    fn test_statistics_computation() {
        let mut results = ClosureAnalysisResults::new(ClosureAnalysisConfig::default());

        // Add some mock closure info
        results.closures.insert(DefId::new(1), ClosureInfo {
            def_id: DefId::new(1),
            capture_count: 0,
            env_size: 0,
            capture_types: vec![],
            is_inline_candidate: true,
            is_zero_capture: true,
            parent_fn: DefId::new(0),
        });

        results.closures.insert(DefId::new(2), ClosureInfo {
            def_id: DefId::new(2),
            capture_count: 1,
            env_size: 8,
            capture_types: vec![Type::i64()],
            is_inline_candidate: true,
            is_zero_capture: false,
            parent_fn: DefId::new(0),
        });

        results.closures.insert(DefId::new(3), ClosureInfo {
            def_id: DefId::new(3),
            capture_count: 4,
            env_size: 32,
            capture_types: vec![Type::i64(), Type::i64(), Type::i64(), Type::i64()],
            is_inline_candidate: false,
            is_zero_capture: false,
            parent_fn: DefId::new(0),
        });

        let analyzer = ClosureAnalyzer::new();
        analyzer.compute_statistics(&mut results);

        assert_eq!(results.stats.total_closures, 3);
        assert_eq!(results.stats.zero_capture, 1);
        assert_eq!(results.stats.inline_eligible, 2);
        assert_eq!(results.stats.pointer_based, 1);
        assert_eq!(results.stats.total_env_bytes, 40);
        assert_eq!(results.stats.max_env_size, 32);
    }

    #[test]
    fn test_summary_generation() {
        let results = ClosureAnalysisResults::new(ClosureAnalysisConfig::default());
        let summary = results.summary();

        assert!(summary.contains("Closure Analysis Summary"));
        assert!(summary.contains("Total closures:"));
        assert!(summary.contains("Inline eligible"));
    }
}
