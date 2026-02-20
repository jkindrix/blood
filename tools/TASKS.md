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

- [ ] **T08: MIR Validation Gate** `tools/validate-all-mir.sh`
  Run `--validate-mir` on a configurable set of inputs (ground-truth tests or the compiler itself) and report any MIR structural errors. Intended as a pre-codegen gate.
  - Input: directory of `.blood` files or single file
  - Output: pass/fail per file with error details
  - Status: not started

### Lower Priority — Good Practice, Build Later

- [ ] **T09: Ground-Truth Regression Tracker** `tools/track-regression.sh`
  Run ground-truth tests, compare results against a stored baseline, and report new passes, new failures, and flips. Prevents regressions from going unnoticed.
  - Input: test runner output
  - Output: diff against baseline (new PASS, new FAIL, new CRASH)
  - Status: not started

- [ ] **T10: Agent Convergence Guardrails** `tools/AGENT_PROTOCOL.md`
  Written protocol for AI agent sessions: time-box rules, mandatory commit/report intervals, "stop and yield" criteria, and failure log update requirements. Not code — a process document referenced from CLAUDE.md.
  - Status: not started

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
