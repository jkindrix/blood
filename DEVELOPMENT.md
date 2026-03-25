# Blood Compiler Development Methodology

**Version:** 1.1
**Established:** 2026-02-27
**Status:** Active and Enforced

---

## Supporting Documents

This document defines the CCV methodology — *what* to do and *why*. Operational procedures, tooling workflows, and project history live in dedicated documents:

| Document | Location | Purpose |
|----------|----------|---------|
| **AGENT_PROTOCOL.md** | `tools/AGENT_PROTOCOL.md` | Session start/end procedures, investigation workflow, hard stop conditions, tool escalation chain |
| **FAILURE_LOG.md** | `tools/FAILURE_LOG.md` | Structured bug history — root causes, resolutions, lessons learned |
| **CLAUDE.md** | `CLAUDE.md` | Build commands, tool quick-reference, key patterns, active limitations |
| **build_selfhost.sh** | `src/selfhost/build_selfhost.sh` | All build modes (`--help` for full list); CCV uses `timings`, `golden`, `rebuild` |

**Tool escalation chain** (for debugging regressions — see AGENT_PROTOCOL.md for full workflow):

```
phase-compare.sh → difftest.sh → minimize.sh → asan-selfcompile.sh
```

---

## Canary-Cluster-Verify (CCV) Method

All mechanical or semantic changes to the self-hosted compiler follow this three-step protocol. No exceptions.

### Step 1: Canary

Before mass-converting any pattern, write a targeted golden test that exercises the pattern with all relevant types (`i32`, `u32`, `usize`, `u64`, `bool`, etc.). Run against first_gen.

**Purpose:** Catch type-specific bugs (e.g., a feature that works for `i32` but segfaults for `usize`) *before* making hundreds of changes, not after.

**If the canary fails:**
- Do NOT proceed with mass conversion.
- Diagnose root cause.
- If the root cause is in seed compiler, invoke the Bootstrap Gate (see below).
- If the root cause is in the self-hosted compiler, fix it, rebuild, re-run canary.

### Step 2: Cluster

Group changes by compiler phase with explicit boundaries:

| Cluster | Files | Subsystem |
|---------|-------|-----------|
| **A** | `common`, `interner`, `source`, `error`, `reporter` | Utilities |
| **B** | `lexer`, `token`, `parser_*`, `macro_expand` | Frontend |
| **C** | `ast`, `hir`, `hir_expr`, `hir_item`, `hir_ty`, `hir_def` | AST/HIR Definitions |
| **D** | `hir_lower`, `hir_lower_*`, `resolve` | HIR Lowering |
| **E** | `typeck`, `typeck_*`, `unify`, `type_intern` | Type Checking |
| **F** | `mir_*`, `validate_mir` | MIR |
| **G** | `codegen`, `codegen_*` | Code Generation |
| **H** | `driver`, `main`, `project`, `package`, `build_cache` | Driver + Project |
| **I** | `stdlib/*.blood` | Standard Library |

If Cluster E regresses, the bug is in type checking. No guessing, no binary search across the full codebase.

Apply changes within one cluster at a time. Keep clusters small enough that any regression can be attributed to a specific compiler subsystem.

### Step 3: Verify

After **each** cluster:

```bash
cd src/selfhost

# 1. Build first_gen (seed compiles the modified self-hosted source)
./build_selfhost.sh build first_gen --timings

# 2. Run golden — all tests must pass (check the "Passed: N" total in output)
./build_selfhost.sh test golden

# 3. Self-compile: first_gen → second_gen, then second_gen → third_gen byte-compare
./build_selfhost.sh build second_gen
./build_selfhost.sh build third_gen

# 4. Commit the cluster (clean rollback point)
git add <cluster files> && git commit -m "refactor(selfhost): <description> (Cluster X)"
```

**Commit after each verified cluster.** This gives clean rollback points — if cluster F regresses, `git revert` the cluster F commit. Without per-cluster commits, reverting requires manually identifying which files belong to which cluster.

**Why bootstrap after every cluster, not just at the end:**
- Golden tests verify that user programs compile correctly.
- Bootstrap verification verifies that the *compiler itself* compiles correctly when it contains the new patterns.
- These are different questions. A feature can work in user code but break when used in the compiler's own source (different code paths, different type contexts, different optimization interactions).
- If bootstrap breaks after cluster E but not after cluster D, the bug is in the typeck files. If you only check at the end, it could be any cluster.

**If regression is found:**
1. **STOP.** Do not continue to the next cluster.
2. Revert the cluster (`git revert` or `git checkout -- <files>`).
3. Diagnose the root cause — do not guess, do not shotgun-fix. Use the tool escalation chain (see Supporting Documents).
4. If root cause is in seed compiler, invoke the Bootstrap Gate.
5. If root cause is in the self-hosted compiler, fix it, rebuild, re-verify from scratch.
6. Log the bug in `tools/FAILURE_LOG.md` with root cause and resolution.
7. Only proceed to the next cluster after a clean pass (golden + bootstrap).

---

## Bootstrap Gate

**The bootstrap seed compiles first_gen.** The seed is a prebuilt, frozen compiler binary (`bootstrap/seed`). If the seed has a bug that affects compilation of the selfhost source, first_gen will be miscompiled.

### The Rule

> If any issue traces to the seed compiler, **use a two-stage bootstrap**: Stage 1 adds new code paths without activating them (the seed can compile this), gate to get a new seed, then Stage 2 activates the new behavior.

### The Protocol

```
1. Run the full gate:
     cd src/selfhost && ./build_selfhost.sh gate
2. This builds: first_gen → golden → second_gen → golden → third_gen → byte-compare
3. On success, the seed is updated automatically.
4. Log any bugs encountered in tools/FAILURE_LOG.md.
```

### Build Cache Warning

Build caches are **compiler-version-specific**. The gate script automatically clears caches between generations. When debugging gate failures manually:

```bash
rm -rf build/.content_hashes build/obj/.hashes build/.blood-cache
```

Symptom of stale cache: `undefined value '@.str.NNNN'` errors during llc.

### Two-Stage Bootstrap

When a change can't be compiled by the current seed (e.g., the change fixes a codegen bug that the seed also has):

1. **Stage 1:** Add the new code path but don't activate it. Gate to get a new seed.
2. **Stage 2:** Activate the new behavior. The new seed can compile it. Gate again.

---

## Change Classification

Not all changes carry the same risk. Separate mechanical changes from semantic changes and handle them independently.

### Mechanical Changes

**Definition:** Direct textual substitution where the semantics are provably identical.

**Examples:**
- `x = x + 1` → `x += 1`
- `x = x - 1` → `x -= 1`
- `x = x * 2` → `x *= 2`

**Properties:**
- Can be automated with scripts.
- Each individual transformation is provably equivalent.
- Risk comes from volume (many changes) and type-specific compiler bugs.
- Canary testing catches the type-specific bugs before mass conversion.

### Semantic Changes

**Definition:** Changes that alter control flow, data flow, or code structure in ways that require human judgment.

**Examples:**
- Refactoring a loop body to use `continue` instead of nested `if`/`else`
- Restructuring expressions to use `|>` pipeline
- Replacing `if`/`else if` chains with `match`

**Properties:**
- Cannot be safely automated.
- Each instance requires understanding the surrounding code.
- Risk comes from misunderstanding intent, not from compiler bugs.
- Must be reviewed case-by-case.

### The Separation Rule

> Mechanical and semantic changes are NEVER mixed in the same cluster. They are separate sub-phases with independent canary-cluster-verify cycles.

---

## Phase Decomposition

Large phases from the plan are decomposed into sub-phases, each with its own CCV cycle.

### Example: Phase 3 (Compiler Modernization — Compound Assignment + Misc)

**Original scope (too broad):**
- Replace `x = x + 1` → `x += 1`
- Adopt `continue`
- Adopt `|>` pipeline

**Decomposed:**

| Sub-Phase | Type | Scope | Risk |
|-----------|------|-------|------|
| **3a** | Mechanical | `x = x + 1` → `x += 1` (all compound ops) | Low (with canary) |
| **3b** | Semantic | Adopt `continue` in loops | Medium |
| **3c** | Semantic | Adopt `|>` pipeline | Medium |

Each sub-phase completes its full CCV cycle (canary → cluster → verify) before the next begins.

---

## Debugging Protocol

When a regression is found:

### 1. Classify the Failure

| Symptom | Likely Source |
|---------|-------------|
| Compile error in first_gen build | Parser or type checker change in the cluster |
| Golden test compile failure | Self-hosted compiler bug in changed code |
| Golden test wrong result | Codegen or runtime bug |
| Segmentation fault during compilation | Memory corruption, type mismatch, or seed compiler bug |
| Segmentation fault in test binary | Codegen bug or seed compiler miscompilation |
| Non-deterministic failure (heisenbug) | Memory corruption — likely seed compiler or runtime |

### 2. Isolate

- Revert the cluster.
- Re-apply changes one file at a time until the regression appears.
- The last file applied contains or triggers the bug.

### 3. Diagnose Root Cause

- If the bug is in the changed code: fix the self-hosted compiler.
- If the bug is in seed compiler's compilation of the changed code: invoke the Bootstrap Gate.
- If the bug is pre-existing but was masked: document it, fix it, add a regression test.

### 4. Hard Stop Conditions

If debugging is not converging, stop and reassess:

- **3 failed fix attempts** for the same regression → reassess the diagnosis. The root cause hypothesis is likely wrong.
- **5 fruitless investigation steps** (tool calls that don't narrow the problem) → stop, summarize what's known, escalate or take a different approach.
- **Any suspicion of seed compiler miscompilation** → invoke Bootstrap Gate immediately. Do not continue debugging the self-hosted compiler.

See `tools/AGENT_PROTOCOL.md` for the full investigation workflow and tool escalation chain.

### 5. Record the Bug

When a bug is found and resolved during CCV:
- Add an entry to `tools/FAILURE_LOG.md` with: symptom, root cause, resolution, and which cluster/phase triggered it.
- If the bug required a code fix (not just a revert), add a regression test to `tests/golden/`.

### 6. Never Do These Things

- **Do not shotgun-fix.** Changing multiple things hoping one works wastes time and masks the real issue.
- **Do not work around seed compiler bugs.** The self-hosted compiler must express correct semantics.
- **Do not skip the canary.** "It worked for i32" does not mean it works for usize.
- **Do not batch debug.** One regression, one root cause, one fix. Then re-verify everything.

---

## Lessons Learned

These are real bugs encountered during Phase 2 that motivated this methodology.

### Lesson 1: Type-Specific Bugs

**What happened:** `for i in 0i32..10` worked perfectly. `for i in 0usize..10` caused a segfault during type checking.

**Root cause:** The increment literal (`1`) was hard-coded to `ty: Some(PrimitiveTy::I32)` instead of `ty: None` (inferred from context).

**What would have caught it:** A canary test running for-loops with i32, u32, usize, and u64.

### Lesson 2: Both Compilers Had the Same Bug

**What happened:** For-loop `continue` caused infinite loops. Fixed in self-hosted compiler. Still broken in first_gen output.

**Root cause:** Blood-rust had the exact same desugaring bug. first_gen was compiled by buggy seed compiler, so the fix was miscompiled.

**What would have caught it:** Testing the canary with seed compiler first, then with first_gen.

### Lesson 3: Mass Conversion Cascading Failures

**What happened:** 919 while→for conversions, 6 stdlib tests failed. Could have been any of the 919 changes.

**Root cause:** Not any of the 919 changes — it was the literal type bug in the compiler itself. But diagnosing this required narrowing from 919 suspects.

**What would have caught it:** Smaller clusters with per-cluster verification.
