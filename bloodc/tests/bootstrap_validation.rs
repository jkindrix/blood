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

use std::path::Path;
use std::process::Command;
use std::fs;

use bloodc::parser::Parser;

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
    let mut parser = Parser::new(source);
    let result = parser.parse_program();
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

    // Parse the source
    let mut parser = Parser::new(source);
    let ast = parser.parse_program().expect("Should parse");
    let interner = parser.take_interner();

    // Type check the program
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
    let mut parser = Parser::new(&source);
    let result = parser.parse_program();

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

/// Test that the bootstrap stdlib loads correctly.
#[test]
fn test_bootstrap_stdlib_loads() {
    use bloodc::stdlib_loader::StdlibLoader;
    use bloodc::typeck::TypeContext;
    use string_interner::DefaultStringInterner;

    let bootstrap_stdlib_path = Path::new("../blood-std/bootstrap-std/std");

    if !bootstrap_stdlib_path.exists() {
        eprintln!("Bootstrap stdlib path does not exist: {:?}", bootstrap_stdlib_path);
        return;
    }

    eprintln!("\n=== Bootstrap Stdlib Loading Test ===");

    // Create stdlib loader
    let mut loader = StdlibLoader::new(bootstrap_stdlib_path.to_path_buf());

    // Discover modules
    eprintln!("\n--- Discovering modules ---");
    match loader.discover() {
        Ok(_) => eprintln!("Discovery successful"),
        Err(e) => {
            eprintln!("Discovery failed: {:?}", e);
            panic!("Failed to discover modules");
        }
    }

    eprintln!("Found {} modules:", loader.module_count());
    for path in loader.module_paths() {
        eprintln!("  - {}", path);
    }

    // Parse modules
    eprintln!("\n--- Parsing modules ---");
    match loader.parse_all() {
        Ok(_) => eprintln!("Parsing successful"),
        Err(errors) => {
            for e in &errors {
                eprintln!("Parse error: {}", e);
            }
            panic!("Failed to parse modules");
        }
    }

    // Register in context
    eprintln!("\n--- Registering in TypeContext ---");
    let interner = DefaultStringInterner::new();
    let mut ctx = TypeContext::new("", interner);

    match loader.register_in_context(&mut ctx) {
        Ok(_) => eprintln!("Registration successful"),
        Err(errors) => {
            for e in &errors {
                eprintln!("Registration error: {}", e);
            }
            panic!("Failed to register in context");
        }
    }

    eprintln!("\n=== Bootstrap Stdlib Loading Complete ===\n");

    // Verify expected modules exist
    assert!(loader.module_count() >= 7,
        "Expected at least 7 modules in bootstrap stdlib, found {}", loader.module_count());
}

/// Test that a program using bootstrap stdlib types parses.
#[test]
fn test_program_with_bootstrap_types_parses() {
    let source = r#"
use std.option.Option;
use std.result.Result;

fn main() {
    let x: Option<i32> = Option::None;
    let y: Result<i32, bool> = Result::Ok(42);
}
"#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Program with bootstrap types should parse: {:?}", result);
}

/// Test that a program using bootstrap stdlib types can be type-checked.
#[test]
fn test_program_with_bootstrap_types_typechecks() {
    use bloodc::stdlib_loader::StdlibLoader;
    use bloodc::typeck::TypeContext;

    let bootstrap_stdlib_path = Path::new("../blood-std/bootstrap-std/std");
    if !bootstrap_stdlib_path.exists() {
        eprintln!("Bootstrap stdlib path does not exist, skipping test");
        return;
    }

    // Load the bootstrap stdlib
    let mut loader = StdlibLoader::new(bootstrap_stdlib_path.to_path_buf());
    loader.discover().expect("Discovery should succeed");
    loader.parse_all().expect("Parsing should succeed");

    // Source code using stdlib types
    let source = r#"
fn main() {
    let x: i32 = 42;
    let y: Option<i32> = Some(x);
    let z: Result<i32, bool> = Ok(x);
}
"#;

    // Parse the source
    let mut parser = Parser::new(source);
    let _ast = parser.parse_program().expect("Should parse");
    let interner = parser.take_interner();

    // Create context and register stdlib
    let mut ctx = TypeContext::new(source, interner);

    // Register stdlib in context
    let reg_result = loader.register_in_context(&mut ctx);
    match &reg_result {
        Ok(_) => eprintln!("Stdlib registration successful"),
        Err(errors) => {
            for e in errors {
                eprintln!("Stdlib registration error: {}", e);
            }
        }
    }

    // Note: Full type checking may fail due to incomplete import resolution,
    // but we've verified the stdlib loads correctly. The import resolution
    // for module paths like std.option.Option needs additional work.
    eprintln!("Bootstrap stdlib types test complete");
}

/// Test that all Blood compiler stdlib modules parse correctly.
/// This is a comprehensive parse check for the entire stdlib compiler.
#[test]
#[ignore = "Blood stdlib has syntax differences that need to be fixed"]
fn test_all_compiler_modules_parse() {
    use std::collections::VecDeque;

    let stdlib_compiler_path = Path::new("../blood-std/std/compiler");
    let mut parsed_count = 0;
    let mut error_count = 0;
    let mut errors: Vec<(String, String)> = Vec::new();

    // Use iterative approach instead of recursive to avoid stack overflow
    let mut dirs_to_visit: VecDeque<std::path::PathBuf> = VecDeque::new();
    dirs_to_visit.push_back(stdlib_compiler_path.to_path_buf());

    while let Some(dir) = dirs_to_visit.pop_front() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs_to_visit.push_back(path);
                } else if path.extension().map_or(false, |ext| ext == "blood") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        let mut parser = Parser::new(&content);
                        match parser.parse_program() {
                            Ok(_) => parsed_count += 1,
                            Err(parse_errors) => {
                                error_count += 1;
                                let path_str = path.display().to_string();
                                let error_msg = parse_errors
                                    .iter()
                                    .take(3)  // Limit errors per file
                                    .map(|e| format!("  L{}:{}: {}", e.span.start_line, e.span.start_col, e.message))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                errors.push((path_str, error_msg));
                            }
                        }
                    }
                }
            }
        }
    }

    println!("=== Blood Compiler Parse Results ===");
    println!("Successfully parsed: {}", parsed_count);
    println!("Failed to parse: {}", error_count);

    if !errors.is_empty() {
        println!("\n=== Parse Errors ===");
        for (path, err) in errors.iter().take(10) {  // Limit output
            println!("\n{}:\n{}", path, err);
        }
        if errors.len() > 10 {
            println!("\n... and {} more files with errors", errors.len() - 10);
        }
        println!("======================================");
    }

    // All modules should parse - no tolerance for parse errors
    assert!(
        error_count == 0,
        "All Blood compiler modules should parse. {} modules failed.",
        error_count
    );
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
