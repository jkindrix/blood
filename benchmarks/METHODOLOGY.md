# Blood Compiler Benchmarking Methodology

## Purpose

This document establishes rigorous, honest, and reproducible benchmarking standards for the Blood compiler. The goal is truth, not marketing. We benchmark to identify weaknesses and guide improvements, not to cherry-pick favorable results.

**Foundational Principle**: Lies and deceit benefit nobody. We cannot improve what we do not honestly measure.

## Principles

### 1. Honesty Over Optics

- **Show all results**, including unfavorable ones
- **Never cherry-pick** benchmarks where Blood performs well while hiding poor results
- **Acknowledge limitations** explicitly
- **Document methodology** completely so results can be scrutinized
- **Include absolute numbers**, not just ratios (ratios alone prevent sanity checks)

### 2. Fair Comparison

- **Same algorithm**: Implementations must use identical algorithms, not just produce the same output
- **Same input size**: All implementations must process the same workload
- **Same optimization level**: Compare release builds to release builds
- **Equivalent code structure**: Don't compare unrolled code to loops unless explicitly testing that difference
- **Equal optimization effort**: Competitors' code must receive comparable tuning to our own

### 3. Reproducibility

- **Document everything**: Compiler versions, flags, system specs, methodology
- **Provide source code**: All benchmark source must be in the repository
- **Automated runners**: Scripts that anyone can execute to reproduce results
- **Version control results**: Track benchmark results over time
- **Sufficient detail**: An independent party must be able to reproduce results exactly

## Standards (Based on CLBG)

We follow the [Computer Language Benchmarks Game](https://benchmarksgame-team.pages.debian.net/benchmarksgame/) methodology where applicable.

### Input Sizes

| Benchmark | Standard Input | Notes |
|-----------|----------------|-------|
| n-body | 50,000,000 | Iterations/timesteps |
| binary-trees | 21 | Tree depth |
| fannkuch-redux | 12 | Permutation size |
| spectral-norm | 5500 | Matrix size |
| fasta | 25,000,000 | Output length |

These are the CLBG standard sizes. Using smaller inputs invalidates comparisons to published results.

## System Isolation Protocol

Before any benchmark run, the system must be properly isolated to minimize measurement noise.

### Required Isolation Steps

1. **Network isolation**: Disconnect host from all networks
   ```bash
   # Disable network interfaces (or physically disconnect)
   sudo ip link set eth0 down
   ```

2. **Process isolation**: No user applications running
   - Close all browsers, editors, IDEs
   - Verify with `htop` or `ps aux`

3. **Service isolation**: Disable background services
   ```bash
   # Stop non-essential services
   sudo systemctl stop cron
   sudo systemctl stop unattended-upgrades
   sudo systemctl stop snapd
   # List and disable others as needed
   ```

4. **Cache clearing**: Clear filesystem and memory caches
   ```bash
   sync
   echo 3 | sudo tee /proc/sys/vm/drop_caches
   sudo swapoff -a && sudo swapon -a
   ```

5. **CPU governor**: Set to performance mode
   ```bash
   sudo cpupower frequency-set -g performance
   # Verify: cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
   ```

6. **Turbo boost**: Disable for reproducibility

   Turbo boost creates variance even with performance governor, as frequency depends on thermal headroom and power budget. Disable it for consistent measurements:
   ```bash
   # AMD systems
   echo 0 | sudo tee /sys/devices/system/cpu/cpufreq/boost

   # Intel systems (intel_pstate driver)
   echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

   # Verify turbo is disabled
   cat /sys/devices/system/cpu/cpufreq/boost  # Should show 0
   # or
   cat /sys/devices/system/cpu/intel_pstate/no_turbo  # Should show 1
   ```

   **Note**: Document whether turbo was enabled or disabled. Results with turbo enabled are valid but may show higher variance.

7. **CPU affinity/pinning**: Pin benchmarks to specific cores

   CPU pinning significantly reduces variance by avoiding scheduler migrations and ensuring consistent cache behavior:
   ```bash
   # Pin to a single physical core (avoid hyperthreading siblings)
   taskset -c 0 ./benchmark

   # For NUMA systems, also bind memory
   numactl --cpunodebind=0 --membind=0 ./benchmark

   # With hyperfine
   hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' \
             'taskset -c 0 ./benchmark'
   ```

   **Core selection**: Use `lscpu -e` to identify physical cores vs hyperthreading siblings. Pin to physical cores only.

   **Document**: Always document which core(s) were used and whether pinning was applied.

8. **Thermal management**: Ensure adequate cooling
   - Monitor CPU temperature during runs (use `sensors` or similar)
   - Target temperatures: idle 35-45°C, load <85°C
   - Document if thermal throttling is detected (frequency drops when temp exceeds TJ Max, typically 95-100°C)
   - Allow CPU to cool between benchmark suites if needed
   - **Critical**: A thermal throttling event invalidates that measurement run

9. **ASLR (Address Space Layout Randomization)**: Control for reproducibility

   ASLR randomizes memory layout, which can affect cache behavior and cause measurement variance. Two approaches:

   **Option A**: Disable ASLR for maximum reproducibility (re-enable after!)
   ```bash
   # Disable system-wide (requires root, affects all processes)
   echo 0 | sudo tee /proc/sys/kernel/randomize_va_space

   # Re-enable after benchmarking!
   echo 2 | sudo tee /proc/sys/kernel/randomize_va_space
   ```

   **Option B**: Disable per-process (preferred, no system-wide impact)
   ```bash
   setarch $(uname -m) -R ./benchmark
   ```

   **Option C**: Vary environment size (CLBG approach)

   The CLBG varies environment variables between runs to detect cache layout sensitivity. If results vary significantly with ASLR on vs off, document this as a cache-sensitive benchmark.

   **Document**: Always state whether ASLR was enabled, disabled, or varied.

10. **Additional kernel tuning** (optional, for maximum isolation)
    ```bash
    # Disable NMI watchdog (reduces interrupts)
    echo 0 | sudo tee /proc/sys/kernel/nmi_watchdog

    # For extreme isolation, consider kernel parameters:
    # isolcpus=2,3 rcu_nocbs=2,3  (isolate cores 2,3 from scheduler)
    # nohz_full=2,3               (disable timer ticks on isolated cores)
    ```

    **Note**: These are optional and primarily useful for sub-millisecond measurements. Document if used.

### Verification

Before benchmarking, verify isolation:
```bash
# Should show minimal processes
ps aux | wc -l

# Should show near-zero CPU usage
top -bn1 | head -20

# Should show performance governor
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor

# Verify turbo boost status
cat /sys/devices/system/cpu/cpufreq/boost 2>/dev/null || \
cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null

# Check CPU temperature (should be at idle levels)
sensors | grep -i "core\|temp"

# Verify ASLR status
cat /proc/sys/kernel/randomize_va_space  # 0=off, 1=partial, 2=full
```

### Isolation Checklist

Before each benchmark session, confirm:

- [ ] Network disconnected
- [ ] Non-essential services stopped
- [ ] CPU governor set to `performance`
- [ ] Turbo boost disabled (or documented as enabled)
- [ ] CPU temperature at idle levels (<45°C)
- [ ] ASLR handling documented
- [ ] Filesystem caches cleared
- [ ] No other user processes running

## Measurement Protocol

### Tooling

**REQUIRED**: Use a proper benchmarking harness, not raw `time`.

**Primary recommendation**: [BenchExec](https://github.com/sosy-lab/benchexec)
- Uses Linux cgroups for process isolation
- Correctly handles multi-process programs
- Provides CPU time, wall time, and memory in one measurement
- Prevents interference from host system
- Used by the Computer Language Benchmarks Game

**Alternative**: [hyperfine](https://github.com/sharkdp/hyperfine)
```bash
hyperfine --warmup 1 --runs 12 --export-json results.json './blood_binary'
```

**DO NOT** use the shell `time` builtin alone—it lacks statistical rigor and memory reporting.

For memory measurement, use `/usr/bin/time -v` (not the builtin):
```bash
/usr/bin/time -v ./blood_binary 2>&1 | grep "Maximum resident set size"
```

### Execution Protocol

1. **System Preparation**
   - Complete all isolation steps above
   - Clear filesystem caches before each benchmark suite
   - Document CPU model, memory, OS version, kernel version

2. **Warmup**
   - Run benchmark once, discard result
   - This warms filesystem caches, CPU caches, branch predictors
   - For Blood (AOT-compiled), one warmup run is sufficient

3. **Measurement Runs**
   - Minimum 12 runs per benchmark (11 measured after warmup discard)
   - For high-variance benchmarks, increase to 20+ runs
   - Run consecutively without system changes between runs

4. **Validation**
   - Output must match reference exactly
   - Use `diff` for exact match
   - Use `ndiff` for floating-point comparison (see Output Validation section below)
   - A benchmark that produces wrong output is a **failed** benchmark, not a fast one

### Output Validation

Correct output is non-negotiable. A fast but wrong result is worthless.

**For exact text output**:
```bash
diff expected_output.txt actual_output.txt
# Exit code 0 = match, non-zero = mismatch
```

**For floating-point output**:

Use `ndiff` (from [NIST](https://www.itl.nist.gov/div898/strd/nls/nls_main.shtml) or available in many package managers):
```bash
ndiff -abserr 1.0e-8 -relerr 1.0e-8 expected.txt actual.txt
```

Parameters explained:
- `-abserr 1.0e-8`: Absolute error tolerance. Numbers differing by less than 10⁻⁸ are considered equal.
- `-relerr 1.0e-8`: Relative error tolerance. Numbers differing by less than 10⁻⁸ × |expected| are considered equal.

These tolerances match CLBG standards and account for:
- Different FP rounding modes between compilers
- Non-associative FP arithmetic (order of operations matters)
- Platform-specific FP implementations

**For JSON output**:
```bash
jq --sort-keys . expected.json > /tmp/expected_sorted.json
jq --sort-keys . actual.json > /tmp/actual_sorted.json
diff /tmp/expected_sorted.json /tmp/actual_sorted.json
```

**Validation failure protocol**:
1. If output differs, the benchmark **fails** regardless of performance
2. Investigate the difference before re-running
3. Document any expected differences (e.g., different whitespace) and justify why they don't affect correctness

### Warmup Protocol by Language Type

| Language Type | Warmup Requirement | Rationale |
|---------------|-------------------|-----------|
| AOT (Blood, C, Rust) | 1 run | Filesystem/CPU cache warming |
| JIT (Java, JavaScript) | 100-3000 iterations | JIT compilation threshold |
| Interpreted | 1 run | Cache warming only |

**JIT warmup methodology** (per [Marr et al., "Are We Fast Yet?" DLS 2016](https://stefan-marr.de/papers/dls-marr-et-al-cross-language-compiler-benchmarking-are-we-fast-yet/)):

JIT compilers require in-process warmup to reach peak performance. The number of iterations depends on the JIT's compilation threshold:

| Runtime | Recommended Warmup Iterations |
|---------|------------------------------|
| V8 (JavaScript) | 1000+ |
| JVM (HotSpot) | 10,000+ (tiered compilation) |
| GraalVM | 3000+ |
| PyPy | 1000+ |
| JRuby | 250+ |
| MRI Ruby | 100+ (no JIT, cache warming only) |

**Protocol for JIT comparisons**:
1. Run benchmark in a loop **within the same process**
2. Discard first N iterations (warmup)
3. Measure subsequent iterations
4. Report both warmup time and steady-state time

**Critical**: External process restarts reset JIT state. For fair JIT comparison, all iterations must occur in a single process invocation.

## Statistical Requirements

### Primary Metric: Median

Use **median** for comparing typical performance. The median correctly ignores outliers like GC pauses, kernel interrupts, and thermal events.

**Mean** is reserved for:
- Capacity planning (total throughput calculations)
- Little's Law applications (L = λ × W requires mean)

### Required Statistics

For each benchmark, report:

| Statistic | Description | Purpose |
|-----------|-------------|---------|
| Median | Middle value of sorted runs | Primary comparison metric |
| Mean | Arithmetic average | Throughput calculations |
| Min | Fastest run | Best-case capability |
| Max | Slowest run | Worst-case bound |
| Std Dev | Standard deviation | Spread indicator |
| CV% | (std_dev / mean) × 100 | Reliability indicator |

### Coefficient of Variation (CV%) Thresholds

CV% indicates measurement reliability. These thresholds align with industry standards and academic research:

| CV% Range | Reliability | Interpretation | Action |
|-----------|-------------|----------------|--------|
| < 10% | Excellent | Ratios trustworthy to 2 decimal places | Publish with confidence |
| 10-20% | Good | Rankings reliable, ratios approximate | Acceptable for publication |
| 20-30% | Acceptable | Directional conclusions only | Flag and document variance source |
| > 30% | Poor | Results dominated by noise | Investigate and re-run; do not publish |

**If CV% > 20%**: Flag results, investigate cause (thermal throttling? background process? GC? insufficient isolation?), and document the variance source before publishing.

**If CV% > 30%**: Results are unreliable. Do NOT publish. Investigate:
1. Was turbo boost disabled?
2. Was CPU pinning used?
3. Were there thermal throttling events?
4. Were background processes interfering?
5. Is the benchmark itself non-deterministic?

**Rule**: If you cannot get CV% < 20% after proper isolation, document why and consider whether the benchmark is appropriate.

### Cross-Benchmark Aggregation

When computing aggregate scores across multiple benchmarks:

**USE GEOMETRIC MEAN**, never arithmetic mean.

The arithmetic mean on normalized values is demonstrably incorrect (Fleming & Wallace, 1986). It allows manipulation through choice of reference baseline.

```
Example: Blood vs C ratios
- n-body:       0.95x (Blood is 5% slower)
- fannkuch:     1.10x (Blood is 10% faster)
- binary-trees: 0.80x (Blood is 20% slower)

CORRECT (Geometric mean):
  (0.95 × 1.10 × 0.80)^(1/3) = 0.936x

INCORRECT (Arithmetic mean):
  (0.95 + 1.10 + 0.80) / 3 = 0.950x  ← Misleading!
```

The geometric mean is invariant to normalization choice—the ranking stays the same regardless of which language is the reference.

**Important clarification**: The geometric mean requirement applies to **normalized scores** (ratios, speedups). For **raw times** measured in the same units:
- Arithmetic mean is acceptable
- Harmonic mean is appropriate for rates (ops/sec)
- Geometric mean is still valid but not required

**Counterargument acknowledgment** (for completeness): Some researchers argue that weighted arithmetic mean on raw times can be more rigorous for specific use cases, as "consistent results is not always equal to giving the correct results." However, for our use case (comparing normalized performance ratios across benchmarks), geometric mean is the mathematically correct choice and is the industry standard (used by SPEC, CLBG, etc.).

### Confidence Intervals

For rigorous comparison, report 95% confidence intervals:

**Calculating 95% CI** (assuming approximately normal distribution):
```
CI = mean ± (1.96 × std_dev / √n)
```

Where `n` is the number of measured runs (typically 11 after discarding warmup).

**Interpretation**:
- If confidence intervals of two implementations **overlap**, the difference may not be statistically significant
- If confidence intervals **don't overlap**, the difference is likely real
- For formal significance, use Student's t-test or Welch's t-test

**CLBG approach**: For long-running programs, CLBG reports "a 95% confidence interval from 11 CPU time measurements."

**When to use CI vs just median**:
- Always compute and record CI internally
- Report CI when differences are small (< 10%)
- For large differences (> 20%), median comparison is sufficient

### Statistical Significance

When claiming one implementation is faster than another:
- Difference should exceed measurement noise (CV%)
- For close results (< 5% difference), consider overlapping confidence intervals
- Report 95% confidence intervals when possible
- Consider Student's t-test for formal significance

**Rule of thumb**: If the difference is smaller than the CV%, it's not a meaningful difference.

## Baseline Selection

When claiming "Blood matches C performance," the C baseline must meet these criteria:

### Baseline Requirements

1. **Idiomatic code**: Written by a competent C programmer
   - Not Blood code transliterated to C
   - Uses appropriate C idioms and patterns
   - Would pass code review by experienced C developers

2. **Standard optimization**: Release-level flags only
   - GCC/Clang: `-O3 -march=native` (or specified architecture)
   - NOT hand-tuned assembly
   - NOT profile-guided optimization (unless Blood also uses it)
   - NOT link-time optimization (unless Blood also uses it)

3. **Standard libraries**: Use system libc
   - NOT custom SIMD implementations (unless Blood also has SIMD)
   - NOT specialized allocators (unless explicitly comparing allocators)

4. **Reasonable structure**: Representative of production code
   - NOT unrolled to absurd degrees
   - NOT micro-optimized beyond maintainability

### CLBG Baseline Selection

For CLBG comparisons, use the fastest implementation that matches Blood's capabilities:

| Blood Capability | Appropriate C Baseline |
|------------------|----------------------|
| Single-threaded | Fastest single-threaded C |
| No SIMD | Fastest non-SIMD C |
| With parallelism | Fastest parallel C (same thread count) |

**Document which baseline was chosen and why.**

### Unfair Baseline Examples

These baselines would be unfair for general claims:

- **Too weak**: Unoptimized (`-O0`) or debug builds
- **Too strong**: Hand-tuned assembly, SIMD intrinsics (if Blood lacks SIMD)
- **Wrong algorithm**: Different algorithmic approach
- **Wrong structure**: Comparing Blood structs to C with pointer chasing

## Algorithm Equivalence Checklist

For a fair comparison, implementations MUST match on:

### Must Match

| Aspect | Requirement |
|--------|-------------|
| Data structures | Same fundamental structures (array vs linked list matters) |
| Algorithm | Same core algorithm (quicksort vs mergesort vs heapsort) |
| Parallelism | Same thread/core count, or explicitly labeled |
| I/O approach | Same buffering strategy, same output format |
| Precision | Same floating-point precision (f32 vs f64) |

### May Differ

| Aspect | Acceptable Difference |
|--------|----------------------|
| Language idioms | Iterators vs manual loops (if semantically equivalent) |
| Standard library | Different implementations of same algorithm |
| Compiler optimizations | This is what we're measuring |
| Memory layout | Only if not performance-critical for benchmark |

### Must Document

Any structural difference must be explicitly documented with expected impact:

```markdown
**Structural Difference**: Blood uses explicit struct fields while C
uses array indexing. Expected impact: minimal for compute-bound
workloads, may affect cache behavior.
```

## Memory Measurement

### Metrics to Report

| Metric | How to Measure | Notes |
|--------|----------------|-------|
| Peak RSS | `/usr/bin/time -v` → "Maximum resident set size" | Total memory footprint |
| Peak heap | Allocator instrumentation (if available) | Heap-only allocation |

### Measurement Limitations

**Sampling rate caveat**: Memory measurement tools sample RSS at intervals (typically 0.2 seconds for BenchExec, varies for other tools). This means:

- **Programs running < 1 second**: Memory measurements may miss the true peak
- **Programs running < 0.2 seconds**: Memory measurements are essentially unreliable
- **Short-lived allocations**: May not be captured if freed between samples

**Recommendation**: For memory-sensitive benchmarks, ensure runtime exceeds 1 second. For sub-second benchmarks, either:
1. Increase input size to extend runtime
2. Use allocator instrumentation for precise heap tracking
3. Document that memory measurements are approximate

**RSS vs Heap**:
- **RSS** (Resident Set Size): Total physical memory in RAM, includes code, stack, heap, shared libraries
- **Heap**: Only dynamically allocated memory (malloc/new)

For comparing language implementations, RSS is the appropriate metric as it captures the full memory cost. Heap-only measurements miss stack usage and runtime overhead.

### Blood-Specific Memory Considerations

Document these memory characteristics in all reports:

1. **128-bit fat pointers**: Blood uses 16-byte pointers (vs 8-byte in C)
   - ~2x pointer memory overhead
   - Significant for pointer-heavy workloads (trees, linked structures)
   - Report expected overhead: `(pointer_count × 8 bytes) / total_memory`

2. **Runtime memory**: Blood runtime requires initialization memory
   - Document runtime overhead separately from benchmark memory

3. **Allocator overhead**: Blood's allocator may have different overhead
   - Particularly relevant for many-small-allocation benchmarks

### Memory Reporting Format

```markdown
| Benchmark | Blood RSS | C RSS | Blood/C | Pointer Overhead Est. |
|-----------|-----------|-------|---------|----------------------|
| binary-trees | 312 MB | 156 MB | 2.0x | ~98 MB (fat pointers) |
| n-body | 1.2 MB | 1.0 MB | 1.2x | ~0.1 MB |
```

## Known Limitations of Blood

Document these honestly in ALL benchmark reports:

1. **No CLI argument parsing**: Blood benchmarks use hardcoded sizes
   - Impact: Cannot run standard CLBG input sizes dynamically
   - Mitigation: Compile multiple versions or use compile-time constants

2. **128-bit fat pointers**: 16-byte pointers vs 8-byte
   - Impact: ~2x memory overhead for pointer-heavy structures
   - Impact: Reduced cache efficiency for pointer-chasing workloads
   - Benchmarks affected: binary-trees, linked structures

3. **Runtime initialization**: Startup cost
   - Impact: Dominates benchmarks < 10ms
   - Mitigation: Use standard CLBG sizes where compute dominates

4. **Heap allocator overhead**: Not yet optimized
   - Impact: Higher overhead for small allocations
   - Benchmarks affected: binary-trees, any allocation-heavy benchmark

5. **No SIMD support** (if applicable): Manual vectorization not available
   - Impact: Cannot match SIMD-optimized C/Rust implementations
   - Mitigation: Compare only to non-SIMD baselines

## Anti-Patterns to Avoid

### Benchmarking "Crimes" (per Van der Kouwe et al. and Gernot Heiser)

These crimes are found in 96% of academic papers. We will not commit them:

1. **Cherry-picking**: Showing only favorable results
2. **Inconsistent configuration**: Different flags/settings between implementations
3. **Missing parameters**: Not documenting settings that affect results
4. **Non-reproducible setup**: Results that others cannot verify
5. **Misleading comparisons**: Different algorithms or workloads
6. **Selective benchmarking**: Not evaluating performance degradation in non-target areas
7. **Benchmark subsetting**: Using partial benchmark suites without justification
8. **Arithmetic mean on normalized scores**: Use geometric mean instead
9. **Missing variance indicators**: Raw averages without std dev are meaningless
10. **Microbenchmarks as overall claims**: Microbenchmarks probe specifics, not overall performance
11. **Unfair competitor testing**: Competitors must receive equal optimization effort
12. **Outdated baselines**: Compare to current state-of-the-art, not obsolete versions
13. **Relative numbers only**: Always include absolute numbers for sanity checking
14. **Confusing overhead with throughput**: 20% throughput drop ≠ 20% overhead (see below)
15. **Self-comparison only**: Comparing only against your previous work rather than established baselines provides limited insight
16. **Calibration/validation dataset reuse**: Using the same data for training and testing destroys validity—the calibration and evaluation workloads must be different
17. **No proper baseline**: Comparing virtualization solutions without showing native performance, or lacking industry-standard reference points
18. **Percentage point confusion**: Confusing percentage points with percentages (going from 6% to 13% overhead is **doubled**, not "7% increase")

### Throughput vs Overhead: A Critical Distinction

**Crime #14 explained with concrete example**:

Gernot Heiser specifically warns: "If the throughput of a system is degraded by a certain percentage, it does not follow that the same percentage represents the overhead. In many cases the overhead is much higher."

```
Baseline:  100 requests/second
Degraded:   90 requests/second

WRONG interpretation:
  "10% overhead" (confusing throughput drop with overhead)

CORRECT calculation:
  Throughput drop = (100 - 90) / 100 = 10%
  Overhead = (100 - 90) / 90 = 11.1%

  The system now takes 1.111x as long per request,
  which is 11.1% overhead, not 10%.
```

**General formula**:
```
If baseline throughput = B and degraded throughput = D:
  Throughput drop = (B - D) / B
  Overhead = (B - D) / D = B/D - 1
```

**The relationship**:
- 10% throughput drop → 11.1% overhead
- 20% throughput drop → 25% overhead
- 50% throughput drop → 100% overhead (2x slower!)

**Rule**: Always report both throughput drop AND overhead, or be explicit about which you're reporting.

### Specific Pitfalls

1. **Constant folding**: Don't use compile-time constants that LLVM optimizes away
   - BAD: `let x = 1.0 + 2.0` in a loop (folds to 3.0)
   - GOOD: Values derived from input or loop variables

2. **Degenerate cases**: Don't use trivially small inputs where startup dominates
   - BAD: 1000 iterations where runtime is <1ms
   - GOOD: Standard CLBG sizes where compute dominates

3. **Structural differences**: Don't compare unrolled Blood to looped C
   - BAD: Blood with 5 explicit variables vs C with array[5]
   - GOOD: Same structure in both, or explicitly note the difference

4. **Hiding results**: Don't omit benchmarks where Blood performs poorly
   - BAD: Only showing n-body (good) while hiding binary-trees (bad)
   - GOOD: Show complete results with honest analysis

5. **Dead code elimination**: Ensure computed results are used
   - BAD: Computing result but never using it (compiler may eliminate)
   - GOOD: Print result, return it, or use volatile/black_box

6. **Warm filesystem but cold CPU**: Inconsistent cache state
   - BAD: Running warmup with different input size
   - GOOD: Warmup with identical workload

## Benchmark Categories

### Tier 1: Core Performance (Must Pass)

These benchmarks must show Blood within 2x of C to claim "C-level performance":

- **n-body**: Floating-point compute, struct handling, cache efficiency
- **spectral-norm**: Dense linear algebra, loop performance
- **fannkuch-redux**: Integer permutations, array access patterns

### Tier 2: Memory-Intensive

These benchmarks expose allocation and pointer overhead:

- **binary-trees**: Heap allocation, tree traversal, pointer overhead
- **reverse-complement**: Large buffer handling, memory bandwidth

### Tier 3: I/O-Bound

These benchmarks are less relevant for compiler performance:

- **fasta**: Output generation, string handling
- **k-nucleotide**: Hash table performance (standard library dependent)

### Tier Weighting

When computing overall scores:
- Tier 1 benchmarks carry full weight
- Tier 2 benchmarks carry full weight (they expose real issues)
- Tier 3 benchmarks are informational only (don't include in aggregate)

## Full Disclosure Requirements

All benchmark reports must include complete disclosure per SPEC CPU 2017 principles.

### Hardware Configuration

| Category | Required Information |
|----------|---------------------|
| CPU | Model, base frequency, turbo frequency, core count |
| Cache | L1/L2/L3 sizes and configuration |
| Memory | Size, speed (MT/s), channels, timings if relevant |
| Storage | Type (SSD/NVMe/HDD), model (if I/O-relevant) |
| Cooling | Adequate? Any thermal throttling observed? |

### Software Configuration

| Category | Required Information |
|----------|---------------------|
| OS | Distribution, version, kernel version |
| Blood | Exact git commit hash, build configuration |
| LLVM | Version (Blood's backend) |
| C compiler | Version, exact flags used |
| Rust compiler | Version, profile, exact flags |
| libc | Version (glibc, musl, etc.) |

### Methodology Details

| Category | Required Information |
|----------|---------------------|
| Isolation | Network disconnected? Services stopped? |
| CPU governor | Performance mode verified? |
| Turbo boost | Enabled or disabled? |
| CPU pinning | Which core(s)? Taskset/numactl command used? |
| ASLR | Enabled, disabled, or varied? |
| Cache clearing | Exact command used |
| Runs | Total runs, warmup runs discarded |
| Statistics | Median, mean, min, max, std dev, CV%, 95% CI |
| Tooling | BenchExec, hyperfine, or other (with version) |
| Thermal | Any throttling events observed? Max temperature? |

### Non-Default Settings

**ANY system tuning must be disclosed:**
- Kernel parameters (vm.*, kernel.*)
- CPU affinity/pinning (taskset, numactl commands)
- Turbo boost status
- ASLR configuration
- NUMA configuration
- Huge pages
- Compiler flags beyond standard release
- Any kernel boot parameters (isolcpus, nohz_full, etc.)

## Reporting Format

All benchmark reports must include:

```markdown
# Blood Benchmark Report: [Date]

## System Configuration

### Hardware
- CPU: Intel Core i7-12700K @ 3.6GHz (boost to 5.0GHz), 12 cores (8P+4E)
- Cache: L1 48KB/32KB, L2 1.25MB/2MB, L3 25MB
- Memory: 32GB DDR5-4800, dual channel
- Storage: Samsung 980 Pro NVMe SSD
- Cooling: Noctua NH-D15, no thermal throttling observed

### Software
- OS: Ubuntu 24.04 LTS, kernel 6.8.0-35-generic
- Blood: commit abc1234 (2024-01-15)
- LLVM: 17.0.6
- GCC: 13.2.0, flags: -O3 -march=native
- Rust: 1.75.0, profile: release

## Methodology
- Isolation: Network disconnected, cron/snapd/unattended-upgrades stopped
- CPU governor: performance (verified)
- Turbo boost: Disabled (`echo 1 > /sys/devices/system/cpu/intel_pstate/no_turbo`)
- CPU pinning: Core 0 (`taskset -c 0`)
- ASLR: Disabled per-process (`setarch $(uname -m) -R`)
- Cache clearing: `sync && echo 3 > /proc/sys/vm/drop_caches`
- Runs: 12 total, first discarded as warmup
- Tooling: hyperfine 1.18.0
- Thermal: No throttling observed, max temp 72°C

## Results

### Tier 1: Core Performance

| Benchmark | Blood | C | Rust | Blood/C | CV% |
|-----------|-------|---|------|---------|-----|
| n-body | 8.42s | 8.51s | 8.38s | 0.99x | 2.1% |
| fannkuch | 12.3s | 11.8s | 11.9s | 1.04x | 1.8% |
| spectral | 3.21s | 3.18s | 3.19s | 1.01x | 0.9% |

**Geometric mean (Tier 1)**: 1.01x

### Tier 2: Memory-Intensive

| Benchmark | Blood | C | Rust | Blood/C | CV% | Notes |
|-----------|-------|---|------|---------|-----|-------|
| binary-trees | 4.82s | 2.41s | 2.38s | 2.00x | 3.2% | Fat pointer overhead |

### Memory Usage

| Benchmark | Blood RSS | C RSS | Blood/C | Fat Ptr Overhead |
|-----------|-----------|-------|---------|------------------|
| binary-trees | 312 MB | 156 MB | 2.0x | ~156 MB est. |
| n-body | 1.2 MB | 1.0 MB | 1.2x | minimal |

## Analysis

### Where Blood Matches C
- **n-body**: Blood achieves parity (0.99x) due to compute-bound nature
- **spectral-norm**: Blood matches (1.01x), dominated by FP arithmetic
- **fannkuch**: Slight overhead (1.04x) within measurement noise

### Where Blood Is Slower
- **binary-trees**: 2.0x slower due to:
  - 128-bit fat pointers (16B vs 8B) doubling pointer memory
  - Reduced cache efficiency from larger pointer size
  - Allocator overhead for many small allocations

### Known Issues Affecting Results
- Fat pointer overhead is architectural, not a bug to fix
- Allocator optimization could improve binary-trees by ~20% (estimated)

## Limitations
- Blood cannot parse CLI arguments; sizes are compile-time constants
- Single-threaded only; parallel C/Rust implementations would be faster
- No SIMD; compares to non-vectorized baselines only

## Reproduction

```bash
# Clone and build Blood
git checkout abc1234
cargo build --release

# Build benchmarks
cd benchmarks
make all

# Run with isolation
sudo systemctl stop cron unattended-upgrades snapd
sudo cpupower frequency-set -g performance

# Disable turbo boost (Intel example)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Clear caches
sync && echo 3 | sudo tee /proc/sys/vm/drop_caches

# Execute with CPU pinning and ASLR disabled
hyperfine --warmup 1 --runs 12 \
  'taskset -c 0 setarch $(uname -m) -R ./blood_nbody' \
  'taskset -c 0 setarch $(uname -m) -R ./c_nbody' \
  'taskset -c 0 setarch $(uname -m) -R ./rust_nbody'

# Re-enable turbo boost after benchmarking
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
```

## References

### Primary Sources

1. [Computer Language Benchmarks Game](https://benchmarksgame-team.pages.debian.net/benchmarksgame/)
2. [How programs are measured (CLBG)](https://benchmarksgame-team.pages.debian.net/benchmarksgame/how-programs-are-measured.html)
3. [Cross-Language Compiler Benchmarking: Are We Fast Yet?](https://stefan-marr.de/papers/dls-marr-et-al-cross-language-compiler-benchmarking-are-we-fast-yet/) - Stefan Marr et al., DLS 2016
4. [Fair Benchmarking Considered Difficult](https://dl.acm.org/doi/10.1145/3209950.3209955) - Raasveldt et al., DBTest 2018
5. [Benchmarking Crimes: An Emerging Threat in Systems Security](https://arxiv.org/abs/1801.02381) - Van der Kouwe et al., 2018
6. [Gernot Heiser's List of Systems Benchmarking Crimes](https://gernot-heiser.org/benchmarking-crimes.html)

### Statistical Methods

7. [How Not to Lie with Statistics: The Correct Way to Summarize Benchmark Results](https://dl.acm.org/doi/10.1145/5666.5673) - Fleming & Wallace, CACM 1986
8. [Statistical Methods for Reliable Benchmarks](https://modulovalue.com/blog/statistical-methods-for-reliable-benchmarks/)
9. [SPEC CPU 2017 Run and Reporting Rules](https://www.spec.org/cpu2017/Docs/runrules.html)

### Tooling

10. [BenchExec](https://github.com/sosy-lab/benchexec) - Reliable benchmarking and resource measurement
11. [hyperfine](https://github.com/sharkdp/hyperfine) - Command-line benchmarking tool

### System Tuning

12. [pyperf - Tune the system for benchmarks](https://pyperf.readthedocs.io/en/latest/system.html)
13. [Google Benchmark - Reducing Variance](https://github.com/google/benchmark/blob/main/docs/reducing_variance.md)
14. [CPU frequency scaling - ArchWiki](https://wiki.archlinux.org/title/CPU_frequency_scaling)
15. [CPU Performance Scaling - Linux Kernel Documentation](https://docs.kernel.org/admin-guide/pm/cpufreq.html)

### Academic Papers

16. [Reliable Benchmarking: Requirements and Solutions](https://link.springer.com/article/10.1007/s10009-017-0469-y) - Beyer et al., STTT 2019 (BenchExec methodology)
17. [SoK: Benchmarking Flaws in Systems Security](https://www.vusec.net/projects/benchmarking-crimes/) - VUSec research project

## Commitment

This project commits to honest benchmarking. We will:

- **Show all results**, good and bad
- **Explain why** Blood is slower where it is slower
- **Use standard methodology** that can be scrutinized and reproduced
- **Update benchmarks** as Blood improves, tracking progress over time
- **Never manipulate benchmarks** to make Blood "look better"
- **Use geometric mean** for aggregate scores across benchmarks
- **Report CV%** so readers can assess measurement reliability
- **Report 95% confidence intervals** for close comparisons
- **Include absolute numbers**, not just ratios
- **Document all limitations** and their expected impact
- **Compare to fair baselines** that represent competent implementations
- **Document isolation methodology** completely (turbo boost, CPU pinning, ASLR)
- **Validate output correctness** before accepting any performance result
- **Distinguish throughput from overhead** in all reporting

**The goal is to make Blood actually faster, not to make it appear faster.**

---

## Changelog

| Date | Changes |
|------|---------|
| 2025-01-14 | Added turbo boost control, CPU pinning/affinity, expanded ASLR handling |
| 2025-01-14 | Tightened CV% thresholds (20-30% acceptable, >30% poor) |
| 2025-01-14 | Added confidence intervals section |
| 2025-01-14 | Added memory measurement sampling rate caveat |
| 2025-01-14 | Expanded output validation with ndiff parameter explanation |
| 2025-01-14 | Added JIT warmup iteration table with runtime-specific guidance |
| 2025-01-14 | Added throughput vs overhead calculation example |
| 2025-01-14 | Added 4 new benchmarking crimes (15-18) |
| 2025-01-14 | Added isolation checklist |
| 2025-01-14 | Expanded references with system tuning sources |
| 2025-01-14 | Document validated against academic research (Van der Kouwe et al., Heiser, Fleming & Wallace, Marr et al., CLBG, SPEC CPU 2017, BenchExec) |

---

*This methodology document was validated against current academic research and industry standards as of January 2025. Updates will be made as best practices evolve.*
