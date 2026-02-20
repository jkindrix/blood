# Compiler Tooling & Infrastructure Tasks

**Branch:** `dev/compiler-tooling`
**Goal:** Build the missing development infrastructure that stops the guess-and-check cycle and enables mechanical debugging of the self-hosted compiler.

**Rule:** When a task is completed, update `/CLAUDE.md` with usage documentation so all future sessions know the tool exists.

---

## Task Checklist

### High Priority — Directly Unblocks Self-Hosting

- [x] **T01: Differential Testing Harness** `tools/difftest.sh` *(2026-02-20)*
  Two modes: `--behavioral` (default) compiles+runs with both compilers, compares stdout/exit; `--ir` diffs LLVM IR per-function. Batch mode for whole directories.
  - Baseline run: 223 tests → 166 MATCH, 3 DIVERGE, 54 compile-fail
  - Status: complete

- [x] **T02: Test Case Minimizer** `tools/minimize.sh` *(2026-02-20)*
  Auto-detects failure mode, removes top-level items then individual statements. Tested: 42→5 lines (compile-fail), 11→10 lines (wrong-output).
  - Status: complete

- [x] **T03: Phase-Gated Comparison** `tools/phase-compare.sh` *(2026-02-20)*
  Four-phase comparison (Compilation, MIR, LLVM IR, Behavior) between both compilers on a single file. Identifies which phase first diverges. MIR extracted via `--emit mir` (blood-rust, stdout) and `--dump-mir` (first_gen, stderr). Reports MATCH/DIFFER/DIVERGE per phase with verbose mode for details.
  - Status: complete

- [x] **T04: Memory Budget Tracker** `tools/memprofile.sh` *(2026-02-20)*
  Four modes: `--summary` (peak RSS + timings), `--compare` (side-by-side table), `--sample` (RSS timeline via /proc polling), `--massif` (valgrind heap profile). Uses `/usr/bin/time -v` for peak RSS, both compilers' `--timings` for phase breakdown.
  - Status: complete

### Medium Priority — Improves Development Velocity

- [x] **T05: Failure History Log** `tools/FAILURE_LOG.md` *(2026-02-20)*
  Structured markdown log with active issues, resolved issues table (20+ entries seeded from bug history), common root cause patterns, and debugging workflow. Machine-readable format with date, category, symptom, root cause, resolution, files.
  - Status: complete

- [x] **T06: ASan Self-Compilation Wrapper** `tools/asan-selfcompile.sh` *(2026-02-20)*
  Full pipeline: build first_gen → self-compile → ASan instrument → run + format report. Modes: `--reuse`, `--ir FILE.ll`, `--run-only`, `--test CMD`. Color-formatted ASan output with highlighted functions and stack traces. Requires LLVM 18 tools.
  - Status: complete

- [x] **T07: FileCheck Test Coverage Audit** `tools/filecheck-audit.sh` *(2026-02-20)*
  Four sections: existing test inventory, codegen pattern scan (20+ IR emission types, 20+ feature categories), ground-truth coverage analysis, and gap report with 15 prioritized recommendations (5 HIGH, 6 MEDIUM, 4 LOW). Baseline: 3 existing tests, 0/15 recommended tests exist.
  - Status: complete

- [x] **T08: MIR Validation Gate** `tools/validate-all-mir.sh` *(2026-02-20)*
  Runs `--validate-mir` on configurable inputs (single file, directory, ground-truth, or --self for compiler sources). Reports PASS/FAIL/CRASH/MIR per file. Baseline: 148 pass, 75 fail (compilation failures), 94 skip (COMPILE_FAIL) of 317 ground-truth tests.
  - Status: complete

### Lower Priority — Good Practice, Build Later

- [x] **T09: Ground-Truth Regression Tracker** `tools/track-regression.sh` *(2026-02-20)*
  Runs all 317 ground-truth tests, stores results as baseline, compares future runs to detect regressions/passes. Modes: `--save`, `--show`, `--ref`. Baseline at `tools/.baseline_results.txt`.
  - Baseline run: 210/317 pass (66.2%), 32 fail, 75 compile-fail, 0 crash
  - Status: complete

- [x] **T10: Agent Convergence Guardrails** `tools/AGENT_PROTOCOL.md` *(2026-02-20)*
  Written protocol for AI agent sessions: time-box rules, mandatory commit/report intervals, stop-and-yield criteria, failure log update requirements, investigation workflow, and anti-patterns.
  - Status: complete

---

## Completion Protocol

When a task is marked complete:
1. Check the box above
2. Add the date and a one-line summary of what was built
3. Update `/CLAUDE.md` with a new section documenting the tool:
   - What it does
   - How to run it
   - What output to expect
   - When to use it
4. Commit both the tool and the CLAUDE.md update together
