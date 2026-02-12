// The Computer Language Benchmarks Game
// https://salsa.debian.org/benchmarksgame-team/benchmarksgame/
//
// contributed by the Rust Project Developers

fn fannkuch(n: usize) -> (i32, i32) {
    let mut perm = (0..n).collect::<Vec<_>>();
    let mut perm1 = perm.clone();
    let mut count = vec![0; n];
    let mut max_flips = 0;
    let mut checksum = 0;
    let mut r = n;
    let mut nperm = 0;

    loop {
        while r != 1 {
            count[r - 1] = r;
            r -= 1;
        }

        perm.copy_from_slice(&perm1);

        let mut flips = 0;
        while perm[0] != 0 {
            let k = perm[0];
            perm[0..=k].reverse();
            flips += 1;
        }

        if flips > max_flips {
            max_flips = flips;
        }
        checksum += if nperm & 1 == 0 { flips } else { -flips };
        nperm += 1;

        loop {
            if r == n {
                return (checksum, max_flips);
            }
            let perm0 = perm1[0];
            for i in 0..r {
                perm1[i] = perm1[i + 1];
            }
            perm1[r] = perm0;
            count[r] -= 1;
            if count[r] > 0 {
                break;
            }
            r += 1;
        }
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);

    let (checksum, max_flips) = fannkuch(n);
    println!("{}", checksum);
    println!("Pfannkuchen({}) = {}", n, max_flips);
}
