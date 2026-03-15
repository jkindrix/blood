# Tooling, Diagnostics, and Instrumentation Gaps

**Date:** 2026-03-15
**Status:** Prioritized inventory for systematic improvement

## Critical (block effective debugging)

### GAP-1: DWARF source filename is always "main.blood"
**Impact:** addr2line shows wrong filename for multi-file compilation.
**Root cause:** Span struct has no file_id. All spans in a compilation share
the module-level filename. Line numbers ARE correct within the actual source
file, but the reported filename is "main.blood" for all functions.
**Fix:** Add `file_id: u32` to Span. Map file_id → filename in the source
manager. Pass per-function file_id to `setup_debug_function`. Emit separate
`!DIFile` per source file. This touches: parser (span creation), common.blood
(Span struct), codegen_ctx.blood (debug info emission).
**Effort:** 1 session.

### GAP-2: DWARF only has function-level line numbers
**Impact:** All instructions in a function show the same source line. gdb
can't step through code. Backtraces show function entry line, not the
specific statement that crashed.
**Root cause:** `emit_dbg` uses one DILocation per function. Per-instruction
locations require tracking `current_span` during codegen and emitting new
DILocation nodes when the line changes.
**Fix:** In codegen_stmt.blood and codegen_term.blood, each statement/terminator
has a `span` with a `line`. Emit a new DILocation when the line changes
from the previous instruction. Share DILocation nodes for same-line
instructions.
**Effort:** 1 session.

### GAP-3: MIR validation is opt-in and non-blocking
**Impact:** Invalid MIR reaches codegen silently. Type mismatches, dominance
violations, and use-before-def bugs produce wrong IR instead of errors.
**Root cause:** `--validate-mir` flag gates validation. Errors print to stderr
but compilation continues.
**Fix phase 1:** Make validation blocking — return error count and abort if > 0.
**Fix phase 2:** Add type consistency checks (M-13 from remediation plan).
**Fix phase 3:** Add dominance/use-def chain checking.
**Effort:** Phase 1: 0.5 session. Phase 2-3: 2-3 sessions.

## Significant (cause pain, have workarounds)

### GAP-4: No unified diagnostic command
**Impact:** 12 separate tools in `tools/` with different UIs. Developer must
know which script to use for each problem.
**Fix:** Create `tools/blood-diag` that dispatches to the right tool based on
the problem type: `blood-diag ir-diff`, `blood-diag minimize`, etc.
**Effort:** 0.5 session.

### GAP-5: Golden test debugging is opaque
**Impact:** When a test fails, no easy way to see its compiled IR, MIR, or
runtime output. Must manually invoke the compiler on the test file.
**Fix:** Add `./build_selfhost.sh test golden --debug TEST_NAME` that compiles
with `--dump-mir --dump-ir --trace-codegen` and preserves all artifacts.
**Effort:** 0.5 session.

### GAP-6: Panic messages lack allocation context
**Impact:** `index out of bounds: index 123 but length 4` — but WHICH Vec?
In which data structure? At what source location?
**Root cause:** The bounds check in codegen_expr.blood passes index and length
but not the Vec's identity or the accessing function's context.
**Fix:** Add a source location string parameter to `blood_panic_index_out_of_bounds`.
The codegen can pass the function name or a brief description.
Alternatively, the DWARF fix (GAP-1 + GAP-2) makes the backtrace sufficient.
**Effort:** 0.5 session (or free once GAP-1/GAP-2 are fixed).

### GAP-7: No structured logging or metrics persistence
**Impact:** Can't answer "has selfhost build time been increasing?" No
regression detection beyond golden tests.
**Fix:** Write JSON metrics (build time, IR size, RSS, function count) to
`.logs/metrics.jsonl`. Add `./build_selfhost.sh metrics` to query trends.
**Effort:** 1 session.

## Minor (nice to have)

### GAP-8: No per-function memory tracking
Only phase-level RSS. Can't identify which function or data structure is
the memory hog.

### GAP-9: No CPU flame graphs
Memory profiling exists via massif. No perf/flame graph integration.

### GAP-10: No variable-level DWARF
Function/line mapping works. Local variable tracking (DILocalVariable)
would enable `gdb print x` for Blood variables.

## Remaining Open Items (from review scorecard)

### GAP-1 Residual: Multi-file DWARF filename
**Status:** Single-file fixed (b4f807b). Multi-file selfhost compilation
still shows "main.blood" for all functions because Span has no file_id.
**Root cause:** The bootstrap's `collect_module` resolves external modules
and parses them, but doesn't export a source map (byte_offset → filename)
to the selfhost codegen.
**Fix:** Bootstrap records `{start_byte, end_byte, filename}` per module
during `collect_module`. Exports via a runtime FFI or embedded metadata.
Selfhost codegen queries the source map in `setup_debug_function` to
emit the correct `!DIFile` per function.
**Effort:** 1 session (cross-compiler: bootstrap Rust + selfhost Blood).

### GAP-2 Residual: MIR validation depth
**Status:** Return terminator check added (2ac5336). Unreachable block
detection infrastructure added. Still lacks: dominance checking, use-def
chains, type consistency.
**Fix:** Implement proper dominance tree computation, then check that every
local use is dominated by at least one assignment. Type consistency requires
resolving Rvalue types and comparing against destination place types.
**Effort:** 2-3 sessions for dominance + use-def. 1 session for basic type
consistency.

### GAP-6 Residual: Panic inline context
**Status:** Backtraces + DWARF + addr2line hint added. Panic messages still
show only values (index, length) not source context.
**Fix fully:** Either pass function/variable context from codegen to panic
FFI (adds string constant per check site), or implement runtime DWARF
self-parsing to auto-resolve addresses in backtraces.
**Effort:** 0.5 session for context strings, 2 sessions for self-parsing.

## Priority Order (original, all resolved except residuals above)

1. GAP-1 (DWARF filename) — highest debuggability impact, 1 session
2. GAP-2 (per-instruction lines) — makes gdb usable, 1 session
3. GAP-3 phase 1 (blocking MIR validation) — catches bugs earlier, 0.5 session
4. GAP-5 (test debugging) — day-to-day developer experience, 0.5 session
5. GAP-7 (structured metrics) — regression detection, 1 session
6. GAP-6 (panic context) — free once GAP-1/2 are done
7. GAP-4 (unified command) — convenience, 0.5 session
8. GAP-3 phases 2-3 (advanced MIR validation) — 2-3 sessions
9. GAP-8/9/10 — future work
