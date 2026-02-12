//! Hash stability tests for content-addressed compilation.
//!
//! These tests verify that:
//! 1. Content hashes are stable (same code produces same hash)
//! 2. Different code produces different hashes
//! 3. Handler references affect the hash correctly
//!
//! Note: These tests use the compiler as a black box and examine the
//! verbose output to verify hash behavior.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

// ============================================================================
// Test Infrastructure
// ============================================================================

static BUILD_COMPILER: Once = Once::new();

fn compiler_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir).parent().unwrap();
    workspace_root.join("target/release/blood")
}

fn fixtures_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir).join("tests/fixtures")
}

fn ensure_compiler_built() {
    BUILD_COMPILER.call_once(|| {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = Path::new(manifest_dir).parent().unwrap();

        let status = Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(workspace_root)
            .status()
            .expect("Failed to run cargo build");

        assert!(status.success(), "Failed to build compiler in release mode");
    });
}

fn clear_cache() {
    let cache_dir = dirs::home_dir()
        .expect("No home directory")
        .join(".blood/cache");

    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir).ok();
    }
}

fn create_fixture(name: &str, content: &str) -> PathBuf {
    let fixtures = fixtures_dir();
    fs::create_dir_all(&fixtures).ok();

    let path = fixtures.join(format!("{}.blood", name));
    fs::write(&path, content).expect("Failed to write fixture");
    path
}

fn cleanup_fixture(path: &Path) {
    fs::remove_file(path).ok();

    let obj_dir = path.with_extension("blood_objs");
    if obj_dir.exists() {
        fs::remove_dir_all(&obj_dir).ok();
    }

    let executable = path.with_extension("");
    fs::remove_file(&executable).ok();
}

/// Compile a file with verbose output and return the stderr.
fn compile_verbose(source_path: &Path) -> String {
    ensure_compiler_built();

    let output = Command::new(compiler_path())
        .args(["build", "-vvv", source_path.to_str().unwrap()])
        .output()
        .expect("Failed to run compiler");

    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Extract cache statistics from verbose output.
fn extract_cache_stats(output: &str) -> (usize, usize) {
    // Look for "Compilation: X cached, Y compiled" line
    for line in output.lines() {
        if line.contains("Compilation:") && line.contains("cached") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let mut cached = 0;
            let mut compiled = 0;

            for (i, part) in parts.iter().enumerate() {
                if *part == "cached," && i > 0 {
                    cached = parts[i - 1].parse().unwrap_or(0);
                }
                if *part == "compiled," && i > 0 {
                    compiled = parts[i - 1].parse().unwrap_or(0);
                }
            }
            return (cached, compiled);
        }
    }
    (0, 0)
}

// ============================================================================
// Cache Behavior Tests
// ============================================================================

/// Test that same code produces cache hits on second compilation.
#[test]
fn test_hash_stable_cache_hit() {
    clear_cache();

    let fixture = create_fixture("hash_stable_1", r#"
fn main() -> i32 {
    42
}
"#);

    // First compilation - should be all misses
    let output1 = compile_verbose(&fixture);
    let (cached1, compiled1) = extract_cache_stats(&output1);
    assert!(compiled1 > 0, "First compilation should compile something");

    // Clear object files but keep cache
    let obj_dir = fixture.with_extension("blood_objs");
    if obj_dir.exists() {
        fs::remove_dir_all(&obj_dir).ok();
    }

    // Second compilation - should be cache hits
    let output2 = compile_verbose(&fixture);
    let (cached2, _compiled2) = extract_cache_stats(&output2);
    assert!(cached2 > 0, "Second compilation should have cache hits");

    cleanup_fixture(&fixture);
}

/// Test that different code produces different hashes (no cache hit).
#[test]
fn test_hash_different_no_cache_hit() {
    clear_cache();

    let fixture1 = create_fixture("hash_diff_1", r#"
fn main() -> i32 {
    1
}
"#);

    let fixture2 = create_fixture("hash_diff_2", r#"
fn main() -> i32 {
    2
}
"#);

    // Compile first file
    compile_verbose(&fixture1);

    // Clear first file's objects but keep cache
    let obj_dir = fixture1.with_extension("blood_objs");
    if obj_dir.exists() {
        fs::remove_dir_all(&obj_dir).ok();
    }

    // Compile second file - should NOT use cache from first file
    // (different code should have different hash)
    let output2 = compile_verbose(&fixture2);
    let (cached2, compiled2) = extract_cache_stats(&output2);

    // The main function should be compiled, not cached
    // (stdlib items might be cached, but main should not be)
    assert!(
        compiled2 > 0,
        "Different code should require compilation, not just cache hits"
    );

    cleanup_fixture(&fixture1);
    cleanup_fixture(&fixture2);
}

// ============================================================================
// Handler Hash Tests - Verify handlers don't share cache incorrectly
// ============================================================================

/// Test that different handlers produce different hashes.
/// This is the key test for the DefId index collision bug.
#[test]
fn test_different_handlers_no_cache_collision() {
    clear_cache();

    let handler_a = create_fixture("handler_hash_a", r#"
effect Counter {
    op inc() -> i32;
}

deep handler HandlerAlpha for Counter {
    let mut count: i32
    return(x) { x }
    op inc() {
        count = count + 1;
        resume(count)
    }
}

fn main() -> i32 {
    with HandlerAlpha { count: 0 } handle {
        perform Counter.inc()
    }
}
"#);

    let handler_b = create_fixture("handler_hash_b", r#"
effect Counter {
    op inc() -> i32;
}

deep handler HandlerBeta for Counter {
    let mut count: i32
    return(x) { x + 100 }
    op inc() {
        count = count + 1;
        resume(count)
    }
}

fn main() -> i32 {
    with HandlerBeta { count: 0 } handle {
        perform Counter.inc()
    }
}
"#);

    // Compile handler A
    let _output_a = compile_verbose(&handler_a);

    // Run handler A to verify it works
    let result_a = Command::new(handler_a.with_extension(""))
        .output()
        .expect("Failed to run handler A");
    assert_eq!(
        result_a.status.code(),
        Some(1),
        "HandlerAlpha should return 1"
    );

    // Compile handler B - should NOT incorrectly reuse A's main hash
    let _output_b = compile_verbose(&handler_b);

    // Run handler B to verify it wasn't corrupted by cache
    let result_b = Command::new(handler_b.with_extension(""))
        .output()
        .expect("Failed to run handler B");
    assert_eq!(
        result_b.status.code(),
        Some(101),  // 1 + 100 from return clause
        "HandlerBeta should return 101, not 1 (cache pollution would cause 1)"
    );

    cleanup_fixture(&handler_a);
    cleanup_fixture(&handler_b);
}

/// Regression test for DefId index collision bug.
#[test]
fn regression_defid_index_collision() {
    clear_cache();

    // These two files have handlers with identical structure but different names.
    // The bug was that they would get the same hash because DefId indices matched.

    let file_a = create_fixture("regression_idx_a", r#"
effect State { op get() -> i32; }
deep handler GetterA for State {
    let val: i32
    return(x) { x }
    op get() { resume(val) }
}
fn main() -> i32 {
    with GetterA { val: 42 } handle { perform State.get() }
}
"#);

    let file_b = create_fixture("regression_idx_b", r#"
effect State { op get() -> i32; }
deep handler GetterB for State {
    let val: i32
    return(x) { x }
    op get() { resume(val) }
}
fn main() -> i32 {
    with GetterB { val: 99 } handle { perform State.get() }
}
"#);

    // Compile and run A
    compile_verbose(&file_a);
    let result_a = Command::new(file_a.with_extension(""))
        .output()
        .expect("Failed to run file A");
    assert_eq!(result_a.status.code(), Some(42), "File A should return 42");

    // Compile and run B
    compile_verbose(&file_b);
    let result_b = Command::new(file_b.with_extension(""))
        .output()
        .expect("Failed to run file B");

    // If cache pollution occurred, B would incorrectly return 42 instead of 99
    assert_eq!(
        result_b.status.code(),
        Some(99),
        "REGRESSION: File B should return 99, not 42. Cache pollution detected!"
    );

    // Verify A still works correctly
    let result_a2 = Command::new(file_a.with_extension(""))
        .output()
        .expect("Failed to run file A again");
    assert_eq!(
        result_a2.status.code(),
        Some(42),
        "File A should still return 42"
    );

    cleanup_fixture(&file_a);
    cleanup_fixture(&file_b);
}

/// Test that the order of compilation doesn't affect correctness.
#[test]
fn test_compilation_order_independence() {
    clear_cache();

    let file1 = create_fixture("order_1", r#"
effect E { op get() -> i32; }
deep handler H1 for E {
    return(x) { x }
    op get() { resume(10) }
}
fn main() -> i32 {
    with H1 {} handle { perform E.get() }
}
"#);

    let file2 = create_fixture("order_2", r#"
effect E { op get() -> i32; }
deep handler H2 for E {
    return(x) { x }
    op get() { resume(20) }
}
fn main() -> i32 {
    with H2 {} handle { perform E.get() }
}
"#);

    let file3 = create_fixture("order_3", r#"
effect E { op get() -> i32; }
deep handler H3 for E {
    return(x) { x }
    op get() { resume(30) }
}
fn main() -> i32 {
    with H3 {} handle { perform E.get() }
}
"#);

    // Compile in one order
    compile_verbose(&file1);
    compile_verbose(&file2);
    compile_verbose(&file3);

    // Verify all produce correct results
    let r1 = Command::new(file1.with_extension("")).output().unwrap();
    let r2 = Command::new(file2.with_extension("")).output().unwrap();
    let r3 = Command::new(file3.with_extension("")).output().unwrap();

    assert_eq!(r1.status.code(), Some(10), "File 1 should return 10");
    assert_eq!(r2.status.code(), Some(20), "File 2 should return 20");
    assert_eq!(r3.status.code(), Some(30), "File 3 should return 30");

    // Clean up and recompile in different order
    clear_cache();
    cleanup_fixture(&file1);
    cleanup_fixture(&file2);
    cleanup_fixture(&file3);

    // Recreate fixtures
    let file1 = create_fixture("order_1", r#"
effect E { op get() -> i32; }
deep handler H1 for E {
    return(x) { x }
    op get() { resume(10) }
}
fn main() -> i32 {
    with H1 {} handle { perform E.get() }
}
"#);

    let file2 = create_fixture("order_2", r#"
effect E { op get() -> i32; }
deep handler H2 for E {
    return(x) { x }
    op get() { resume(20) }
}
fn main() -> i32 {
    with H2 {} handle { perform E.get() }
}
"#);

    let file3 = create_fixture("order_3", r#"
effect E { op get() -> i32; }
deep handler H3 for E {
    return(x) { x }
    op get() { resume(30) }
}
fn main() -> i32 {
    with H3 {} handle { perform E.get() }
}
"#);

    // Compile in reverse order
    compile_verbose(&file3);
    compile_verbose(&file2);
    compile_verbose(&file1);

    // Verify all still produce correct results
    let r1 = Command::new(file1.with_extension("")).output().unwrap();
    let r2 = Command::new(file2.with_extension("")).output().unwrap();
    let r3 = Command::new(file3.with_extension("")).output().unwrap();

    assert_eq!(r1.status.code(), Some(10), "File 1 should return 10 regardless of order");
    assert_eq!(r2.status.code(), Some(20), "File 2 should return 20 regardless of order");
    assert_eq!(r3.status.code(), Some(30), "File 3 should return 30 regardless of order");

    cleanup_fixture(&file1);
    cleanup_fixture(&file2);
    cleanup_fixture(&file3);
}
