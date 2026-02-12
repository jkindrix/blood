# Blood Compiler Benchmarks

This directory contains rigorous, reproducible benchmarks comparing Blood to C.

**Methodology**: See [METHODOLOGY.md](METHODOLOGY.md) for complete benchmarking standards.

## Quick Start

```bash
# Build Blood compiler
cargo build --release

# Run benchmarks (12 runs, proper statistics)
./run_benchmarks.sh

# Quick run (3 runs, for development)
./run_benchmarks.sh --quick
```

## Current Results (2026-01-14)

### N-Body (50M iterations, CLBG standard)

| Implementation | Mean (s) | Std Dev | CV% | vs C |
|----------------|----------|---------|-----|------|
| Blood | 1.724 | 0.010 | 0.6% | 0.86x |
| C (gcc -O3) | 2.002 | 0.005 | 0.2% | 1.00x |

**Blood is 14% faster than C on this benchmark.**

### Important Caveats

1. **Structural difference**: Blood uses unrolled function calls, C uses nested loops.
   This is a fair comparison (same algorithm, same computation) but different code structure.
   LLVM (Blood's backend) may optimize the unrolled code differently than GCC optimizes loops.

2. **Single benchmark**: This is one data point. More benchmarks needed for complete picture.

3. **Output format**: Blood outputs 6 significant digits, C outputs 9. Values are mathematically
   equivalent (error < 1e-6).

## Directory Structure

```
benchmarks/
├── README.md              # This file
├── METHODOLOGY.md         # Rigorous benchmarking standards
├── run_benchmarks.sh      # Main benchmark runner
├── tools/
│   └── fpdiff.py          # Floating-point comparison utility
├── clbg/
│   ├── blood/             # Blood benchmark source
│   │   └── nbody.blood    # CLBG n-body (50M iterations)
│   ├── c/                 # C reference implementations
│   │   └── nbody.c        # CLBG n-body
│   ├── bin/               # Compiled binaries (generated)
│   └── expected/          # Reference output files
└── results/               # Timestamped benchmark results
```

## Adding Benchmarks

1. Create Blood source in `clbg/blood/<name>.blood`
2. Create C reference in `clbg/c/<name>.c`
3. Add to `run_benchmarks.sh`
4. Verify output matches between Blood and C
5. Document any structural differences

## What We Measure

Per METHODOLOGY.md:

- **12 runs** minimum, first discarded as warmup
- **Median** for comparison, with min/max/stddev
- **CV% < 20%** required for reliable results
- **Output validation** against C reference

## What We Don't Do

- Cherry-pick favorable results
- Hide benchmarks where Blood is slower
- Use different iteration counts for different languages
- Claim performance without proper statistics
