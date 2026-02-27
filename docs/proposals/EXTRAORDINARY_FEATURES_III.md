# Blood: Features That Could Make It Extraordinary — Part III

**Status:** Research & Ideation
**Date:** 2026-02-26
**Prerequisite:** Read `EXTRAORDINARY_FEATURES.md` (Proposals 1–7) and `EXTRAORDINARY_FEATURES_II.md` (Proposals 8–15) first.

---

## Context

Parts I and II proposed features exploiting Blood's four pillars for verification, security, performance, observability, and reliability. Those proposals targeted *what Blood programs can do*.

This document extends the set with **Proposals 16–23**, discovered through systematic analysis of the AI/LLM + programming language design frontier (February 2026). These proposals target a different question:

> **How do we make Blood the best language for AI to write, verify, and reason about?**

The research basis is extensive. Key findings driving these proposals:

| Finding | Source | Year |
|---------|--------|------|
| 94% of LLM compilation errors are type errors | GitHub Blog | 2025 |
| Type-constrained decoding cuts LLM errors by 50%+ | ETH Zurich, PLDI | 2025 |
| Claude drops from 29% → 3% accuracy at 32K → 256K context | LongCodeBench | 2025 |
| 2.6x token efficiency gap between languages | Alderson | 2025 |
| Single-file AI tasks ~90% solved; multi-file ~54% | Ganhotra | 2025 |
| Dafny verification success: 68% → 96% in 15 months | Vericoding, POPL | 2026 |
| Haskell/F# match dynamic-language token efficiency with full static typing | Alderson | 2025 |
| LLMs write 88% imperative code even in Haskell | FPEval | 2026 |
| MoonBit is the only language designed for AI generation | LLM4Code, ICSE | 2024 |
| Algebraic effects are the right abstraction for compound AI systems | Pangolin, LMPL | 2025 |

The unifying insight:

> **Blood's four pillars independently appear in the research as the most AI-beneficial language features. No language combines them. No language has been designed to be simultaneously AI-native, verification-aware, and content-addressed. Blood can be the first.**

MoonBit optimized syntax for AI but lacks effects and content-addressing. Dafny optimized verification for AI but lacks content-addressing and effects. Unison has content-addressing but no effect system or verification. Lean 4 has dependent types but no content-addressing or algebraic effects.

### Combined Proposal Map (Parts I, II, and III)

| # | Proposal | Document | Pillars | Priority |
|---|----------|----------|---------|----------|
| 1 | Compile-Time WCET Analysis | Part I | Effects | 4 |
| 2 | Session Types for Protocol Safety | Part I | Effects + CAS | 3 |
| 3 | Automatic Memoization via Content-Addressing | Part I | CAS + Effects | 6 |
| 4 | Capability-Based Security via Effects | Part I | Effects | 2 |
| 5 | Effect-Guided Automatic Parallelization | Part I | Effects + Gen | 5 |
| 6 | Provenance Tracking for Compliance | Part I | CAS + Effects | 7 |
| 7 | Gradual Verification (Lightweight Proofs) | Part I | Effects + CAS + Gen | 1 |
| 8 | Deterministic Simulation Testing | Part II | Effects + CAS + Gen + MD | A |
| 9 | Taint Tracking via Effects | Part II | Effects + CAS | B |
| 10 | Proof-Carrying Code | Part II | CAS + Verification | C |
| 11 | Automatic Semantic Versioning | Part II | Effects + CAS | D |
| 12 | Deterministic Replay Debugging | Part II | Effects + CAS + Gen | E |
| 13 | Zero-Code Observability | Part II | Effects + CAS | F |
| 14 | Choreographic Programming | Part II | Effects + CAS | G |
| 15 | Compile-Time Complexity Bounds | Part II | Effects + CAS | H |
| 16 | Type-and-Effect Constrained Decoding Oracle | **Part III** | Effects + Types | **A** |
| 17 | Machine-Readable Structured Diagnostics | **Part III** | Toolchain | **B** |
| 18 | Content-Addressed Verification Cache | **Part III** | CAS + Verification | **A** |
| 19 | Compact Module Signatures as AI Context | **Part III** | Effects + CAS | **B** |
| 20 | First-Class Specification Annotations | **Part III** | Effects + CAS + Verification | **A** |
| 21 | AI-Optimized Syntax Decisions | **Part III** | Syntax | **C** |
| 22 | Toolchain-Integrated Dependency Graph API | **Part III** | CAS + Toolchain | **B** |
| 23 | Effect Handlers as AI Agent Middleware | **Part III** | Effects + CAS + Gen + MD | **C** |

Legend: CAS = Content-Addressed Storage/Code, Gen = Generational Memory, MD = Multiple Dispatch

---

## Proposal 16: Type-and-Effect Constrained Decoding Oracle

### The Problem

LLMs generate tokens left-to-right. When they produce a type error at token 47, they have already committed to a wrong path at tokens 1–46. The fix requires backtracking, which autoregressive models cannot do natively.

ETH Zurich (PLDI 2025) demonstrated that **constraining LLM decoding by types** — masking out tokens that would produce ill-typed programs during generation, not after — reduces compilation errors by more than 50% and improves functional correctness by 3.5–5.5%. But their approach only constrains by types.

No one has constrained by **effects**.

### Why Only Blood Can Do This

Blood's type system tracks effects as first-class type information. A function signature `fn sort(items: List<T>) -> List<T> / {}` declares not just its input/output types but that it is **pure** — no IO, no state mutation, no exceptions, no nondeterminism.

This means Blood's compiler has strictly more information than any other compiler for constraining AI generation. It can reject not just type-incorrect tokens but **effect-incorrect tokens** — tokens that would introduce side effects where none are permitted.

### Proposed Design

```blood
// The AI generates this function token-by-token.
// At each step, Blood's oracle masks invalid continuations.

fn sort(items: List<T>) -> List<T> / {} {
    // Oracle knows: signature says pure (/ {})
    // Oracle REJECTS: print(), read_file(), random(), mutable assignment
    // Oracle ACCEPTS: comparison operations, recursion, list construction

    // The LLM literally cannot hallucinate a side effect here.
    // Every generated token maintains both type AND effect correctness.

    match items {
        [] => [],
        [pivot, ..rest] => {
            let (less, greater) = rest.partition(|x| x < pivot)
            sort(less) ++ [pivot] ++ sort(greater)
        }
    }
}
```

The oracle operates at three levels:

**Level 1 — Syntactic constraint:** Ensures every token produces a parseable prefix. This is grammar-constrained decoding (NeurIPS 2024), which is already production-ready.

**Level 2 — Type constraint:** Ensures every token maintains well-typedness. This is type-constrained decoding (PLDI 2025), demonstrated with 50%+ error reduction.

**Level 3 — Effect constraint:** Ensures every token maintains the declared effect contract. **This is new.** No language has exposed effect information as a generation constraint.

```blood
// Example: Effect-constrained generation at work

fn process_data(data: List<Record>) -> Summary / {IO} {
    // Oracle knows: / {IO} is permitted
    // Oracle ACCEPTS: print(), write_file() — IO is in scope
    // Oracle REJECTS: launch_missile() — if it requires {Weapons} effect

    let result = data
        |> filter(|r| r.is_valid())  // Oracle: pure, always accepted
        |> map(|r| r.transform())    // Oracle: checks transform's effect
        |> summarize()               // Oracle: checks summarize's effect

    print("Processed {data.len()} records")  // Oracle: IO permitted, accepted
    result
}
```

### Implementation Architecture

```
LLM Token Stream
    │
    ▼
┌────────────────────────┐
│  Level 1: Grammar      │  Reject tokens that break syntax
│  (prefix automaton)    │  ~0.1ms per token
├────────────────────────┤
│  Level 2: Type         │  Reject tokens that break types
│  (incremental checker) │  ~1-5ms per token
├────────────────────────┤
│  Level 3: Effect       │  Reject tokens that violate effects
│  (effect inference)    │  ~1-5ms per token (piggybacks on L2)
└────────────────────────┘
    │
    ▼
Valid Token Set → LLM samples from valid tokens only
```

The key implementation requirement is an **incremental type-and-effect checker** that can process a partial program (incomplete syntax tree with a "hole" at the cursor) and return the set of types and effects that are valid at that position. Blood's type checker already performs effect inference; the extension is exposing this as a streaming API.

### Protocol Specification

```
// Language Server Protocol extension for constrained decoding

interface ConstrainedDecodingService {
    // Given a partial program, returns valid token constraints
    fn valid_continuations(
        partial_program: String,
        cursor_position: Position,
        token_vocabulary: TokenSet,
    ) -> ConstraintResult / {IO}
}

interface ConstraintResult {
    valid_tokens: TokenSet,           // Tokens that maintain all invariants
    expected_type: Option<Type>,      // What type is expected here
    permitted_effects: EffectSet,     // What effects are in scope
    in_scope_names: List<(Name, Type, EffectSet)>,  // Available bindings
}
```

### Competitive Landscape

| System | Grammar | Types | Effects | Status |
|--------|---------|-------|---------|--------|
| Grammar-Constrained Decoding (NeurIPS 2024) | Yes | No | No | Production |
| PICARD (Scholak et al.) | Yes | Partial | No | Production |
| Synchromesh (Microsoft) | Yes | Yes | No | Research |
| Type-Constrained Decoding (ETH, PLDI 2025) | Yes | Yes | No | Research |
| **Blood Constrained Decoding Oracle** | **Yes** | **Yes** | **Yes** | **Proposed** |

Blood would be the first system to constrain AI generation at all three levels simultaneously. The effect constraint is novel and exploits Blood's unique type system.

### Impact Estimate

Based on the existing research:
- Grammar constraints eliminate ~8% of LLM bugs (syntax errors)
- Type constraints eliminate ~30% of LLM bugs (wrong type, wrong attribute, hallucinated object)
- Effect constraints would eliminate an additional estimated ~10-15% (unintended IO, state mutation, missing error handling)

Combined: **~50% of all LLM code generation bugs eliminated before the code is even fully generated.**

### References

- [Type-Constrained Code Generation — ETH Zurich, PLDI 2025](https://arxiv.org/abs/2504.09246)
- [Grammar-Aligned Decoding — NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/file/2bdc2267c3d7d01523e2e17ac0a754f3-Paper-Conference.pdf)
- [PICARD: Parsing Incrementally for Constrained Decoding](https://arxiv.org/abs/2109.05093)
- [Synchromesh: Reliable Code Generation from Pre-trained Language Models — Microsoft](https://www.microsoft.com/en-us/research/wp-content/uploads/2022/01/csd_arxiv.pdf)
- [MoonBit: AI-Friendly PL Design — LLM4Code @ ICSE 2024](https://dl.acm.org/doi/10.1145/3643795.3648376)
- [Koka: Programming with Row-polymorphic Effect Types](https://arxiv.org/pdf/1406.2061)

---

## Proposal 17: Machine-Readable Structured Diagnostics

### The Problem

Anthropic's C compiler project (16 parallel Claude instances, 100K lines of Rust) produced the most instructive finding in AI-assisted development: **"Most effort went into designing the environment, not into the programming itself."** The quality of compiler error output determined whether agents succeeded or failed.

94% of LLM compilation errors are type errors (GitHub 2025). Agents need structured feedback to fix them efficiently. Current compiler diagnostics are designed for humans — colorized text, source snippets, suggestive prose. Agents must parse natural language error messages, losing tokens and accuracy at each step.

### Why Blood Should Do This From Day One

Blood has a unique opportunity. Every other language added machine-readable diagnostics as an afterthought (Rust's `--error-format=json`, TypeScript's `--pretty false`). Blood can design its diagnostic protocol for dual consumption — human and machine — from the start. More importantly, Blood is the only language whose diagnostics include **effect information**, which is critical for AI agents.

### Proposed Design

Every compiler diagnostic is natively structured with dual output modes:

```blood
// Human-readable output (default):

error[E0308]: effect mismatch
  --> src/main.blood:12:5
   |
 3 | fn sort(items: List<Int>) -> List<Int> / {} {
   |                                          -- declared pure here
   |
12 |     print("debug: sorting {items.len()} items")
   |     ^^^^^ introduces IO effect
   |
   = expected: pure (/ {})
   = found:    / {IO}
   = help: move print() outside sort(), or change signature to / {IO}
```

```json
// Machine-readable output (--diagnostics=json):

{
  "code": "E0308",
  "severity": "error",
  "category": "effect_mismatch",
  "message": "function declared pure but body introduces IO effect",
  "primary": {
    "file": "src/main.blood",
    "line": 12,
    "column": 5,
    "end_column": 10,
    "text": "print(\"debug: sorting {items.len()} items\")"
  },
  "expected": {
    "type": null,
    "effects": []
  },
  "found": {
    "type": null,
    "effects": ["IO"]
  },
  "provenance": [
    {
      "file": "src/main.blood",
      "line": 3,
      "column": 43,
      "reason": "function signature declares pure (/ {})"
    }
  ],
  "fixes": [
    {
      "id": "add_effect",
      "description": "Add IO effect to function signature",
      "edits": [
        {
          "file": "src/main.blood",
          "line": 3,
          "old": "/ {}",
          "new": "/ {IO}"
        }
      ],
      "consequence": "Callers must also handle IO effect"
    },
    {
      "id": "remove_call",
      "description": "Remove effectful call",
      "edits": [
        {
          "file": "src/main.blood",
          "line": 12,
          "old": "    print(\"debug: sorting {items.len()} items\")",
          "new": ""
        }
      ],
      "consequence": "Debug output will be removed"
    }
  ]
}
```

### Diagnostic Categories

Blood diagnostics are organized into categories that AI agents can route programmatically:

| Category | Error Code Range | AI Fix Strategy |
|----------|-----------------|-----------------|
| `type_mismatch` | E0300–E0399 | Change expression to match expected type |
| `effect_mismatch` | E0400–E0499 | Add effect to signature or remove effectful call |
| `missing_case` | E0500–E0599 | Add missing pattern match arms |
| `contract_violation` | E0600–E0699 | Strengthen implementation to meet contract |
| `unresolved_name` | E0100–E0199 | Add import or fix typo |
| `borrow_error` | E0700–E0799 | Restructure ownership |
| `unused_binding` | W0100–W0199 | Remove or prefix with underscore |
| `deprecated` | W0200–W0299 | Replace with suggested alternative |

Each category maps to a deterministic fix strategy. An AI agent receiving `E0308` (effect mismatch) knows to either modify the function signature or remove the effectful expression — without parsing the English message.

### Key Design Decisions

**1. Constraint provenance chains.** Not just "type mismatch" but *where the constraint originated* and *why*. This is critical for AI agents making multi-file changes — the error is at line 12, but the fix might be at line 3 in a different file.

```json
"provenance": [
  {
    "file": "src/sort.blood",
    "line": 3,
    "reason": "function signature declares pure"
  },
  {
    "file": "src/traits.blood",
    "line": 47,
    "reason": "trait Sortable requires pure comparison"
  }
]
```

**2. Fix suggestions as structured diffs.** Every diagnostic that has a known fix includes the fix as a structured edit. Agents apply fixes programmatically without interpreting prose.

**3. Consequence annotations.** Each fix includes a `consequence` field describing what will change beyond the immediate edit. This is unique — no compiler tells you "if you make this fix, callers must change too."

**4. Stable error codes.** Error codes are part of Blood's public API. They do not change between compiler versions. Agents can build persistent fix strategies indexed by error code.

**5. Incremental diagnostics via LSP.** Errors update in real-time as the agent edits, without requiring a full recompilation cycle. The diagnostic stream is content-addressed: unchanged diagnostics retain their hash, so agents know which errors are new.

### Impact on AI Agent Workflows

The Anthropic C compiler project's agent workflow was:
1. Generate code
2. Compile
3. Parse error output (lossy, expensive in tokens)
4. Generate fix
5. Repeat

With Blood's structured diagnostics:
1. Generate code (constrained by Proposal 16)
2. Compile — receive JSON diagnostics
3. Route by error code to fix strategy (zero parsing overhead)
4. Apply structured fix diff (zero interpretation overhead)
5. Verify fix via incremental diagnostics (sub-second)

Estimated token savings per fix cycle: **60–80%** (no natural language parsing, no source snippet re-reading, no fix interpretation).

### References

- [Building a C Compiler with Parallel Claudes — Anthropic 2025](https://www.anthropic.com/engineering/building-c-compiler)
- [Why AI is Pushing Developers Toward Typed Languages — GitHub 2025](https://github.blog/ai-and-ml/llms/why-ai-is-pushing-developers-toward-typed-languages/)
- [RustAssistant: Using LLMs to Fix Compilation Errors — Microsoft Research 2024](https://www.microsoft.com/en-us/research/wp-content/uploads/2024/08/paper.pdf)
- [LSP Integration Transforms Coding Agents — the/experts 2025](https://tech-talk.the-experts.nl/give-your-ai-coding-agent-eyes-how-lsp-integration-transform-coding-agents-4ccae8444929)
- [Rust Compiler Error Index](https://doc.rust-lang.org/error_codes/error-index.html)

---

## Proposal 18: Content-Addressed Verification Cache for AI Re-Generation Loops

### The Problem

The dominant AI code generation pattern is the re-generation loop:

```
Generate code → Verify → Failure → Regenerate → Verify → ... → Success
```

Every cycle costs time and compute. The vericoding benchmark shows that even at 96% success rate (Dafny, model union), the remaining 4% may require multiple attempts. For Verus/Rust (44%) and Lean (27%), the re-generation loop runs many cycles.

The problem: **verification is expensive, and identical or equivalent code is re-verified from scratch every time.** If an AI generates a sorting function that matches one verified last week in a different project, the verification runs again. If an AI generates code, it fails, and the AI generates the same failing code again (a known LLM failure mode), the verification runs again with the same failure.

### Why Blood's Content-Addressing Solves This

Blood already identifies code by its content hash (BLAKE3). This means:

1. **Identical code has identical hashes** — regardless of when, where, or by whom it was generated.
2. **Verification results can be indexed by hash** — a proof that `sort` with hash `9c4e1d...` satisfies its contract is valid for every occurrence of that hash, forever.
3. **Counterexamples are also indexed by hash** — a failure proof that code with hash `7f3a2b...` violates its contract is instantly available if the same code is re-generated.

### Proposed Design

```blood
// Verification cache structure (conceptual)

VerificationCache {
    // Positive cache: hash → proof artifact
    proofs: Map<ContentHash, ProofArtifact>,

    // Negative cache: hash → counterexample
    failures: Map<ContentHash, Counterexample>,

    // Specification binding: spec_hash × impl_hash → verification_result
    bindings: Map<(SpecHash, ImplHash), VerificationResult>,
}
```

**Scenario 1: Cache hit on verified code**

```blood
// AI generates sort function, Project A, January:
fn sort(items: List<Int>) -> List<Int> / {}
    ensures result.is_sorted()
    ensures result.len() == items.len()
{
    match items {
        [] => [],
        [pivot, ..rest] => {
            let (less, greater) = rest.partition(|x| x < pivot)
            sort(less) ++ [pivot] ++ sort(greater)
        }
    }
}
// Implementation hash: 9c4e1d...
// Spec hash: a1b2c3...
// Verification: PASS (Tier 2, property-based testing, 0.08s)
// Cached: (a1b2c3..., 9c4e1d...) → PASS

// Different AI generates identical sort, Project B, March:
// Implementation hash: 9c4e1d...  ← same hash!
// Spec hash: a1b2c3...            ← same spec!
// Cache lookup: HIT → PASS
// Verification time: 0ms
// Verification cost: $0.00
```

**Scenario 2: Cache hit on failed code**

```blood
// AI generates buggy sort (doesn't handle empty list), attempt 1:
fn sort(items: List<Int>) -> List<Int> / {}
    ensures result.is_sorted()
{
    let pivot = items[0]  // Panics on empty list
    // ...
}
// Implementation hash: 7f3a2b...
// Verification: FAIL
// Counterexample: sort([]) → panic "index out of bounds"
// Cached: 7f3a2b... → FAIL(counterexample)

// Same AI regenerates identical buggy code, attempt 2:
// Implementation hash: 7f3a2b... ← same hash!
// Cache lookup: HIT → FAIL
// Counterexample returned to AI in <1ms
// No re-verification needed
```

**Scenario 3: Specification-aware caching**

```blood
// Same implementation, different specifications:

// Spec A: "ensures result.is_sorted()"
// Spec B: "ensures result.is_sorted() && result.len() == items.len()"

// Spec A (hash: x1) × Impl (hash: 9c4e1d) → PASS
// Spec B (hash: y2) × Impl (hash: 9c4e1d) → needs separate verification

// The cache is keyed on (spec_hash, impl_hash) pairs,
// so the same implementation verified against a weaker spec
// does not count as verified against a stronger spec.
```

### The Compound Effect Over Time

```
Month 1:  AI generates 1,000 functions → 1,000 verifications → 100 unique hashes cached
Month 6:  AI generates 5,000 functions → 2,000 cache hits → 3,000 verifications needed
Month 12: AI generates 10,000 functions → 7,000 cache hits → 3,000 verifications needed
Month 24: AI generates 20,000 functions → 16,000 cache hits → 4,000 verifications needed
```

Common patterns (sorting, searching, string manipulation, data transformation) converge on a small number of optimal implementations. The cache asymptotically covers the most-generated code patterns, and verification cost per function approaches zero for common operations.

### Cache Distribution

Content-addressed caches are naturally distributable:

```
Local Cache (per-project)
    ↓ miss
Organization Cache (shared across team projects)
    ↓ miss
Global Cache (community-shared, opt-in)
    ↓ miss
Full Verification (compute and cache result)
```

The global cache is trustworthy because content-addressing is tamper-evident: the hash of the code is the cache key. If you trust the verification algorithm, you trust any cached result for that hash. (Proposal 10, Proof-Carrying Code, extends this with cryptographic proof certificates.)

### Interaction with Graduated Verification

The cache respects the verification tier hierarchy:

| Tier | Cacheable? | Cache Lifetime | Cache Scope |
|------|-----------|----------------|-------------|
| Tier 1: Static analysis | Yes | Permanent (compiler version-keyed) | Global |
| Tier 2: Property-based testing | Yes (with seed) | Long (probabilistic) | Organization |
| Tier 3: Symbolic execution | Yes | Permanent (deterministic) | Global |
| Tier 4: SMT proof | Yes | Permanent (formal proof) | Global |

Tier 4 proofs are the strongest: a formally verified proof cached against a content hash is a mathematical fact that never expires and is valid for every instance of that hash.

### References

- [Vericoding Benchmark — POPL 2026](https://arxiv.org/abs/2509.22908)
- [ATLAS: Automated Toolkit for Large-Scale Verified Code Synthesis — POPL 2026](https://arxiv.org/abs/2512.10173)
- [Unison: The Big Idea — Content-Addressed Code](https://www.unison-lang.org/docs/the-big-idea/)
- [AutoVerus: Automated Proof Generation for Rust Code — OOPSLA 2025](https://arxiv.org/abs/2409.13082)
- [Proof-Carrying Code — Necula, 1997](https://www.cs.cmu.edu/~necula/Papers/pcc97.pdf)

---

## Proposal 19: Compact Module Signatures as AI Context

### The Problem

Multi-file reasoning is the frontier of AI code generation. The numbers are stark:

- Single-file tasks: ~90% solved by combining all top AI systems (Ganhotra 2025)
- Multi-file tasks: ~54% solved by combining all top AI systems
- No individual system exceeds 30% on multi-file problems

The root cause: AI agents cannot hold enough codebase in context. Claude's accuracy drops from 29% to 3% when context increases from 32K to 256K tokens (LongCodeBench 2025). Yet multi-file reasoning requires understanding module interfaces across the entire project.

The current workaround: agents read entire files to understand their APIs. A 500-line module might have a 30-line public interface. The agent wastes 470 lines of context on implementation details irrelevant to cross-module reasoning.

### Why Blood's Effects Make This Uniquely Powerful

Most languages can generate "header files" or "interface files" (Haskell's `.hi`, OCaml's `.mli`, TypeScript's `.d.ts`). These contain type signatures but not effect information.

Blood's module signatures include **effect types** — a compact, machine-checkable description of what each function can do. An AI agent reading `connect(String) -> Connection / {IO, Error}` knows instantly:
- This function performs IO (network)
- This function can fail (Error effect)
- Calling this from pure code is a compiler error
- Any function calling this inherits IO and Error effects

No other language's module signatures carry this much behavioral information in this few tokens.

### Proposed Design

```blood
// Full module: src/database.blood (500 lines)
module database

use connection.{Connection, ConnectionConfig}
use query.{Query, QueryResult, QueryError}
use pool.{Pool, PoolConfig}

/// Establishes a database connection from a URL string.
pub fn connect(url: String) -> Connection / {IO, Error}
    requires url.starts_with("postgres://") || url.starts_with("mysql://")
    ensures result.is_valid()
{
    let config = ConnectionConfig.parse(url)     // 15 lines
    let socket = tcp_connect(config.host, config.port)  // 20 lines
    let handshake = protocol_handshake(socket, config)   // 40 lines
    // ... 80 more lines of connection logic ...
    Connection.new(socket, handshake.session_id)
}

/// Executes a query on an established connection.
pub fn execute(conn: Connection, query: Query) -> QueryResult / {IO, Error}
    requires conn.is_valid()
    ensures result.rows >= 0
{
    // ... 120 lines of query execution, result parsing, error handling ...
}

/// Runs a function inside a database transaction.
pub fn transaction<T>(
    conn: Connection,
    body: () -> T / {IO, Error}
) -> T / {IO, Error}
    requires conn.is_valid()
{
    // ... 60 lines of transaction management ...
}

// ... 8 more public functions, plus 15 private helper functions ...
```

```blood
// Generated signature: src/database.blood.sig (35 lines)
// Content-hash: a3b7c9...
// Generated by: blood sig src/database.blood

module database

pub fn connect(url: String) -> Connection / {IO, Error}
    requires url.starts_with("postgres://") || url.starts_with("mysql://")
    ensures result.is_valid()

pub fn execute(conn: Connection, query: Query) -> QueryResult / {IO, Error}
    requires conn.is_valid()
    ensures result.rows >= 0

pub fn transaction<T>(conn: Connection, body: () -> T / {IO, Error}) -> T / {IO, Error}
    requires conn.is_valid()

pub fn create_pool(config: PoolConfig) -> Pool / {IO, Error}
    ensures result.max_connections == config.max_connections

pub fn with_connection<T>(pool: Pool, body: (Connection) -> T / {IO, Error}) -> T / {IO, Error}
    requires pool.is_active()

// ... 6 more compact signatures ...
```

**Context compression:** 500 lines → 35 lines = **14x compression** with zero loss of cross-module reasoning information.

### The `blood context` Command

```bash
# Generate minimal AI-ready context for a specific file
$ blood context --for-ai src/api/users.blood

# Output: content-hash d4e5f6...
# Includes:
#   - src/api/users.blood (full source — the file being edited)
#   - src/database.blood.sig (signature — direct dependency)
#   - src/auth.blood.sig (signature — direct dependency)
#   - src/models/user.blood.sig (signature — type dependency)
#   - dependency_graph.json (subgraph of relevant modules)
#
# Total: ~200 lines instead of ~2,000 lines
# Compression: 10x
```

```bash
# Generate context for multi-file refactoring
$ blood context --for-ai --include-dependents src/auth.blood

# Output: context needed to safely modify auth.blood
# Includes:
#   - src/auth.blood (full source)
#   - All modules that IMPORT from auth (signatures)
#   - All modules that auth IMPORTS from (signatures)
#   - Impact analysis: which functions would be affected by signature changes
```

### Content-Addressed Signatures

Module signatures are content-addressed. The signature hash changes only when the public API changes:

```
src/database.blood modified (internal refactor, no API change)
    → database.blood.sig hash: a3b7c9... (unchanged)
    → AI agent's cached understanding: still valid

src/database.blood modified (new public function added)
    → database.blood.sig hash: f7d2a1... (changed)
    → AI agent: invalidate cache, reload signature
```

This means AI agents can cache their understanding of module interfaces and only re-read when the signature hash changes — further reducing context consumption over time.

### References

- [Multi-File Frontier: SWE-Bench Verified Saturation — Ganhotra 2025](https://jatinganhotra.dev/blog/swe-agents/2025/03/30/swe-bench-verified-single-file-saturation.html)
- [LongCodeBench: Evaluating Coding LLMs at 1M Context — 2025](https://arxiv.org/html/2505.07897v1)
- [Statically Contextualizing LLMs with Typed Holes — OOPSLA 2024](https://huggingface.co/papers/2409.00921)
- [LocAgent: Graph-Guided LLM Agents for Code Localization — ACL 2025](https://aclanthology.org/2025.acl-long.426/)
- [Codified Context Infrastructure for AI Agents — Vasilopoulos 2026](https://arxiv.org/abs/2602.20478)
- [Which Programming Languages Are Most Token-Efficient? — Alderson 2025](https://martinalderson.com/posts/which-programming-languages-are-most-token-efficient/)

---

## Proposal 20: First-Class Specification Annotations

### The Problem

The vericoding benchmark (POPL 2026) established a critical finding: **formal specifications are better prompts than natural language** for AI code generation. Dafny achieves 96% success specifically because its contracts are machine-checkable. When an LLM generates code against a formal specification, the specification provides an unambiguous target that eliminates the "misinterpretation" bug category (20.77% of all LLM bugs per Tambon et al. 2024).

Most languages lack native specification syntax. TypeScript has JSDoc (not machine-checkable). Python has type hints (no contracts). Rust has no contract syntax at all. Dafny and SPARK have contracts, but they are niche languages.

### Why Blood Should Have This

Blood's graduated verification continuum (Proposal 7) already envisions specifications at multiple levels. This proposal makes them first-class syntax — not an annotation hack, not a library, not a comment convention, but keywords with formal semantics that the compiler understands.

### Proposed Design

```blood
// Specification keywords: requires, ensures, invariant, decreases

fn binary_search<T: Ord>(items: List<T>, target: T) -> Option<Int> / {}
    requires items.is_sorted()
    requires items.len() < Int.MAX
    ensures match result {
        Some(i) => items[i] == target && 0 <= i && i < items.len(),
        None => !items.contains(target),
    }
    decreases items.len()  // termination measure
{
    if items.is_empty() {
        return None
    }

    let mid = items.len() / 2
    match target.cmp(items[mid]) {
        Equal => Some(mid),
        Less => binary_search(items[..mid], target),
        Greater => binary_search(items[mid+1..], target)
            .map(|i| i + mid + 1),
    }
}
```

### The Triple-Duty Principle

Every specification annotation serves three masters simultaneously:

**1. AI generation prompt.** The specification is the most precise prompt possible. An LLM generating code for a function with `ensures result.is_sorted() && result.len() == items.len()` has an unambiguous behavioral target.

**2. Graduated verification target.** The same specification is checked at increasing rigor:

| Verification Tier | What Happens to the Spec |
|-------------------|--------------------------|
| Tier 0: Unchecked | Spec is documentation only |
| Tier 1: Static analysis | Compiler checks spec syntax and type-correctness |
| Tier 2: Property-based testing | `requires` becomes test precondition, `ensures` becomes test oracle |
| Tier 3: Symbolic execution | Spec becomes path constraint |
| Tier 4: SMT/Formal proof | Spec becomes theorem to prove |

**3. Human documentation.** Specifications are the best documentation because they are precise, complete, and machine-verified.

### Specification Hashing

Specifications participate in content-addressing:

```
Spec hash:  sha3(requires + ensures + invariant + decreases)
Impl hash:  blake3(function body)
Proof key:  (spec_hash, impl_hash) → verification result
```

The implications for the verification cache (Proposal 18):
- If spec changes but impl doesn't → must re-verify
- If impl changes but spec doesn't → must re-verify
- If neither changes → cached proof applies
- If both change to previously-seen pair → cache hit

### Contract Checking Modes

```bash
# Development: runtime checks enabled (like assert)
$ blood build --contracts=runtime

# Testing: contracts become property-based test oracles
$ blood test --contracts=oracle

# Release: contracts compiled away (zero overhead)
$ blood build --release --contracts=none

# Verification: contracts become formal proof obligations
$ blood verify --tier=4
```

### Quantified Specifications

For richer specifications, support bounded quantification:

```blood
fn sort<T: Ord>(items: List<T>) -> List<T> / {}
    ensures forall(i in 0..result.len()-1) { result[i] <= result[i+1] }
    ensures result.len() == items.len()
    ensures forall(x in items) { result.count(x) == items.count(x) }
{
    // The third ensures clause (permutation property) is the one
    // most LLMs miss. With it in the spec, the AI knows it must
    // preserve all elements, not just produce a sorted list.
}
```

### Interaction with Effect System

Specifications can reference effects:

```blood
fn read_config(path: String) -> Config / {IO, Error}
    requires path.ends_with(".toml")
    ensures result.is_valid()
    // Effect-aware specifications:
    performs IO.read_file(path)   // Declares which IO operations occur
    raises Error.FileNotFound    // Declares possible error effects
{
    // ...
}
```

The `performs` and `raises` clauses are effect-level specifications — they constrain not just what values a function produces but what effects it performs. This is novel; no existing specification language integrates with an effect system.

### References

- [Vericoding Benchmark — Bursuc et al., POPL 2026](https://arxiv.org/abs/2509.22908)
- [AI-Assisted Synthesis of Verified Dafny Methods — FSE 2024](https://arxiv.org/html/2402.00247v1)
- [LLM Bug Taxonomy (333 bugs) — Tambon et al. 2024](https://arxiv.org/html/2403.08937v2)
- [nl2postcond: LLMs Transform NL to Formal Postconditions — ACM 2024](https://dl.acm.org/doi/10.1145/3660791)
- [Self-Spec: Model-Authored Specifications — 2025](https://openreview.net/forum?id=6pr7BUGkLp)
- [Dafny as Verification-Aware Intermediate Language — POPL 2025](https://arxiv.org/abs/2501.06283)
- [AutoSpec: Formal Specification Generation via LLMs — 2025](https://www.arxiv.org/pdf/2601.12845)

---

## Proposal 21: AI-Optimized Syntax Decisions

### The Problem

MoonBit (ICSE 2024) is the only language designed with AI code generation as a primary design constraint. Their research validated specific syntax decisions that improve LLM output quality:

- **Flat scope structure** reduces KV-cache pressure in transformer models
- **Structural interfaces** (vs. nominal trait blocks) allow linear code generation
- **Top-level type annotations** provide anchor points for type inference

These are language-level decisions that cannot be retrofitted — they must be made at design time. Blood is still at design time.

### Why This Matters Quantitatively

| Factor | Impact | Source |
|--------|--------|--------|
| Token efficiency | 2.6x gap between most/least efficient languages | Alderson 2025 |
| Context degradation | 29% → 3% accuracy at longer contexts | LongCodeBench 2025 |
| Compilation error rate | 24% in Haskell vs. 5% in Java | FPEval 2026 |
| Imperative pattern leakage | 88% of LLM Haskell code is imperative | FPEval 2026 |

Every syntax decision affects how efficiently AI can generate, read, and modify Blood code.

### Concrete Syntax Decisions

**Decision 1: Expression-oriented everything.**

Every construct returns a value. No statement/expression distinction.

```blood
// if-else is an expression
let result = if condition { a } else { b }

// match is an expression
let value = match shape {
    Circle(r) => pi * r * r,
    Rectangle(w, h) => w * h,
}

// Block is an expression (last expression is the value)
let computed = {
    let x = heavy_computation()
    let y = another_computation()
    x + y  // this is the block's value
}
```

**Rationale:** Expression-oriented code is more locally replaceable. An AI agent can swap any sub-expression without worrying about statement ordering, temporary variables, or void returns. Every piece of code has a type, so the type checker provides feedback on every substitution.

**Token impact:** Eliminates `return` keywords, temporary variables for storing intermediate `if` results, and void function wrappers. Estimated 5–10% token reduction.

**Decision 2: Named arguments by default.**

```blood
// Without named arguments (common AI failure mode):
connect("localhost", 5432, "mydb", "user", "pass")
// AI often confuses which String is which when types are identical

// With named arguments:
connect(host: "localhost", port: 5432, db: "mydb", user: "user", password: "pass")
// Self-documenting. AI cannot confuse argument positions.
// Compiler error if argument name is wrong.
```

**Rationale:** The "Wrong Attribute" bug category is 6.9% of all LLM bugs (Tambon et al. 2024). Named arguments eliminate positional confusion for functions with multiple same-typed parameters. They also serve as inline documentation, reducing the context an AI needs to understand a call site.

**Decision 3: Pipeline operator for linear data flow.**

```blood
// Nested calls (high nesting depth, error-prone for LLMs):
sort(filter(map(data, transform), predicate))

// Pipeline (linear, each step independent, left-to-right):
data
    |> map(transform)
    |> filter(predicate)
    |> sort()
```

**Rationale:** MoonBit's research showed that flat, linear code flow reduces KV-cache pressure and LLM hallucination. Pipeline operators linearize nested function calls, making each transformation step independent and locally modifiable.

**Decision 4: Exhaustive pattern matching as the primary control flow.**

```blood
match result {
    Ok(value) => process(value),
    Err(IOError(path, msg)) => log_io_error(path, msg),
    Err(ParseError(line, col)) => show_parse_error(line, col),
    // Compiler ERROR if any variant is unhandled.
    // The AI cannot ship incomplete case analysis.
}
```

**Rationale:** "Missing Corner Case" is 15.3% of all LLM bugs. Exhaustive pattern matching makes this a compiler error instead of a runtime bug. The compiler lists exactly which cases are missing, providing a concrete fix target for the AI.

**Decision 5: Compact error handling with Result types.**

```blood
// Verbose (Java/Go style):
let result = try {
    let file = open_file(path)
    let content = read(file)
    let parsed = parse(content)
    parsed
} catch (e: IOException) {
    handle_io_error(e)
} catch (e: ParseError) {
    handle_parse_error(e)
}

// Compact (Blood style, using effects):
fn load_config(path: String) -> Config / {Error} {
    let file = open_file(path)?      // ? propagates Error effect
    let content = read(file)?
    parse(content)?
}
```

**Rationale:** Error handling ceremony is one of the largest sources of boilerplate. Effect-based error handling (`?` operator propagating an Error effect) is both more compact (fewer tokens) and more precise (the effect signature documents all possible errors).

**Decision 6: No semicolons. Newline-delimited.**

```blood
// Clean, minimal syntax
let x = 42
let y = compute(x)
let result = x + y
```

**Rationale:** Semicolons are a pure noise token. They carry zero semantic information but cost 1 token each. In a 1,000-line file with ~800 statements, that is 800 wasted tokens. Newline delimiting is unambiguous for any language with expression-oriented syntax.

### Combined Token Impact

| Decision | Estimated Token Savings | Other Benefit |
|----------|------------------------|---------------|
| Expression-oriented | 5–10% | Better local reasoning |
| Named arguments | +5% (slightly more verbose) | Eliminates 6.9% of bug category |
| Pipeline operator | 3–5% (reduces nesting) | Linear code flow |
| Exhaustive matching | Neutral | Eliminates 15.3% of bug category |
| Compact error handling | 10–15% | Cleaner effect tracking |
| No semicolons | 2–3% | Less noise |

Net effect: **~15–25% more compact** than equivalent Rust/TypeScript, while retaining full static typing with effect tracking. This means AI agents can fit ~20% more Blood codebase into their context window compared to equivalent Rust.

### References

- [MoonBit: AI-Friendly PL Design — LLM4Code @ ICSE 2024](https://dl.acm.org/doi/10.1145/3643795.3648376)
- [LLMs Love Python — arXiv 2025](https://arxiv.org/html/2503.17181v1)
- [FPEval: LLMs for Functional Programming — 2026](https://arxiv.org/html/2601.02060)
- [LLM Bug Taxonomy — Tambon et al. 2024](https://arxiv.org/html/2403.08937v2)
- [Token Efficiency by Language — Alderson 2025](https://martinalderson.com/posts/which-programming-languages-are-most-token-efficient/)
- [Context Rot — Chroma Research](https://research.trychroma.com/context-rot)
- [LongCodeBench — 2025](https://arxiv.org/html/2505.07897v1)

---

## Proposal 22: Toolchain-Integrated Dependency Graph API

### The Problem

LocAgent (ACL 2025) demonstrated that representing codebases as **directed heterogeneous graphs** (files, classes, functions, and their relationships) improves AI agent file-level localization to 92.7%. But agents currently must build these graphs themselves by parsing imports and tracing references — an expensive, error-prone process that varies by language.

Microsoft's Sharp Tools study (2025) found that bug fixing had only a 38% AI success rate, primarily because agents struggle with "finding where to put the code." This is a localization problem.

### Why Blood Should Build This Into the Toolchain

Blood's compiler already knows the full dependency graph — it must, to perform effect inference and type checking. Currently, this information lives inside the compiler and is discarded after compilation. Blood should expose it as a queryable, content-addressed toolchain feature.

### Proposed Design

```bash
# What depends on this function? (reverse dependency query)
$ blood deps --reverse src/auth.blood:verify_token
src/middleware.blood:auth_middleware        (direct)
src/api/users.blood:get_profile            (direct)
src/api/admin.blood:admin_panel            (direct)
tests/auth_test.blood:test_verify_token    (direct)
src/api/reports.blood:generate_report      (transitive, via middleware)

# What effects does this call chain produce?
$ blood effects src/api/users.blood:get_profile
IO        (from database.execute at database.blood:45)
Error     (from auth.verify_token at auth.blood:12)
Error     (from database.execute at database.blood:45)

# Impact analysis: what would break if this function's signature changed?
$ blood impact src/auth.blood:verify_token
Direct callers: 3
Transitive dependents: 7
Effect propagation: IO, Error → 5 modules
Contract dependents: 2 (middleware relies on ensures clause)
```

### The AI Context Command

The flagship feature: `blood context --for-ai` generates the **minimal context** an AI agent needs to work on a specific file.

```bash
$ blood context --for-ai src/api/users.blood --format=json
```

```json
{
  "content_hash": "d4e5f6...",
  "target_file": {
    "path": "src/api/users.blood",
    "content": "... full source ...",
    "lines": 200
  },
  "direct_dependencies": [
    {
      "path": "src/database.blood",
      "signature": "... compact signature ...",
      "lines": 35
    },
    {
      "path": "src/auth.blood",
      "signature": "... compact signature ...",
      "lines": 20
    }
  ],
  "transitive_types": [
    {
      "path": "src/models/user.blood",
      "types_used": ["User", "UserRole", "UserStatus"],
      "lines": 15
    }
  ],
  "dependency_graph": {
    "edges": [
      ["src/api/users.blood", "src/database.blood", "imports"],
      ["src/api/users.blood", "src/auth.blood", "imports"],
      ["src/database.blood", "src/models/user.blood", "type_dependency"]
    ]
  },
  "total_context_lines": 270,
  "compression_ratio": "7.4x vs reading all files"
}
```

### Content-Addressed Graph

The dependency graph is content-addressed. When a file changes:
- Only affected subgraphs are recomputed
- Unchanged module relationships retain their hash
- Agents can diff graphs to understand what changed between edits

```bash
# What changed since the last edit?
$ blood deps --diff HEAD~1
Modified: src/auth.blood
  Added export: fn refresh_token(Token) -> Token / {IO, Error}
  Impact: 0 existing callers affected (new function)

Modified: src/database.blood
  Changed: fn execute() effect set: / {IO} → / {IO, Error}
  Impact: 3 callers need Error handling added
    - src/api/users.blood:get_profile
    - src/api/admin.blood:list_users
    - src/api/reports.blood:fetch_data
```

This `--diff` output tells the AI agent exactly what changed and exactly which files need attention — no guessing, no searching.

### References

- [LocAgent: Graph-Guided LLM Agents for Code Localization — ACL 2025](https://aclanthology.org/2025.acl-long.426/)
- [Sharp Tools: How Developers Wield Agentic AI — Microsoft Research 2025](https://arxiv.org/html/2506.12347v2)
- [Codified Context Infrastructure for AI Agents — Vasilopoulos 2026](https://arxiv.org/abs/2602.20478)
- [Building a C Compiler with Parallel Claudes — Anthropic 2025](https://www.anthropic.com/engineering/building-c-compiler)

---

## Proposal 23: Effect Handlers as AI Agent Middleware

### The Problem

Every AI agent framework (LangChain, CrewAI, AutoGen, OpenHands, Claude Code) reinvents the same infrastructure:
- **Tool use:** File read, file write, terminal, browser
- **Sandboxing:** Preventing agents from accessing unauthorized resources
- **Cost tracking:** Limiting API spend per task
- **Replay/debugging:** Reproducing agent behavior for debugging
- **Observability:** Tracing what the agent did and why

Each framework implements these as framework-specific middleware. Swapping between sandboxed and production execution, or adding cost limits, requires framework-specific code changes. There is no universal abstraction.

### Why Blood's Effects Are the Universal Abstraction

An AI agent's capabilities — reading files, writing files, running commands, querying LLMs — are **effects**. Controlling those capabilities is **effect handling**. Blood already has this machinery.

The insight: everything that agent frameworks implement as middleware is an instance of **intercepting and transforming effects**. This is exactly what algebraic effect handlers do. Pangolin (LMPL 2025) independently reached the same conclusion for compound AI systems.

### Proposed Design

```blood
// Step 1: Define agent capabilities as effects

effect FileSystem {
    fn read_file(path: String) -> String
    fn write_file(path: String, content: String)
    fn list_dir(path: String) -> List<String>
}

effect Terminal {
    fn run_command(cmd: String) -> CommandResult
}

effect LLM {
    fn query(prompt: String, model: String) -> String
    fn embed(text: String) -> Vector<Float>
}

effect Browser {
    fn fetch_url(url: String) -> String
    fn search(query: String) -> List<SearchResult>
}
```

```blood
// Step 2: Write the agent program using effects declaratively

fn fix_bug(issue: String) / {FileSystem, Terminal, LLM} {
    // Read the relevant code
    let files = list_dir("src/")
    let relevant = files
        |> filter(|f| f.ends_with(".blood"))
        |> map(|f| (f, read_file(f)))

    // Ask LLM for diagnosis
    let diagnosis = query(
        prompt: "Given this issue: {issue}\n\nCode:\n{relevant}\n\nDiagnose the bug.",
        model: "claude-opus-4-6",
    )

    // Ask LLM for fix
    let fix = query(
        prompt: "Apply this fix:\n{diagnosis}",
        model: "claude-opus-4-6",
    )

    // Apply fix
    write_file(fix.file_path, fix.new_content)

    // Verify
    let result = run_command("blood check")
    if result.exit_code != 0 {
        // Retry with compiler feedback
        fix_bug("{issue}\n\nPrevious fix failed:\n{result.stderr}")
    }
}
```

```blood
// Step 3: Swap behavior entirely through handlers

// Handler A: Production (real filesystem, real LLM, real terminal)
handle fix_bug("null pointer in auth") with ProductionHandler {
    fn read_file(path) => fs.read(path),
    fn write_file(path, content) => fs.write(path, content),
    fn list_dir(path) => fs.list(path),
    fn run_command(cmd) => shell.exec(cmd),
    fn query(prompt, model) => anthropic.complete(prompt, model),
}

// Handler B: Sandboxed (virtual filesystem, real LLM)
handle fix_bug("null pointer in auth") with SandboxHandler {
    fn read_file(path) => {
        assert(path.starts_with("src/"), "sandbox: read restricted to src/")
        virtual_fs.read(path)
    },
    fn write_file(path, content) => {
        assert(path.starts_with("src/"), "sandbox: write restricted to src/")
        assert(!path.contains(".."), "sandbox: no path traversal")
        virtual_fs.write(path, content)
    },
    fn run_command(cmd) => {
        assert(cmd.starts_with("blood "), "sandbox: only blood commands allowed")
        sandboxed_shell.exec(cmd)
    },
    fn query(prompt, model) => anthropic.complete(prompt, model),
}

// Handler C: Cost-limited
handle fix_bug("null pointer in auth") with CostLimitedHandler {
    var budget = 5.00  // $5 max

    fn query(prompt, model) => {
        let cost = estimate_cost(prompt, model)
        budget -= cost
        if budget <= 0.0 {
            raise BudgetExceeded("spent ${5.00 - budget} of $5.00 budget")
        }
        anthropic.complete(prompt, model)
    },
    // ... other handlers pass through to production ...
}

// Handler D: Deterministic replay (for debugging agent behavior)
handle fix_bug("null pointer in auth") with ReplayHandler {
    var trace = load_trace("recordings/fix-auth-2026-02-26.trace")

    fn read_file(path) => trace.next_response("read_file", path),
    fn write_file(path, content) => trace.next_response("write_file", path),
    fn run_command(cmd) => trace.next_response("run_command", cmd),
    fn query(prompt, model) => trace.next_response("query", prompt),
}

// Handler E: Observable (logs all actions)
handle fix_bug("null pointer in auth") with ObservableHandler {
    fn read_file(path) => {
        trace.emit("read_file", path: path)
        let result = fs.read(path)
        trace.emit("read_file_result", path: path, size: result.len())
        result
    },
    fn query(prompt, model) => {
        trace.emit("llm_query", model: model, prompt_tokens: tokenize(prompt).len())
        let start = time.now()
        let result = anthropic.complete(prompt, model)
        trace.emit("llm_response", duration: time.now() - start, tokens: tokenize(result).len())
        result
    },
    // ... all effects wrapped with tracing ...
}
```

### The Composability Advantage

Handlers compose. You can stack them:

```blood
// Sandboxed + Cost-limited + Observable
handle fix_bug(issue)
    with SandboxHandler
    with CostLimitedHandler
    with ObservableHandler
```

This is impossible in current agent frameworks without writing custom middleware composition logic for each framework.

### Comparison with Existing Agent Frameworks

| Capability | LangChain | CrewAI | OpenHands | **Blood Effects** |
|------------|-----------|--------|-----------|-------------------|
| Tool definition | Python decorators | Python classes | Action classes | **Effect types** |
| Sandboxing | Docker container | N/A | Docker + seccomp | **Handler swap** |
| Cost tracking | Callback hooks | N/A | Token counting | **Handler swap** |
| Replay/debug | LangSmith (SaaS) | N/A | Partial | **Handler swap** |
| Observability | LangSmith (SaaS) | Custom logging | Event logging | **Handler swap** |
| Composability | Callback chains | Manual wiring | Plugin system | **Handler stacking** |
| Type safety | None (runtime) | None (runtime) | None (runtime) | **Compile-time** |
| Completeness guarantee | No | No | No | **Yes (if effect not handled → compile error)** |

The fundamental difference: in Blood, if an agent program uses the `FileSystem` effect and the handler doesn't provide `write_file`, the program **does not compile**. No other agent framework provides this guarantee.

### Connection to Other Proposals

This proposal is the natural convergence of:
- **Proposal 8 (DST):** Simulation handler replaces nondeterministic effects with deterministic simulators
- **Proposal 12 (Replay Debugging):** Replay handler provides recorded responses
- **Proposal 13 (Observability):** Observable handler wraps all effects with tracing
- **Proposal 4 (Capability Security):** Sandbox handler restricts available operations

Applied to the AI agent domain, these become a unified framework for building, testing, debugging, securing, and optimizing AI agents — all through the same mechanism: effect handling.

### References

- [Pangolin: Algebraic Effects for Compound AI Systems — LMPL 2025](http://shangyit.me/_assets/files/lmpl2025-paper11.pdf)
- [Building a C Compiler with Parallel Claudes — Anthropic 2025](https://www.anthropic.com/engineering/building-c-compiler)
- [OpenHands CodeAct 2.1 — 2025](https://openhands.dev/blog/openhands-codeact-21-an-open-state-of-the-art-software-development-agent)
- [Sharp Tools: How Developers Wield Agentic AI — Microsoft Research 2025](https://arxiv.org/html/2506.12347v2)
- [SWE-Bench Verified Leaderboard — Epoch AI](https://epoch.ai/benchmarks/swe-bench-verified)

---

## Priority Ranking

### Tier A: Foundational (must build first — everything else depends on these)

| # | Proposal | Rationale |
|---|----------|-----------|
| 16 | Type-and-Effect Constrained Decoding Oracle | The single highest-impact feature for AI code quality. First-ever effect-constrained decoding. Requires incremental type checker (prerequisite for everything). |
| 18 | Content-Addressed Verification Cache | Makes the re-generation loop economically viable. Transforms verification from per-invocation cost to amortized-to-zero cost. Leverages Blood's existing content-addressing. |
| 20 | First-Class Specification Annotations | The bridge between AI generation and formal verification. Dafny's 96% success rate proves specs-as-prompts work. Prerequisite for graduated verification. |

### Tier B: High Impact (immediately valuable once Tier A exists)

| # | Proposal | Rationale |
|---|----------|-----------|
| 17 | Machine-Readable Structured Diagnostics | Directly addresses the #1 bottleneck in AI agent loops. Moderate implementation effort with immediate payoff. |
| 19 | Compact Module Signatures | 10x context compression for multi-file reasoning. Addresses the frontier problem (28% → potential 60%+ multi-file success). |
| 22 | Dependency Graph API | Complements Proposal 19. Makes `blood context --for-ai` possible. Engineering effort, not research risk. |

### Tier C: Valuable (quality-of-life and ecosystem features)

| # | Proposal | Rationale |
|---|----------|-----------|
| 21 | AI-Optimized Syntax | Must be decided at language design time (cannot retrofit). Net ~20% token efficiency gain. MoonBit validated the approach. |
| 23 | Effect Handlers as AI Agent Middleware | Novel application of existing machinery. High value for the AI agent ecosystem. Requires mature effect system. |

### Implementation Order

**Phase 1 (compiler infrastructure):** Build the incremental type-and-effect checker that Proposals 16, 17, 19, and 22 all depend on. Add specification syntax (Proposal 20). Make syntax decisions (Proposal 21).

**Phase 2 (AI toolchain):** Expose the checker as a constrained decoding API (Proposal 16). Add structured diagnostics (Proposal 17). Build module signatures and context generation (Proposals 19, 22).

**Phase 3 (verification ecosystem):** Build the content-addressed verification cache (Proposal 18). Connect specifications to graduated verification tiers.

**Phase 4 (agent ecosystem):** Build the effect-based agent middleware (Proposal 23). Create reference implementations for sandboxing, cost tracking, replay, and observability handlers.

---

## The Unified Architecture: All 23 Proposals

### How AI-Native Proposals Connect to the Existing 15

```
Blood's Effect System (Pillar 1)
    ├── WHAT code does        → Verification (7), Specs (20), Contracts
    ├── WHAT code CAN do      → Capabilities (4), Taint (9), Agent Middleware (23)
    ├── WHAT code MUST NOT do → Constrained Decoding (16), Sandbox Handlers
    ├── What WENT WRONG       → DST (8), Replay (12), Structured Diagnostics (17)
    └── What IS HAPPENING     → Observability (13), Cost Tracking (23)

Blood's Content-Addressing (Pillar 2)
    ├── Code IDENTITY         → Verification Cache (18), Proof-Carrying (10), Semver (11)
    ├── Code CONTEXT          → Module Signatures (19), Dependency Graph (22)
    ├── Code PROVENANCE       → Taint (9), Provenance (6)
    └── Code CACHING          → Verification Cache (18), Memoization (3)

Blood's Type System
    ├── WHAT types allow       → Type-Constrained Decoding (16)
    ├── WHAT effects allow     → Effect-Constrained Decoding (16)
    ├── WHAT specs require     → Specification Annotations (20)
    └── WHAT diagnostics say   → Structured Diagnostics (17)

Blood's Syntax
    ├── Token EFFICIENCY       → AI-Optimized Syntax (21)
    ├── Code READABILITY       → Expression-oriented, pipelines (21)
    └── Error PREVENTION       → Named args, exhaustive matching (21)
```

### The Meta-Pattern

Parts I and II answered: **"What can Blood programs do?"**
- Verify, simulate, trace, observe, secure, debug, parallelize, choreograph.

Part III answers: **"What can Blood do for AI?"**
- Constrain generation, cache verification, compress context, structure feedback, optimize tokens, control agents.

The connection is that **the same four pillars serve both purposes**. Effects that enable DST also enable agent sandboxing. Content-addressing that enables proof caching also enables context compression. Specifications that enable graduated verification also enable AI-guided generation.

These are not 23 independent features. They are 23 facets of a language architecture that serves both human developers and AI agents through the same underlying mechanisms.

---

## References (Collected)

### AI-Native Language Design
- [MoonBit: AI-Friendly PL Design — LLM4Code @ ICSE 2024](https://dl.acm.org/doi/10.1145/3643795.3648376)
- [MoonBit AI-Native Toolchain Blog](https://www.moonbitlang.com/blog/moonbit-ai)
- [Pangolin: Algebraic Effects for Compound AI Systems — LMPL 2025](http://shangyit.me/_assets/files/lmpl2025-paper11.pdf)
- [Dana: Agent-Native Programming — AI Alliance 2025](https://thealliance.ai/blog/the-ai-alliance-releases-new-ai-powered-programmin)
- [Compiler.next: Search-Based Compilation — arXiv 2025](https://arxiv.org/abs/2510.24799)

### Type-Constrained and Grammar-Constrained Decoding
- [Type-Constrained Code Generation — ETH Zurich, PLDI 2025](https://arxiv.org/abs/2504.09246)
- [Grammar-Aligned Decoding — NeurIPS 2024](https://proceedings.neurips.cc/paper_files/paper/2024/file/2bdc2267c3d7d01523e2e17ac0a754f3-Paper-Conference.pdf)
- [PICARD: Parsing Incrementally for Constrained Decoding](https://arxiv.org/abs/2109.05093)
- [Synchromesh: Reliable Code Generation — Microsoft](https://www.microsoft.com/en-us/research/wp-content/uploads/2022/01/csd_arxiv.pdf)
- [TyFlow: Augmenting Programs with Type Correctness Proofs — arXiv 2025](https://arxiv.org/abs/2510.10216)

### LLM Code Generation Research
- [LLM Bug Taxonomy (333 bugs) — Tambon et al. 2024](https://arxiv.org/html/2403.08937v2)
- [LLM Hallucinations in Practical Code Generation — 2024](https://arxiv.org/html/2409.20550v1)
- [LLMs Love Python — arXiv 2025](https://arxiv.org/html/2503.17181v1)
- [FPEval: LLMs for Functional Programming — 2026](https://arxiv.org/html/2601.02060)
- [94% of AI Errors Are Type Errors — GitHub Blog 2025](https://github.blog/ai-and-ml/llms/why-ai-is-pushing-developers-toward-typed-languages/)
- [Survey of Bugs in AI-Generated Code — 2024](https://arxiv.org/html/2512.05239v1)
- [Code Complexity and LLM Reasoning — 2025](https://arxiv.org/html/2601.21894)

### Context Windows and Token Efficiency
- [LongCodeBench: Evaluating Coding LLMs at 1M Context — 2025](https://arxiv.org/html/2505.07897v1)
- [Context Rot — Chroma Research](https://research.trychroma.com/context-rot)
- [Token Efficiency by Language — Alderson 2025](https://martinalderson.com/posts/which-programming-languages-are-most-token-efficient/)
- [Token Efficiency and LLM Performance — CodeAnt](https://www.codeant.ai/blogs/token-efficiency-llm-performance)

### AI Agent Development
- [Building a C Compiler with Parallel Claudes — Anthropic 2025](https://www.anthropic.com/engineering/building-c-compiler)
- [Multi-File Frontier: SWE-Bench Saturation — Ganhotra 2025](https://jatinganhotra.dev/blog/swe-agents/2025/03/30/swe-bench-verified-single-file-saturation.html)
- [Sharp Tools: How Developers Wield Agentic AI — Microsoft Research 2025](https://arxiv.org/html/2506.12347v2)
- [LocAgent: Graph-Guided Code Localization — ACL 2025](https://aclanthology.org/2025.acl-long.426/)
- [Codified Context for AI Agents — Vasilopoulos 2026](https://arxiv.org/abs/2602.20478)
- [RustAssistant: LLMs Fix Compilation Errors — Microsoft Research 2024](https://www.microsoft.com/en-us/research/wp-content/uploads/2024/08/paper.pdf)
- [SWE-Bench Pro — arXiv 2025](https://arxiv.org/abs/2509.16941)
- [OpenHands CodeAct 2.1 — 2025](https://openhands.dev/blog/openhands-codeact-21-an-open-state-of-the-art-software-development-agent)

### Vericoding and AI-Assisted Verification
- [Vericoding Benchmark — POPL 2026](https://arxiv.org/abs/2509.22908)
- [ATLAS: Verified Code Synthesis Training Data — POPL 2026](https://arxiv.org/abs/2512.10173)
- [DeepSeek-Prover-V2 — arXiv 2025](https://arxiv.org/abs/2504.21801)
- [AlphaProof — Nature 2025](https://www.nature.com/articles/s41586-025-09833-y)
- [AutoVerus: Rust Proof Generation — OOPSLA 2025](https://arxiv.org/abs/2409.13082)
- [AI Will Make Formal Verification Mainstream — Kleppmann 2025](https://martin.kleppmann.com/2025/12/08/ai-formal-verification.html)
- [The Coming Need for Formal Specification — Congdon 2025](https://benjamincongdon.me/blog/2025/12/12/The-Coming-Need-for-Formal-Specification/)

### Specification-Driven Development
- [Dafny as Verification-Aware Intermediate Language — POPL 2025](https://arxiv.org/abs/2501.06283)
- [AI-Assisted Synthesis of Verified Dafny Methods — FSE 2024](https://arxiv.org/html/2402.00247v1)
- [nl2postcond: NL to Formal Postconditions — ACM 2024](https://dl.acm.org/doi/10.1145/3660791)
- [Self-Spec: Model-Authored Specifications — 2025](https://openreview.net/forum?id=6pr7BUGkLp)
- [AutoSpec: Formal Specification Generation — 2025](https://www.arxiv.org/pdf/2601.12845)
- [Spec-Driven Development — Thoughtworks 2025](https://www.thoughtworks.com/en-us/insights/blog/agile-engineering-practices/spec-driven-development-unpacking-2025-new-engineering-practices)
- [GitHub Spec Kit — 2025](https://github.blog/ai-and-ml/generative-ai/spec-driven-development-with-ai-get-started-with-a-new-open-source-toolkit/)

### Functional Programming and AI
- [Statically Contextualizing LLMs with Typed Holes — OOPSLA 2024](https://huggingface.co/papers/2409.00921)
- [Evaluating AI Impact on Haskell — Well-Typed 2025](https://well-typed.com/blog/2025/04/ai-impact-open-source-haskell/)
- [Functional Programming for AI Code Generation — 2024](https://adamloving.com/2024/08/06/functional-programming-is-better-than-object-oriented-for-ai-code-generation/)
- [Programming Languages in the Age of AI Agents — Nedelcu 2025](https://alexn.org/blog/2025/11/16/programming-languages-in-the-age-of-ai-agents/)

### Content-Addressed Code
- [Unison: The Big Idea](https://www.unison-lang.org/docs/the-big-idea/)
- [Reproducible Builds](https://reproducible-builds.org/)

### Code Editing and AI
- [Code Surgery: How AI Assistants Make Edits — Hertwig 2025](https://fabianhertwig.com/blog/coding-assistants-file-edits/)
- [AI Edit Formats — Morph](https://www.morphllm.com/edit-formats)
- [Aider: Edit Formats](https://aider.chat/docs/more/edit-formats.html)

### Effect Systems
- [Koka: Programming with Row-polymorphic Effect Types](https://arxiv.org/pdf/1406.2061)
- [Composable Effect Handling for LLM Scripts — 2025](https://arxiv.org/pdf/2507.22048)
