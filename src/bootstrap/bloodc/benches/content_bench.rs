//! Content addressing benchmarks using criterion.
//!
//! Benchmarks for content hash computation, build cache operations,
//! and incremental compilation performance.
//!
//! Run with: cargo bench --bench content_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

/// Simple HIR-like structures for benchmarking hash computation
mod mock_hir {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    #[derive(Clone)]
    pub struct Function {
        pub name: String,
        pub params: Vec<(String, Type)>,
        pub return_type: Type,
        pub body: Vec<Statement>,
    }

    #[derive(Clone)]
    pub enum Type {
        Int,
        Float,
        Bool,
        String,
        Custom(String),
        Array(Box<Type>),
        Tuple(Vec<Type>),
    }

    #[derive(Clone)]
    pub enum Statement {
        Let { name: String, value: Expr },
        Return(Expr),
        If { cond: Expr, then: Vec<Statement>, else_: Vec<Statement> },
    }

    #[derive(Clone)]
    pub enum Expr {
        Int(i64),
        Float(f64),
        Bool(bool),
        Var(String),
        Binary { op: &'static str, left: Box<Expr>, right: Box<Expr> },
        Call { name: String, args: Vec<Expr> },
    }

    impl Function {
        pub fn simple(name: &str) -> Self {
            Self {
                name: name.to_string(),
                params: vec![("x".into(), Type::Int)],
                return_type: Type::Int,
                body: vec![Statement::Return(Expr::Binary {
                    op: "+",
                    left: Box::new(Expr::Var("x".into())),
                    right: Box::new(Expr::Int(1)),
                })],
            }
        }

        pub fn complex(name: &str, depth: usize) -> Self {
            let mut stmts = Vec::new();
            for i in 0..depth {
                stmts.push(Statement::Let {
                    name: format!("v{}", i),
                    value: Expr::Binary {
                        op: "+",
                        left: Box::new(Expr::Var(if i == 0 { "x".into() } else { format!("v{}", i - 1) })),
                        right: Box::new(Expr::Int(i as i64)),
                    },
                });
            }
            stmts.push(Statement::Return(Expr::Var(format!("v{}", depth - 1))));

            Self {
                name: name.to_string(),
                params: vec![("x".into(), Type::Int)],
                return_type: Type::Int,
                body: stmts,
            }
        }
    }

    /// Compute a hash of a function (simulating content hashing)
    pub fn hash_function(func: &Function) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash name
        func.name.hash(&mut hasher);

        // Hash params
        for (name, ty) in &func.params {
            name.hash(&mut hasher);
            hash_type(ty, &mut hasher);
        }

        // Hash return type
        hash_type(&func.return_type, &mut hasher);

        // Hash body
        for stmt in &func.body {
            hash_statement(stmt, &mut hasher);
        }

        hasher.finish()
    }

    fn hash_type(ty: &Type, h: &mut impl Hasher) {
        match ty {
            Type::Int => 0u8.hash(h),
            Type::Float => 1u8.hash(h),
            Type::Bool => 2u8.hash(h),
            Type::String => 3u8.hash(h),
            Type::Custom(name) => { 4u8.hash(h); name.hash(h); }
            Type::Array(inner) => { 5u8.hash(h); hash_type(inner, h); }
            Type::Tuple(types) => { 6u8.hash(h); for t in types { hash_type(t, h); } }
        }
    }

    fn hash_statement(stmt: &Statement, h: &mut impl Hasher) {
        match stmt {
            Statement::Let { name, value } => {
                0u8.hash(h);
                name.hash(h);
                hash_expr(value, h);
            }
            Statement::Return(expr) => {
                1u8.hash(h);
                hash_expr(expr, h);
            }
            Statement::If { cond, then, else_ } => {
                2u8.hash(h);
                hash_expr(cond, h);
                for s in then { hash_statement(s, h); }
                for s in else_ { hash_statement(s, h); }
            }
        }
    }

    fn hash_expr(expr: &Expr, h: &mut impl Hasher) {
        match expr {
            Expr::Int(v) => { 0u8.hash(h); v.hash(h); }
            Expr::Float(v) => { 1u8.hash(h); v.to_bits().hash(h); }
            Expr::Bool(v) => { 2u8.hash(h); v.hash(h); }
            Expr::Var(name) => { 3u8.hash(h); name.hash(h); }
            Expr::Binary { op, left, right } => {
                4u8.hash(h);
                op.hash(h);
                hash_expr(left, h);
                hash_expr(right, h);
            }
            Expr::Call { name, args } => {
                5u8.hash(h);
                name.hash(h);
                for arg in args { hash_expr(arg, h); }
            }
        }
    }
}

/// Benchmark blake3 hash computation on various input sizes
fn bench_blake3_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_hash");

    for size in [64, 256, 1024, 4096, 16384] {
        let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("bytes", size),
            &data,
            |b, data| {
                b.iter(|| {
                    black_box(blake3::hash(data))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark content hash computation on HIR-like structures
fn bench_hir_hash(c: &mut Criterion) {
    use mock_hir::*;

    let mut group = c.benchmark_group("hir_hash");

    // Simple function
    group.bench_function("simple_function", |b| {
        let func = Function::simple("add_one");
        b.iter(|| {
            black_box(hash_function(&func))
        });
    });

    // Complex function with varying depths
    for depth in [5, 10, 20, 50] {
        group.bench_with_input(
            BenchmarkId::new("complex_function", depth),
            &depth,
            |b, &depth| {
                let func = Function::complex("compute", depth);
                b.iter(|| {
                    black_box(hash_function(&func))
                });
            },
        );
    }

    // Batch hash computation
    for count in [10, 50, 100] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("batch", count),
            &count,
            |b, &count| {
                let funcs: Vec<_> = (0..count)
                    .map(|i| Function::simple(&format!("fn_{}", i)))
                    .collect();
                b.iter(|| {
                    for func in &funcs {
                        black_box(hash_function(func));
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark hash comparison operations
fn bench_hash_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_comparison");

    // Generate some hashes to compare
    let hashes: Vec<blake3::Hash> = (0..1000)
        .map(|i| blake3::hash(format!("item_{}", i).as_bytes()))
        .collect();

    // Single hash comparison
    group.bench_function("hash_eq", |b| {
        let h1 = hashes[0];
        let h2 = hashes[1];
        let h3 = hashes[0]; // Same as h1
        b.iter(|| {
            black_box(h1 == h2);
            black_box(h1 == h3);
        });
    });

    // Hash lookup in set
    group.bench_function("hash_lookup", |b| {
        use std::collections::HashSet;
        let set: HashSet<_> = hashes.iter().cloned().collect();
        let target = hashes[500];
        b.iter(|| {
            black_box(set.contains(&target))
        });
    });

    // Hash to hex string
    group.bench_function("hash_to_hex", |b| {
        let hash = hashes[0];
        b.iter(|| {
            black_box(hash.to_hex())
        });
    });

    group.finish();
}

/// Benchmark cache-like operations
fn bench_cache_operations(c: &mut Criterion) {
    use std::collections::HashMap;

    let mut group = c.benchmark_group("cache_operations");

    // Build a cache-like structure
    let hashes: Vec<(blake3::Hash, Vec<u8>)> = (0..1000)
        .map(|i| {
            let key = blake3::hash(format!("item_{}", i).as_bytes());
            let value: Vec<u8> = (0..1024).map(|j| ((i + j) % 256) as u8).collect();
            (key, value)
        })
        .collect();

    // Cache insertion
    group.bench_function("cache_insert", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for i in 0..iters {
                let mut cache: HashMap<blake3::Hash, Vec<u8>> = HashMap::new();
                let idx = (i % 1000) as usize;
                cache.insert(hashes[idx].0, hashes[idx].1.clone());
                black_box(cache);
            }
            start.elapsed()
        });
    });

    // Cache lookup (hit)
    group.bench_function("cache_lookup_hit", |b| {
        let cache: HashMap<_, _> = hashes.iter().cloned().collect();
        let target = hashes[500].0;
        b.iter(|| {
            black_box(cache.get(&target))
        });
    });

    // Cache lookup (miss)
    group.bench_function("cache_lookup_miss", |b| {
        let cache: HashMap<_, _> = hashes.iter().cloned().collect();
        let target = blake3::hash(b"nonexistent");
        b.iter(|| {
            black_box(cache.get(&target))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_blake3_hash,
    bench_hir_hash,
    bench_hash_comparison,
    bench_cache_operations,
);
criterion_main!(benches);
