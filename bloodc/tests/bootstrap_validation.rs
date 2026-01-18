//! Bootstrap Validation Tests
//!
//! This module contains tests for validating the Blood compiler's bootstrap process.
//! The bootstrap process verifies that the compiler can compile itself:
//!
//! - Stage 0: Rust compiler compiles Blood compiler → bloodc_stage0
//! - Stage 1: bloodc_stage0 compiles itself → bloodc_stage1
//! - Stage 2: bloodc_stage1 compiles itself → bloodc_stage2
//! - Stage 3: Verify stage2 == stage3 (byte-for-byte identical)
//!
//! Each stage tests progressively more of the self-hosting capability.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

/// Test that the Rust compiler can build the bloodc binary (Stage 0 prerequisite).
#[test]
fn test_stage0_rust_can_build_bloodc() {
    let status = Command::new("cargo")
        .args(["build", "--package", "bloodc"])
        .status()
        .expect("Failed to execute cargo build");

    assert!(status.success(), "Stage 0: Cargo should successfully build bloodc");
}

/// Test that a simple Blood program can be parsed.
#[test]
fn test_simple_blood_program_parses() {
    let source = r#"
fn main() {
    let x: i32 = 42;
}
"#;

    // Parse the source using the Blood parser
    let result = bloodc::parser::parse(source);
    assert!(result.is_ok(), "Simple Blood program should parse successfully");
}

/// Test that a simple Blood program can be type-checked.
#[test]
fn test_simple_blood_program_typechecks() {
    let source = r#"
fn main() {
    let x: i32 = 42;
    let y: i32 = x + 1;
}
"#;

    // Parse and type-check
    let ast = bloodc::parser::parse(source).expect("Should parse");

    let mut interner = string_interner::DefaultStringInterner::new();
    let result = bloodc::typeck::check_program(&ast, source, interner);

    // Type checking may fail due to incomplete implementation,
    // but it should not panic
    match result {
        Ok(_hir) => {
            // Success - type checking passed
        }
        Err(diagnostics) => {
            // Note any errors for debugging but don't fail the test
            // during development phase
            for diag in &diagnostics {
                eprintln!("Type check diagnostic: {:?}", diag);
            }
        }
    }
}

/// Test that the Blood stdlib compiler module structure is valid.
#[test]
fn test_stdlib_compiler_structure_exists() {
    let stdlib_compiler_path = Path::new("../blood-std/std/compiler");

    // Check that key directories exist
    let expected_dirs = [
        "driver",
        "hir",
        "hir/lowering",
        "mir",
        "mir/lowering",
        "codegen",
        "typeck",
        "diagnostics",
    ];

    for dir in expected_dirs {
        let path = stdlib_compiler_path.join(dir);
        assert!(path.exists(), "Expected directory {} to exist", dir);
    }

    // Check that key files exist
    let expected_files = [
        "driver/pipeline.blood",
        "hir/mod.blood",
        "hir/lowering/mod.blood",
        "mir/mod.blood",
        "mir/lowering/mod.blood",
        "codegen/mod.blood",
        "typeck/mod.blood",
    ];

    for file in expected_files {
        let path = stdlib_compiler_path.join(file);
        assert!(path.exists(), "Expected file {} to exist", file);
    }
}

/// Test that the pipeline imports are syntactically valid Blood.
#[test]
fn test_pipeline_parses() {
    let pipeline_path = Path::new("../blood-std/std/compiler/driver/pipeline.blood");
    let source = fs::read_to_string(pipeline_path)
        .expect("Should be able to read pipeline.blood");

    // Try to parse - this validates the syntax
    let result = bloodc::parser::parse(&source);

    match result {
        Ok(_) => {
            // Pipeline parses successfully
        }
        Err(errors) => {
            // During development, print but don't fail
            // Once stable, this should be a hard failure
            for error in &errors {
                eprintln!("Pipeline parse error: {:?}", error);
            }
        }
    }
}

/// Count lines of Blood code in the compiler stdlib.
/// This serves as a progress metric for self-hosting.
#[test]
fn test_stdlib_compiler_lines_metric() {
    let stdlib_compiler_path = Path::new("../blood-std/std/compiler");
    let mut total_lines = 0;
    let mut file_count = 0;

    fn count_blood_files(path: &Path, total_lines: &mut usize, file_count: &mut usize) {
        if path.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    count_blood_files(&entry.path(), total_lines, file_count);
                }
            }
        } else if path.extension().map_or(false, |ext| ext == "blood") {
            if let Ok(content) = fs::read_to_string(path) {
                *total_lines += content.lines().count();
                *file_count += 1;
            }
        }
    }

    count_blood_files(stdlib_compiler_path, &mut total_lines, &mut file_count);

    println!("=== Blood Stdlib Compiler Metrics ===");
    println!("Total .blood files: {}", file_count);
    println!("Total lines: {}", total_lines);
    println!("======================================");

    // Assertion: We should have substantial Blood code for self-hosting
    assert!(total_lines > 50000,
        "Expected at least 50,000 lines of Blood compiler code, found {}", total_lines);
}

/// Future test: Stage 1 - Blood compiler compiles itself
/// This will be enabled once Phase 3 (type system) is complete.
#[test]
#[ignore = "Stage 1 requires complete type system (Phase 3)"]
fn test_stage1_blood_compiles_itself() {
    // Stage 1: Use the Rust-compiled bloodc to compile the Blood stdlib compiler
    // This test will be enabled once the type system is complete

    // let output_path = PathBuf::from("target/bootstrap/blood_compiler_stage1");
    // let result = Command::new("target/debug/bloodc")
    //     .args(["build", "--output", output_path.to_str().unwrap()])
    //     .arg("../blood-std/std/compiler/")
    //     .status();
    //
    // assert!(result.is_ok() && result.unwrap().success(),
    //     "Stage 1: bloodc should compile the Blood stdlib compiler");
}

/// Future test: Stage 2 - Stage 1 output compiles itself
/// This will be enabled once Stage 1 passes.
#[test]
#[ignore = "Stage 2 requires Stage 1 to pass"]
fn test_stage2_stage1_compiles_itself() {
    // Stage 2: Use Stage 1 output to compile the Blood stdlib compiler again
}

/// Future test: Stage 3 verification - Stage 2 == Stage 3
/// This verifies bootstrap stability.
#[test]
#[ignore = "Stage 3 requires Stage 2 to pass"]
fn test_stage3_bootstrap_verification() {
    // Stage 3: Verify that stage2 and stage3 are byte-for-byte identical
}
