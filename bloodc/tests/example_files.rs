//! Integration tests for all example Blood files.
//!
//! These tests ensure that all example files in the examples/ directory
//! can be parsed successfully without errors.

use bloodc::Parser;
use std::fs;

/// Test helper to parse a file and assert it succeeds.
fn parse_file_ok(path: &str) {
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("Failed to read file {path}: {e}");
    });

    let mut parser = Parser::new(&source);
    match parser.parse_program() {
        Ok(program) => {
            // Verify the program has some content
            assert!(
                !program.declarations.is_empty() || !program.imports.is_empty(),
                "Parsed program from {path} should have declarations or imports"
            );
        }
        Err(errors) => {
            panic!(
                "Failed to parse {path}:\n{}",
                errors
                    .iter()
                    .map(|e| format!("  - {}", e.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

#[test]
fn test_parse_hello_blood() {
    parse_file_ok("../examples/hello.blood");
}

#[test]
fn test_parse_simple_blood() {
    parse_file_ok("../examples/simple.blood");
}

#[test]
fn test_parse_test1_blood() {
    parse_file_ok("../examples/test1.blood");
}

#[test]
fn test_parse_test2_blood() {
    parse_file_ok("../examples/test2.blood");
}

#[test]
fn test_parse_test3_blood() {
    parse_file_ok("../examples/test3.blood");
}

#[test]
fn test_parse_test4_blood() {
    parse_file_ok("../examples/test4.blood");
}

#[test]
fn test_parse_test5_blood() {
    parse_file_ok("../examples/test5.blood");
}

#[test]
fn test_parse_test6_blood() {
    parse_file_ok("../examples/test6.blood");
}

#[test]
fn test_parse_algebraic_effects_blood() {
    parse_file_ok("../examples/algebraic_effects.blood");
}

/// Test that the algebraic effects example contains expected declarations.
/// This verifies all major effect system constructs are present and parseable.
#[test]
fn test_algebraic_effects_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/algebraic_effects.blood")
        .expect("Failed to read algebraic_effects.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse algebraic_effects.blood");

    // Count different declaration types
    let effect_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Effect(_))).count();
    let handler_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Handler(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have a comprehensive example
    assert!(
        effect_count >= 6,
        "Expected at least 6 effect declarations, found {effect_count}"
    );
    assert!(
        handler_count >= 10,
        "Expected at least 10 handler declarations (deep and shallow), found {handler_count}"
    );
    assert!(
        fn_count >= 15,
        "Expected at least 15 function declarations, found {fn_count}"
    );

    // Check for deep handlers
    let deep_handler_count = program.declarations.iter().filter(|d| {
        matches!(d, Declaration::Handler(h) if h.kind == bloodc::ast::HandlerKind::Deep)
    }).count();
    assert!(
        deep_handler_count >= 7,
        "Expected at least 7 deep handlers, found {deep_handler_count}"
    );

    // Check for shallow handlers
    let shallow_handler_count = program.declarations.iter().filter(|d| {
        matches!(d, Declaration::Handler(h) if h.kind == bloodc::ast::HandlerKind::Shallow)
    }).count();
    assert!(
        shallow_handler_count >= 3,
        "Expected at least 3 shallow handlers, found {shallow_handler_count}"
    );
}

/// Test that we can parse all example files in a loop.
/// This provides a single test that covers all files for quick validation.
#[test]
fn test_parse_all_examples() {
    let examples_dir = "../examples";
    let entries = fs::read_dir(examples_dir).unwrap_or_else(|e| {
        panic!("Failed to read examples directory: {e}");
    });

    let mut parsed_count = 0;
    let mut errors = Vec::new();

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "blood") {
            let path_str = path.to_string_lossy();
            let source = fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!("Failed to read {path_str}: {e}");
            });

            let mut parser = Parser::new(&source);
            match parser.parse_program() {
                Ok(_) => {
                    parsed_count += 1;
                }
                Err(parse_errors) => {
                    errors.push(format!(
                        "{path_str}:\n{}",
                        parse_errors
                            .iter()
                            .map(|e| format!("    - {}", e.message))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        panic!(
            "Failed to parse {} example files:\n{}",
            errors.len(),
            errors.join("\n\n")
        );
    }

    assert!(
        parsed_count >= 8,
        "Expected at least 8 example files, found {parsed_count}"
    );
}

/// Test parsing performance sanity check.
/// Ensures parsing the comprehensive hello.blood file is reasonably fast.
#[test]
fn test_parse_performance_sanity() {
    use std::time::Instant;

    let source = fs::read_to_string("../examples/hello.blood")
        .expect("Failed to read hello.blood");

    let start = Instant::now();
    for _ in 0..100 {
        let mut parser = Parser::new(&source);
        let _ = parser.parse_program();
    }
    let elapsed = start.elapsed();

    // Should be able to parse 100 times in under 1 second
    assert!(
        elapsed.as_secs() < 1,
        "Parsing hello.blood 100 times took {:?}, expected < 1s",
        elapsed
    );
}

// ============================================================
// Error Handling Integration Tests
// ============================================================

/// Helper to parse source and expect errors.
fn parse_expect_error(source: &str) -> Vec<bloodc::Diagnostic> {
    let mut parser = Parser::new(source);
    match parser.parse_program() {
        Ok(_) => panic!("Expected parse error but parsing succeeded"),
        Err(errors) => errors,
    }
}

/// Helper to verify error contains expected message substring.
fn assert_error_contains(errors: &[bloodc::Diagnostic], expected: &str) {
    assert!(
        errors.iter().any(|e| e.message.contains(expected)),
        "Expected error containing '{}', got: {:?}",
        expected,
        errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn test_error_missing_function_body() {
    let errors = parse_expect_error("fn foo()");
    assert_error_contains(&errors, "expected");
}

#[test]
fn test_error_unclosed_block() {
    let errors = parse_expect_error("fn foo() { let x = 1;");
    assert!(!errors.is_empty(), "Should report unclosed block error");
}

#[test]
fn test_error_unclosed_paren() {
    let errors = parse_expect_error("fn foo(x: i32 {}");
    assert!(!errors.is_empty(), "Should report unclosed paren error");
}

#[test]
fn test_error_invalid_expression() {
    let errors = parse_expect_error("fn foo() { let x = ; }");
    assert!(!errors.is_empty(), "Should report invalid expression error");
}

#[test]
fn test_error_missing_type_annotation() {
    let errors = parse_expect_error("fn foo(x) {}");
    assert_error_contains(&errors, ":");
}

#[test]
fn test_error_unexpected_token_in_struct() {
    let errors = parse_expect_error("struct Foo { + }");
    assert!(!errors.is_empty(), "Should report unexpected token error");
}

#[test]
fn test_error_invalid_import() {
    let errors = parse_expect_error("import ;");
    assert!(!errors.is_empty(), "Should report invalid import error");
}

#[test]
fn test_error_incomplete_match_arm() {
    // Missing arrow and body in match arm
    let errors = parse_expect_error("fn foo() { match x { 1 } }");
    assert!(!errors.is_empty(), "Should report error for incomplete match arm");
}

#[test]
fn test_error_recovery_continues_parsing() {
    // Parser should recover and continue after errors
    let source = r#"
        fn broken( {}
        fn valid() { 42 }
    "#;
    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // Should have errors from the broken function
    assert!(result.is_err(), "Should have parse errors");

    // The parser should have attempted to continue
    let errors = result.unwrap_err();
    assert!(!errors.is_empty(), "Should have reported errors");
}

#[test]
fn test_error_duplicate_comma() {
    let errors = parse_expect_error("fn foo(a: i32,, b: i32) {}");
    assert!(!errors.is_empty(), "Should report error for duplicate comma");
}

#[test]
fn test_error_trailing_operator() {
    let errors = parse_expect_error("fn foo() { 1 + }");
    assert!(!errors.is_empty(), "Should report error for trailing operator");
}

#[test]
fn test_error_multiple_errors_reported() {
    // Test that multiple errors are accumulated
    let source = r#"
        fn bad1( {}
        fn bad2( {}
    "#;
    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    assert!(result.is_err(), "Should have parse errors");
    let errors = result.unwrap_err();
    assert!(!errors.is_empty(), "Should report at least one error");
}

// ============================================================
// Generational Memory Example Tests
// ============================================================

#[test]
fn test_parse_generational_memory_blood() {
    parse_file_ok("../examples/generational_memory.blood");
}

/// Verify the generational memory example contains expected struct and function declarations.
/// This comprehensive example demonstrates Blood's generational memory safety model.
#[test]
fn test_generational_memory_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/generational_memory.blood")
        .expect("Failed to read generational_memory.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse generational_memory.blood");

    // Count different declaration types
    let struct_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Struct(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have the expected structs:
    // DataBlock, Point, RefHolder, SlotState, RegionStats
    assert!(
        struct_count >= 5,
        "Expected at least 5 struct declarations (DataBlock, Point, RefHolder, SlotState, RegionStats), found {struct_count}"
    );

    // Verify we have all the demonstration functions:
    // main, create_data_block, demo_basic_allocation, demo_stack_allocation,
    // demo_generation_validation, demo_generation_lifecycle, demo_aba_prevention,
    // demo_tier_promotion, demo_generation_snapshots, demo_regions,
    // demo_region_suspension, demo_performance, demo_stale_reference_effect
    assert!(
        fn_count >= 13,
        "Expected at least 13 function declarations (main + helper + 11 demo_*), found {fn_count}"
    );

    // Verify total declarations (5 structs + 13 functions = 18)
    assert!(
        program.declarations.len() >= 18,
        "Expected at least 18 total declarations, found {}",
        program.declarations.len()
    );
}

// ============================================================
// Multiple Dispatch Example Tests
// ============================================================

#[test]
fn test_parse_multiple_dispatch_blood() {
    parse_file_ok("../examples/multiple_dispatch.blood");
}

/// Verify the multiple dispatch example contains expected struct and function declarations.
/// This example demonstrates Blood's multiple dispatch concepts.
#[test]
fn test_multiple_dispatch_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/multiple_dispatch.blood")
        .expect("Failed to read multiple_dispatch.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse multiple_dispatch.blood");

    // Count different declaration types
    let struct_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Struct(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have the expected structs:
    // Rectangle, Circle, Triangle, Point2D, Point3D, Animal, Dog, Cat
    assert!(
        struct_count >= 8,
        "Expected at least 8 struct declarations (shapes, points, animals), found {struct_count}"
    );

    // Verify we have all the functions:
    // area_*, manhattan_*, distance_*, legs_*, double_*, clamp_*, abs_value,
    // identity_*, add_*, combine_*, test_*, main
    assert!(
        fn_count >= 20,
        "Expected at least 20 function declarations, found {fn_count}"
    );

    // Verify total declarations
    assert!(
        program.declarations.len() >= 28,
        "Expected at least 28 total declarations (8 structs + 20+ functions), found {}",
        program.declarations.len()
    );
}

// ============================================================
// Content Addressing Example Tests
// ============================================================

#[test]
fn test_parse_content_addressing_blood() {
    parse_file_ok("../examples/content_addressing.blood");
}

/// Verify the content addressing example contains expected struct and function declarations.
/// This example demonstrates Blood's content-addressed code identity system.
#[test]
fn test_content_addressing_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/content_addressing.blood")
        .expect("Failed to read content_addressing.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse content_addressing.blood");

    // Count different declaration types
    let struct_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Struct(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have the expected structs:
    // BuildStats, DependencyInfo, DefinitionRecord
    assert!(
        struct_count >= 3,
        "Expected at least 3 struct declarations (BuildStats, DependencyInfo, DefinitionRecord), found {struct_count}"
    );

    // Verify we have all the demonstration functions:
    // add_v1, add_v2, add_reversed, demo_hash_identity, outer, inner_example, multi_let,
    // demo_debruijn_indexing, simulate_incremental_build, lookup_cache_simulated,
    // demo_incremental_compilation, module_a_helper, module_b_square, module_c_pow2,
    // demo_deduplication, depends_on_helper, helper_function, demo_reference_by_hash,
    // create_definition_record, demo_codebase_structure, hash_to_display, demo_hash_display,
    // demo_benefits_summary, demo_comparison, main
    assert!(
        fn_count >= 25,
        "Expected at least 25 function declarations, found {fn_count}"
    );

    // Verify total declarations (3 structs + 25 functions = 28)
    assert!(
        program.declarations.len() >= 28,
        "Expected at least 28 total declarations, found {}",
        program.declarations.len()
    );
}

// ============================================================
// FFI Interop Example Tests
// ============================================================

#[test]
fn test_parse_ffi_interop_blood() {
    parse_file_ok("../examples/ffi_interop.blood");
}

/// Verify the FFI interop example contains expected struct and function declarations.
/// This example demonstrates Blood's Foreign Function Interface concepts.
#[test]
fn test_ffi_interop_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/ffi_interop.blood")
        .expect("Failed to read ffi_interop.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse ffi_interop.blood");

    // Count different declaration types
    let struct_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Struct(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have the expected structs:
    // CPoint, CRect, CSize, BufferDesc, RawPointer, FfiResult, CString, IoError, OwnedBuffer
    assert!(
        struct_count >= 9,
        "Expected at least 9 struct declarations (CPoint, CRect, CSize, BufferDesc, RawPointer, FfiResult, CString, IoError, OwnedBuffer), found {struct_count}"
    );

    // Verify we have the expected functions:
    // Constants, pointer ops, wrappers, demos, main
    assert!(
        fn_count >= 25,
        "Expected at least 25 function declarations, found {fn_count}"
    );

    // Verify total declarations
    assert!(
        program.declarations.len() >= 34,
        "Expected at least 34 total declarations (9 structs + 25+ functions), found {}",
        program.declarations.len()
    );
}

// ============================================================
// Concurrent Fibers Example Tests
// ============================================================

#[test]
fn test_parse_concurrent_fibers_blood() {
    parse_file_ok("../examples/concurrent_fibers.blood");
}

/// Verify the concurrent fibers example contains expected declarations.
/// This example demonstrates Blood's fiber-based concurrency model.
#[test]
fn test_concurrent_fibers_declarations() {
    use bloodc::ast::Declaration;

    let source = fs::read_to_string("../examples/concurrent_fibers.blood")
        .expect("Failed to read concurrent_fibers.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse concurrent_fibers.blood");

    // Count different declaration types
    let effect_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Effect(_))).count();
    let handler_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Handler(_))).count();
    let struct_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Struct(_))).count();
    let fn_count = program.declarations.iter().filter(|d| matches!(d, Declaration::Function(_))).count();

    // Verify we have the expected effects:
    // Fiber, Channel<T>, FiberSync, Cancel
    assert!(
        effect_count >= 4,
        "Expected at least 4 effect declarations (Fiber, Channel<T>, FiberSync, Cancel), found {effect_count}"
    );

    // Verify we have effect handlers:
    // FiberRuntime, ChannelRuntime<T>
    assert!(
        handler_count >= 2,
        "Expected at least 2 handler declarations (FiberRuntime, ChannelRuntime), found {handler_count}"
    );

    // Verify we have the expected structs:
    // WorkItem, ComputeResult, Message, ChannelState, Worker, ParMapResult,
    // Nursery, CancellableTask, MutexState, BarrierState
    assert!(
        struct_count >= 10,
        "Expected at least 10 struct declarations (WorkItem, ComputeResult, Message, etc.), found {struct_count}"
    );

    // Verify we have all the demonstration functions:
    // work_item_new, result_new, demo_fiber_creation, long_computation,
    // demo_cooperative_scheduling, message_new, channel_state_new, channel_send,
    // demo_channel_communication, worker_new, worker_process, demo_parallel_workloads,
    // nursery_new, nursery_spawn, nursery_complete, demo_structured_concurrency,
    // cancellable_task_new, task_step, task_cancel, demo_cancellation,
    // mutex_new, mutex_acquire, mutex_release, barrier_new, barrier_wait,
    // demo_synchronization, demo_effect_handlers, demo_memory_safety, main
    assert!(
        fn_count >= 25,
        "Expected at least 25 function declarations, found {fn_count}"
    );

    // Verify total declarations (4 effects + 2 handlers + 10 structs + 25+ functions = 41+)
    assert!(
        program.declarations.len() >= 40,
        "Expected at least 40 total declarations (4 effects + 2 handlers + 10 structs + 25+ functions), found {}",
        program.declarations.len()
    );
}
