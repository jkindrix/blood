// The Computer Language Benchmarks Game
// https://salsa.debian.org/benchmarksgame-team/benchmarksgame/
//
// contributed by the Rust Project Developers

fn a(i: usize, j: usize) -> f64 {
    ((i + j) * (i + j + 1) / 2 + i + 1) as f64
}

fn av(n: usize, v: &[f64], out: &mut [f64]) {
    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..n {
            sum += v[j] / a(i, j);
        }
        out[i] = sum;
    }
}

fn atv(n: usize, v: &[f64], out: &mut [f64]) {
    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..n {
            sum += v[j] / a(j, i);
        }
        out[i] = sum;
    }
}

fn atav(n: usize, v: &[f64], out: &mut [f64], tmp: &mut [f64]) {
    av(n, v, tmp);
    atv(n, tmp, out);
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut u = vec![1.0; n];
    let mut v = vec![0.0; n];
    let mut tmp = vec![0.0; n];

    for _ in 0..10 {
        atav(n, &u, &mut v, &mut tmp);
        atav(n, &v, &mut u, &mut tmp);
    }

    let mut vbv = 0.0;
    let mut vv = 0.0;
    for i in 0..n {
        vbv += u[i] * v[i];
        vv += v[i] * v[i];
    }

    println!("{:.9}", (vbv / vv).sqrt());
}
