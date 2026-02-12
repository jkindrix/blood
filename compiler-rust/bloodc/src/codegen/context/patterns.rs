//! Pattern matching compilation for codegen.
//!
//! This module handles compilation of match expressions and
//! pattern testing/binding.

use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::types::BasicType;
use inkwell::IntPredicate;
use inkwell::FloatPredicate;

use crate::hir::{self, Type};
use crate::diagnostics::Diagnostic;

use crate::ice_err;

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compile a match expression.
    pub(super) fn compile_match(
        &mut self,
        scrutinee: &hir::Expr,
        arms: &[hir::MatchArm],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("Match outside function", self.current_span())])?;

        // Evaluate scrutinee once
        let scrutinee_val = self.compile_expr(scrutinee)?;

        // Create blocks for each arm and merge block
        let merge_bb = self.context.append_basic_block(fn_value, "match.end");

        let mut arm_blocks = Vec::new();
        for (i, _) in arms.iter().enumerate() {
            let test_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.test", i));
            let body_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.body", i));
            arm_blocks.push((test_bb, body_bb));
        }

        // Create unreachable block for when no pattern matches
        let unreachable_bb = self.context.append_basic_block(fn_value, "match.unreachable");

        // Jump to first arm's test
        if let Some((first_test, _)) = arm_blocks.first() {
            self.builder.build_unconditional_branch(*first_test)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        } else {
            // No arms - should not happen with exhaustive patterns
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        }

        // Collect results for phi node
        let mut incoming: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

        // Compile each arm
        for (i, arm) in arms.iter().enumerate() {
            let (test_bb, body_bb) = arm_blocks[i];
            let next_test = if i + 1 < arms.len() {
                arm_blocks[i + 1].0
            } else {
                unreachable_bb
            };

            // Test block: check if pattern matches
            self.builder.position_at_end(test_bb);

            let matches = if let Some(scrutinee_val) = &scrutinee_val {
                self.compile_pattern_test(&arm.pattern, scrutinee_val)?
            } else {
                // Scrutinee is unit - only wildcard/binding patterns match
                match &arm.pattern.kind {
                    hir::PatternKind::Wildcard | hir::PatternKind::Binding { .. } => {
                        self.context.bool_type().const_int(1, false)
                    }
                    _ => self.context.bool_type().const_int(0, false),
                }
            };

            // Check guard if present
            let final_match = if let Some(guard) = &arm.guard {
                // If pattern matches, check guard
                let guard_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.guard", i));
                self.builder.build_conditional_branch(matches, guard_bb, next_test)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                self.builder.position_at_end(guard_bb);
                // Bind pattern variables for guard evaluation
                if let Some(scrutinee_val) = &scrutinee_val {
                    self.compile_pattern_bindings(&arm.pattern, scrutinee_val)?;
                }

                let guard_val = self.compile_expr(guard)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected guard value", guard.span)])?;

                guard_val.into_int_value()
            } else {
                // Bind pattern variables directly and branch
                self.builder.build_conditional_branch(matches, body_bb, next_test)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                continue; // Branch already built
            };

            // Branch based on guard result
            self.builder.build_conditional_branch(final_match, body_bb, next_test)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        }

        // Build unreachable block
        self.builder.position_at_end(unreachable_bb);
        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

        // Compile each arm body
        for (i, arm) in arms.iter().enumerate() {
            let (_, body_bb) = arm_blocks[i];
            self.builder.position_at_end(body_bb);

            // Bind pattern variables
            if let Some(scrutinee_val) = &scrutinee_val {
                self.compile_pattern_bindings(&arm.pattern, scrutinee_val)?;
            }

            // Compile body
            let body_val = self.compile_expr(&arm.body)?;

            // Track incoming values for phi
            if let Some(val) = body_val {
                let current_bb = self.get_current_block()?;
                incoming.push((val, current_bb));
            }

            // Jump to merge
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        }

        // Position at merge block
        self.builder.position_at_end(merge_bb);

        // Create phi node if result type is non-unit
        if !result_ty.is_unit() && !incoming.is_empty() {
            let phi = self.builder.build_phi(self.lower_type(result_ty), "match.result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

            for (val, bb) in &incoming {
                phi.add_incoming(&[(val, *bb)]);
            }

            Ok(Some(phi.as_basic_value()))
        } else {
            Ok(None)
        }
    }

    /// Test if a pattern matches a value.
    /// Returns a boolean i1 value.
    pub(super) fn compile_pattern_test(
        &mut self,
        pattern: &hir::Pattern,
        scrutinee: &BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        match &pattern.kind {
            hir::PatternKind::Wildcard => {
                // Wildcard always matches
                Ok(self.context.bool_type().const_int(1, false))
            }
            hir::PatternKind::Binding { subpattern, .. } => {
                // Binding matches if subpattern matches (or always if no subpattern)
                if let Some(subpat) = subpattern {
                    self.compile_pattern_test(subpat, scrutinee)
                } else {
                    Ok(self.context.bool_type().const_int(1, false))
                }
            }
            hir::PatternKind::Literal(lit) => {
                // Compare scrutinee to literal
                let lit_val = self.compile_literal(lit)?;
                self.compile_value_eq(scrutinee, &lit_val)
            }
            hir::PatternKind::Tuple(patterns) => {
                // All sub-patterns must match
                let struct_val = scrutinee.into_struct_value();
                let mut result = self.context.bool_type().const_int(1, false);

                for (i, sub_pat) in patterns.iter().enumerate() {
                    let elem = self.builder
                        .build_extract_value(struct_val, i as u32, "tuple.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    let sub_match = self.compile_pattern_test(sub_pat, &elem)?;
                    result = self.builder
                        .build_and(result, sub_match, "and")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Struct { fields, .. } => {
                // All field patterns must match
                let struct_val = scrutinee.into_struct_value();
                let mut result = self.context.bool_type().const_int(1, false);

                for field in fields {
                    let field_val = self.builder
                        .build_extract_value(struct_val, field.field_idx, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    let sub_match = self.compile_pattern_test(&field.pattern, &field_val)?;
                    result = self.builder
                        .build_and(result, sub_match, "and")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Or(patterns) => {
                // Any sub-pattern may match
                let mut result = self.context.bool_type().const_int(0, false);

                for sub_pat in patterns {
                    let sub_match = self.compile_pattern_test(sub_pat, scrutinee)?;
                    result = self.builder
                        .build_or(result, sub_match, "or")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Variant { variant_idx, fields, .. } => {
                // First check discriminant, then check field patterns
                // Handle both unit enums (just i32 discriminant) and payload enums (struct with discriminant + fields)
                let expected = self.context.i32_type().const_int(*variant_idx as u64, false);

                // Check if scrutinee is a struct (payload enum) or int (unit enum)
                let (discriminant, struct_val_opt) = match scrutinee {
                    BasicValueEnum::IntValue(iv) => {
                        // Unit enum - the value IS the discriminant
                        (*iv, None)
                    }
                    BasicValueEnum::StructValue(sv) => {
                        // Payload enum - extract discriminant from first field
                        let discrim = self.builder
                            .build_extract_value(*sv, 0, "discrim")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                            .into_int_value();
                        (discrim, Some(*sv))
                    }
                    _ => {
                        return Err(vec![Diagnostic::error(
                            format!("Expected enum value in variant pattern, found {:?}", scrutinee.get_type()),
                            pattern.span,
                        )]);
                    }
                };

                let mut result = self.builder
                    .build_int_compare(IntPredicate::EQ, discriminant, expected, "variant.check")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                // Check field patterns (offset by 1 for discriminant) - only for payload enums
                if !fields.is_empty() {
                    let struct_val = struct_val_opt.ok_or_else(|| vec![Diagnostic::error(
                        "Cannot match on fields of unit enum variant",
                        pattern.span,
                    )])?;

                    for (i, field_pat) in fields.iter().enumerate() {
                        let field_val = self.builder
                            .build_extract_value(struct_val, (i + 1) as u32, "field")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                        let sub_match = self.compile_pattern_test(field_pat, &field_val)?;
                        result = self.builder
                            .build_and(result, sub_match, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    }
                }

                Ok(result)
            }
            hir::PatternKind::Ref { inner, .. } => {
                // Dereference and match inner pattern
                // For now, just match the inner pattern directly (simplified)
                self.compile_pattern_test(inner, scrutinee)
            }
            hir::PatternKind::Slice { prefix, slice, suffix } => {
                // Slice pattern matching - check length and elements
                let mut result = self.context.bool_type().const_int(1, false);

                if let BasicValueEnum::ArrayValue(arr) = scrutinee {
                    // Get array size from the pattern's type
                    let array_size = match pattern.ty.kind() {
                        hir::TypeKind::Array { size, .. } => {
                            size.as_u64().ok_or_else(|| vec![Diagnostic::error(
                                "Array size must be concrete in pattern matching",
                                pattern.span,
                            )])?
                        }
                        _ => {
                            return Err(vec![Diagnostic::error(
                                "Expected array type for slice pattern",
                                pattern.span,
                            )]);
                        }
                    };

                    let prefix_len = prefix.len() as u64;
                    let suffix_len = suffix.len() as u64;
                    let min_required = prefix_len + suffix_len;

                    // Length check: array must be at least prefix_len + suffix_len
                    if array_size < min_required {
                        // Pattern can never match - return false
                        return Ok(self.context.bool_type().const_int(0, false));
                    }

                    // Test prefix patterns (indices 0, 1, ..., prefix_len-1)
                    for (i, pat) in prefix.iter().enumerate() {
                        let elem = self.builder
                            .build_extract_value(*arr, i as u32, "slice.prefix")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                        let sub_match = self.compile_pattern_test(pat, &elem)?;
                        result = self.builder
                            .build_and(result, sub_match, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    }

                    // Test suffix patterns (indices array_size-suffix_len, ..., array_size-1)
                    for (i, pat) in suffix.iter().enumerate() {
                        let suffix_offset = array_size - suffix_len + i as u64;
                        let elem = self.builder
                            .build_extract_value(*arr, suffix_offset as u32, "slice.suffix")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                        let sub_match = self.compile_pattern_test(pat, &elem)?;
                        result = self.builder
                            .build_and(result, sub_match, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    }

                    // If there's a rest pattern with a binding, we don't need to test it
                    // (wildcards and bindings always match)
                    let _ = slice; // Rest pattern doesn't affect the test
                }

                Ok(result)
            }
            hir::PatternKind::Range { start, end, inclusive } => {
                // Range pattern: check if value is within range
                // Generate: start <= value && value < end (or value <= end if inclusive)
                let mut result = self.context.bool_type().const_int(1, false);
                let scrutinee_int = scrutinee.into_int_value();

                // Check lower bound: value >= start
                if let Some(start_pat) = start {
                    if let hir::PatternKind::Literal(lit) = &start_pat.kind {
                        let start_val = self.compile_literal(lit)?.into_int_value();
                        let ge_check = self.builder
                            .build_int_compare(IntPredicate::SGE, scrutinee_int, start_val, "range.ge")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                        result = self.builder
                            .build_and(result, ge_check, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    }
                }

                // Check upper bound: value < end (or value <= end if inclusive)
                if let Some(end_pat) = end {
                    if let hir::PatternKind::Literal(lit) = &end_pat.kind {
                        let end_val = self.compile_literal(lit)?.into_int_value();
                        let cmp_pred = if *inclusive { IntPredicate::SLE } else { IntPredicate::SLT };
                        let bound_check = self.builder
                            .build_int_compare(cmp_pred, scrutinee_int, end_val, "range.bound")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                        result = self.builder
                            .build_and(result, bound_check, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    }
                }

                Ok(result)
            }
        }
    }

    /// Compile value equality comparison.
    pub(super) fn compile_value_eq(
        &mut self,
        a: &BasicValueEnum<'ctx>,
        b: &BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        match (a, b) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) => {
                self.builder
                    .build_int_compare(IntPredicate::EQ, *a, *b, "eq")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
            }
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) => {
                self.builder
                    .build_float_compare(FloatPredicate::OEQ, *a, *b, "eq")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
            }
            (BasicValueEnum::PointerValue(a), BasicValueEnum::PointerValue(b)) => {
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        self.builder.build_ptr_to_int(*a, self.context.i64_type(), "ptr_a")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?,
                        self.builder.build_ptr_to_int(*b, self.context.i64_type(), "ptr_b")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?,
                        "eq",
                    )
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
            }
            (a_val, b_val) => {
                // In a properly type-checked program, we should never compare incompatible types.
                // If we reach here, it's an internal compiler error.
                Err(vec![ice_err!(
                    self.current_span(),
                    "cannot compare values of incompatible types in pattern match";
                    "lhs_type" => a_val.get_type(),
                    "rhs_type" => b_val.get_type()
                )])
            }
        }
    }

    /// Bind pattern variables to the matched value.
    pub(super) fn compile_pattern_bindings(
        &mut self,
        pattern: &hir::Pattern,
        scrutinee: &BasicValueEnum<'ctx>,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            hir::PatternKind::Wildcard => {
                // No bindings
                Ok(())
            }
            hir::PatternKind::Binding { local_id, subpattern, .. } => {
                // Allocate local and store value
                let llvm_type = self.lower_type(&pattern.ty);
                let alloca = self.builder
                    .build_alloca(llvm_type, &format!("match.bind{}", local_id.index))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                self.builder.build_store(alloca, *scrutinee)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                self.locals.insert(*local_id, alloca);
                self.local_types.insert(*local_id, llvm_type);

                // Bind subpattern if present
                if let Some(subpat) = subpattern {
                    self.compile_pattern_bindings(subpat, scrutinee)?;
                }

                Ok(())
            }
            hir::PatternKind::Literal(_) => {
                // No bindings in literals
                Ok(())
            }
            hir::PatternKind::Tuple(patterns) => {
                let struct_val = scrutinee.into_struct_value();
                for (i, sub_pat) in patterns.iter().enumerate() {
                    let elem = self.builder
                        .build_extract_value(struct_val, i as u32, "tuple.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    self.compile_pattern_bindings(sub_pat, &elem)?;
                }
                Ok(())
            }
            hir::PatternKind::Struct { fields, .. } => {
                let struct_val = scrutinee.into_struct_value();
                for field in fields {
                    let field_val = self.builder
                        .build_extract_value(struct_val, field.field_idx, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    self.compile_pattern_bindings(&field.pattern, &field_val)?;
                }
                Ok(())
            }
            hir::PatternKind::Variant { fields, .. } => {
                // For unit enums (no fields), there's nothing to bind
                // For payload enums, extract fields from the struct (starting at index 1 after discriminant)
                if !fields.is_empty() {
                    match scrutinee {
                        BasicValueEnum::StructValue(sv) => {
                            for (i, field_pat) in fields.iter().enumerate() {
                                let field_val = self.builder
                                    .build_extract_value(*sv, (i + 1) as u32, "field")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                                self.compile_pattern_bindings(field_pat, &field_val)?;
                            }
                        }
                        _ => {
                            return Err(vec![Diagnostic::error(
                                "Cannot bind fields from unit enum variant",
                                pattern.span,
                            )]);
                        }
                    }
                }
                Ok(())
            }
            hir::PatternKind::Or(patterns) => {
                // Bind from first pattern (all should bind same variables)
                if let Some(first) = patterns.first() {
                    self.compile_pattern_bindings(first, scrutinee)?;
                }
                Ok(())
            }
            hir::PatternKind::Ref { inner, .. } => {
                self.compile_pattern_bindings(inner, scrutinee)
            }
            hir::PatternKind::Slice { prefix, slice, suffix } => {
                if let BasicValueEnum::ArrayValue(arr) = scrutinee {
                    // Get array size from the pattern's type
                    let array_size = match pattern.ty.kind() {
                        hir::TypeKind::Array { size, .. } => {
                            size.as_u64().ok_or_else(|| vec![Diagnostic::error(
                                "Array size must be concrete in pattern matching",
                                pattern.span,
                            )])?
                        }
                        _ => {
                            return Err(vec![Diagnostic::error(
                                "Expected array type for slice pattern",
                                pattern.span,
                            )]);
                        }
                    };

                    let suffix_len = suffix.len() as u64;

                    // Bind prefix patterns (indices 0, 1, ..., prefix_len-1)
                    for (i, pat) in prefix.iter().enumerate() {
                        let elem = self.builder
                            .build_extract_value(*arr, i as u32, "slice.prefix")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                        self.compile_pattern_bindings(pat, &elem)?;
                    }

                    // Handle slice binding (rest pattern)
                    // The rest pattern captures the middle portion of the array
                    if let Some(slice_pat) = slice {
                        match &slice_pat.kind {
                            hir::PatternKind::Wildcard => {
                                // Wildcard rest - nothing to bind
                            }
                            hir::PatternKind::Binding { local_id, .. } => {
                                // Binding rest pattern: extract middle elements into a subarray
                                let prefix_len = prefix.len() as u64;
                                let rest_len = array_size - prefix_len - suffix_len;

                                if rest_len > 0 {
                                    // Get the element type from the slice pattern's type
                                    let elem_llvm_ty = match slice_pat.ty.kind() {
                                        hir::TypeKind::Slice { element } => {
                                            self.lower_type(element)
                                        }
                                        hir::TypeKind::Array { element, .. } => {
                                            self.lower_type(element)
                                        }
                                        _ => {
                                            return Err(vec![Diagnostic::error(
                                                "Expected slice or array type for rest pattern binding",
                                                slice_pat.span,
                                            )]);
                                        }
                                    };

                                    // Create a new array type for the rest elements
                                    let rest_array_ty = elem_llvm_ty.array_type(rest_len as u32);

                                    // Allocate space for the subarray
                                    let rest_alloca = self.builder
                                        .build_alloca(rest_array_ty, "rest.subarray")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), slice_pat.span)])?;

                                    // Copy elements from the middle of the array
                                    for i in 0..rest_len {
                                        let src_idx = prefix_len + i;
                                        let elem = self.builder
                                            .build_extract_value(*arr, src_idx as u32, "rest.elem")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), slice_pat.span)])?;

                                        // Get pointer to destination element
                                        let zero = self.context.i32_type().const_int(0, false);
                                        let idx = self.context.i32_type().const_int(i, false);
                                        let dest_ptr = unsafe {
                                            self.builder.build_gep(
                                                rest_array_ty,
                                                rest_alloca,
                                                &[zero, idx],
                                                "rest.dest",
                                            )
                                        }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), slice_pat.span)])?;

                                        self.builder.build_store(dest_ptr, elem)
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), slice_pat.span)])?;
                                    }

                                    // Bind the alloca to the local variable
                                    // (locals store pointers to values, not values directly)
                                    self.locals.insert(*local_id, rest_alloca);
                                    self.local_types.insert(*local_id, rest_array_ty.into());
                                } else {
                                    // Empty rest - create an empty array
                                    let elem_llvm_ty = match slice_pat.ty.kind() {
                                        hir::TypeKind::Slice { element } | hir::TypeKind::Array { element, .. } => {
                                            self.lower_type(element)
                                        }
                                        _ => {
                                            return Err(vec![Diagnostic::error(
                                                format!(
                                                    "Rest pattern in non-slice/array context: expected slice or array type, found {:?}",
                                                    slice_pat.ty.kind()
                                                ),
                                                slice_pat.span
                                            )]);
                                        }
                                    };
                                    let empty_array_ty = elem_llvm_ty.array_type(0);
                                    let empty_alloca = self.builder
                                        .build_alloca(empty_array_ty, "rest.empty")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), slice_pat.span)])?;
                                    self.locals.insert(*local_id, empty_alloca);
                                    self.local_types.insert(*local_id, empty_array_ty.into());
                                }
                            }
                            _ => {
                                return Err(vec![Diagnostic::error(
                                    format!(
                                        "Unsupported pattern kind in rest position: {:?}. Only binding patterns and wildcards are allowed.",
                                        slice_pat.kind
                                    ),
                                    slice_pat.span
                                )]);
                            }
                        }
                    }

                    // Bind suffix patterns (indices array_size-suffix_len, ..., array_size-1)
                    for (i, pat) in suffix.iter().enumerate() {
                        let suffix_offset = array_size - suffix_len + i as u64;
                        let elem = self.builder
                            .build_extract_value(*arr, suffix_offset as u32, "slice.suffix")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                        self.compile_pattern_bindings(pat, &elem)?;
                    }
                }
                Ok(())
            }
            hir::PatternKind::Range { .. } => {
                // Range patterns don't bind any variables
                Ok(())
            }
        }
    }
}
