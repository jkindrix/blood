//! Escape analysis benchmarks using the REAL implementation.
//!
//! These benchmarks test the actual escape analysis code in bloodc::mir::escape,
//! NOT a simulation. This ensures benchmarks accurately reflect compiler performance.
//!
//! Run with: cargo bench --bench escape_bench
//!
//! PERF-005: Benchmark escape analysis optimization effectiveness
//! PERF-IMPL-001: Validates O(n) worklist algorithm for closure chains

use bloodc::hir::{DefId, LocalId, Type};
use bloodc::mir::body::{LocalKind, MirBody};
use bloodc::mir::escape::EscapeAnalyzer;
use bloodc::mir::types::{
    AggregateKind, Constant, ConstantKind, Operand, Place, Rvalue, Statement, StatementKind,
    Terminator, TerminatorKind,
};
use bloodc::span::Span;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

// ============================================================================
// Test Program Generators (Real MIR)
// ============================================================================

fn dummy_def_id() -> DefId {
    DefId::new(0)
}

/// Generate a MIR body with local computations (high stack promotion potential).
fn generate_local_computation(num_locals: usize) -> MirBody {
    let mut body = MirBody::new(dummy_def_id(), Span::dummy());

    // Return place
    body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

    // Create locals
    let mut temps = Vec::with_capacity(num_locals);
    for _ in 0..num_locals {
        let local = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        temps.push(local);
    }

    let bb = body.new_block();

    // Chain of assignments: each local assigned from previous
    for i in 1..temps.len() {
        body.push_statement(
            bb,
            Statement::new(
                StatementKind::Assign(
                    Place::local(temps[i]),
                    Rvalue::Use(Operand::Copy(Place::local(temps[i - 1]))),
                ),
                Span::dummy(),
            ),
        );
    }

    body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));
    body
}

/// Generate a MIR body with function calls (values escape via arguments).
fn generate_call_heavy(num_locals: usize, num_calls: usize) -> MirBody {
    let mut body = MirBody::new(dummy_def_id(), Span::dummy());

    body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

    let mut temps = Vec::with_capacity(num_locals);
    for _ in 0..num_locals {
        let local = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        temps.push(local);
    }

    let bb = body.new_block();

    // Make calls that cause values to escape
    for i in 0..num_calls {
        let arg_idx = i % temps.len().max(1);
        body.set_terminator(
            bb,
            Terminator::new(
                TerminatorKind::Call {
                    func: Operand::Constant(Constant::new(
                        Type::unit(),
                        ConstantKind::FnDef(DefId::new(100)),
                    )),
                    args: vec![Operand::Copy(Place::local(temps[arg_idx]))],
                    destination: Place::local(LocalId::new(0)),
                    target: None,
                    unwind: None,
                },
                Span::dummy(),
            ),
        );
    }

    body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));
    body
}

/// Generate a MIR body with closures that capture values.
fn generate_closure_heavy(num_locals: usize, num_closures: usize) -> MirBody {
    let mut body = MirBody::new(dummy_def_id(), Span::dummy());

    body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());

    // Create locals to be captured
    let mut captured = Vec::with_capacity(num_locals);
    for _ in 0..num_locals {
        let local = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        captured.push(local);
    }

    // Create closures
    let mut closures = Vec::with_capacity(num_closures);
    for _ in 0..num_closures {
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
        closures.push(closure);
    }

    let bb = body.new_block();

    // Each closure captures some locals
    for (i, &closure) in closures.iter().enumerate() {
        let captures: Vec<_> = (0..3.min(captured.len()))
            .map(|j| Operand::Copy(Place::local(captured[(i + j) % captured.len()])))
            .collect();

        body.push_statement(
            bb,
            Statement::new(
                StatementKind::Assign(
                    Place::local(closure),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Closure {
                            def_id: DefId::new(100 + i as u32),
                        },
                        operands: captures,
                    },
                ),
                Span::dummy(),
            ),
        );

        // Some closures escape via return
        if i % 3 == 0 {
            body.push_statement(
                bb,
                Statement::new(
                    StatementKind::Assign(
                        Place::local(LocalId::new(0)),
                        Rvalue::Use(Operand::Copy(Place::local(closure))),
                    ),
                    Span::dummy(),
                ),
            );
        }
    }

    body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));
    body
}

/// Generate a closure chain where each closure captures the previous one.
///
/// This is the pathological case that was O(n²) before PERF-IMPL-001.
/// With the worklist algorithm, it should be O(n).
fn generate_closure_chain(chain_length: usize) -> MirBody {
    let mut body = MirBody::new(dummy_def_id(), Span::dummy());

    body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());

    // Create closures
    let mut closures = Vec::with_capacity(chain_length);
    for _ in 0..chain_length {
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
        closures.push(closure);
    }

    let bb = body.new_block();

    // First closure captures a dummy local
    let captured = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
    body.push_statement(
        bb,
        Statement::new(
            StatementKind::Assign(
                Place::local(closures[0]),
                Rvalue::Aggregate {
                    kind: AggregateKind::Closure {
                        def_id: DefId::new(100),
                    },
                    operands: vec![Operand::Copy(Place::local(captured))],
                },
            ),
            Span::dummy(),
        ),
    );

    // Each subsequent closure captures the previous one
    for i in 1..chain_length {
        body.push_statement(
            bb,
            Statement::new(
                StatementKind::Assign(
                    Place::local(closures[i]),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Closure {
                            def_id: DefId::new(100 + i as u32),
                        },
                        operands: vec![Operand::Copy(Place::local(closures[i - 1]))],
                    },
                ),
                Span::dummy(),
            ),
        );
    }

    // Make last closure escape (return it)
    body.push_statement(
        bb,
        Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Copy(Place::local(closures[chain_length - 1]))),
            ),
            Span::dummy(),
        ),
    );

    body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));
    body
}

/// Generate a mixed workload representative of real functions.
fn generate_mixed_workload(num_locals: usize) -> MirBody {
    let mut body = MirBody::new(dummy_def_id(), Span::dummy());

    body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

    // 60% local computations
    let local_count = (num_locals * 60) / 100;
    let mut temps = Vec::with_capacity(local_count);
    for _ in 0..local_count {
        let local = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        temps.push(local);
    }

    // 20% closures
    let closure_count = (num_locals * 20) / 100;
    let mut closures = Vec::with_capacity(closure_count);
    for _ in 0..closure_count {
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
        closures.push(closure);
    }

    let bb = body.new_block();

    // Local assignments
    for i in 1..temps.len() {
        body.push_statement(
            bb,
            Statement::new(
                StatementKind::Assign(
                    Place::local(temps[i]),
                    Rvalue::Use(Operand::Copy(Place::local(temps[i - 1]))),
                ),
                Span::dummy(),
            ),
        );
    }

    // Closure creations
    for (i, &closure) in closures.iter().enumerate() {
        if !temps.is_empty() {
            body.push_statement(
                bb,
                Statement::new(
                    StatementKind::Assign(
                        Place::local(closure),
                        Rvalue::Aggregate {
                            kind: AggregateKind::Closure {
                                def_id: DefId::new(100 + i as u32),
                            },
                            operands: vec![Operand::Copy(Place::local(temps[i % temps.len()]))],
                        },
                    ),
                    Span::dummy(),
                ),
            );
        }
    }

    body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));
    body
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_escape_analysis_speed(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_analysis_speed");

    for size in [10, 50, 100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("local_computation", size),
            size,
            |b, &size| {
                let body = generate_local_computation(size);
                b.iter(|| {
                    let mut analyzer = EscapeAnalyzer::new();
                    black_box(analyzer.analyze(&body))
                });
            },
        );

        group.bench_with_input(BenchmarkId::new("call_heavy", size), size, |b, &size| {
            let body = generate_call_heavy(size, size / 5);
            b.iter(|| {
                let mut analyzer = EscapeAnalyzer::new();
                black_box(analyzer.analyze(&body))
            });
        });

        group.bench_with_input(BenchmarkId::new("closure_heavy", size), size, |b, &size| {
            let body = generate_closure_heavy(size, size / 5);
            b.iter(|| {
                let mut analyzer = EscapeAnalyzer::new();
                black_box(analyzer.analyze(&body))
            });
        });

        group.bench_with_input(BenchmarkId::new("mixed_workload", size), size, |b, &size| {
            let body = generate_mixed_workload(size);
            b.iter(|| {
                let mut analyzer = EscapeAnalyzer::new();
                black_box(analyzer.analyze(&body))
            });
        });
    }

    group.finish();
}

fn bench_stack_promotion_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("stack_promotion_effectiveness");

    for size in [100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("local_computation", size),
            size,
            |b, &size| {
                let body = generate_local_computation(size);
                b.iter(|| {
                    let mut analyzer = EscapeAnalyzer::new();
                    let results = analyzer.analyze(&body);
                    let rate = results.stack_promotable.len() as f64 / results.states.len() as f64;
                    black_box(rate)
                });
            },
        );

        group.bench_with_input(BenchmarkId::new("mixed_workload", size), size, |b, &size| {
            let body = generate_mixed_workload(size);
            b.iter(|| {
                let mut analyzer = EscapeAnalyzer::new();
                let results = analyzer.analyze(&body);
                let rate = results.stack_promotable.len() as f64 / results.states.len() as f64;
                black_box(rate)
            });
        });
    }

    group.finish();
}

/// Benchmark closure chain propagation (PERF-IMPL-001).
///
/// This is the key benchmark for validating the worklist optimization.
/// Before PERF-IMPL-001: O(n²) - 50→83µs, 100→307µs, 500→6.3ms
/// After PERF-IMPL-001: O(n) - should show linear scaling
fn bench_closure_chain_convergence(c: &mut Criterion) {
    let mut group = c.benchmark_group("closure_chain_convergence");

    for size in [50, 100, 200, 500].iter() {
        group.bench_with_input(BenchmarkId::new("closure_chains", size), size, |b, &size| {
            let body = generate_closure_chain(size);
            b.iter(|| {
                let mut analyzer = EscapeAnalyzer::new();
                black_box(analyzer.analyze(&body))
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_escape_analysis_speed,
    bench_stack_promotion_effectiveness,
    bench_closure_chain_convergence,
);
criterion_main!(benches);
