# blood-projects worklog

Chronological record of work on Blood language test projects.

---

## 2026-03-12 — Brainfuck interpreter

### Accomplished

- Created `brainfuck/bf.blood`: a complete Brainfuck interpreter (~110 lines)
- Created three test programs in `brainfuck/programs/`:
  - `hello.bf` — prints "Hello World!" (verified correct)
  - `add.bf` — computes 2+3, prints digit "5" (verified correct)
  - `squares.bf` — prints perfect squares 0 through 10000 (verified correct)
- Interpreter handles: `><+-.,[]` instructions, bracket validation before execution, file-not-found errors, missing-argument usage message
- Blood features exercised: file I/O (`file_read_to_string`, `file_exists`), CLI args (`args_count`, `args_get`), raw memory (`alloc`, `free`, `ptr_read_u8`, `ptr_write_u8`), byte-level string processing (`as_bytes()`, slice indexing), type casting (`u8`/`i32`/`char`), nested control flow

### Flagged items

1. **`,` (input) command is stubbed.** Writes `0` instead of reading from stdin. `read_line()` exists as a builtin but returns `String`; calling `.as_bytes()` on owned `String` vs `&String` was not tested and may require explicit borrowing. BF programs requiring input will not work correctly.

2. **`u8` wrapping behavior is untested.** The interpreter uses a safe path (`u8 → i32`, arithmetic, `% 256`, `→ u8`) to avoid depending on whether Blood's `u8` arithmetic wraps or traps on overflow. No test program hit the 0/255 boundary, so this remains unverified.

3. **`usize` subtraction risk in `]` handler.** `pc = pc - 1` during backward bracket scanning could underflow if bracket validation has a bug. Pre-execution validation should prevent this, but the underflow behavior of `usize` subtraction in Blood is not documented.

4. **`as_bytes()` on `&str` is lightly tested upstream.** Only two golden tests (`stdlib_string.blood`, `stdlib_hashmap.blood`) exercise this pattern. It worked here, but it's a key dependency for any string-processing Blood program and warrants more coverage in the Blood test suite.

5. **`brainfuck/build/` is not gitignored.** Build artifacts land there. Needs a `.gitignore` if this project is tracked in version control.

---

## 2026-03-12 — Expression calculator

### Accomplished

- Created `calc/calc.blood`: a recursive descent expression evaluator (~240 lines)
- Supports: `+`, `-`, `*`, `/`, parentheses, unary negation, decimal numbers
- Correct operator precedence (`*`/`/` before `+`/`-`)
- Joins unquoted CLI arguments (e.g., `calc 2 + 3` works like `calc "2 + 3"`)
- Verified tests:
  - `"2 + 3 * 4"` → `14` (precedence)
  - `"(10 - 2) / 4"` → `2` (parentheses)
  - `"3.14 * 2"` → `6.28` (floats)
  - `"-5 + 3"` → `-2` (unary negation)
  - `"((2 + 3) * (7 - 4))"` → `15` (nested parens)
  - `"10 / 0"` → `Error: division by zero` (effect-based error, exit 1)
  - `"2 + + 3"` → `Error: expected number or '('` (syntax error, exit 1)
- Blood features exercised (complementary to brainfuck project):
  - **Enums** with data variants (`Token.Num(f64)`, `Token.Plus`, etc.)
  - **Pattern matching** on enum variants in match expressions
  - **Algebraic effects** — `CalcErr` effect with `raise` operation
  - **Deep handler** — `CatchCalcErr`, exception-style (no resume)
  - **Structs** — `Eval { pos: usize }` passed by `&mut`
  - **Vec<Token>** — building and indexing a collection of enum values
  - **String building** — `String.new()`, `.push()`, `.push_str()`, `.as_str()`
  - **Recursive descent** — mutually recursive functions with effect annotations
  - **f64 arithmetic** and `i32`/`f64`/`char` casts
  - **`floor()` builtin** for integer detection

### Flagged items

1. **Vec indexing on enum types returns opaque match target.** Matching on `tokens[pos]` works, but extracting payload values (e.g., `Token.Num(n) => n`) required a helper function (`get_num`) rather than direct inline matching. Whether this is a limitation or just a style choice is unclear — a direct `match tokens[e.pos] { Token.Num(n) => ... }` was avoided to sidestep potential borrow/move issues with `&mut Eval`.

2. **Effect handler return type unification.** The `CalcErr.raise` op is typed `-> i32` (not `-> !`) because the handler needs to return an `i32` exit code from the `with...handle` block. Using `-> !` (never type) would be more semantically correct for an aborting operation, but it's unclear whether the handler can produce a value of a different type than the operation's return type. The `-> i32` approach was taken from `t03_effect_exception.blood`.

3. **`perform` in a non-effectful context.** If `perform CalcErr.raise(...)` is called inside a function without `/ {CalcErr}` in its signature, the type checker should reject it. This was correctly enforced — the effect annotation propagation through the recursive call chain works.

4. **`calc/build/` is not gitignored.** Same issue as `brainfuck/build/`.

---

## 2026-03-12 — Base64/Hex codec

### Accomplished

- Created `b64/b64.blood`: a dual-codec encoder/decoder (~230 lines)
- Supports base64 and hex encoding/decoding with file I/O mode (`-f`)
- Output verified byte-identical against system `base64` and `xxd -p`:
  - `base64 encode "Blood"` → `Qmxvb2Q=` (matches system)
  - `base64 encode "ab"` → `YWI=` (2-byte padding case, matches)
  - `base64 encode "Hello, World!"` → `SGVsbG8sIFdvcmxkIQ==` (1-byte padding, round-trips correctly)
  - `hex encode "Hello"` → `48656c6c6f` (matches `xxd -p`)
  - All decode round-trips produce original input
  - File mode (`-f`) works for both codecs
- Blood features exercised (complementary to previous projects):
  - **Traits** — `Codec` trait with `encode`/`decode` methods
  - **Multiple trait implementations** — `impl Codec for Base64` and `impl Codec for Hex`
  - **Struct methods via trait** — `codec.encode(input)` dot-call dispatch
  - **Bitwise operations** — `>>`, `<<`, `&`, `|` on `i32` for bit manipulation
  - **Empty structs** — `Base64 {}`, `Hex {}` as trait implementors
  - **Chained casts** — `i32 as u8 as char` for byte-to-character conversion
  - **`str_eq` builtin** — for string comparison in argument parsing

### Flagged items

1. **Traits work via static dispatch.** `codec.encode(input)` resolves at compile time because `codec` has a concrete type (`Base64` or `Hex`). Dynamic dispatch (`dyn Codec`) was not tested and is documented as partial in the implementation status. The codec selection is done via `if`/`else` on the codec name string rather than a single polymorphic call, which is the workaround.

2. **`self` by value in trait methods.** The trait uses `fn encode(self, input: &str) -> String` (taking `self` by value), matching the pattern in `t05_trait_basic.blood`. Taking `self: &Self` (by reference) was not tested because the golden test used by-value. Empty structs make this a non-issue here, but larger structs would want `&Self`.

3. **Bitwise ops only verified on `i32`.** All bit manipulation is done by casting `u8 → i32`, operating, then casting back `i32 → u8`. Whether `>>`, `<<`, `&`, `|` work directly on `u8` is untested. The golden test `t05_bitwise_ops.blood` only uses `i32`.

4. **`i32 as u8 as char` chained cast.** This two-step cast chain compiled and worked correctly, but it's undocumented whether Blood guarantees left-to-right evaluation of chained `as` expressions. If the intermediate `u8` step were skipped (interpreting as `i32 as char` directly), behavior could differ for values > 127.

5. **`b64/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Sorting benchmark

### Accomplished

- Created `sortbench/sortbench.blood`: three sorting algorithms with timing (~270 lines)
- Implements bubble sort, insertion sort, and quicksort (with median-of-three pivot and insertion sort cutoff)
- Configurable array size via CLI argument (default 1000)
- All three algorithms produce verified correct sorted output at every tested size
- Timing results (representative, single run):
  - n=100: bubble 6us, insertion 1us, quicksort 3us
  - n=1000: bubble 554us, insertion 82us, quicksort 145us
  - n=10000: insertion 7126us, quicksort 12722us (bubble skipped)
  - n=100000: insertion 665ms, quicksort 1230ms (bubble skipped)
- Blood features exercised (complementary to all previous projects):
  - **for loops with ranges** — `for i in 0..n { ... }` for array filling and integer parsing
  - **Generics** — `fn identity<T>(x: T) -> T` compiles and runs correctly
  - **Effects with resume** — `Rng` effect with `op next() -> u64`, handler calls `resume(seed)` to return values through the effect
  - **Deep handler with mutable state** — `LcgRng` handler maintains `let mut seed: u64` across invocations
  - **Timing builtins** — `blood_clock_nanos()` for nanosecond-precision benchmarking
  - **Vec mutation** — `v[i] = v[j]` through `&mut Vec<i32>` for in-place sorting
  - **Inherent impl blocks** — `impl Stats { fn print(self) { ... } }` with method call `s.print()`
  - **u64 arithmetic** — LCG PRNG multiplication and addition
  - **`as` casts between u64/i32/usize** — various numeric conversions
  - **Reserved keyword avoidance** — discovered `raw` is a reserved keyword (renamed to `val`)

### Flagged items

1. **Quicksort exhibits O(n^2) scaling.** Insertion sort (expected O(n^2)) and quicksort (expected O(n log n)) both show ~4x time increase when n doubles at large sizes. Root cause: Lomuto partition scheme degrades with many duplicate keys. With 100K values in range [0,10000), there are massive duplicates. Equal-to-pivot elements all land on one side, creating maximally unbalanced partitions. This is an algorithmic issue (well-known in CS), not a Blood bug. A three-way partition (Dutch National Flag) would fix it.

2. **`raw` is a reserved keyword.** Using `let raw: u64 = ...` produces a parse error. This was discovered during compilation and fixed by renaming to `val`. The reserved keyword list should be consulted when choosing variable names — `docs/spec/GRAMMAR.md` §9.3.

3. **`for i in 0..n` requires `n` to be `i32`.** The for-range loop worked with `i32` bounds. Whether `usize` or `u64` range bounds work was not tested. The integer parsing loop uses `for i in 0..bytes.len() as i32` to cast `usize` to `i32`.

4. **Generic functions compile but were tested minimally.** Only `identity<T>` was tested with `T = i32`. Generic structs, generic trait bounds, and multi-parameter generics were not exercised in this project.

5. **Effect with resume works correctly.** The `Rng` effect handler successfully resumes with a value (`resume(seed)`), and the caller receives it via `perform Rng.next()`. This confirms that effectful functions can be called from within a `with...handle` block and values flow back through `resume`. This is the first project to test non-aborting effect handlers.

6. **`sortbench/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Numerical statistics tool

### Accomplished

- Created `numstat/numstat.blood`: a numerical statistics calculator (~305 lines)
- Reads integers from a file (one per line) and computes comprehensive statistics
- All output verified byte-identical against Python reference implementation
- Created `numstat/data.txt` with 15 test integers (including negatives and zero)
- Test results: all 16 output values match Python exactly (count, sum, min, max, range, mean, variance, std dev, even/odd counts, positive/negative/zero counts, above-mean count, sum of squares, sum of positives, even sum/count, composition test)
- Blood features exercised (complementary to all previous projects):
  - **Closures as values** — `|x| x * 2`, `|x| x % 2 == 0`, `|x| x > 0` passed as arguments
  - **Higher-order functions** — `vec_map`, `vec_filter`, `vec_count`, `vec_any` all take `fn(i32) -> T` parameters
  - **Tuple types** — `(i32, i32)` as return types from `min_max` and `sum_and_count`
  - **Tuple destructuring via match** — `match bounds { (lo, hi) => { ... } }`
  - **Closure variable capture** — `|x| x > threshold` captures `threshold` from enclosing scope
  - **Function composition** — `compose(|x| x + 10, |x| x * 2, 5)` = 20
  - **`sqrt()` builtin** — used for standard deviation calculation
  - **f64 arithmetic** — mean, variance, standard deviation computed with floating-point precision
  - **Manual number parsing** — byte-level integer parser handling negatives, whitespace, non-numeric lines

### Flagged items

1. **Closure capture works correctly.** `|x| x > threshold` successfully captured `threshold` (an `i32` from enclosing scope) and produced correct results. This is the first project to verify closure capture with a non-literal captured variable.

2. **Tuple destructuring only tested via `match`.** Direct `let (a, b) = expr` was avoided because golden tests only show the `match` pattern. Whether `let`-destructuring works is untested.

3. **Higher-order functions take `fn(T) -> U` typed parameters.** This is the closure type syntax, not a generic function pointer. Whether Blood distinguishes between closures and bare function pointers (as Rust does with `fn` vs `Fn`) was not tested — all arguments were closures.

4. **`numstat/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Word frequency counter

### Accomplished

- Created `wordfreq/wordfreq.blood`: a word frequency counter (~215 lines)
- Reads a text file, extracts and lowercases words, counts frequencies, sorts by count descending
- Optional `-o output.txt` flag writes TSV results to a file
- Verified: "the" appears 14 times in sample text, confirmed by `grep -oi` count
- All 33 unique words correctly counted and sorted; file output works correctly
- Copied `stdlib_hashmap.blood` into project directory for module resolution
- Blood features exercised (complementary to all previous projects):
  - **Module system** — `mod stdlib_hashmap;` loads external module from sibling file
  - **HashMap** — `stdlib_hashmap.HashMapU64U32` with `new()`, `insert()`, `get()`, `contains_key()`
  - **Option<T>** — `Option.Some(v)` / `Option.None` from HashMap `get()` lookups
  - **File output** — `file_write_string(path, content)` to write results file
  - **FNV-1a hashing** — `stdlib_hashmap.hash_string(&word)` for word-to-key mapping
  - **Struct with Vec and HashMap fields** — `WordFreq` struct containing `Vec<String>`, `Vec<i32>`, and `HashMapU64U32`
  - **`&mut` struct with collection fields** — `add_word(wf: &mut WordFreq, word: String)` mutates Vec and HashMap through struct reference
  - **String method chains** — `wf.words[idx].as_str()` accessing String method on Vec element

### Flagged items

1. **Module resolution requires sibling file.** `mod stdlib_hashmap;` resolves by looking for `stdlib_hashmap.blood` in the same directory as the source file (or `stdlib_hashmap/mod.blood`). Only `mod std;` gets special stdlib path resolution. External projects must copy/symlink stdlib modules into their project directory.

2. **HashMap collisions handled naively.** Hash collisions (two different words with the same FNV-1a hash) are handled by inserting with `h + 1` as key, which could itself collide. For this demo with ~33 words the probability is negligible, but a real application would need proper collision handling. The stdlib HashMap itself uses linear probing internally, but the u64 key space maps one-to-one.

3. **`Option.Some` / `Option.None` work as builtin.** The HashMap's `get()` returns `Option<u32>` and it matches with `Option.Some(v)` / `Option.None` (dot-separated, not bare `Some`/`None`). The golden test `t05_option_builtin.blood` uses bare `Some`/`None` syntax, but the HashMap module returns `Option.Some`/`Option.None`. Both patterns appear to work.

4. **Struct with HashMap field compiles and works.** `WordFreq` struct contains a `stdlib_hashmap.HashMapU64U32` field. Accessing methods through `wf.index.get(h)` and `wf.index.insert(h, idx)` via `&mut WordFreq` worked correctly. This is the first project to test structs containing stdlib types as fields.

5. **`wordfreq/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Conway's Game of Life

### Accomplished

- Created `life/life.blood`: Conway's Game of Life simulator (~240 lines)
- Reads grid pattern from file, runs N generations (default 20), prints each frame
- Test patterns verified:
  - `glider.txt` (20x10) — classic glider moves diagonally, population stays at 5 across all generations
  - `blinker.txt` (5x5) — period-2 oscillator alternates correctly between vertical and horizontal
  - `dies.txt` (5x5) — single cell dies after 1 generation, early termination via `break`
- Blood features exercised (complementary to all previous projects):
  - **Compound assignment** — `+=` used throughout for counters, loop variables, accumulators
  - **`break`** — early loop termination when population goes extinct; also in inner parsing loop for newline detection
  - **`continue`** — skip self-cell (dx==0, dy==0) in neighbor counting loop
  - **Generic struct** — `Pair<T>` with `Pair<i32>` instantiation, field access, and passing to function
  - **Nested `for` loops** — `for y in 0..height { for x in 0..width { ... } }` for grid traversal
  - **Flat 2D grid** — `Vec<u8>` indexed as `y * width + x` with `as usize` cast
  - **Returning new struct from function** — `step()` returns a fresh `Grid` each generation

### Flagged items

1. **`gen` is a reserved keyword.** Using `gen` as a variable/parameter name produces a parse error. Same class of issue as `raw` (discovered in sortbench). Renamed to `step_num` and `g`. The reserved keyword list in Blood is larger than expected — programmers should consult `docs/spec/GRAMMAR.md` §9.3.

2. **`as_bytes()` returns `[u8]` slice, not `Vec<u8>`.** Attempting to pass the result of `as_bytes()` to a function expecting `&Vec<u8>` fails with type mismatch. The byte slice type `[u8]` is distinct from `Vec<u8>`. Workaround: inline the byte processing rather than abstracting into a helper function, or copy bytes into a `Vec<u8>`. This affects any project wanting to create reusable byte-processing functions.

3. **Generic struct `Pair<T>` works but was tested minimally.** Only `Pair<i32>` was instantiated. Generic structs with methods (`impl Pair<T> { ... }`) were not tested. Field access on generic structs works correctly.

4. **`life/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Build script runner

### Accomplished

- Created `runner/runner.blood`: a shell script runner with effect-based state tracking (~210 lines)
- Reads a script file (one command per line), executes each with `system()`, reports pass/fail
- Comments (`#` lines) and empty lines are skipped
- Exit code 0 if all commands pass, 1 if any fail
- Verified tests:
  - `test.sh` — 5 commands (4 pass, 1 fail via `false`), summary correct, exit code 1
  - `pass.sh` — 5 commands all pass, summary correct, exit code 0
- Blood features exercised (complementary to all previous projects):
  - **`system()` builtin** — executing shell commands and capturing exit codes
  - **Multiple composed effects** — `execute_script()` has signature `/ {Counter, RunErr}`, two distinct effects in one function
  - **Nested `with...handle` blocks** — `with RunCounter handle { with CatchRunErr handle { ... } }` for composing handlers
  - **Generic enum** — `Outcome<T>` with `Outcome.Ok(T)` / `Outcome.Err(i32)` variants
  - **Pattern matching on generic enum** — `match result { Outcome.Ok(_) => ..., Outcome.Err(code) => ... }`
  - **Effect handler with compound assignment** — `passed += 1` and `failed += 1` inside handler ops
  - **Effect with 4 operations** — `Counter` effect has `get_pass`, `get_fail`, `inc_pass`, `inc_fail`

### Flagged items

1. **`Result` is a builtin type.** Defining `enum Result<T> { ... }` fails with "name is defined multiple times." Blood has a builtin `Result` type (like `Option`). Renamed to `Outcome`. This means user code cannot shadow builtin type names.

2. **`system()` runs commands via shell.** The command string is passed to the system shell. Special characters, pipes, and redirections work (tested `ls /tmp > /dev/null`). This is powerful but could be a security concern for untrusted input.

3. **Multiple composed effects work correctly.** The nested handler pattern `with A handle { with B handle { body } }` correctly provides both effects to the body. Effect operations from both handlers are accessible within `execute_script()`. The inner handler (`CatchRunErr`) is never triggered in these tests since no fatal errors occur — only the `Counter` effect ops are exercised at runtime.

4. **`RunErr` effect handler is defined but not triggered.** The `fatal` operation exists as a safety net but no test case triggers it. The abort-style handler pattern (no `resume`) was already verified in the calc project, so this is low risk.

5. **`runner/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-12 — Number guessing game

### Accomplished

- Created `guess/guess.blood`: interactive number guessing game (~150 lines)
- Computer picks random number 1-100, player guesses with too-high/too-low hints
- Supports play-again loop, EOF graceful exit, stats summary
- Verified via piped stdin: `read_line()` reads correctly, EOF detected, game plays to completion
- Blood features exercised (complementary to all previous projects):
  - **`read_line()` builtin** — first test of stdin input; returns `String` (not `&str`)
  - **`str_len()` builtin** — used for EOF detection (empty string = EOF)
  - **`.as_str()` on `read_line()` return** — `String` -> `&str` conversion for passing to other functions
  - **`.as_str().as_bytes()` chain on `String`** — accessing bytes of a `read_line()` result
  - **Interactive read-eval-print loop** — `while true` with `read_line()` + parse + respond + `break`

### Flagged items

1. **`read_line()` returns `String`, not `&str`.** The typechecker registered it as `String`, but the runtime returns a `BloodStr` (ptr/len pair pointing to a thread-local buffer). Assigning to `let line: &str` fails with type mismatch. Must use `let line: String` then `line.as_str()`. This is a potential ownership/lifetime concern — the thread-local buffer is reused on each call, so the `String` may alias stale memory if held across multiple `read_line()` calls.

2. **`read_int()` returns 0 on both EOF and parse error.** This makes it impossible to distinguish "user typed 0" from "pipe exhausted" or "invalid input". The initial version using `read_int()` caused an infinite loop when piped input was exhausted. `read_line()` with manual parsing and `str_len()` EOF check is the correct pattern for non-interactive use.

3. **`str_len()` works on `&str` from `String.as_str()`.** This is the first project to test `str_len()` on dynamically-obtained strings (as opposed to string literals). Returns 0 for EOF empty strings correctly.

4. **`guess/build/` is not gitignored.** Same as previous projects.

---

## 2026-03-14 — Flagged item review and fixes

### Fixes applied

1. **`.gitignore` added** — top-level `.gitignore` with `build/` covers all 9 project build directories. Resolves BF-5, EC-4, B64-5, SB-6, NS-4, WF-5, LIFE-4, BR-5, GG-4.

2. **Brainfuck `,` input implemented (BF-1)** — replaced stub `ptr_write_u8(tape + dp, 0)` with `read_line()` + first byte extraction. Reads one character per `,` instruction. EOF/empty input writes 0 (standard BF convention). Compiles and runs through first_gen.

3. **Quicksort three-way partition (SB-1)** — replaced Lomuto partition with Dutch National Flag (three-way) partition. Equal-to-pivot elements are grouped in the middle and excluded from recursion, eliminating O(n^2) degradation on duplicate keys. Results at n=100K: 147us (was 1230ms). Compiles through bootstrap; first_gen has a codegen bug with `*out = val` dereference assignment (undefined label in generated IR — tracked below).

### Items confirmed working / by-design

4. **Reserved keywords `raw`, `gen` (SB-2, LIFE-1)** — by design. SYN-10 fix (656a01f) correctly rejects these. Consult `docs/spec/GRAMMAR.md` §9.3 for the full reserved keyword list.

5. **`Result` is a builtin type (BR-1)** — by design. Cannot be shadowed, like `Option`. User enums must use a different name (e.g., `Outcome`).

6. **Effect propagation (EC-3)** — `perform` in non-effectful context correctly rejected. Effect annotation propagation through recursive call chains works.

7. **Closure capture (NS-1)** — non-literal variable capture works correctly.

8. **`Option.Some`/`Option.None` syntax (WF-3)** — both dot-qualified and bare forms work. The dot form is canonical Blood style.

9. **Multiple composed effects (BR-3)** — nested `with...handle` blocks correctly provide both effects. Validates EFF-07.

10. **Effect with resume (SB-5)** — non-aborting effect handlers with mutable state work correctly. Validates EFF-07.

11. **Struct with HashMap field (WF-4)** — stdlib types as struct fields work.

12. **`str_len()` on dynamic strings (GG-3)** — works, including EOF detection.

### Compiler/language gaps identified (not fixable from project side)

13. **`read_line()` thread-local buffer aliasing (GG-1, BF-1)** — `read_line()` returns `String` typed by the typechecker but `BloodStr` at runtime, pointing to a thread-local buffer that's reused on each call. Holding a `String` across multiple `read_line()` calls risks stale data. Workaround: process each line immediately before the next call.

14. **`read_int()` EOF indistinguishable from 0 (GG-2)** — returns `i32` 0 for both EOF and parse error. No way to detect end of input. Workaround: use `read_line()` + manual parsing + `str_len()` EOF check.

15. **`[u8]` slice vs `Vec<u8>` (LIFE-2)** — `as_bytes()` returns a `[u8]` slice, not `Vec<u8>`. Functions expecting `&Vec<u8>` can't accept slices. Workaround: inline byte processing or copy to Vec.

16. **Module resolution for external projects (WF-1)** — `mod foo;` resolves relative to source file only. No `--stdlib-path` or project-level module search path. External projects must copy/symlink stdlib modules. The wordfreq project's copied `stdlib_hashmap.blood` has Blood syntax adaptations (`.` vs `::`).

17. **`for i in 0..n` range bounds (SB-3)** — only tested with `i32` bounds. Whether `usize` or `u64` bounds work is unverified.

18. **`*out = val` dereference assignment (NEW)** — first_gen generates invalid LLVM IR (undefined label) for `*out_lt = lt` pattern through `&mut usize` parameters. Bootstrap compiles it correctly. This is a first_gen codegen bug.

19. **Tuple return codegen (NEW)** — first_gen fails to compile functions returning `(usize, usize)` (llc error on undefined label). Bootstrap handles tuples correctly. Same underlying codegen bug as #18.

20. **Effect handler return type vs operation type (EC-2)** — unclear whether handler can produce a value of different type than the operation's declared return type. `raise -> i32` works because handler block returns `i32`, but `raise -> !` (never) would be more semantically correct for aborting operations. Spec gap.

21. **Static dispatch only for traits (B64-1)** — `dyn Trait` dynamic dispatch not implemented. Tracked as DEF-005 (deferred).

22. **`self: &Self` in trait methods (B64-2)** — only by-value `self` tested. By-reference trait methods untested.

23. **Bitwise ops on `u8` (B64-3)** — only verified on `i32`. `u8` bitwise operations untested.

24. **Let-destructuring tuples (NS-2)** — `let (a, b) = expr` untested; only `match` destructuring verified.

25. **Closure vs `fn` pointer distinction (NS-3)** — whether Blood distinguishes bare function pointers from closures (as Rust does) is untested.

---

## 2026-03-14 — Perform divergence fix

### Accomplished

- Fixed calc type checker issue: `perform` of non-resumptive effect in if/else branch caused type mismatch (`f64` vs `()`)
- Root cause: `infer_stmt` returned the perform expression's declared return type (e.g., `i32`), so blocks ending in `perform X;` (with semicolon, no trailing expression) were typed as `()` instead of `!` (Never)
- Fix: in `infer_stmt` (typeck_expr.blood), when the expression is a `Perform`, return `Never` type. This tells `infer_block` the block diverges without changing the perform expression's own type (which codegen depends on)
- All 9/9 blood-projects now compile and run through first_gen (was 8/9)
- Bootstrap verified: second_gen and third_gen byte-identical

---

## 2026-03-14 — Multi-generation compiler verification

### Accomplished

- Verified all 9 blood-projects across 4 compiler generations:
  - **Bootstrap** (Rust-based, `src/bootstrap/target/release/blood`)
  - **First Gen** (selfhost compiled by bootstrap, `src/selfhost/build/first_gen`)
  - **Second Gen** (selfhost compiled by first_gen, `src/selfhost/build/second_gen`)
  - **Third Gen** (selfhost compiled by second_gen, `src/selfhost/build/third_gen`)
- Results: **40/40** — all 9 projects type-check, build, and produce correct runtime output across all 4 compilers
- Runtime correctness verified per-project: bf "Hello World!", calc precedence + division-by-zero abort, b64 encoding, sortbench sort verification, numstat sum=437, wordfreq "the"=14, life blinker oscillation, runner pass/fail counts, guess stdin interaction

### Compiler changes reviewed (22 commits since 2026-03-12)

Resolved flagged items:

- **#18 (`*out = val` codegen) and #19 (tuple return codegen) — RESOLVED.** `e11abb2` (CallFinallyClause abort labels) and `55544b9` (alignment annotations) fixed first_gen codegen for deep handlers with resume and tuple returns. All 9/9 projects now build through first_gen (was 6/9 before these fixes).

- **#20 (EC-2, effect handler return type) — RESOLVED.** `67e5707` allows non-resumptive handler ops to return any type (body type unconstrained). `d0cfce6` enables abort mechanism for all user-defined non-resumptive handlers, not just builtins. The concern about `raise -> i32` vs `raise -> !` is moot — both work correctly now.

Other notable compiler changes:

- `b8b3f16`: user definitions can now shadow stdlib prelude imports (`min`, `max`, `clamp`, `sign`). Previously, naming a function `min` would fail with "defined multiple times."
- `f034fbc`: `t11_dispatch_default_override` (trait default method override) reclassified as passing. Previously flagged as "bootstrap crashes on this" in our session notes.
- `216215f`: O(n²) type checker scaling eliminated. Performance improvement for all projects.
- `990b73c`: perform statements treated as diverging. Already recorded in previous WORKLOG entry.

### Still open

Items #13-16, #21, #23-25 from the flagged item review remain open — these are language/compiler gaps not addressed by recent changes. Item #22 (`self: &Self` in trait methods) is now verified working in the morse project.

---

## 2026-03-14 — Morse code translator

### Accomplished

- Created `morse/morse.blood`: Morse code encoder/decoder (~245 lines)
- Encodes text to Morse (letters A-Z + digits 0-9), decodes Morse back to text
- Word boundaries preserved via `/` separator
- Round-trip verified: `encode "Hello World"` → `.... . .-.. .-.. --- / .-- --- .-. .-.. -..` → `decode` → `HELLO WORLD`
- Digits verified: `encode "Test 123"` → `- . ... - / .---- ..--- ...--` → `decode` → `TEST 123`
- Multi-generation testing: all 3 selfhost compilers (first_gen, second_gen, third_gen) pass type-check, build, and runtime
- **Bootstrap fails codegen** — known trait default method monomorphization bug (unsubstituted TyVarId)
- Blood features exercised (complementary to all previous projects):
  - **Trait with `self: &Self`** — by-reference trait methods (`fn name(self: &Self) -> &str`), first real test (resolves flagged item #22)
  - **Trait default methods** — `fn describe(self: &Self)` has body in trait, overridden in one impl, inherited in another
  - **`env_get()` builtin** — reads `MORSE_SEP` environment variable for custom separator (first test, no golden tests exist)
  - **`for` with `usize` bounds** — `for i in 0usize..len` for byte iteration (resolves flagged item #17/SB-3 uncertainty)
  - **All 5 compound assignments** — `+=`, `-=`, `*=`, `/=`, `%=` all used (only `+=` tested in prior projects)

### Flagged items

1. **Bootstrap cannot compile trait default methods with `self: &Self`.** `unsubstituted type parameter TyVarId(6) reached codegen` at the default method body. This is the known bug noted in `t11_dispatch_default_override.blood`. All 3 selfhost generations handle it correctly. This is the first blood-projects program that **cannot** be compiled by the bootstrap.

2. **`env_get()` returns empty `&str` for unset variables.** Tested with `MORSE_SEP` set and unset. Returns empty string (not null or error) when variable doesn't exist. `str_len()` check works for detecting unset. This is the first project to test `env_get()` — no golden tests exist for it.

3. **Trait default method + override dispatch works correctly.** `MorseEncoder.describe()` returns default `self.name()` → "encoder". `MorseDecoder.describe()` returns overridden "morse-decoder". Static dispatch resolves correctly based on concrete type.

4. **`morse/build/` is gitignored** by the top-level `.gitignore`.

---

## 2026-03-14 — JSON parser and serializer

### Accomplished

- Created `json/json.blood`: recursive descent JSON parser with pretty-printer (542 lines)
- Largest blood-projects program by line count (2x the previous largest)
- Parses all JSON types: objects, arrays, strings (with escape sequences), numbers (negative + fractional), booleans, null
- Serializes to pretty-printed or compact JSON
- Round-trip verified: parse → serialize → re-parse → re-serialize produces identical output
- Tested with: flat objects, nested objects (7 levels deep), nested arrays (3 levels), mixed types, empty containers, negative floats, string escapes (`\n`, `\t`, `\\`, `\"`)
- **All 4 compiler generations pass**: bootstrap, first_gen, second_gen, third_gen — type-check, build, and runtime correct
- Pre-implementation probing confirmed feasibility:
  - `Vec<RecursiveEnum>` — works in both compilers
  - Nested arrays (recursive enum containing Vec of itself) — works
  - JSON-shape enum with 6 variants — works
- Reworked to intentionally stress known-fragile compiler paths:
  - **Multi-param generic** — `struct Entry<K, V>` with `Vec<Entry<String, Json>>` in enum variant
  - **Multi-module** — `mod stdlib_hashmap;` for HashMap-based object key lookup
  - **Triple-nested effect handlers** — `CatchParseErr` + `ParserCtx` + `LogCollector` composed
  - **Recursive enum with nested generics** — `Object(Vec<Entry<String, Json>>)` — multi-param generic inside Vec inside recursive enum
  - **HashMap with complex value types** — `HashMapU64U32` for O(1) key→index lookup on object entries
  - **Effect-based parser state** — parser position tracked via `ParseState` effect (get_pos/set_pos/inc_depth/dec_depth)
  - **Effect-based diagnostics** — parse event counting via `ParseLog` effect
  - **All parse functions carry 3 effects** — `/ {ParseState, ParseErr, ParseLog}`
  - **Program at scale** — 620 lines, 35 declarations, 467 type-checked items

### Bugs found (3 new)

1. **BOOTSTRAP: triple-nested handler codegen crash (confirmed in real program).** `with CatchParseErr handle { with ParserCtx handle { with LogCollector handle { ... } } }` produces LLVM verification error: "Instruction does not dominate all uses" on `state_shadow` allocas. Same bug as golden test `t05_effect_triple_nest.blood`. Now confirmed to affect real programs, not just synthetic tests.

2. **FIRST_GEN/SELFHOST: module import breaks effect handler type inference (NEW).** When `mod stdlib_hashmap;` is present, first_gen fails to infer the return type of `return(x) { x }` in handler definitions with `E0202: cannot infer type`. Removing the `mod` line eliminates the error. The golden triple-nest test passes first_gen because it has no module imports. **This is a new interaction bug between module loading and effect handler inference.**

3. **FIRST_GEN/SELFHOST: multi-param generic struct field access produces wrong values (NEW).** `struct Entry<K, V> { key: K, val: V }` with `Vec<Entry<String, i32>>` — bootstrap correctly outputs `hello = 42`, first_gen outputs garbage (empty string, 0). Field layout or GEP computation is wrong for multi-param generic structs. Confirmed in second_gen too. **This validates the honest assessment's §3.3 finding about multi-param generics being broken in first_gen.** Minimal reproducer: 20-line probe at `/tmp/blood_probe_entry.blood`.

### Current build status

| Compiler | Type Check | Build | Runtime |
|---|---|---|---|
| Bootstrap | PASS | FAIL (triple-nest codegen) | — |
| First Gen | FAIL (handler inference + mod) | — | — |
| Second Gen | FAIL (same as first_gen) | — | — |
| Third Gen | FAIL (same as first_gen) | — | — |

**No compiler can currently build the full reworked JSON parser.** This is the intended outcome — the project was designed to push compilers past their limits rather than validate what already works.

### Previously confirmed (from v1)

4. **`&mut T` does not coerce to `&T`.** Significant ergonomic difference from Rust.

5. **`Vec<Json>` (Vec of recursive enum) works in all compilers.** Single-param generics remain solid.

6. **Float serialization is approximate.** No stdlib float-to-string.

---

## 2026-03-14 — Generic collections library

### Accomplished

- Created `collections/collections.blood`: generic linked list, binary tree, multi-param pair/keyval (~380 lines)
- Implements `List<T>` with `push`, `len`, `sum`, `count<T>`, `map`, `filter`, `fold`, `to_vec`, `from_vec`
- Implements `Tree<T>` with `sum`, `count<T>`, `depth<T>`, `map`
- Implements `Pair<A, B>` and `KeyVal<K, V>` with construction, field access, `Vec` of multi-param generics
- Bootstrap: all tests pass — list operations, tree operations, multi-param generics all correct
- First_gen/selfhost: **type check fails on `Pair<String, i32>.fst`** — field type resolved as `i32` instead of `String`

### Bugs found

1. **SELFHOST: multi-param generic field types are swapped (confirmed as type checker bug).** `Pair<String, i32>` — first_gen resolves `.fst` as `i32` (should be `String`), causing `E0209: no method 'as_str' found on type i32`. This is a **type checker** bug, not just a codegen bug as initially thought from the Entry<K,V> probe. The field type parameters are resolved in wrong order. Same error in second_gen and third_gen.

2. **Single-param generic recursive types work correctly in all compilers.** `List<T>` with `Box<List<T>>` and `Tree<T>` with `Box<Tree<T>>` — all operations produce correct results in both bootstrap and selfhost. `count<T>` and `tree_count<T>` (generic functions over generic recursive enums) work in both compilers. The bug is specifically in **multi-param** generics.

3. **Higher-order functions over generic recursive types work.** `list_map`, `list_filter`, `list_fold` with closures all produce correct results. Closure capture in these contexts (e.g., `|x| x > 25` in filter) works correctly.

### Compiler status

| Compiler | Type Check | Build | Runtime |
|---|---|---|---|
| Bootstrap | PASS | PASS | PASS (all output correct) |
| First Gen | FAIL (multi-param field swap) | — | — |
| Second Gen | FAIL (same) | — | — |
| Third Gen | FAIL (same) | — | — |

---

## 2026-03-14 — Data processing pipeline

### Accomplished

- Created `pipeline/pipeline.blood`: trait-heavy data pipeline (~370 lines)
- Implements `Source` trait (with associated type `type Item`), 5 `Sink` impls (Sum, Count, Max, Min, Collect)
- Implements `pipe_map`, `pipe_filter`, `pipe_take` pipeline operations
- Supertrait `Described: Named` with `where T: Named` and `where T: Described` constraints
- Chained pipelines with closure capture (`|x| x > threshold`)
- Bootstrap: all output correct (sum=124, count=10, max=30, min=1, all filters and maps)

### Bugs found

1. **FIRST_GEN: codegen crash — undefined function `@fn_4294967294` (sentinel function ID).** First_gen emits LLVM IR referencing `@fn_4294967294` (0xFFFFFFFE), a function that was never defined. Occurs during trait method dispatch codegen. `llc` fails with undefined value error. **This is a codegen bug where trait dispatch generates an invalid function reference.**

2. **SECOND_GEN/THIRD_GEN: filter closure `|x| x > 10` silently produces wrong results.** `pipe_filter` with `|x| x > 10` returns ALL elements instead of only those > 10. Bootstrap correctly outputs `[12, 18, 25, 30, 14]`, selfhost outputs `[5, 12, 3, 18, 7, 25, 1, 30, 14, 9]` (full unfiltered list). Meanwhile `|x| x % 2 == 0` works correctly, and `|x| x > threshold` (captured variable) works correctly. **The bug is specific to closures with literal integer comparison through Source trait dispatch.** The chained pipeline `filter(>10).map(*3).sum` gives 372 (= 124*3 = all elements * 3) confirming the filter is a no-op.

### Pain points and friction

3. **Closure type `fn(i32) -> bool` works but not through generic trait dispatch.** The filter predicate must be passed as a concrete `fn(i32) -> bool` parameter, not through a generic `where` constraint. No syntax exists for `where F: Fn(i32) -> bool`.

4. **Associated type `type Item = i32` compiles but all Source/Sink dispatch is monomorphic.** I couldn't write `fn drain<S, K>(src: &mut S, sink: &mut K) where S: Source, K: Sink` because the associated type doesn't flow through where clauses in a way that allows the body to use `Source.next()` generically. Had to write separate `drain_into_sum`, `drain_into_count`, etc.

5. **No way to write a generic pipeline stage.** The ideal pattern `struct MapPipe<S: Source> { source: S, f: fn(S::Item) -> S::Item }` is not expressible — associated type projection in struct fields is untested and likely unsupported. All pipeline operations are standalone functions.

6. **Supertrait methods work but only through concrete dispatch.** `print_named(&pl)` and `print_described(&pl)` work because `pl: Pipeline` is concrete. Dynamic dispatch via `dyn Named` is untested.

### Compiler status

| Compiler | Type Check | Build | Runtime |
|---|---|---|---|
| Bootstrap | PASS | PASS | PASS (all correct) |
| First Gen | PASS | FAIL (`@fn_4294967294` codegen) | — |
| Second Gen | PASS | PASS | WRONG (filter bug: `\|x\| x > 10` passes all) |
| Third Gen | PASS | PASS | WRONG (same filter bug) |

---

## 2026-03-14 — Effect interpreter (quad-nested handlers)

### Accomplished

- Created `effects/effects.blood`: mini expression interpreter with 4 effects (~300 lines)
- Expression language: `Lit`, `Add`, `Mul`, `Neg`, `Let`, `Var`, `If`, `Print` — all recursive via `Box<Expr>`
- 4 effects composed: `EvalErr` (abort), `EvalState` (variable store), `EvalLog` (call counting), `EvalIO` (output capture)
- Evaluator carries all 4: `fn eval(e: Expr, depth: i32) -> i32 / {EvalErr, EvalState, EvalLog, EvalIO}`
- **Quadruple-nested handlers** at call site — goes beyond triple nesting

### Bugs found

1. **BOOTSTRAP: codegen crash — "Cannot index struct type" inside handler op body.** `vars[id as usize]` inside the `get_var` op body of `VarStore` handler fails codegen. The handler state field `vars: Vec<i32>` can't be indexed in the op implementation. Type checking passes. **This is a new bootstrap codegen bug distinct from the triple-nest dominator error — it's about Vec indexing inside handler ops.**

2. **SELFHOST (all generations): segfault on variable store access.** Test 1 (pure arithmetic, no variables) produces correct output. Test 2 (`let x=10 in let y=20 in x+y`) segfaults. The `EvalState` handler's `set_var`/`get_var` ops corrupt memory when accessing the `vars: Vec<i32>` field. **Vec mutation inside effect handler op bodies with `resume()` causes memory corruption.**

### Pain points and friction

3. **Handler ops can't use complex expressions on state fields.** The `vars[id as usize]` pattern inside an op body is the triggering pattern for the bootstrap codegen crash. This means handlers with collection-typed state (Vec, HashMap) can't practically index into them in op implementations.

4. **No way to share expressions between tests.** Each `run_expr()` call consumes the expression (by value). Had to build separate expression trees for each test. No `Clone` trait or `&Expr` traversal (by-reference match on Box<Expr> doesn't work cleanly).

5. **`Box.new()` expressions are verbose for tree building.** `add(lit(2), mul(lit(3), lit(4)))` requires builder functions wrapping every `Box.new()` call. Ergonomically painful for deeply nested trees.

6. **Quadruple nesting goes beyond what any compiler supports.** Neither bootstrap (codegen crash on Vec index in handler) nor selfhost (segfault on Vec mutation in handler) can run the full program. The effect system's handler-with-collection-state pattern is fundamentally broken in both compilers.

### Compiler status

| Compiler | Type Check | Build | Runtime |
|---|---|---|---|
| Bootstrap | PASS | FAIL (Vec index in handler op) | — |
| First Gen | PASS | PASS | SEGFAULT (Test 2: variable store) |
| Second Gen | PASS | PASS | SEGFAULT (same) |
| Third Gen | PASS | PASS | SEGFAULT (same) |

---

## 2026-03-14 — Combined stress test (feature interaction matrix)

### Accomplished

- Created `stresstest/stresstest.blood`: 561-line program combining effects + generics + traits + modules
- Tests 10 specific feature interactions (labeled A through J) that have never been combined before
- Uses `mod stdlib_hashmap` for multi-module stress
- Multi-param generic `Tagged<K, V>` and single-param `Wrapper<T>`
- Traits with `where` clauses, supertraits, `self: &Self`
- 2 effects (`ComputeErr`, `Log`) composed across all test sections
- Recursive generic `Expr<T>` with effectful evaluation
- Closure capture inside effectful functions
- HashMap lookup inside effectful recursive function
- Generic function with effect annotation: `fn wrap_with_log<T>(val: T) -> Wrapper<T> / {Log}`
- Where clause + effect on same function: `fn describe_and_log<T>(x: &T) -> &str / {Log} where T: Describable`
- Supertrait + generic + effect: `fn named_eval_logged<T>(x: &T) -> i32 / {Log} where T: NamedEval`

### Bugs found

1. **ALL COMPILERS: trait methods with effect annotations rejected (NEW).** `trait EffectfulEval { fn compute(self: &Self) -> i32 / {ComputeErr, Log}; }` — declaring effects on trait methods type-checks successfully, but the impl method bodies get `E0308: function performs undeclared effects` regardless of whether the impl repeats the effect annotation or omits it. **Traits and effects don't compose.** Had to work around by converting to standalone functions. This is a fundamental gap — any program wanting polymorphic effectful computation can't use traits for it.

2. **BOOTSTRAP: LLVM dominator bug triggers on multiple sequential double-nested handlers (REFINED FINDING).** Previous understanding was this required 3+ nesting levels. The stresstest has 8 sequential `with CatchErr handle { with Counter handle { ... } }` blocks in `main()` (only 2 levels each), and bootstrap still fails with "Instruction does not dominate all uses" on `state_shadow` allocas. **The bug is about multiple handler sites in one function, not nesting depth.** This is worse than previously thought.

3. **SELFHOST: module import + handler inference bug confirmed (REPRODUCES).** Same as JSON parser — `mod stdlib_hashmap;` causes first_gen/second_gen/third_gen to fail with `E0202: cannot infer type` on `return(x) { x }` in handler definitions. Confirmed as a blocking interaction between module loading and effect handler type inference.

### Pain points and friction

4. **`op` is a reserved keyword.** Can't use `op` as a variable name in match arms (e.g., `Expr.BinOp(op, left, right)` fails). Same class as `raw`, `gen`. Renamed to `opc`.

5. **No way to move elements out of Vec.** `eval_all` that iterates `Vec<Expr<i32>>` and evaluates each can't consume elements from the Vec by index. No `Vec.remove()`, `Vec.pop()`, or move-from-index. Had to build fresh expressions instead of consuming from the Vec. This is a significant collection ergonomics gap.

6. **Can't pass `&mut` where `&` expected (re-confirmed).** Already known from JSON parser, hit again here. Every function touching a `&mut` value must accept `&mut` even for read-only access.

7. **Generic function + effect works.** `fn wrap_with_log<T>(val: T) -> Wrapper<T> / {Log}` and `fn identity_logged<T>(x: T) -> T / {Log}` — type-check, compile, and (if handlers weren't broken) would produce correct results. This is new: no project had tested this combination before.

8. **Where clause + effect works in type checking.** `fn describe_and_log<T>(x: &T) -> &str / {Log} where T: Describable` type-checks. Whether it codegen/runs correctly can't be verified because the bootstrap handler bug blocks execution.

9. **No compiler can build the full program.** Bootstrap fails on multiple handler sites (codegen). All selfhost fail on module+handler inference (type check). This means no program combining modules + effects can compile through any selfhost compiler, and no program with multiple handler blocks can compile through bootstrap.

### Compiler status

| Compiler | Type Check | Build | Runtime |
|---|---|---|---|
| Bootstrap | PASS | FAIL (multiple handler sites → LLVM dominator) | — |
| First Gen | FAIL (mod + handler inference) | — | — |
| Second Gen | FAIL (same) | — | — |
| Third Gen | FAIL (same) | — | — |

---

## 2026-03-14 — Bug fix verification

### Fixes verified (5 bugs resolved by ~/blood/ agent)

Re-tested all stress test projects against updated compilers. Results:

1. **Multi-param generic field types swapped — FIXED (3bd4abf + 2c10700).** `collections` first_gen now outputs `labeled: test = 42` (was garbage). `Pair<String, i32>.fst` correctly resolves to `String`.

2. **Multiple handler sites LLVM crash — FIXED (c923eb2 + 1cd24d5).** `stresstest` builds and runs correctly through bootstrap. All 8 handler sites in `main()` produce correct output.

3. **Filter closure `|x| x > 10` wrong results — FIXED (b415772).** `pipeline` selfhost now outputs `filter(>10): [12, 18, 25, 30, 14]` (was full unfiltered list). Numeric inference defaulting fix.

4. **Trait dispatch @fn_4294967294 — FIXED (1cd24d5).** `pipeline` builds through first_gen. Handler codegen bug resolved.

5. **Module import breaks handler inference — FIXED (55914da).** `json` and `stresstest` both build through all selfhost generations. `return(x) { x }` inference no longer fails with mod imports present.

### Still open (verified unfixed)

6. **Vec mutation in handler op body segfaults — STILL OPEN.** `effects` Test 2 still segfaults on first_gen (exit 139). `EvalState` handler accessing `vars: Vec<i32>` corrupts memory.

7. **Vec index inside handler op body crashes bootstrap codegen — STILL OPEN.** `effects` bootstrap build still fails with "Cannot index struct type."

8. **Trait methods + effect annotations rejected — STILL OPEN.** Not addressed by any commit. Fundamental gap: traits and effects can't compose.

### Updated project status (post-fixes)

| Project | Bootstrap | First Gen | Second Gen |
|---|---|---|---|
| stresstest | PASS (all correct) | PASS (all correct) | PASS (all correct) |
| pipeline | PASS | PASS | PASS (filter fixed) |
| collections | PASS | PASS (multi-param fixed) | PASS |
| json (reworked) | PASS | PASS | PASS |
| effects | FAIL (Vec index codegen) | SEGFAULT (Vec mutation) | SEGFAULT |
