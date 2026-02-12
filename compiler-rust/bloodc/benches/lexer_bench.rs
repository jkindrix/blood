//! Lexer benchmarks using criterion.
//!
//! Run with: cargo bench --bench lexer_bench

use bloodc::Lexer;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

/// A comprehensive Blood source file for benchmarking.
const HELLO_BLOOD: &str = include_str!("../../examples/hello.blood");

/// A simple expression for micro-benchmarking.
const SIMPLE_EXPR: &str = "1 + 2 * 3 - 4 / 5";

/// Keywords-heavy source for keyword recognition benchmarking.
const KEYWORDS_SOURCE: &str = r#"
fn struct enum trait impl effect handler perform resume with handle
let mut const static if else match while for loop break continue return
pub async unsafe pure linear affine true false self Self
"#;

/// Numeric literals for number parsing benchmarking.
const NUMERIC_SOURCE: &str = r#"
42 3.14159 0xFF 0b1010 0o777 1_000_000 3.14e-10 0xDEAD_BEEF
100 200 300 400 500 600 700 800 900 1000
1.0 2.0 3.0 4.0 5.0 6.0 7.0 8.0 9.0 10.0
"#;

/// String and comment source for string handling benchmarking.
const STRING_SOURCE: &str = r#"
"hello" "world" "escape\n\t\r" "unicode: \u{1F600}"
// line comment
/* block comment */
/* nested /* block */ comment */
"#;

fn lex_to_vec(source: &str) -> Vec<bloodc::TokenKind> {
    Lexer::new(source).map(|t| t.kind).collect()
}

fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer");

    // Benchmark hello.blood (comprehensive)
    group.throughput(Throughput::Bytes(HELLO_BLOOD.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("hello.blood", HELLO_BLOOD.len()),
        HELLO_BLOOD,
        |b, source| {
            b.iter(|| lex_to_vec(black_box(source)));
        },
    );

    // Benchmark simple expression
    group.throughput(Throughput::Bytes(SIMPLE_EXPR.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("simple_expr", SIMPLE_EXPR.len()),
        SIMPLE_EXPR,
        |b, source| {
            b.iter(|| lex_to_vec(black_box(source)));
        },
    );

    // Benchmark keywords
    group.throughput(Throughput::Bytes(KEYWORDS_SOURCE.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("keywords", KEYWORDS_SOURCE.len()),
        KEYWORDS_SOURCE,
        |b, source| {
            b.iter(|| lex_to_vec(black_box(source)));
        },
    );

    // Benchmark numeric literals
    group.throughput(Throughput::Bytes(NUMERIC_SOURCE.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("numeric", NUMERIC_SOURCE.len()),
        NUMERIC_SOURCE,
        |b, source| {
            b.iter(|| lex_to_vec(black_box(source)));
        },
    );

    // Benchmark strings and comments
    group.throughput(Throughput::Bytes(STRING_SOURCE.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("strings_comments", STRING_SOURCE.len()),
        STRING_SOURCE,
        |b, source| {
            b.iter(|| lex_to_vec(black_box(source)));
        },
    );

    group.finish();
}

fn bench_lexer_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer_scaling");

    // Test scaling with input size
    for size in [1, 2, 4, 8, 16] {
        let source = HELLO_BLOOD.repeat(size);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("hello_repeated", size),
            &source,
            |b, source| {
                b.iter(|| lex_to_vec(black_box(source)));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_lexer, bench_lexer_scaling);
criterion_main!(benches);
