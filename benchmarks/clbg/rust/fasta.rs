// The Computer Language Benchmarks Game
// https://salsa.debian.org/benchmarksgame-team/benchmarksgame/
//
// contributed by the Rust Project Developers

use std::io::{self, Write, BufWriter};

const IM: u32 = 139968;
const IA: u32 = 3877;
const IC: u32 = 29573;

const LINE_LENGTH: usize = 60;

static mut LAST: u32 = 42;

fn gen_random(max: f64) -> f64 {
    unsafe {
        LAST = (LAST * IA + IC) % IM;
        max * (LAST as f64) / (IM as f64)
    }
}

struct AminoAcid {
    c: u8,
    p: f64,
}

fn make_cumulative(genelist: &mut [AminoAcid]) {
    let mut cp = 0.0;
    for aa in genelist.iter_mut() {
        cp += aa.p;
        aa.p = cp;
    }
}

fn select_random(genelist: &[AminoAcid]) -> u8 {
    let r = gen_random(1.0);
    for aa in genelist {
        if r < aa.p {
            return aa.c;
        }
    }
    genelist.last().unwrap().c
}

fn make_random_fasta<W: Write>(
    out: &mut W,
    id: &str,
    desc: &str,
    genelist: &[AminoAcid],
    n: usize,
) -> io::Result<()> {
    writeln!(out, ">{} {}", id, desc)?;

    let mut remaining = n;
    let mut line = [0u8; LINE_LENGTH + 1];

    while remaining > 0 {
        let len = if remaining < LINE_LENGTH { remaining } else { LINE_LENGTH };
        for i in 0..len {
            line[i] = select_random(genelist);
        }
        line[len] = b'\n';
        out.write_all(&line[..len + 1])?;
        remaining -= len;
    }

    Ok(())
}

fn make_repeat_fasta<W: Write>(
    out: &mut W,
    id: &str,
    desc: &str,
    alu: &[u8],
    n: usize,
) -> io::Result<()> {
    writeln!(out, ">{} {}", id, desc)?;

    let mut remaining = n;
    let mut alu_pos = 0;
    let alu_len = alu.len();
    let mut line = [0u8; LINE_LENGTH + 1];

    while remaining > 0 {
        let len = if remaining < LINE_LENGTH { remaining } else { LINE_LENGTH };
        for i in 0..len {
            line[i] = alu[alu_pos];
            alu_pos = (alu_pos + 1) % alu_len;
        }
        line[len] = b'\n';
        out.write_all(&line[..len + 1])?;
        remaining -= len;
    }

    Ok(())
}

fn main() -> io::Result<()> {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let mut iub = vec![
        AminoAcid { c: b'a', p: 0.27 },
        AminoAcid { c: b'c', p: 0.12 },
        AminoAcid { c: b'g', p: 0.12 },
        AminoAcid { c: b't', p: 0.27 },
        AminoAcid { c: b'B', p: 0.02 },
        AminoAcid { c: b'D', p: 0.02 },
        AminoAcid { c: b'H', p: 0.02 },
        AminoAcid { c: b'K', p: 0.02 },
        AminoAcid { c: b'M', p: 0.02 },
        AminoAcid { c: b'N', p: 0.02 },
        AminoAcid { c: b'R', p: 0.02 },
        AminoAcid { c: b'S', p: 0.02 },
        AminoAcid { c: b'V', p: 0.02 },
        AminoAcid { c: b'W', p: 0.02 },
        AminoAcid { c: b'Y', p: 0.02 },
    ];

    let mut homosapiens = vec![
        AminoAcid { c: b'a', p: 0.3029549426680 },
        AminoAcid { c: b'c', p: 0.1979883004921 },
        AminoAcid { c: b'g', p: 0.1975473066391 },
        AminoAcid { c: b't', p: 0.3015094502008 },
    ];

    let alu = b"GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGG\
GAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGA\
CCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAAT\
ACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCA\
GCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGG\
AGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCC\
AGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA";

    make_cumulative(&mut iub);
    make_cumulative(&mut homosapiens);

    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    make_repeat_fasta(&mut out, "ONE", "Homo sapiens alu", alu, n * 2)?;
    make_random_fasta(&mut out, "TWO", "IUB ambiguity codes", &iub, n * 3)?;
    make_random_fasta(&mut out, "THREE", "Homo sapiens frequency", &homosapiens, n * 5)?;

    Ok(())
}
