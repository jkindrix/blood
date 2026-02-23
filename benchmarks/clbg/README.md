# CLBG Benchmarks - Blood vs C

Computer Language Benchmarks Game implementations for Blood and C.

## Methodology

All benchmarks use **algorithmically equivalent implementations**:

- Same data structures (arrays, structs)
- Same control flow (loops, recursion)
- Same algorithms (no unrolling tricks in one language but not the other)

### Compilation

**Blood:**
```bash
blood build --release <benchmark>.blood
```

**C:**
```bash
gcc -O3 -march=native -fomit-frame-pointer <benchmark>.c -o <benchmark> -lm
```

## Benchmarks

| Benchmark | Blood Source | C Source | CLBG Size |
|-----------|--------------|----------|-----------|
| N-Body | `blood/nbody.blood` | `c/nbody.c` | N=50,000,000 |
| Fannkuch-Redux | `blood/fannkuchredux.blood` | `c/fannkuchredux_fixed.c` | N=12 |
| Binary-Trees | `blood/binarytrees.blood` | `c/binarytrees.c` | depth=21 |
| Spectral-Norm | `blood/spectralnorm.blood` | `c/spectralnorm.c` | N=5500 |

### Notes

- **Fannkuch-Redux**: C uses fixed N=12 (not VLA) for fair comparison with Blood's compile-time arrays
- **N-Body**: Blood uses array + loops (not unrolled), matching C algorithm exactly
- **Binary-Trees**: Both use malloc/free pattern; Blood's `alloc`/`free` maps to Rust allocator
- **Spectral-Norm**: Both use heap-allocated arrays with ptr_read/ptr_write (Blood) vs pointer arithmetic (C)

## Running Benchmarks

```bash
./run_benchmarks.sh
```

Or manually:

```bash
cd bin
/usr/bin/time -f "%e" ./nbody_blood
/usr/bin/time -f "%e" ./nbody_c 50000000

/usr/bin/time -f "%e" ./fannkuchredux_blood
/usr/bin/time -f "%e" ./fannkuchredux_c_fixed

/usr/bin/time -f "%e" ./binarytrees_blood 21
/usr/bin/time -f "%e" ./binarytrees_c 21

/usr/bin/time -f "%e" ./spectralnorm_blood 5500
/usr/bin/time -f "%e" ./spectralnorm_c 5500
```

## Results

See `results/latest.txt` for most recent benchmark run.

## Verification

All benchmarks produce output matching the CLBG reference:

```bash
./verify_output.sh
```
