//! Parser benchmarks using criterion.
//!
//! Run with: cargo bench --bench parser_bench

use bloodc::Parser;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

/// A comprehensive Blood source file for benchmarking.
const HELLO_BLOOD: &str = include_str!("../../examples/hello.blood");

/// A simple function for micro-benchmarking.
const SIMPLE_FUNCTION: &str = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;

/// Complex expressions for expression parsing benchmarking.
const COMPLEX_EXPR: &str = r#"
fn complex() {
    let x = 1 + 2 * 3 - 4 / 5 % 6;
    let y = a && b || c && d || e;
    let z = foo.bar().baz(1, 2, 3).qux;
    let w = data |> transform |> filter |> collect;
}
"#;

/// Pattern matching for pattern parsing benchmarking.
const PATTERN_HEAVY: &str = r#"
fn patterns(x: Option<(i32, String)>) -> i32 {
    match x {
        Option::Some((n, s)) if n > 0 => n,
        Option::Some((0, _)) => 0,
        Option::Some((n, ref s)) => -n,
        Option::None => -1,
    }
}
"#;

/// Effect declarations for effect system benchmarking.
const EFFECT_DECLS: &str = r#"
effect Console {
    op print(msg: String) -> unit;
    op read_line() -> String;
}

effect State<S> {
    op get() -> S;
    op put(s: S) -> unit;
}

deep handler LocalState<S> for State<S> {
    let mut state: S

    return(x) { x }

    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
}
"#;

/// Type declarations for type parsing benchmarking.
const TYPE_DECLS: &str = r#"
struct Point { x: i32, y: i32 }
struct GenericBox<T> { value: T }
enum Result<T, E> { Ok(T), Err(E) }
enum List<T> { Nil, Cons(T, Box<List<T>>) }
type IntPair = (i32, i32);
type Callback<T> = fn(T) -> bool;
"#;

fn parse_program(source: &str) -> Result<bloodc::ast::Program, String> {
    let mut parser = Parser::new(source);
    parser.parse_program().map_err(|e| format!("{:?}", e))
}

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    // Benchmark hello.blood (comprehensive)
    group.throughput(Throughput::Bytes(HELLO_BLOOD.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("hello.blood", HELLO_BLOOD.len()),
        HELLO_BLOOD,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    // Benchmark simple function
    group.throughput(Throughput::Bytes(SIMPLE_FUNCTION.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("simple_function", SIMPLE_FUNCTION.len()),
        SIMPLE_FUNCTION,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    // Benchmark complex expressions
    group.throughput(Throughput::Bytes(COMPLEX_EXPR.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("complex_expr", COMPLEX_EXPR.len()),
        COMPLEX_EXPR,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    // Benchmark pattern matching
    group.throughput(Throughput::Bytes(PATTERN_HEAVY.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("pattern_heavy", PATTERN_HEAVY.len()),
        PATTERN_HEAVY,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    // Benchmark effect declarations
    group.throughput(Throughput::Bytes(EFFECT_DECLS.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("effect_decls", EFFECT_DECLS.len()),
        EFFECT_DECLS,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    // Benchmark type declarations
    group.throughput(Throughput::Bytes(TYPE_DECLS.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("type_decls", TYPE_DECLS.len()),
        TYPE_DECLS,
        |b, source| {
            b.iter(|| parse_program(black_box(source)));
        },
    );

    group.finish();
}

fn bench_parser_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_scaling");

    // Test scaling with input size (limited to avoid OOM)
    for size in [1, 2, 4] {
        let source = HELLO_BLOOD.repeat(size);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("hello_repeated", size),
            &source,
            |b, source| {
                b.iter(|| parse_program(black_box(source)));
            },
        );
    }

    group.finish();
}

fn bench_expression_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("expression_parsing");

    // Deeply nested arithmetic
    let nested_arith = "(((((1 + 2) * 3) - 4) / 5) % 6)";
    group.bench_function("nested_arithmetic", |b| {
        let source = format!("fn f() {{ {} }}", nested_arith);
        b.iter(|| parse_program(black_box(&source)));
    });

    // Long method chain
    let method_chain = "x.a().b().c().d().e().f().g().h().i().j()";
    group.bench_function("method_chain", |b| {
        let source = format!("fn f() {{ {} }}", method_chain);
        b.iter(|| parse_program(black_box(&source)));
    });

    // Binary operator chain
    let binary_chain = "a + b + c + d + e + f + g + h + i + j";
    group.bench_function("binary_chain", |b| {
        let source = format!("fn f() {{ {} }}", binary_chain);
        b.iter(|| parse_program(black_box(&source)));
    });

    group.finish();
}

criterion_group!(benches, bench_parser, bench_parser_scaling, bench_expression_parsing);
criterion_main!(benches);
