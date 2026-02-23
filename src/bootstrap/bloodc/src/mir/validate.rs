//! MIR validation pass.
//!
//! Checks MIR bodies for well-formedness before codegen, catching malformed
//! MIR early with clear diagnostics rather than letting it produce cryptic
//! LLVM errors downstream.

use std::collections::HashMap;

use crate::diagnostics::Diagnostic;
use crate::hir::DefId;
use crate::hir::ty::TypeKind;
use crate::mir::body::{MirBody, LocalKind};
use crate::mir::types::{
    PlaceBase, PlaceElem,
    Operand, Rvalue, StatementKind, TerminatorKind,
};

/// Results from MIR validation.
pub struct ValidationResults {
    /// Fatal errors that will cause codegen to fail or produce wrong code.
    pub errors: Vec<Diagnostic>,
    /// Non-fatal warnings about technically malformed MIR that codegen
    /// handles gracefully (e.g. unterminated blocks get `unreachable`).
    pub warnings: Vec<Diagnostic>,
}

/// Validate all MIR bodies for well-formedness.
///
/// Returns validation results containing both fatal errors and non-fatal
/// warnings. Callers should emit warnings but only abort on errors.
pub fn validate_mir_bodies(
    mir_bodies: &HashMap<DefId, MirBody>,
) -> ValidationResults {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (&def_id, body) in mir_bodies {
        validate_body(def_id, body, &mut errors, &mut warnings);
    }
    ValidationResults { errors, warnings }
}

/// Validate a single MIR body for well-formedness.
fn validate_body(def_id: DefId, body: &MirBody, errors: &mut Vec<Diagnostic>, warnings: &mut Vec<Diagnostic>) {
    let num_blocks = body.basic_blocks.len();
    let num_locals = body.locals.len();

    // Check 1: Entry block exists
    if num_blocks == 0 {
        errors.push(Diagnostic::error(
            format!("MIR body for {:?} has no basic blocks", def_id),
            body.span,
        ));
        return; // No point checking further
    }

    // Check 2: Return place exists and is at index 0
    if num_locals == 0 {
        errors.push(Diagnostic::error(
            format!("MIR body for {:?} has no locals (missing return place)", def_id),
            body.span,
        ));
    } else if body.locals[0].kind != LocalKind::ReturnPlace {
        errors.push(Diagnostic::error(
            format!(
                "MIR body for {:?}: local 0 should be ReturnPlace but is {:?}",
                def_id, body.locals[0].kind
            ),
            body.span,
        ));
    }

    // Check 3: All blocks have terminators (warning — codegen inserts `unreachable`)
    for (idx, block) in body.basic_blocks.iter().enumerate() {
        if block.terminator.is_none() {
            warnings.push(Diagnostic::warning(
                format!(
                    "MIR body for {:?}: block bb{} has no terminator",
                    def_id, idx
                ),
                body.span,
            ));
        }
    }

    // Check 4: Terminator targets are valid block indices
    for (idx, block) in body.basic_blocks.iter().enumerate() {
        if let Some(ref term) = block.terminator {
            for target in term.successors() {
                if target.index() >= num_blocks {
                    errors.push(Diagnostic::error(
                        format!(
                            "MIR body for {:?}: bb{} terminator references \
                             non-existent block bb{} (body has {} blocks)",
                            def_id, idx, target.index(), num_blocks
                        ),
                        term.span,
                    ));
                }
            }
        }
    }

    // Check 5: Locals referenced in statements and terminators exist
    for (idx, block) in body.basic_blocks.iter().enumerate() {
        for stmt in &block.statements {
            validate_statement_locals(def_id, idx, stmt, num_locals, errors);
        }
        if let Some(ref term) = block.terminator {
            validate_terminator_locals(def_id, idx, term, num_locals, errors);
        }
    }

    // Check 6: No unresolved type parameters or inference variables in locals
    for local in &body.locals {
        check_type_resolved(&local.ty, def_id, body, errors, warnings);
    }
}

/// Check that a type contains no unresolved parameters or inference variables.
fn check_type_resolved(
    ty: &crate::hir::Type,
    def_id: DefId,
    body: &MirBody,
    errors: &mut Vec<Diagnostic>,
    warnings: &mut Vec<Diagnostic>,
) {
    match ty.kind() {
        TypeKind::Param(id) => {
            errors.push(Diagnostic::error(
                format!(
                    "MIR body for {:?}: unsubstituted type parameter {:?} \
                     found in local type; monomorphization incomplete",
                    def_id, id
                ),
                body.span,
            ));
        }
        TypeKind::Infer(id) => {
            errors.push(Diagnostic::error(
                format!(
                    "MIR body for {:?}: unresolved inference variable {:?} \
                     found in local type; type inference incomplete",
                    def_id, id
                ),
                body.span,
            ));
        }
        // Recurse into compound types
        TypeKind::Tuple(fields) => {
            for f in fields {
                check_type_resolved(f, def_id, body, errors, warnings);
            }
        }
        TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
            check_type_resolved(element, def_id, body, errors, warnings);
        }
        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
            check_type_resolved(inner, def_id, body, errors, warnings);
        }
        TypeKind::Adt { args, .. } => {
            for arg in args {
                check_type_resolved(arg, def_id, body, errors, warnings);
            }
        }
        TypeKind::Fn { params, ret, .. } => {
            for p in params {
                check_type_resolved(p, def_id, body, errors, warnings);
            }
            check_type_resolved(ret, def_id, body, errors, warnings);
        }
        _ => {}
    }
}

/// Validate that all locals referenced in a statement exist.
fn validate_statement_locals(
    def_id: DefId,
    block_idx: usize,
    stmt: &crate::mir::types::Statement,
    num_locals: usize,
    errors: &mut Vec<Diagnostic>,
) {
    match &stmt.kind {
        StatementKind::Assign(place, rvalue) => {
            validate_place_local(def_id, block_idx, place, num_locals, stmt.span, errors);
            validate_rvalue_locals(def_id, block_idx, rvalue, num_locals, stmt.span, errors);
        }
        StatementKind::StorageLive(local_id)
        | StatementKind::StorageDead(local_id)
        | StatementKind::CaptureSnapshot(local_id) => {
            if (local_id.index() as usize) >= num_locals {
                errors.push(Diagnostic::error(
                    format!(
                        "MIR body for {:?}: bb{} references non-existent local _{} \
                         (body has {} locals)",
                        def_id, block_idx, local_id.index(), num_locals
                    ),
                    stmt.span,
                ));
            }
        }
        StatementKind::Drop(place)
        | StatementKind::IncrementGeneration(place) => {
            validate_place_local(def_id, block_idx, place, num_locals, stmt.span, errors);
        }
        StatementKind::ValidateGeneration { ptr, expected_gen } => {
            validate_place_local(def_id, block_idx, ptr, num_locals, stmt.span, errors);
            validate_operand_local(def_id, block_idx, expected_gen, num_locals, stmt.span, errors);
        }
        StatementKind::PushHandler { state_place, .. } => {
            validate_place_local(def_id, block_idx, state_place, num_locals, stmt.span, errors);
        }
        StatementKind::PushInlineHandler { .. } => {
            // Inline handler operations reference synthetic DefIds, not locals
        }
        StatementKind::CallReturnClause { body_result, state_place, destination, .. } => {
            validate_operand_local(def_id, block_idx, body_result, num_locals, stmt.span, errors);
            validate_place_local(def_id, block_idx, state_place, num_locals, stmt.span, errors);
            validate_place_local(def_id, block_idx, destination, num_locals, stmt.span, errors);
        }
        StatementKind::PopHandler | StatementKind::Nop => {}
    }
}

/// Validate that all locals referenced in a terminator exist.
fn validate_terminator_locals(
    def_id: DefId,
    block_idx: usize,
    term: &crate::mir::types::Terminator,
    num_locals: usize,
    errors: &mut Vec<Diagnostic>,
) {
    match &term.kind {
        TerminatorKind::Goto { .. }
        | TerminatorKind::Return
        | TerminatorKind::Unreachable => {}

        TerminatorKind::SwitchInt { discr, .. } => {
            validate_operand_local(def_id, block_idx, discr, num_locals, term.span, errors);
        }

        TerminatorKind::Call { func, args, destination, .. } => {
            validate_operand_local(def_id, block_idx, func, num_locals, term.span, errors);
            for arg in args {
                validate_operand_local(def_id, block_idx, arg, num_locals, term.span, errors);
            }
            validate_place_local(def_id, block_idx, destination, num_locals, term.span, errors);
        }

        TerminatorKind::Assert { cond, .. } => {
            validate_operand_local(def_id, block_idx, cond, num_locals, term.span, errors);
        }

        TerminatorKind::DropAndReplace { place, value, .. } => {
            validate_place_local(def_id, block_idx, place, num_locals, term.span, errors);
            validate_operand_local(def_id, block_idx, value, num_locals, term.span, errors);
        }

        TerminatorKind::Perform { args, destination, .. } => {
            for arg in args {
                validate_operand_local(def_id, block_idx, arg, num_locals, term.span, errors);
            }
            validate_place_local(def_id, block_idx, destination, num_locals, term.span, errors);
        }

        TerminatorKind::Resume { value } => {
            if let Some(operand) = value {
                validate_operand_local(def_id, block_idx, operand, num_locals, term.span, errors);
            }
        }

        TerminatorKind::StaleReference { ptr, .. } => {
            validate_place_local(def_id, block_idx, ptr, num_locals, term.span, errors);
        }
    }
}

/// Validate that a place's base local (if any) exists.
fn validate_place_local(
    def_id: DefId,
    block_idx: usize,
    place: &crate::mir::types::Place,
    num_locals: usize,
    span: crate::span::Span,
    errors: &mut Vec<Diagnostic>,
) {
    if let PlaceBase::Local(local_id) = &place.base {
        if (local_id.index() as usize) >= num_locals {
            errors.push(Diagnostic::error(
                format!(
                    "MIR body for {:?}: bb{} references non-existent local _{} \
                     (body has {} locals)",
                    def_id, block_idx, local_id.index(), num_locals
                ),
                span,
            ));
        }
    }
    // Also check Index projections which reference locals
    for elem in &place.projection {
        if let PlaceElem::Index(local_id) = elem {
            if (local_id.index() as usize) >= num_locals {
                errors.push(Diagnostic::error(
                    format!(
                        "MIR body for {:?}: bb{} Index projection references \
                         non-existent local _{} (body has {} locals)",
                        def_id, block_idx, local_id.index(), num_locals
                    ),
                    span,
                ));
            }
        }
    }
}

/// Validate that an operand's place local (if any) exists.
fn validate_operand_local(
    def_id: DefId,
    block_idx: usize,
    operand: &Operand,
    num_locals: usize,
    span: crate::span::Span,
    errors: &mut Vec<Diagnostic>,
) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            validate_place_local(def_id, block_idx, place, num_locals, span, errors);
        }
        Operand::Constant(_) => {}
    }
}

/// Validate that all locals referenced in an rvalue exist.
fn validate_rvalue_locals(
    def_id: DefId,
    block_idx: usize,
    rvalue: &Rvalue,
    num_locals: usize,
    span: crate::span::Span,
    errors: &mut Vec<Diagnostic>,
) {
    match rvalue {
        Rvalue::Use(op)
        | Rvalue::UnaryOp { operand: op, .. }
        | Rvalue::Cast { operand: op, .. }
        | Rvalue::NullCheck(op)
        | Rvalue::ArrayToSlice { array_ref: op, .. } => {
            validate_operand_local(def_id, block_idx, op, num_locals, span, errors);
        }
        Rvalue::Ref { place, .. }
        | Rvalue::AddressOf { place, .. }
        | Rvalue::Discriminant(place)
        | Rvalue::Len(place)
        | Rvalue::VecLen(place)
        | Rvalue::ReadGeneration(place) => {
            validate_place_local(def_id, block_idx, place, num_locals, span, errors);
        }
        Rvalue::BinaryOp { left, right, .. }
        | Rvalue::CheckedBinaryOp { left, right, .. }
        | Rvalue::StringIndex { base: left, index: right } => {
            validate_operand_local(def_id, block_idx, left, num_locals, span, errors);
            validate_operand_local(def_id, block_idx, right, num_locals, span, errors);
        }
        Rvalue::Aggregate { operands, .. } => {
            for op in operands {
                validate_operand_local(def_id, block_idx, op, num_locals, span, errors);
            }
        }
        Rvalue::MakeGenPtr { address, generation, metadata } => {
            validate_operand_local(def_id, block_idx, address, num_locals, span, errors);
            validate_operand_local(def_id, block_idx, generation, num_locals, span, errors);
            validate_operand_local(def_id, block_idx, metadata, num_locals, span, errors);
        }
        Rvalue::ZeroInit(_) => {}
    }
}
