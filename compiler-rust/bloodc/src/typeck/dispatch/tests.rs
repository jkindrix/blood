//! Tests for dispatch resolution.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::hir::{DefId, Type, TypeKind, PrimitiveTy, TyVarId};
use crate::typeck::unify::Unifier;

use super::types::{MethodCandidate, TypeParam, Constraint, InstantiationResult};
use super::effect_row::EffectRow;
use super::result::{DispatchResult, AmbiguityError, TraitChecker};
use super::resolver::{DispatchResolver, compare_type_param_specificity};
use super::stability::{TypeStabilityChecker, TypeStabilityResult, TypeStabilityError};
use super::constraints::{ConstraintChecker, TraitConstraintChecker};

fn make_candidate(name: &str, params: Vec<Type>, ret: Type) -> MethodCandidate {
    MethodCandidate {
        def_id: DefId::new(0),
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        type_params: vec![],
        effects: None,
        trait_id: None,
    }
}

#[test]
fn test_exact_match() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
        make_candidate("add", vec![Type::i64(), Type::i64()], Type::i64()),
    ];

    let result = resolver.resolve("add", &[Type::i32(), Type::i32()], &candidates);
    assert!(matches!(result, DispatchResult::Resolved(m) if m.name == "add" && *m.return_type.kind == TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32))));
}

#[test]
fn test_no_match() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
    ];

    let result = resolver.resolve("add", &[Type::str()], &candidates);
    assert!(matches!(result, DispatchResult::NoMatch(_)));
}

#[test]
fn test_arity_mismatch() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
    ];

    let result = resolver.resolve("add", &[Type::i32()], &candidates);
    assert!(matches!(result, DispatchResult::NoMatch(_)));
}

#[test]
fn test_ambiguous_candidates() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Two methods with same signature = ambiguous
    let candidates = vec![
        make_candidate("foo", vec![Type::i32()], Type::i32()),
        make_candidate("foo", vec![Type::i32()], Type::i64()),
    ];

    let result = resolver.resolve("foo", &[Type::i32()], &candidates);
    match result {
        DispatchResult::Ambiguous(err) => {
            assert_eq!(err.method_name, "foo");
            assert_eq!(err.candidates.len(), 2);
        }
        other => panic!("Expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn test_diamond_conflict_detection() {
    // Simulate the diamond problem: two traits define the same method
    let trait1_id = DefId::new(100);
    let trait2_id = DefId::new(200);

    let mut m1 = make_candidate("render", vec![Type::i32()], Type::i32());
    m1.trait_id = Some(trait1_id);

    let mut m2 = make_candidate("render", vec![Type::i32()], Type::string());
    m2.trait_id = Some(trait2_id);

    let err = AmbiguityError {
        method_name: "render".to_string(),
        arg_types: vec![Type::i32()],
        candidates: vec![m1, m2],
    };

    // Should detect as diamond conflict
    assert!(err.is_diamond_conflict());

    // Should list both conflicting traits
    let conflicting = err.conflicting_trait_ids();
    assert_eq!(conflicting.len(), 2);
    assert!(conflicting.contains(&trait1_id));
    assert!(conflicting.contains(&trait2_id));

    // Test suggestion generation
    let mut trait_names = HashMap::new();
    trait_names.insert(trait1_id, "Drawable".to_string());
    trait_names.insert(trait2_id, "Printable".to_string());

    let suggestion = err.diamond_suggestion(&trait_names);
    assert!(suggestion.contains("Drawable"));
    assert!(suggestion.contains("Printable"));
}

#[test]
fn test_not_diamond_conflict_same_trait() {
    // Same trait, different overloads - not a diamond conflict
    let trait_id = DefId::new(100);

    let mut m1 = make_candidate("process", vec![Type::i32()], Type::i32());
    m1.trait_id = Some(trait_id);

    let mut m2 = make_candidate("process", vec![Type::i32()], Type::i64());
    m2.trait_id = Some(trait_id);

    let err = AmbiguityError {
        method_name: "process".to_string(),
        arg_types: vec![Type::i32()],
        candidates: vec![m1, m2],
    };

    // Should NOT be a diamond conflict (same trait)
    assert!(!err.is_diamond_conflict());
}

#[test]
fn test_not_diamond_conflict_no_traits() {
    // Free functions, no traits - not a diamond conflict
    let m1 = make_candidate("compute", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("compute", vec![Type::i32()], Type::i64());

    let err = AmbiguityError {
        method_name: "compute".to_string(),
        arg_types: vec![Type::i32()],
        candidates: vec![m1, m2],
    };

    // Should NOT be a diamond conflict (no traits)
    assert!(!err.is_diamond_conflict());
}

#[test]
fn test_single_candidate() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("single", vec![Type::i32()], Type::bool()),
    ];

    let result = resolver.resolve("single", &[Type::i32()], &candidates);
    match result {
        DispatchResult::Resolved(m) => {
            assert_eq!(m.name, "single");
        }
        other => panic!("Expected Resolved, got {:?}", other),
    }
}

#[test]
fn test_empty_candidates() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates: Vec<MethodCandidate> = vec![];
    let result = resolver.resolve("missing", &[Type::i32()], &candidates);

    match result {
        DispatchResult::NoMatch(err) => {
            assert_eq!(err.method_name, "missing");
            assert!(err.candidates.is_empty());
        }
        other => panic!("Expected NoMatch, got {:?}", other),
    }
}

#[test]
fn test_is_applicable_arity() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let method = make_candidate("test", vec![Type::i32(), Type::i32()], Type::i32());

    // Wrong arity
    assert!(!resolver.is_applicable(&method, &[Type::i32()]));
    assert!(!resolver.is_applicable(&method, &[Type::i32(), Type::i32(), Type::i32()]));

    // Correct arity
    assert!(resolver.is_applicable(&method, &[Type::i32(), Type::i32()]));
}

#[test]
fn test_never_type_subtyping() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Never is subtype of any type
    let method = make_candidate("test", vec![Type::i32()], Type::i32());
    assert!(resolver.is_applicable(&method, &[Type::never()]));
}

#[test]
fn test_is_more_specific() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let m1 = make_candidate("f", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("f", vec![Type::i32()], Type::i32());

    // Same signature: neither is more specific
    assert!(!resolver.is_more_specific(&m1, &m2));
    assert!(!resolver.is_more_specific(&m2, &m1));
}

#[test]
fn test_compare_specificity_equal() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let m1 = make_candidate("f", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("f", vec![Type::i32()], Type::i32());

    assert_eq!(resolver.compare_specificity(&m1, &m2), Ordering::Equal);
}

#[test]
fn test_types_equal_primitives() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    assert!(resolver.types_equal(&Type::i32(), &Type::i32()));
    assert!(!resolver.types_equal(&Type::i32(), &Type::i64()));
    assert!(resolver.types_equal(&Type::bool(), &Type::bool()));
}

#[test]
fn test_types_equal_tuples() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let t1 = Type::tuple(vec![Type::i32(), Type::bool()]);
    let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);
    let t3 = Type::tuple(vec![Type::i32(), Type::i32()]);
    let t4 = Type::tuple(vec![Type::i32()]);

    assert!(resolver.types_equal(&t1, &t2));
    assert!(!resolver.types_equal(&t1, &t3));
    assert!(!resolver.types_equal(&t1, &t4));
}

#[test]
fn test_multiple_args_dispatch() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("process", vec![Type::i32(), Type::str()], Type::unit()),
        make_candidate("process", vec![Type::i64(), Type::str()], Type::unit()),
        make_candidate("process", vec![Type::i32(), Type::i32()], Type::unit()),
    ];

    // First candidate matches
    let result = resolver.resolve("process", &[Type::i32(), Type::str()], &candidates);
    assert!(matches!(result, DispatchResult::Resolved(m) if m.param_types.len() == 2));

    // Third candidate matches
    let result = resolver.resolve("process", &[Type::i32(), Type::i32()], &candidates);
    assert!(matches!(result, DispatchResult::Resolved(_)));

    // No match
    let result = resolver.resolve("process", &[Type::bool(), Type::bool()], &candidates);
    assert!(matches!(result, DispatchResult::NoMatch(_)));
}

#[test]
fn test_compare_type_param_specificity_fn() {
    // Concrete type is more specific than type variable
    let concrete = Type::i32();
    let param = Type::new(TypeKind::Param(TyVarId::new(42)));

    let result = compare_type_param_specificity(&concrete, &param);
    assert_eq!(result, Ordering::Less);

    let result = compare_type_param_specificity(&param, &concrete);
    assert_eq!(result, Ordering::Greater);

    // Both concrete: equal
    let result = compare_type_param_specificity(&Type::i32(), &Type::i64());
    assert_eq!(result, Ordering::Equal);
}

#[test]
fn test_find_maximal_single() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("f", vec![Type::i32()], Type::i32()),
    ];

    let maximal = resolver.find_maximal(&candidates);
    assert_eq!(maximal.len(), 1);
}

#[test]
fn test_find_maximal_multiple_equal() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Two methods with same signature: both maximal
    let candidates = vec![
        make_candidate("f", vec![Type::i32()], Type::i32()),
        make_candidate("f", vec![Type::i32()], Type::i64()),
    ];

    let maximal = resolver.find_maximal(&candidates);
    assert_eq!(maximal.len(), 2);
}

#[test]
fn test_error_contains_all_candidates() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let candidates = vec![
        make_candidate("f", vec![Type::i32()], Type::i32()),
        make_candidate("f", vec![Type::i64()], Type::i64()),
    ];

    let result = resolver.resolve("f", &[Type::bool()], &candidates);
    match result {
        DispatchResult::NoMatch(err) => {
            assert_eq!(err.candidates.len(), 2);
            assert_eq!(err.arg_types.len(), 1);
        }
        other => panic!("Expected NoMatch, got {:?}", other),
    }
}

// === Variance Tests ===

#[test]
fn test_immutable_ref_covariance() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Immutable references are covariant
    let ref_i32 = Type::reference(Type::i32(), false);
    let ref_i32_2 = Type::reference(Type::i32(), false);

    // Same type
    assert!(resolver.is_subtype(&ref_i32, &ref_i32_2));
}

#[test]
fn test_mutable_ref_invariance() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Mutable references are invariant
    let mut_ref_i32 = Type::reference(Type::i32(), true);
    let mut_ref_i32_2 = Type::reference(Type::i32(), true);

    // Same type
    assert!(resolver.is_subtype(&mut_ref_i32, &mut_ref_i32_2));

    // Different inner type
    let mut_ref_i64 = Type::reference(Type::i64(), true);
    assert!(!resolver.is_subtype(&mut_ref_i32, &mut_ref_i64));
}

#[test]
fn test_mutable_to_immutable_ref() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Mutable ref can be used where immutable ref is expected
    let mut_ref_i32 = Type::reference(Type::i32(), true);
    let ref_i32 = Type::reference(Type::i32(), false);

    assert!(resolver.is_subtype(&mut_ref_i32, &ref_i32));

    // But not the other way around
    assert!(!resolver.is_subtype(&ref_i32, &mut_ref_i32));
}

#[test]
fn test_tuple_covariance() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Tuples are covariant in each position
    let t1 = Type::tuple(vec![Type::i32(), Type::bool()]);
    let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);

    assert!(resolver.is_subtype(&t1, &t2));

    // Different element types
    let t3 = Type::tuple(vec![Type::i64(), Type::bool()]);
    assert!(!resolver.is_subtype(&t1, &t3));

    // Different lengths
    let t4 = Type::tuple(vec![Type::i32()]);
    assert!(!resolver.is_subtype(&t1, &t4));
}

#[test]
fn test_never_is_subtype_of_all() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Never (bottom type) is a subtype of everything
    let never = Type::never();

    assert!(resolver.is_subtype(&never, &Type::i32()));
    assert!(resolver.is_subtype(&never, &Type::bool()));
    assert!(resolver.is_subtype(&never, &Type::str()));
    assert!(resolver.is_subtype(&never, &Type::unit()));

    // But nothing is a subtype of never (except never itself)
    assert!(!resolver.is_subtype(&Type::i32(), &never));
    assert!(resolver.is_subtype(&never, &never)); // never <: never
}

#[test]
fn test_array_covariance() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Arrays are covariant but size must match
    let arr1 = Type::array(Type::i32(), 5);
    let arr2 = Type::array(Type::i32(), 5);
    let arr3 = Type::array(Type::i32(), 10);

    assert!(resolver.is_subtype(&arr1, &arr2));
    assert!(!resolver.is_subtype(&arr1, &arr3)); // Different size
}

// === DynTrait Subtyping Tests ===

#[test]
fn test_dyn_trait_equality() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let trait_id = DefId::new(100);
    let auto1 = DefId::new(101);
    let auto2 = DefId::new(102);

    // Same trait, no auto traits
    let dyn1 = Type::dyn_trait(trait_id, vec![]);
    let dyn2 = Type::dyn_trait(trait_id, vec![]);
    assert!(resolver.types_equal(&dyn1, &dyn2));

    // Same trait, same auto traits
    let dyn3 = Type::dyn_trait(trait_id, vec![auto1, auto2]);
    let dyn4 = Type::dyn_trait(trait_id, vec![auto1, auto2]);
    assert!(resolver.types_equal(&dyn3, &dyn4));

    // Same trait, different order of auto traits (should still be equal)
    let dyn5 = Type::dyn_trait(trait_id, vec![auto2, auto1]);
    assert!(resolver.types_equal(&dyn3, &dyn5));

    // Different trait
    let other_trait = DefId::new(200);
    let dyn6 = Type::dyn_trait(other_trait, vec![]);
    assert!(!resolver.types_equal(&dyn1, &dyn6));
}

#[test]
fn test_dyn_trait_subtyping() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let trait_id = DefId::new(100);
    let send_id = DefId::new(101);
    let sync_id = DefId::new(102);

    // dyn Trait <: dyn Trait
    let dyn1 = Type::dyn_trait(trait_id, vec![]);
    let dyn2 = Type::dyn_trait(trait_id, vec![]);
    assert!(resolver.is_subtype(&dyn1, &dyn2));

    // dyn Trait + Send + Sync <: dyn Trait (superset of auto traits)
    let dyn_send_sync = Type::dyn_trait(trait_id, vec![send_id, sync_id]);
    assert!(resolver.is_subtype(&dyn_send_sync, &dyn1));

    // dyn Trait + Send <: dyn Trait
    let dyn_send = Type::dyn_trait(trait_id, vec![send_id]);
    assert!(resolver.is_subtype(&dyn_send, &dyn1));

    // dyn Trait NOT <: dyn Trait + Send (missing auto trait)
    assert!(!resolver.is_subtype(&dyn1, &dyn_send));

    // dyn Trait + Send + Sync <: dyn Trait + Send
    assert!(resolver.is_subtype(&dyn_send_sync, &dyn_send));

    // Different primary trait: dyn TraitA NOT <: dyn TraitB
    let other_trait = DefId::new(200);
    let dyn_other = Type::dyn_trait(other_trait, vec![]);
    assert!(!resolver.is_subtype(&dyn1, &dyn_other));
    assert!(!resolver.is_subtype(&dyn_other, &dyn1));
}

#[test]
fn test_type_subtype_dyn_trait_with_checker() {
    let unifier = Unifier::new();

    let trait_id = DefId::new(100);

    // Create a trait checker that says i32 implements trait 100
    let checker: &TraitChecker = &|ty: &Type, tid: DefId| {
        matches!(ty.kind.as_ref(), TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32)))
            && tid.index == 100
    };

    let resolver = DispatchResolver::with_trait_checker(&unifier, checker);

    let dyn_trait = Type::dyn_trait(trait_id, vec![]);

    // i32 <: dyn Trait (because our checker says i32 implements trait 100)
    assert!(resolver.is_subtype(&Type::i32(), &dyn_trait));

    // i64 NOT <: dyn Trait (our checker says no)
    assert!(!resolver.is_subtype(&Type::i64(), &dyn_trait));
}

#[test]
fn test_type_subtype_dyn_trait_without_checker() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let trait_id = DefId::new(100);
    let dyn_trait = Type::dyn_trait(trait_id, vec![]);

    // Without a trait checker, T <: dyn Trait should conservatively return false
    assert!(!resolver.is_subtype(&Type::i32(), &dyn_trait));
}

// === Effect-Aware Dispatch Tests ===

fn make_candidate_with_effects(
    name: &str,
    params: Vec<Type>,
    ret: Type,
    effects: Option<EffectRow>,
) -> MethodCandidate {
    MethodCandidate {
        def_id: DefId::new(0),
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        type_params: vec![],
        effects,
        trait_id: None,
    }
}

#[test]
fn test_effect_row_pure() {
    let pure = EffectRow::pure();
    assert!(pure.is_pure());
    assert_eq!(pure.effect_count(), 0);
    assert!(!pure.is_open);
}

#[test]
fn test_effect_row_with_effects() {
    let row = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);
    assert!(!row.is_pure());
    assert_eq!(row.effect_count(), 2);
    assert!(!row.is_open);
}

#[test]
fn test_effect_row_open() {
    let row = EffectRow::open(vec!["IO".to_string()]);
    assert!(!row.is_pure());
    assert!(row.is_open);
}

#[test]
fn test_effect_row_subset_pure_of_any() {
    let pure = EffectRow::pure();
    let io = EffectRow::with_effects(vec!["IO".to_string()]);
    let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

    // Pure is subset of everything
    assert!(pure.is_subset_of(&pure));
    assert!(pure.is_subset_of(&io));
    assert!(pure.is_subset_of(&io_error));
}

#[test]
fn test_effect_row_subset_closed() {
    let io = EffectRow::with_effects(vec!["IO".to_string()]);
    let error = EffectRow::with_effects(vec!["Error".to_string()]);
    let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

    // IO is subset of IO+Error
    assert!(io.is_subset_of(&io_error));

    // Error is subset of IO+Error
    assert!(error.is_subset_of(&io_error));

    // IO is NOT subset of Error
    assert!(!io.is_subset_of(&error));

    // IO+Error is NOT subset of IO
    assert!(!io_error.is_subset_of(&io));
}

#[test]
fn test_effect_row_subset_open_rows() {
    let io_open = EffectRow::open(vec!["IO".to_string()]);
    let io_closed = EffectRow::with_effects(vec!["IO".to_string()]);
    let io_error_open = EffectRow::open(vec!["IO".to_string(), "Error".to_string()]);

    // Open row is subset of open row with same or more effects
    assert!(io_open.is_subset_of(&io_open));
    assert!(io_open.is_subset_of(&io_error_open));

    // Open row is NOT subset of closed row (open could expand to anything)
    assert!(!io_open.is_subset_of(&io_closed));

    // Closed row IS subset of open row with same effects
    assert!(io_closed.is_subset_of(&io_open));
}

#[test]
fn test_effect_specificity_pure_vs_effectful() {
    let pure = EffectRow::pure();
    let io = EffectRow::with_effects(vec!["IO".to_string()]);

    // Pure is more specific than effectful
    assert_eq!(pure.compare_specificity(&io), Ordering::Less);
    assert_eq!(io.compare_specificity(&pure), Ordering::Greater);
}

#[test]
fn test_effect_specificity_closed_vs_open() {
    let io_closed = EffectRow::with_effects(vec!["IO".to_string()]);
    let io_open = EffectRow::open(vec!["IO".to_string()]);

    // Closed is more specific than open
    assert_eq!(io_closed.compare_specificity(&io_open), Ordering::Less);
    assert_eq!(io_open.compare_specificity(&io_closed), Ordering::Greater);
}

#[test]
fn test_effect_specificity_fewer_effects() {
    let io = EffectRow::with_effects(vec!["IO".to_string()]);
    let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

    // Fewer effects is more specific
    assert_eq!(io.compare_specificity(&io_error), Ordering::Less);
    assert_eq!(io_error.compare_specificity(&io), Ordering::Greater);
}

#[test]
fn test_effects_compatible_pure_always_compatible() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let context = EffectRow::with_effects(vec!["IO".to_string()]);

    // Pure method (None) is always compatible
    assert!(resolver.effects_compatible(&None, Some(&context)));
    assert!(resolver.effects_compatible(&None, None));
}

#[test]
fn test_effects_compatible_subset() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let io = EffectRow::with_effects(vec!["IO".to_string()]);
    let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

    // IO method is compatible with IO+Error context
    assert!(resolver.effects_compatible(&Some(io.clone()), Some(&io_error)));

    // IO+Error method is NOT compatible with IO-only context
    assert!(!resolver.effects_compatible(&Some(io_error), Some(&io)));
}

#[test]
fn test_pure_method_preferred_over_effectful() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Two methods with same parameter types, one pure, one with IO effect
    let pure_method = make_candidate_with_effects(
        "compute",
        vec![Type::i32()],
        Type::i32(),
        None, // Pure
    );
    let io_method = make_candidate_with_effects(
        "compute",
        vec![Type::i32()],
        Type::i32(),
        Some(EffectRow::with_effects(vec!["IO".to_string()])),
    );

    // Pure method is more specific
    assert!(resolver.is_more_specific(&pure_method, &io_method));
    assert!(!resolver.is_more_specific(&io_method, &pure_method));

    // Dispatch should resolve to pure method
    let candidates = vec![pure_method.clone(), io_method.clone()];
    let result = resolver.resolve("compute", &[Type::i32()], &candidates);

    match result {
        DispatchResult::Resolved(m) => {
            assert!(m.effects.is_none(), "Should resolve to pure method");
        }
        other => panic!("Expected Resolved, got {:?}", other),
    }
}

#[test]
fn test_effect_row_with_row_variable_compatibility() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Open row {IO | rho}
    let open_io = EffectRow::open(vec!["IO".to_string()]);
    // Closed row {IO, Error}
    let closed_io_error = EffectRow::with_effects(
        vec!["IO".to_string(), "Error".to_string()]
    );
    // Open row {IO, Error | rho}
    let open_io_error = EffectRow::open(
        vec!["IO".to_string(), "Error".to_string()]
    );

    // Open row method NOT compatible with closed context
    // (row variable could expand to include unhandled effects)
    assert!(!resolver.effects_compatible(&Some(open_io.clone()), Some(&closed_io_error)));

    // Open row method IS compatible with open context that has superset
    assert!(resolver.effects_compatible(&Some(open_io), Some(&open_io_error)));
}

#[test]
fn test_effect_specificity_as_tiebreaker() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Same parameter types, different effects
    let io_method = make_candidate_with_effects(
        "f",
        vec![Type::i32()],
        Type::i32(),
        Some(EffectRow::with_effects(vec!["IO".to_string()])),
    );
    let io_error_method = make_candidate_with_effects(
        "f",
        vec![Type::i32()],
        Type::i32(),
        Some(EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()])),
    );

    // IO method is more specific than IO+Error method (fewer effects)
    assert!(resolver.is_more_specific(&io_method, &io_error_method));
    assert!(!resolver.is_more_specific(&io_error_method, &io_method));
}

#[test]
fn test_dispatch_resolves_to_less_effectful() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let io = make_candidate_with_effects(
        "f",
        vec![Type::i32()],
        Type::i32(),
        Some(EffectRow::with_effects(vec!["IO".to_string()])),
    );
    let io_error = make_candidate_with_effects(
        "f",
        vec![Type::i32()],
        Type::i32(),
        Some(EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()])),
    );

    let candidates = vec![io_error.clone(), io.clone()];
    let result = resolver.resolve("f", &[Type::i32()], &candidates);

    match result {
        DispatchResult::Resolved(m) => {
            // Should resolve to the method with fewer effects (IO only)
            assert_eq!(m.effects.as_ref().unwrap().effect_count(), 1);
        }
        other => panic!("Expected Resolved, got {:?}", other),
    }
}

// === Generic Instantiation Tests ===

fn make_generic_candidate(
    name: &str,
    type_params: Vec<TypeParam>,
    params: Vec<Type>,
    ret: Type,
) -> MethodCandidate {
    MethodCandidate {
        def_id: DefId::new(0),
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        type_params,
        effects: None,
        trait_id: None,
    }
}

#[test]
fn test_instantiate_generic_single_type_param() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn identity<T>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);

    let method = make_generic_candidate(
        "identity",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    // Call with i32
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    match result {
        InstantiationResult::Success { substitutions, candidate } => {
            // T should be substituted with i32
            assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
            // Parameter should be i32
            assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
            // Return should be i32
            assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
            // No longer generic
            assert!(candidate.type_params.is_empty());
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

#[test]
fn test_instantiate_generic_multiple_type_params() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn pair<T, U>(x: T, y: U) -> (T, U)
    let t_id = TyVarId::new(1);
    let u_id = TyVarId::new(2);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let u_param = TypeParam {
        name: "U".to_string(),
        id: u_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);
    let u_type = Type::param(u_id);
    let ret_type = Type::tuple(vec![t_type.clone(), u_type.clone()]);

    let method = make_generic_candidate(
        "pair",
        vec![t_param, u_param],
        vec![t_type, u_type],
        ret_type,
    );

    // Call with (i32, bool)
    let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::bool()]);
    match result {
        InstantiationResult::Success { substitutions, candidate } => {
            // T should be i32, U should be bool
            assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
            assert_eq!(substitutions.get(&u_id), Some(&Type::bool()));
            // Parameters should be i32, bool
            assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
            assert!(resolver.types_equal(&candidate.param_types[1], &Type::bool()));
            // Return should be (i32, bool)
            let expected_ret = Type::tuple(vec![Type::i32(), Type::bool()]);
            assert!(resolver.types_equal(&candidate.return_type, &expected_ret));
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

#[test]
fn test_instantiate_generic_nested_types() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn wrap<T>(x: &T) -> [T; 1]
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);
    let param_type = Type::reference(t_type.clone(), false);
    let ret_type = Type::array(t_type, 1);

    let method = make_generic_candidate(
        "wrap",
        vec![t_param],
        vec![param_type],
        ret_type,
    );

    // Call with &i64
    let arg = Type::reference(Type::i64(), false);
    let result = resolver.instantiate_generic(&method, &[arg]);
    match result {
        InstantiationResult::Success { substitutions, candidate } => {
            // T should be i64
            assert_eq!(substitutions.get(&t_id), Some(&Type::i64()));
            // Parameter should be &i64
            let expected_param = Type::reference(Type::i64(), false);
            assert!(resolver.types_equal(&candidate.param_types[0], &expected_param));
            // Return should be [i64; 1]
            let expected_ret = Type::array(Type::i64(), 1);
            assert!(resolver.types_equal(&candidate.return_type, &expected_ret));
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

#[test]
fn test_instantiate_generic_consistent_substitution() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn same<T>(x: T, y: T) -> T
    // Both parameters use the same type param
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);

    let method = make_generic_candidate(
        "same",
        vec![t_param],
        vec![t_type.clone(), t_type.clone()],
        t_type,
    );

    // Call with (i32, i32) - should succeed
    let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i32()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // Call with (i32, i64) - should fail (inconsistent T)
    let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i64()]);
    match result {
        InstantiationResult::TypeMismatch { param_id, expected, found } => {
            assert_eq!(param_id, t_id);
            assert!(resolver.types_equal(&expected, &Type::i32()));
            assert!(resolver.types_equal(&found, &Type::i64()));
        }
        other => panic!("Expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn test_instantiate_generic_arity_mismatch() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);

    let method = make_generic_candidate(
        "identity",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    // Call with wrong arity
    let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i32()]);
    match result {
        InstantiationResult::ArityMismatch { expected, found } => {
            assert_eq!(expected, 1);
            assert_eq!(found, 2);
        }
        other => panic!("Expected ArityMismatch, got {:?}", other),
    }
}

#[test]
fn test_is_applicable_generic_method() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn identity<T>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);

    let method = make_generic_candidate(
        "identity",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    // Should be applicable for any type
    assert!(resolver.is_applicable(&method, &[Type::i32()]));
    assert!(resolver.is_applicable(&method, &[Type::bool()]));
    assert!(resolver.is_applicable(&method, &[Type::str()]));

    // Wrong arity should not be applicable
    assert!(!resolver.is_applicable(&method, &[]));
    assert!(!resolver.is_applicable(&method, &[Type::i32(), Type::i32()]));
}

#[test]
fn test_dispatch_with_generic_method() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Generic method: fn show<T>(x: T) -> ()
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);

    let generic_method = make_generic_candidate(
        "show",
        vec![t_param],
        vec![t_type],
        Type::unit(),
    );

    // Specific method: fn show(x: i32) -> ()
    let specific_method = make_candidate("show", vec![Type::i32()], Type::unit());

    let candidates = vec![generic_method, specific_method];

    // Calling with i32 should match both, but specific should win
    let result = resolver.resolve("show", &[Type::i32()], &candidates);
    match result {
        DispatchResult::Resolved(m) => {
            // Should resolve to the specific method (no type params)
            assert!(m.type_params.is_empty());
            assert!(resolver.types_equal(&m.param_types[0], &Type::i32()));
        }
        other => panic!("Expected Resolved, got {:?}", other),
    }

    // Calling with bool should only match generic
    let result = resolver.resolve("show", &[Type::bool()], &candidates);
    match result {
        DispatchResult::Resolved(m) => {
            // Should resolve to generic method
            assert_eq!(m.name, "show");
        }
        other => panic!("Expected Resolved, got {:?}", other),
    }
}

#[test]
fn test_instantiate_generic_with_adt() {
    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Simulate: fn get<T>(container: Option<T>) -> T
    // Where Option is an ADT with def_id 100
    let option_def_id = DefId::new(100);
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![],
    };
    let t_type = Type::param(t_id);
    let option_t = Type::adt(option_def_id, vec![t_type.clone()]);

    let method = make_generic_candidate(
        "get",
        vec![t_param],
        vec![option_t],
        t_type,
    );

    // Call with Option<i32>
    let option_i32 = Type::adt(option_def_id, vec![Type::i32()]);
    let result = resolver.instantiate_generic(&method, &[option_i32]);
    match result {
        InstantiationResult::Success { substitutions, candidate } => {
            // T should be i32
            assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
            // Return should be i32
            assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

// ============================================================
// Type Stability Tests
// ============================================================

#[test]
fn test_type_stability_same_return_type() {
    // Stable: Two methods with same input types returning same type
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![
        make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
        make_candidate("add", vec![Type::i64(), Type::i64()], Type::i64()),
    ];

    let result = checker.check_family("add", &candidates);
    assert!(result.is_stable);
    assert!(result.errors.is_empty());
}

#[test]
fn test_type_stability_different_input_types() {
    // Stable: Different input types, different return types (no overlap)
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![
        make_candidate("process", vec![Type::i32()], Type::i32()),
        make_candidate("process", vec![Type::str()], Type::str()),
    ];

    let result = checker.check_family("process", &candidates);
    assert!(result.is_stable);
    assert!(result.errors.is_empty());
}

#[test]
fn test_type_stability_unstable_same_inputs() {
    // Unstable: Same input types but different return types
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![
        make_candidate("convert", vec![Type::i32()], Type::i32()),
        make_candidate("convert", vec![Type::i32()], Type::str()),
    ];

    let result = checker.check_family("convert", &candidates);
    assert!(!result.is_stable);
    assert_eq!(result.errors.len(), 1);

    let error = &result.errors[0];
    assert_eq!(error.method_family, "convert");
    assert!(error.conflict_inputs.is_some());
}

#[test]
fn test_type_stability_different_arity() {
    // Stable: Different arities can't conflict
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![
        make_candidate("func", vec![Type::i32()], Type::i32()),
        make_candidate("func", vec![Type::i32(), Type::i32()], Type::str()),
    ];

    let result = checker.check_family("func", &candidates);
    assert!(result.is_stable);
    assert!(result.errors.is_empty());
}

#[test]
fn test_type_stability_generic_methods_compatible() {
    // Stable: Two generic methods with compatible return types
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let t_id = TyVarId::new(0);
    let t_type = Type::new(TypeKind::Param(t_id));

    let m1 = MethodCandidate {
        def_id: DefId::new(1),
        name: "identity".to_string(),
        param_types: vec![t_type.clone()],
        return_type: t_type.clone(),
        type_params: vec![TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        }],
        effects: None,
        trait_id: None,
    };

    let u_id = TyVarId::new(1);
    let u_type = Type::new(TypeKind::Param(u_id));

    let m2 = MethodCandidate {
        def_id: DefId::new(2),
        name: "identity".to_string(),
        param_types: vec![u_type.clone()],
        return_type: u_type.clone(),
        type_params: vec![TypeParam {
            name: "U".to_string(),
            id: u_id,
            constraints: vec![],
        }],
        effects: None,
        trait_id: None,
    };

    let candidates = vec![m1, m2];
    let result = checker.check_family("identity", &candidates);

    // Both are generic with type param in return position determined by input
    assert!(result.is_stable);
}

#[test]
fn test_type_stability_error_display() {
    let m1 = make_candidate("foo", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("foo", vec![Type::i32()], Type::str());

    let error = TypeStabilityError {
        method_family: "foo".to_string(),
        method1: m1,
        method2: m2,
        conflict_inputs: Some(vec![Type::i32()]),
        explanation: "Test explanation".to_string(),
    };

    let display = format!("{}", error);
    assert!(display.contains("type instability detected"));
    assert!(display.contains("foo"));
    assert!(display.contains("conflicting methods"));
}

#[test]
fn test_type_stability_tuple_inputs_no_overlap() {
    // Stable: Tuple inputs with different structures don't overlap
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![
        make_candidate(
            "process",
            vec![Type::tuple(vec![Type::i32(), Type::bool()])],
            Type::i32(),
        ),
        make_candidate(
            "process",
            vec![Type::tuple(vec![Type::str(), Type::bool()])],
            Type::str(),
        ),
    ];

    let result = checker.check_family("process", &candidates);
    assert!(result.is_stable);
}

#[test]
fn test_type_stability_single_method() {
    // Stable: Single method is always stable
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates = vec![make_candidate("single", vec![Type::i32()], Type::bool())];

    let result = checker.check_family("single", &candidates);
    assert!(result.is_stable);
    assert!(result.errors.is_empty());
}

#[test]
fn test_type_stability_empty_family() {
    // Stable: Empty method family is trivially stable
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let candidates: Vec<MethodCandidate> = vec![];
    let result = checker.check_family("empty", &candidates);
    assert!(result.is_stable);
}

#[test]
fn test_type_stability_result_constructors() {
    // Test TypeStabilityResult constructors
    let stable = TypeStabilityResult::stable();
    assert!(stable.is_stable);
    assert!(stable.errors.is_empty());

    let error = TypeStabilityError {
        method_family: "test".to_string(),
        method1: make_candidate("test", vec![], Type::unit()),
        method2: make_candidate("test", vec![], Type::bool()),
        conflict_inputs: None,
        explanation: "test".to_string(),
    };

    let unstable = TypeStabilityResult::unstable(vec![error]);
    assert!(!unstable.is_stable);
    assert_eq!(unstable.errors.len(), 1);
}

#[test]
fn test_type_stability_check_pair_directly() {
    // Test check_type_stability method directly
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let m1 = make_candidate("test", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("test", vec![Type::i32()], Type::i64());

    // These have overlapping inputs but different return types
    let error = checker.check_type_stability("test", &m1, &m2);
    assert!(error.is_some());

    let err = error.unwrap();
    assert_eq!(err.method_family, "test");
}

#[test]
fn test_type_stability_no_conflict_for_non_overlapping() {
    // Test that non-overlapping types don't trigger stability errors
    let unifier = Unifier::new();
    let checker = TypeStabilityChecker::new(&unifier);

    let m1 = make_candidate("test", vec![Type::i32()], Type::i32());
    let m2 = make_candidate("test", vec![Type::bool()], Type::str());

    // Different input types - no overlap possible
    let error = checker.check_type_stability("test", &m1, &m2);
    assert!(error.is_none());
}

// ============================================================
// Constraint Checking Tests
// ============================================================

fn make_constrained_generic_candidate(
    name: &str,
    type_params: Vec<TypeParam>,
    params: Vec<Type>,
    ret: Type,
) -> MethodCandidate {
    MethodCandidate {
        def_id: DefId::new(0),
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        type_params,
        effects: None,
        trait_id: None,
    }
}

#[test]
fn test_constraint_checker_ord_satisfied_by_i32() {
    // Generic method: fn sort<T: Ord>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Ord".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "sort",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Call with i32 - should satisfy Ord constraint
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    match result {
        InstantiationResult::Success { substitutions, candidate } => {
            assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
            assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
            assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

#[test]
fn test_constraint_checker_ord_not_satisfied_by_adt() {
    // Generic method: fn sort<T: Ord>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Ord".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "sort",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Call with an ADT (custom struct) - should NOT satisfy Ord constraint
    // ADTs don't implement Ord by default
    let custom_struct = Type::adt(DefId::new(999), vec![]);
    let result = resolver.instantiate_generic(&method, &[custom_struct.clone()]);
    match result {
        InstantiationResult::ConstraintNotSatisfied(err) => {
            assert_eq!(err.param_name, "T");
            assert_eq!(err.param_id, t_id);
            assert_eq!(err.constraint.trait_name, "Ord");
            assert!(resolver.types_equal(&err.concrete_type, &custom_struct));
        }
        other => panic!("Expected ConstraintNotSatisfied, got {:?}", other),
    }
}

#[test]
fn test_constraint_checker_multiple_constraints() {
    // Generic method: fn process<T: Ord + Hash>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![
            Constraint { trait_name: "Ord".to_string() },
            Constraint { trait_name: "Hash".to_string() },
        ],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "process",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Call with i32 - should satisfy both Ord and Hash constraints
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    match result {
        InstantiationResult::Success { substitutions, .. } => {
            assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
        }
        other => panic!("Expected Success for i32, got {:?}", other),
    }

    // Call with f64 - should fail because floats don't implement Ord/Hash
    let result = resolver.instantiate_generic(&method, &[Type::f64()]);
    match result {
        InstantiationResult::ConstraintNotSatisfied(err) => {
            assert_eq!(err.param_name, "T");
            // Should fail on first unsatisfied constraint (Ord)
            assert_eq!(err.constraint.trait_name, "Ord");
        }
        other => panic!("Expected ConstraintNotSatisfied for f64, got {:?}", other),
    }
}

#[test]
fn test_constraint_checker_copy_constraint() {
    // Generic method: fn copy_val<T: Copy>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Copy".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "copy_val",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // i32 is Copy
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // Tuple of Copy types is Copy
    let tuple = Type::tuple(vec![Type::i32(), Type::bool()]);
    let result = resolver.instantiate_generic(&method, &[tuple]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // ADTs are not Copy by default
    let adt = Type::adt(DefId::new(100), vec![]);
    let result = resolver.instantiate_generic(&method, &[adt]);
    assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
}

#[test]
fn test_constraint_checker_sized_constraint() {
    // Generic method: fn sized_val<T: Sized>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Sized".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "sized_val",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // i32 is Sized
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // str is NOT Sized
    let result = resolver.instantiate_generic(&method, &[Type::str()]);
    assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));

    // Slices are NOT Sized
    let slice = Type::slice(Type::i32());
    let result = resolver.instantiate_generic(&method, &[slice]);
    assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
}

#[test]
fn test_constraint_checker_default_constraint() {
    // Generic method: fn make_default<T: Default>() -> T
    // (For testing, we use a parameter to infer T)
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Default".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "make_default",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // i32 has Default (0)
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // bool has Default (false)
    let result = resolver.instantiate_generic(&method, &[Type::bool()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // str does NOT have Default
    let result = resolver.instantiate_generic(&method, &[Type::str()]);
    assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
}

#[test]
fn test_constraint_checker_with_custom_trait_checker() {
    // Generic method: fn custom<T: MyTrait>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "MyTrait".to_string() }],
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "custom",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    // Custom trait checker: only i32 implements MyTrait
    let custom_checker: &TraitConstraintChecker = &|ty: &Type, trait_name: &str| {
        trait_name == "MyTrait"
            && matches!(ty.kind.as_ref(), TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32)))
    };
    let constraint_checker = ConstraintChecker::with_trait_checker(custom_checker);

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // i32 implements MyTrait
    let result = resolver.instantiate_generic_with_constraint_checker(
        &method, &[Type::i32()], Some(&constraint_checker)
    );
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // i64 does NOT implement MyTrait
    let result = resolver.instantiate_generic_with_constraint_checker(
        &method, &[Type::i64()], Some(&constraint_checker)
    );
    match result {
        InstantiationResult::ConstraintNotSatisfied(err) => {
            assert_eq!(err.constraint.trait_name, "MyTrait");
        }
        other => panic!("Expected ConstraintNotSatisfied, got {:?}", other),
    }
}

#[test]
fn test_constraint_checker_no_constraints_succeeds() {
    // Generic method without constraints: fn identity<T>(x: T) -> T
    let t_id = TyVarId::new(1);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![], // No constraints
    };
    let t_type = Type::param(t_id);

    let method = make_constrained_generic_candidate(
        "identity",
        vec![t_param],
        vec![t_type.clone()],
        t_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Should succeed for any type
    let result = resolver.instantiate_generic(&method, &[Type::i32()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    let adt = Type::adt(DefId::new(100), vec![]);
    let result = resolver.instantiate_generic(&method, &[adt]);
    assert!(matches!(result, InstantiationResult::Success { .. }));
}

#[test]
fn test_constraint_checker_multiple_type_params() {
    // Generic method: fn pair<T: Ord, U: Copy>(x: T, y: U) -> (T, U)
    let t_id = TyVarId::new(1);
    let u_id = TyVarId::new(2);
    let t_param = TypeParam {
        name: "T".to_string(),
        id: t_id,
        constraints: vec![Constraint { trait_name: "Ord".to_string() }],
    };
    let u_param = TypeParam {
        name: "U".to_string(),
        id: u_id,
        constraints: vec![Constraint { trait_name: "Copy".to_string() }],
    };
    let t_type = Type::param(t_id);
    let u_type = Type::param(u_id);
    let ret_type = Type::tuple(vec![t_type.clone(), u_type.clone()]);

    let method = make_constrained_generic_candidate(
        "pair",
        vec![t_param, u_param],
        vec![t_type, u_type],
        ret_type,
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // Both i32 (satisfies both Ord and Copy)
    let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::bool()]);
    assert!(matches!(result, InstantiationResult::Success { .. }));

    // T=f64 fails (f64 doesn't satisfy Ord)
    let result = resolver.instantiate_generic(&method, &[Type::f64(), Type::i32()]);
    match result {
        InstantiationResult::ConstraintNotSatisfied(err) => {
            assert_eq!(err.param_name, "T");
            assert_eq!(err.constraint.trait_name, "Ord");
        }
        other => panic!("Expected ConstraintNotSatisfied for T, got {:?}", other),
    }

    // U=ADT fails (ADT doesn't satisfy Copy)
    let adt = Type::adt(DefId::new(100), vec![]);
    let result = resolver.instantiate_generic(&method, &[Type::i32(), adt]);
    match result {
        InstantiationResult::ConstraintNotSatisfied(err) => {
            assert_eq!(err.param_name, "U");
            assert_eq!(err.constraint.trait_name, "Copy");
        }
        other => panic!("Expected ConstraintNotSatisfied for U, got {:?}", other),
    }
}

#[test]
fn test_constraint_checker_dispatch_filters_by_constraints() {
    // Two generic methods with different constraints:
    // fn process<T: Ord>(x: T) -> T       // More specific (requires Ord)
    // fn process<T>(x: T) -> T            // Less specific (no constraints)

    let t_id_constrained = TyVarId::new(1);
    let t_id_unconstrained = TyVarId::new(2);

    // Constrained version: T: Ord
    let constrained_method = make_constrained_generic_candidate(
        "process",
        vec![TypeParam {
            name: "T".to_string(),
            id: t_id_constrained,
            constraints: vec![Constraint { trait_name: "Ord".to_string() }],
        }],
        vec![Type::param(t_id_constrained)],
        Type::param(t_id_constrained),
    );

    // Unconstrained version
    let unconstrained_method = make_constrained_generic_candidate(
        "process",
        vec![TypeParam {
            name: "T".to_string(),
            id: t_id_unconstrained,
            constraints: vec![],
        }],
        vec![Type::param(t_id_unconstrained)],
        Type::param(t_id_unconstrained),
    );

    let unifier = Unifier::new();
    let resolver = DispatchResolver::new(&unifier);

    // For i32, constrained version should be applicable (i32: Ord)
    assert!(resolver.is_applicable(&constrained_method, &[Type::i32()]));

    // For ADT, constrained version should NOT be applicable (ADT: !Ord)
    let adt = Type::adt(DefId::new(100), vec![]);
    assert!(!resolver.is_applicable(&constrained_method, &[adt.clone()]));

    // But unconstrained version should be applicable for ADT
    assert!(resolver.is_applicable(&unconstrained_method, &[adt]));
}

#[test]
fn test_constraint_checker_default_impl() {
    // Test Default implementation
    let checker = ConstraintChecker::default();
    let substitutions: HashMap<TyVarId, Type> = HashMap::new();
    let type_params: Vec<TypeParam> = vec![];

    // Empty params should succeed
    assert!(checker.check_constraints(&type_params, &substitutions).is_ok());
}
