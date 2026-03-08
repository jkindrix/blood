# Quality Profile: Blood

> A self-hosting systems language compiler pursuing spec-implementation alignment, with algebraic effects, generational memory safety, and multiple dispatch.

Quality in a compiler is experienced through failures, not successes. Every program that compiles correctly is invisible; every error message is a moment of truth. The quality bar is set by what happens when the user's code is wrong.

## Priority Dimensions (ranked)

1. **Diagnostic Fidelity** — Error messages must display user-written types (not internal IDs like `def351`), point to all relevant source locations (error site AND definition site), and be regression-tested as artifacts (snapshot or golden-file tests that verify exact message content, not just exit codes). A compiler's error output is its primary user interface. Test: every compile-fail test verifies the actual diagnostic text, not just that compilation failed.

2. **Spec-Implementation Conformance** — Every behavior prescribed by the spec (GRAMMAR.md, DIAGNOSTICS.md, SPECIFICATION.md) must be implemented, tested, and verified. Gaps between spec and implementation must be tracked and measured. The spec defines 237 error codes; the implementation should emit the correct code for each case. Test: a coverage matrix mapping spec-prescribed behaviors to tests that exercise them.

3. **Example and Entry Point Integrity** — Every example in `examples/` must compile and run. The README must accurately describe the project's current state. CLI `--help` must be complete and correct. A user following the Quick Start in README.md must succeed without encountering broken paths. Test: CI runs `blood check` on every example; README claims are verified against reality.

4. **Bootstrap Stability** — The self-hosting chain (blood-rust → first_gen → second_gen → third_gen) must remain byte-identical and all ground-truth tests must pass. This is the project's core engineering achievement and regression here is catastrophic. Test: the existing build_selfhost.sh pipeline (already strong).

5. **Error Recovery and Cascading** — The compiler should report multiple independent errors in a single pass without cascading false positives. A single typo should produce one error, not five. Test: files with N independent errors produce exactly N diagnostics.

## Anti-Targets

- **Broad Platform Support** — Blood targets Linux with LLVM 18. Supporting macOS/Windows/other LLVM versions is deferred until the compiler is mature. Sacrificed for: faster iteration and simpler CI.

- **IDE/LSP Integration** — Language server protocol support, syntax highlighting, and editor tooling are deferred. The compiler's CLI is the only supported interface. Sacrificed for: compiler correctness and spec alignment work.

- **Library Ecosystem Growth** — Growing the stdlib beyond what the self-hosted compiler needs is deferred. The stdlib exists to support bootstrapping, not to attract library authors. Sacrificed for: compiler self-hosting completeness.

- **Performance Optimization** — Compilation speed and output binary performance are not current priorities. The compiler uses ~23-29 GB RSS for self-compilation. Sacrificed for: correctness and feature completeness.

## Current State vs. Target

| Dimension | Current State | Current Quality | Target | Key Gaps |
|-----------|--------------|-----------------|--------|----------|
| Diagnostic Fidelity | ariadne-based rendering, 78 error codes, source context shown, "did you mean?" for methods, DefIds resolved to type names, secondary "defined here" labels on key errors, compile-fail tests verify diagnostic text via EXPECT directives | Good — DefId leak fixed, secondary labels added for 6+ error types, 95/97 compile-fail tests have EXPECT verification, UI snapshots capture secondary labels | All errors show user-written types, multi-location diagnostics, snapshot tests for every error code | No `--explain` for error codes; selfhost still shows `Adt(N)` in errors; not all error types have secondary labels yet |
| Spec-Implementation Conformance | Active audit process (AUDIT.md, WORKLOAD.md), spec-first principle enforced, recent commits fix audit items | Strong process, actively closing gaps | Every spec-prescribed behavior has a corresponding test; divergence count tracked to zero | Spec defines 237 error codes, implementation has 78; no automated coverage matrix; some spec sections (E04xx-E07xx) not yet implemented |
| Example and Entry Point Integrity | 63 examples, GETTING_STARTED.md exists, CLI help is well-structured | 42/61 examples pass type checking (68%); 19 remaining failures use unimplemented features (HashMap, Vec methods, turbofish, std.env/fs, binary literals) | 100% of examples compile; README accurately reflects project stage | 19 broken examples (aspirational — require unimplemented stdlib/language features); no built-in test framework (#[test]); README version/structure inaccuracies; no getting-started tutorial |
| Bootstrap Stability | 3-stage bootstrap, byte-identical second_gen/third_gen, 355/356 ground-truth passing, CCV methodology | Excellent — this is the project's strongest dimension | Maintained at current level; zero ground-truth regressions | 1 known failure (t06_err_import_error); otherwise strong |
| Error Recovery | Single-error reporting per phase; parsing can report multiple errors | Basic — parser recovers, typechecker often stops at first error | Multiple independent errors reported per compilation; cascading suppressed | No cascading error suppression; typechecker reports errors sequentially without recovery |

## Exemplars Referenced

- **Gleam** (gleam-lang/gleam) — Demonstrated that a small-team Rust compiler can have excellent diagnostics through comprehensive snapshot testing (3,539 snapshots, 384 error-specific tests) and rich error context (secondary labels, suggestions, type-aware hints). Similar scale to Blood (137K lines Rust vs 125K).

- **Rust** (rust-lang/rust) — Demonstrated the gold standard for compiler diagnostics: 12,594 `.stderr` golden files ensure every error message is regression-tested, `--explain` provides detailed error documentation, multi-location diagnostics show both error and definition sites, and fix-it suggestions include concrete replacement code.

- **Elm** (elm/compiler) — Demonstrated that diagnostic quality IS the product: 23% of compiler source (11,492 lines of 49,369) is dedicated to error reporting. Levenshtein-distance suggestions, conversational error tone, and errors that teach the language. Proves that investing disproportionately in error messages pays dividends in adoption.
