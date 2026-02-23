//! End-to-end integration tests for the Blood compiler.
//!
//! These tests compile source files to executables and verify:
//! 1. Compilation succeeds
//! 2. The executable runs and produces the correct exit code
//! 3. Cache behavior is correct across multiple compilations
//! 4. Sequential compilation of different files works correctly
//!
//! These tests would have caught the cache pollution bug where handlers
//! at the same DefId index in different files caused incorrect cache reuse.
//!
//! # Test Isolation
//!
//! Each test uses an isolated cache directory to prevent interference
//! when tests run in parallel. The cache directory is created in a
//! temporary location and cleaned up after each test.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, Once};
use std::thread;
use std::time::Duration;

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Ensures the compiler is built before running tests.
static BUILD_COMPILER: Once = Once::new();

/// Mutex to serialize tests that modify shared state (like cache tests).
/// This prevents race conditions when multiple tests try to manipulate
/// the same cache or object files simultaneously.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// Path to the blood compiler binary.
fn compiler_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir).parent().unwrap();
    workspace_root.join("target/release/blood")
}

/// Path to the examples directory.
fn examples_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir).parent().unwrap();
    workspace_root.join("examples")
}

/// Path to the test fixtures directory.
fn fixtures_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir).join("tests/fixtures")
}

/// Ensure the compiler is built in release mode.
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

/// Create a unique test cache directory.
/// Returns the path to the cache directory.
fn create_test_cache(test_name: &str) -> PathBuf {
    let temp_dir = std::env::temp_dir();
    let cache_dir = temp_dir.join(format!("blood_test_cache_{}", test_name));
    fs::create_dir_all(&cache_dir).ok();
    cache_dir
}

/// Clean up a test cache directory.
fn cleanup_test_cache(cache_dir: &Path) {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).ok();
    }
}

/// Clear object files for a specific source file.
fn clear_obj_files(source_path: &Path) {
    let obj_dir = source_path.with_extension("blood_objs");
    if obj_dir.exists() {
        fs::remove_dir_all(&obj_dir).ok();
    }
    // Also clear the executable
    let executable = source_path.with_extension("");
    if executable.exists() {
        fs::remove_file(&executable).ok();
    }
}

/// Result of compiling a Blood source file.
#[derive(Debug)]
struct CompileResult {
    success: bool,
    stdout: String,
    stderr: String,
    executable: Option<PathBuf>,
}

/// Compile a Blood source file using a specific cache directory.
fn compile_with_cache(source_path: &Path, cache_dir: &Path) -> CompileResult {
    ensure_compiler_built();

    let output = Command::new(compiler_path())
        .args(["build", source_path.to_str().unwrap()])
        .env("BLOOD_CACHE", cache_dir)
        .output()
        .expect("Failed to run compiler");

    let executable = source_path.with_extension("");

    // Wait for the executable to appear on disk (handles async filesystem operations)
    if output.status.success() {
        let max_wait = 10; // 100ms total max wait
        for i in 0..max_wait {
            if executable.exists() {
                // Give the filesystem a bit more time to sync
                thread::sleep(Duration::from_millis(20));

                // On Unix, sync to ensure data is flushed
                #[cfg(unix)]
                {
                    use std::process::Command;
                    Command::new("sync").output().ok();
                }
                break;
            }
            thread::sleep(Duration::from_millis(10 * (i + 1)));
        }
    }

    let executable_exists = executable.exists();

    CompileResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        executable: if executable_exists { Some(executable) } else { None },
    }
}

/// Result of running a compiled executable.
#[derive(Debug)]
struct RunResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

/// Run a compiled executable and return the result.
/// Includes retry logic to handle race conditions where the file
/// may not be fully written or have permissions set yet.
fn run_executable(executable: &Path) -> RunResult {
    // Ensure the file has execute permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(executable) {
            let mut permissions = metadata.permissions();
            let mode = permissions.mode();
            // Add execute permission for owner if not already set
            if mode & 0o100 == 0 {
                permissions.set_mode(mode | 0o100);
                fs::set_permissions(executable, permissions).ok();
            }
        }
    }

    // Retry logic for race conditions
    let max_retries = 3;
    let mut last_error = None;

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff: 50ms, 100ms, 200ms
            thread::sleep(Duration::from_millis(50 * (1 << attempt)));
        }

        match Command::new(executable).output() {
            Ok(output) => {
                return RunResult {
                    exit_code: output.status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                };
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    panic!(
        "Failed to run executable {:?} after {} attempts: {:?}",
        executable, max_retries, last_error
    );
}

// ============================================================================
// Test Fixtures - Simple programs with known outputs
// ============================================================================

/// Create a test fixture file with the given content.
fn create_fixture(name: &str, content: &str) -> PathBuf {
    let fixtures = fixtures_dir();
    fs::create_dir_all(&fixtures).ok();

    let path = fixtures.join(format!("{}.blood", name));
    fs::write(&path, content).expect("Failed to write fixture");
    path
}

/// Clean up a test fixture.
fn cleanup_fixture(path: &Path) {
    fs::remove_file(path).ok();
    clear_obj_files(path);

    // Also remove the executable
    let executable = path.with_extension("");
    fs::remove_file(&executable).ok();
}

// ============================================================================
// Basic Compilation Tests
// ============================================================================

#[test]
fn test_compile_simple_main() {
    let cache_dir = create_test_cache("simple_main");

    let fixture = create_fixture("simple_main", r#"
fn main() -> i32 {
    42
}
"#);

    let result = compile_with_cache(&fixture, &cache_dir);
    assert!(result.success, "Compilation failed: {}", result.stderr);
    let exit_code = run_executable(result.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_code, 42, "Simple main should return 42");

    cleanup_fixture(&fixture);
    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_compile_arithmetic() {
    let cache_dir = create_test_cache("arithmetic");

    let fixture = create_fixture("arithmetic", r#"
fn main() -> i32 {
    let x = 10;
    let y = 5;
    x + y + 3
}
"#);

    let result = compile_with_cache(&fixture, &cache_dir);
    assert!(result.success, "Compilation failed: {}", result.stderr);
    let exit_code = run_executable(result.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_code, 18, "Arithmetic should return 18");

    cleanup_fixture(&fixture);
    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_compile_conditionals() {
    let cache_dir = create_test_cache("conditionals");

    let fixture = create_fixture("conditionals", r#"
fn main() -> i32 {
    let x = 10;
    if x > 5 {
        1
    } else {
        0
    }
}
"#);

    let result = compile_with_cache(&fixture, &cache_dir);
    assert!(result.success, "Compilation failed: {}", result.stderr);
    let exit_code = run_executable(result.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_code, 1, "Conditional should return 1");

    cleanup_fixture(&fixture);
    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Effect Handler Tests
// ============================================================================
//
// These tests use serialized execution via TEST_MUTEX because they compile
// example files that live in the shared examples/ directory. Without
// serialization, parallel test execution can cause race conditions where:
// - One test clears object files while another is linking
// - Multiple tests write to the same executable simultaneously
// - Cache pollution between parallel compilations

#[test]
fn test_simple_deep_handler() {
    // Serialize access to shared example files
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("simple_deep");
    let source = examples_dir().join("simple_deep.blood");
    if !source.exists() {
        eprintln!("Skipping test_simple_deep_handler: file not found");
        return;
    }

    clear_obj_files(&source);
    let compile_result = compile_with_cache(&source, &cache_dir);

    assert!(
        compile_result.success,
        "Compilation failed:\nstdout: {}\nstderr: {}",
        compile_result.stdout,
        compile_result.stderr
    );

    let executable = compile_result.executable.expect("No executable found");
    let run_result = run_executable(&executable);

    // simple_deep.blood increments a counter once, should return 1
    assert_eq!(run_result.exit_code, 1, "simple_deep should return 1");

    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_non_tail_resume_handler() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("non_tail_resume");
    let source = examples_dir().join("non_tail_resume.blood");
    if !source.exists() {
        eprintln!("Skipping test_non_tail_resume_handler: file not found");
        return;
    }

    clear_obj_files(&source);
    let compile_result = compile_with_cache(&source, &cache_dir);

    assert!(
        compile_result.success,
        "Compilation failed:\nstdout: {}\nstderr: {}",
        compile_result.stdout,
        compile_result.stderr
    );

    let executable = compile_result.executable.expect("No executable found");
    let run_result = run_executable(&executable);

    // non_tail_resume.blood: result = 1, then adds 10, should return 11
    assert_eq!(run_result.exit_code, 11, "non_tail_resume should return 11");

    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_multi_perform_handler() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("multi_perform");
    let source = examples_dir().join("multi_perform_test.blood");
    if !source.exists() {
        eprintln!("Skipping test_multi_perform_handler: file not found");
        return;
    }

    clear_obj_files(&source);
    let compile_result = compile_with_cache(&source, &cache_dir);

    assert!(
        compile_result.success,
        "Compilation failed:\nstdout: {}\nstderr: {}",
        compile_result.stdout,
        compile_result.stderr
    );

    let executable = compile_result.executable.expect("No executable found");
    let run_result = run_executable(&executable);

    // multi_perform_test.blood: performs inc() 3 times, returns sum 1+2+3=6
    assert_eq!(run_result.exit_code, 6, "multi_perform_test should return 6");

    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_nested_handler() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("nested_handler");
    let source = examples_dir().join("nested_handler_test.blood");
    if !source.exists() {
        eprintln!("Skipping test_nested_handler: file not found");
        return;
    }

    clear_obj_files(&source);
    let compile_result = compile_with_cache(&source, &cache_dir);

    assert!(
        compile_result.success,
        "Compilation failed:\nstdout: {}\nstderr: {}",
        compile_result.stdout,
        compile_result.stderr
    );

    let executable = compile_result.executable.expect("No executable found");
    let run_result = run_executable(&executable);

    // nested_handler_test.blood should return 5
    assert_eq!(run_result.exit_code, 5, "nested_handler_test should return 5");

    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Cache Correctness Tests - These would have caught the DefId bug!
// ============================================================================
//
// These tests verify cache behavior across multiple compilations.
// They use serialized execution because they test shared example files.

/// Test that compiling different handler files sequentially doesn't cause
/// cache pollution. This is the exact bug we fixed.
#[test]
fn test_cache_no_pollution_between_handlers() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("cache_pollution");
    let simple_deep = examples_dir().join("simple_deep.blood");
    let non_tail_resume = examples_dir().join("non_tail_resume.blood");

    if !simple_deep.exists() || !non_tail_resume.exists() {
        eprintln!("Skipping test_cache_no_pollution_between_handlers: files not found");
        return;
    }

    clear_obj_files(&simple_deep);
    clear_obj_files(&non_tail_resume);

    // Compile in order: non_tail_resume first, then simple_deep
    let result1 = compile_with_cache(&non_tail_resume, &cache_dir);
    assert!(result1.success, "non_tail_resume failed: {}", result1.stderr);
    let exit1 = run_executable(result1.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit1, 11, "non_tail_resume should return 11");

    clear_obj_files(&simple_deep); // Ensure fresh compile
    let result2 = compile_with_cache(&simple_deep, &cache_dir);
    assert!(result2.success, "simple_deep failed: {}", result2.stderr);
    let exit2 = run_executable(result2.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit2, 1, "simple_deep should return 1 (not polluted by cache)");

    // Now compile in reverse order with fresh cache
    cleanup_test_cache(&cache_dir);
    let cache_dir2 = create_test_cache("cache_pollution_reverse");
    clear_obj_files(&simple_deep);
    clear_obj_files(&non_tail_resume);

    let result3 = compile_with_cache(&simple_deep, &cache_dir2);
    assert!(result3.success, "simple_deep failed: {}", result3.stderr);
    let exit3 = run_executable(result3.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit3, 1, "simple_deep should return 1");

    clear_obj_files(&non_tail_resume);
    let result4 = compile_with_cache(&non_tail_resume, &cache_dir2);
    assert!(result4.success, "non_tail_resume failed: {}", result4.stderr);
    let exit4 = run_executable(result4.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit4, 11, "non_tail_resume should return 11 (not polluted by cache)");

    cleanup_test_cache(&cache_dir2);
}

/// Test that recompiling the same file uses cache correctly.
#[test]
fn test_cache_hit_same_file() {
    let cache_dir = create_test_cache("cache_hit");

    let fixture = create_fixture("cache_test", r#"
fn main() -> i32 {
    77
}
"#);

    // First compilation
    let result1 = compile_with_cache(&fixture, &cache_dir);
    assert!(result1.success, "First compilation failed");
    let exit1 = run_executable(result1.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit1, 77, "First compilation should work");

    // Second compilation should use cache
    let result2 = compile_with_cache(&fixture, &cache_dir);
    assert!(result2.success, "Second compilation failed");
    let exit2 = run_executable(result2.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit2, 77, "Cached compilation should produce same result");

    cleanup_fixture(&fixture);
    cleanup_test_cache(&cache_dir);
}

/// Test that different files with identical structure but different names
/// produce different results.
#[test]
fn test_different_handlers_different_hashes() {
    let cache_dir = create_test_cache("different_hashes");

    // Create two handlers with identical structure but different names
    let handler_a = create_fixture("handler_a", r#"
effect Counter {
    op inc() -> i32;
}

deep handler HandlerA for Counter {
    let mut count: i32
    return(x) { x }
    op inc() {
        count = count + 1;
        resume(count)
    }
}

fn main() -> i32 {
    with HandlerA { count: 0 } handle {
        perform Counter.inc()
    }
}
"#);

    let handler_b = create_fixture("handler_b", r#"
effect Counter {
    op inc() -> i32;
}

deep handler HandlerB for Counter {
    let mut count: i32
    return(x) { x + 100 }  // Different return clause!
    op inc() {
        count = count + 1;
        resume(count)
    }
}

fn main() -> i32 {
    with HandlerB { count: 0 } handle {
        perform Counter.inc()
    }
}
"#);

    // Compile both
    let result_a = compile_with_cache(&handler_a, &cache_dir);
    assert!(result_a.success, "HandlerA failed: {}", result_a.stderr);
    let exit_a = run_executable(result_a.executable.as_ref().unwrap()).exit_code;

    let result_b = compile_with_cache(&handler_b, &cache_dir);
    assert!(result_b.success, "HandlerB failed: {}", result_b.stderr);
    let exit_b = run_executable(result_b.executable.as_ref().unwrap()).exit_code;

    // HandlerA returns 1, HandlerB returns 1 + 100 = 101
    assert_eq!(exit_a, 1, "HandlerA should return 1");
    assert_eq!(exit_b, 101, "HandlerB should return 101");

    // Verify they still work after both are compiled
    let result_a2 = compile_with_cache(&handler_a, &cache_dir);
    let exit_a2 = run_executable(result_a2.executable.as_ref().unwrap()).exit_code;

    let result_b2 = compile_with_cache(&handler_b, &cache_dir);
    let exit_b2 = run_executable(result_b2.executable.as_ref().unwrap()).exit_code;

    assert_eq!(exit_a2, 1, "HandlerA should still return 1");
    assert_eq!(exit_b2, 101, "HandlerB should still return 101");

    cleanup_fixture(&handler_a);
    cleanup_fixture(&handler_b);
    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Sequential Compilation Tests
// ============================================================================

/// Test that compiling many files sequentially doesn't cause issues.
#[test]
fn test_sequential_compilation_many_files() {
    let cache_dir = create_test_cache("sequential");

    // Create multiple fixtures with different return values
    let fixtures: Vec<(PathBuf, i32)> = (1..=5)
        .map(|i| {
            let fixture = create_fixture(
                &format!("sequential_{}", i),
                &format!(r#"
fn main() -> i32 {{
    {}
}}
"#, i * 10)
            );
            (fixture, i * 10)
        })
        .collect();

    // Compile and run each
    for (path, expected) in &fixtures {
        let result = compile_with_cache(path, &cache_dir);
        assert!(result.success, "Compilation of {:?} failed: {}", path, result.stderr);
        let exit_code = run_executable(result.executable.as_ref().unwrap()).exit_code;
        assert_eq!(
            exit_code, *expected,
            "File {:?} should return {}",
            path.file_name(),
            expected
        );
    }

    // Run them all again to verify cache doesn't corrupt anything
    for (path, expected) in &fixtures {
        let result = compile_with_cache(path, &cache_dir);
        assert!(result.success, "Cached compilation of {:?} failed: {}", path, result.stderr);
        let exit_code = run_executable(result.executable.as_ref().unwrap()).exit_code;
        assert_eq!(
            exit_code, *expected,
            "Cached file {:?} should still return {}",
            path.file_name(),
            expected
        );
    }

    // Cleanup
    for (path, _) in fixtures {
        cleanup_fixture(&path);
    }
    cleanup_test_cache(&cache_dir);
}

/// Test that modifying a file invalidates the cache correctly.
#[test]
fn test_cache_invalidation_on_modification() {
    let cache_dir = create_test_cache("modification");

    let fixture = create_fixture("modify_test", r#"
fn main() -> i32 {
    1
}
"#);

    // First compilation
    let result1 = compile_with_cache(&fixture, &cache_dir);
    assert!(result1.success, "First compilation failed");
    let exit1 = run_executable(result1.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit1, 1, "Initial compilation should return 1");

    // Modify the file
    fs::write(&fixture, r#"
fn main() -> i32 {
    2
}
"#).expect("Failed to modify fixture");

    // Recompile - should pick up the change
    clear_obj_files(&fixture);  // Force recompilation
    let result2 = compile_with_cache(&fixture, &cache_dir);
    assert!(result2.success, "Second compilation failed");
    let exit2 = run_executable(result2.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit2, 2, "Modified file should return 2");

    cleanup_fixture(&fixture);
    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Example File Tests - Verify all examples compile and run
// ============================================================================

/// Test that example files that should work actually work.
/// Add expected exit codes here as we verify them.
fn example_expected_exits() -> HashMap<&'static str, i32> {
    let mut map = HashMap::new();
    // Effect handler tests
    map.insert("simple_deep.blood", 1);      // inc() once returns 1
    map.insert("non_tail_resume.blood", 11); // 1 + 10 = 11
    map.insert("multi_perform_test.blood", 6); // 1 + 2 + 3 = 6
    map.insert("nested_handler_test.blood", 5);

    // Basic tests
    map.insert("simple.blood", 7);           // add(3, 4) = 7
    map.insert("hello.blood", 0);            // prints and returns unit (0)
    map
}

#[test]
fn test_all_known_examples() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let cache_dir = create_test_cache("all_examples");
    let examples = examples_dir();
    let expected = example_expected_exits();

    for (filename, expected_exit) in expected {
        let source = examples.join(filename);
        if !source.exists() {
            eprintln!("Skipping {}: file not found", filename);
            continue;
        }

        clear_obj_files(&source);
        let result = compile_with_cache(&source, &cache_dir);

        if !result.success {
            panic!(
                "Failed to compile {}: {}",
                filename,
                result.stderr
            );
        }

        if let Some(ref executable) = result.executable {
            let run_result = run_executable(executable);
            assert_eq!(
                run_result.exit_code, expected_exit,
                "{} should return {}, got {}",
                filename, expected_exit, run_result.exit_code
            );
        }
    }

    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Regression Tests - Add tests here for bugs we've fixed
// ============================================================================

/// Regression test for the cache pollution bug where handlers at the same
/// DefId index in different files would cause incorrect cache reuse.
///
/// Bug: Content hashing used DefId.index instead of item names, causing
/// two files with handlers at the same index to have the same hash.
///
/// Fixed in: Content hashing now uses item names for all definition references.
#[test]
fn regression_test_defid_cache_pollution() {
    let cache_dir = create_test_cache("regression_defid");

    // Create two files with handlers that would have the same DefId index
    // but different behavior
    let file_a = create_fixture("regression_a", r#"
effect State {
    op get() -> i32;
}

deep handler GetterA for State {
    let val: i32
    return(x) { x }
    op get() { resume(val) }
}

fn main() -> i32 {
    with GetterA { val: 42 } handle {
        perform State.get()
    }
}
"#);

    let file_b = create_fixture("regression_b", r#"
effect State {
    op get() -> i32;
}

deep handler GetterB for State {
    let val: i32
    return(x) { x }
    op get() { resume(val) }
}

fn main() -> i32 {
    with GetterB { val: 99 } handle {
        perform State.get()
    }
}
"#);

    // Compile file_a first
    let result_a = compile_with_cache(&file_a, &cache_dir);
    assert!(result_a.success, "File A compilation failed");
    let exit_a = run_executable(result_a.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_a, 42, "File A should return 42");

    // Compile file_b - if cache pollution occurs, this might incorrectly
    // reuse file_a's cached objects and return 42 instead of 99
    let result_b = compile_with_cache(&file_b, &cache_dir);
    assert!(result_b.success, "File B compilation failed");
    let exit_b = run_executable(result_b.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_b, 99, "File B should return 99 (not polluted by cache)");

    // Verify file_a still works correctly
    let result_a2 = compile_with_cache(&file_a, &cache_dir);
    let exit_a2 = run_executable(result_a2.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_a2, 42, "File A should still return 42");

    cleanup_fixture(&file_a);
    cleanup_fixture(&file_b);
    cleanup_test_cache(&cache_dir);
}

/// Test that files with identical function content but different source paths
/// produce different cache entries. This prevents cache contamination when
/// building multiple files in sequence.
#[test]
fn test_cache_isolation_by_source_path() {
    let cache_dir = create_test_cache("source_path_isolation");

    // Two files with identical function names and signatures but different return values
    let file_a = create_fixture("cache_iso_a", r#"
fn compute() -> i32 {
    100
}

fn main() -> i32 {
    compute()
}
"#);

    let file_b = create_fixture("cache_iso_b", r#"
fn compute() -> i32 {
    200
}

fn main() -> i32 {
    compute()
}
"#);

    // Compile file_a first
    let result_a = compile_with_cache(&file_a, &cache_dir);
    assert!(result_a.success, "File A compilation failed");
    let exit_a = run_executable(result_a.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_a, 100, "File A should return 100");

    // Compile file_b - if source path isn't included in cache hash,
    // this would incorrectly reuse file_a's cached code and return 100
    let result_b = compile_with_cache(&file_b, &cache_dir);
    assert!(result_b.success, "File B compilation failed");
    let exit_b = run_executable(result_b.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_b, 200, "File B should return 200 (not polluted by file A's cache)");

    // Verify file_a still works correctly after file_b was compiled
    let result_a2 = compile_with_cache(&file_a, &cache_dir);
    let exit_a2 = run_executable(result_a2.executable.as_ref().unwrap()).exit_code;
    assert_eq!(exit_a2, 100, "File A should still return 100");

    cleanup_fixture(&file_a);
    cleanup_fixture(&file_b);
    cleanup_test_cache(&cache_dir);
}

// ============================================================================
// Effect System Tests (Aether Patterns)
// ============================================================================
//
// These tests validate the algebraic effect system using patterns discovered
// during the development of the Aether reactive stream processing library.
// They serve as regression tests for previously fixed compiler bugs:
//
// 1. Struct values through effect operations
// 2. Enum values through effects
// 3. Match on enum fields in handlers
// 4. Array/struct field assignment in handlers
// 5. Nested effect handlers (5+ levels)
// 6. Stateful accumulation in handlers
// 7. Effect-annotated closures

/// Path to the effect test fixtures directory.
fn effects_fixtures_dir() -> PathBuf {
    fixtures_dir().join("effects")
}

/// Run an effect test fixture and verify it succeeds.
fn run_effect_test(name: &str) {
    let cache_dir = create_test_cache(&format!("effect_{}", name));
    let source = effects_fixtures_dir().join(format!("{}.blood", name));

    assert!(source.exists(), "Effect test fixture not found: {:?}", source);

    let result = compile_with_cache(&source, &cache_dir);
    assert!(
        result.success,
        "Effect test '{}' compilation failed: {}",
        name, result.stderr
    );

    let executable = result.executable.expect("No executable produced");
    let run_result = run_executable(&executable);

    assert_eq!(
        run_result.exit_code, 0,
        "Effect test '{}' failed with exit code {} (0 = pass, non-zero = assertion failure)",
        name, run_result.exit_code
    );

    // Clean up - but leave the fixture files (they're part of the repo)
    clear_obj_files(&source);
    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_effect_aether_streams() {
    // Tests: Basic Emit<T> effects, stream operations, nested handlers
    // 11 test cases covering range, map, filter, take, skip, chained operations
    run_effect_test("aether_streams");
}

#[test]
fn test_effect_aether_structs() {
    // Tests: Struct/enum values through effects, match on enum fields,
    //        struct field assignment in handlers, multi-stage pipelines
    // 8 test cases covering sensor readings, alerts, statistics
    run_effect_test("aether_structs");
}

#[test]
fn test_effect_field_match_handler() {
    // Tests: Match on enum field of handler-bound struct
    // Regression test for: "Found IntValue but expected StructValue"
    run_effect_test("field_match_handler");
}

#[test]
fn test_effect_handler_assignment() {
    // Tests: Struct field assignment inside handlers
    // Regression test for: "Cannot assign to this expression"
    run_effect_test("handler_assignment");
}

#[test]
fn test_effect_struct_emit() {
    // Tests: Passing struct values through effect operations
    // Regression test for: "unsupported argument type in perform expression"
    run_effect_test("struct_emit");
}

#[test]
fn test_effect_primitive_emit() {
    // Tests: Using primitive types (i32) directly as effect type parameters.
    // Previously a known limitation (LLVM type mismatch), now working.
    run_effect_test("primitive_emit");
}

#[test]
fn test_effect_option_emit_unification() {
    // Tests: Pattern matching a generic enum in a function using effects.
    // Previously a known limitation (type variable unification), now working.
    run_effect_test("option_effect_unify");
}

#[test]
fn test_effect_record_through_effects() {
    // Tests: Record types (structs with named fields) created, passed through
    // effect operations, and field-accessed in handlers.
    run_effect_test("record_through_effects");
}

#[test]
fn test_effect_snapshot_effect_resume() {
    // Tests: Struct data integrity across multiple suspend/resume cycles
    // with nested (up to 3-level) deep handlers. Indirectly validates
    // generation snapshot capture/restore correctness.
    // 5 test cases covering single resume, multiple resumes, 2-level nesting,
    // interleaved accumulation, and 3-level nesting.
    run_effect_test("snapshot_effect_resume");
}

/// Compile-failure test: expects compilation to fail with a specific error code.
fn run_effect_compile_failure_test(name: &str, expected_error: &str) {
    let cache_dir = create_test_cache(&format!("effect_{}", name));
    let source = effects_fixtures_dir().join(format!("{}.blood", name));

    assert!(source.exists(), "Effect test fixture not found: {:?}", source);

    let result = compile_with_cache(&source, &cache_dir);
    assert!(
        !result.success,
        "Expected compilation failure for '{}' but it succeeded",
        name
    );
    assert!(
        result.stderr.contains(expected_error),
        "Expected error '{}' in output for '{}', got: {}",
        expected_error, name, result.stderr
    );

    clear_obj_files(&source);
    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_effect_linear_multishot_reject() {
    // Tests: Compiler rejects linear state fields in multi-shot handlers (E0304).
    // A handler with a linear state field and multiple resume calls must fail.
    run_effect_compile_failure_test("linear_multishot_reject", "E0304");
}

/// Run a fixture test from the fixtures/ directory (not effects/).
fn run_fixture_test(name: &str) {
    let cache_dir = create_test_cache(&format!("fixture_{}", name));
    let source = fixtures_dir().join(format!("{}.blood", name));

    assert!(source.exists(), "Fixture test not found: {:?}", source);

    let result = compile_with_cache(&source, &cache_dir);
    assert!(
        result.success,
        "Fixture test '{}' compilation failed: {}",
        name, result.stderr
    );

    let executable = result.executable.expect("No executable produced");
    let run_result = run_executable(&executable);

    assert_eq!(
        run_result.exit_code, 0,
        "Fixture test '{}' failed with exit code {}",
        name, run_result.exit_code
    );

    clear_obj_files(&source);
    cleanup_test_cache(&cache_dir);
}

/// Run a fixture test expecting a specific exit code.
fn run_fixture_test_exit_code(name: &str, expected_exit: i32) {
    let cache_dir = create_test_cache(&format!("fixture_{}", name));
    let source = fixtures_dir().join(format!("{}.blood", name));

    assert!(source.exists(), "Fixture test not found: {:?}", source);

    let result = compile_with_cache(&source, &cache_dir);
    assert!(
        result.success,
        "Fixture test '{}' compilation failed: {}",
        name, result.stderr
    );

    let executable = result.executable.expect("No executable produced");
    let run_result = run_executable(&executable);

    assert_eq!(
        run_result.exit_code, expected_exit,
        "Fixture test '{}' expected exit code {} but got {}",
        name, expected_exit, run_result.exit_code
    );

    clear_obj_files(&source);
    cleanup_test_cache(&cache_dir);
}

#[test]
fn test_dispatch_basic() {
    // Tests: Trait-based dispatch with multiple implementations compiles and runs.
    run_fixture_test("dispatch_basic");
}

#[test]
fn test_option_struct_unwrap() {
    // Tests: Option::unwrap() for struct payloads with size > 4 but alignment 4.
    // Regression test for BUG-006: payload offset corruption.
    run_fixture_test("option_struct_unwrap");
}

#[test]
fn test_mut_ref_field_mutations_preserved() {
    // Regression test for BUG-005: Mutations through &mut struct.field were
    // silently lost because lower_unary evaluated the operand as an rvalue
    // (copying to a temp) instead of using lower_place for the original location.
    run_fixture_test("mut_ref_field");
}

#[test]
fn test_enum_nested_basic() {
    // Regression test for BUG-002: enum-in-struct-in-enum nesting with i32 payload.
    // Verifies the basic nesting pattern works correctly.
    run_fixture_test_exit_code("enum_nested_basic", 42);
}

#[test]
fn test_enum_nested_i64() {
    // Regression test for BUG-002: enum-in-struct-in-enum nesting with i64 payload.
    // Tests 8-byte aligned payload through nested enum pattern.
    run_fixture_test_exit_code("enum_nested_i64", 42);
}

#[test]
fn test_enum_nested_i128() {
    // Regression test for BUG-002: enum-in-struct-in-enum nesting with i128 payload.
    // The exact pattern that triggered payload corruption in the self-hosted compiler.
    run_fixture_test_exit_code("enum_nested_i128", 42);
}

#[test]
fn test_enum_nested_multi_field() {
    // Regression test for BUG-002: multi-field struct containing an enum,
    // wrapped in another enum. Verifies both fields are accessible (7 + 42 = 49).
    run_fixture_test_exit_code("enum_nested_multi_field", 49);
}

#[test]
fn test_enum_ref_match() {
    // Regression test for BUG-006: match on enum references was reading the
    // pointer address as the discriminant instead of dereferencing first.
    // Tests tag-only enums, payload enums, and &self method dispatch.
    run_fixture_test("enum_ref_match");
}

#[test]
fn test_region_block() {
    // Tests region { } block codegen: creates/activates/deactivates/destroys
    // regions around the block body. Verifies scalar return values pass through
    // correctly and multiple region blocks can be used sequentially.
    run_fixture_test("region_block");
}
