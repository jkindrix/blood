# Blood Implementation Roadmap

> Prioritized plan for addressing design oversights before production readiness.
>
> **Reference**: See [DESIGN_OVERSIGHTS.md](./DESIGN_OVERSIGHTS.md) for detailed descriptions of each oversight.

---

## Quick Reference

| Phase | Focus | Items | Target |
|-------|-------|-------|--------|
| **1** | Security Fixes | 4 | Immediate |
| **2** | Runtime Foundation | 4 | Before any deployment |
| **3** | Error Handling | 4 | Before production |
| **4** | Self-Hosting Prerequisites | 4 | Before self-hosted compiler |
| **5** | Production Readiness | 5 | General availability |
| **—** | Deferred | 34 | Post-1.0 |

**Critical path**: Security (#11) → Runtime (#5, #2, #3, #1) → Self-hosting (#22, #14, #21)

---

## Component Classification

### Bootstrap Compiler (`bloodc` — Rust)

| # | Oversight | Complexity | Blocks | Phase |
|---|-----------|------------|--------|-------|
| 21 | DWARF debug symbols | High | Debugging (#4) | 4 |
| 22 | `#[test]` framework | Medium | Self-hosted testing | 4 |
| 23 | Code coverage | Medium | — | Deferred |
| 24 | Variance rules | High | Type soundness | 4 |
| 30 | Deprecation | Low | API evolution | Deferred |
| 51 | Linter | Medium | — | Deferred |
| 53 | Type recursion limits | Low | **Security** | 1 |
| 54 | Coherence rules | High | Type soundness | Deferred |
| 55 | Shared library output | Medium | Plugin system (#52) | Deferred |

### Runtime (`blood-runtime` — Rust)

| # | Oversight | Complexity | Blocks | Phase |
|---|-----------|------------|--------|-------|
| 1 | Memory limits | Medium | Production deployment | 2 |
| 2 | Signal handling | Medium | Graceful shutdown (#3) | 2 |
| 3 | Cancellation | High | Timeouts (#6), resources | 2 |
| 4 | Observability | High | Production debugging | 5 |
| 5 | Runtime configuration | Low | Tuning #1, #7 | 2 |
| 6 | Timeout enforcement | Medium | Requires #3 | 5 |
| 7 | Stack overflow (handlers) | Medium | — | Deferred |
| 8 | Determinism mode | Medium | Testing concurrency | Deferred |
| 9 | Panic handling | Medium | Error recovery | 3 |
| 11 | UTF-8 validation | **Low** | **SECURITY** | 1 |
| 12 | FD limits | Low | — | Deferred |
| 17 | Priority inversion | Medium | Real-time scheduling | Deferred |
| 18 | Backpressure | Medium | — | Deferred |
| 28 | Memory leak detection | Medium | — | Deferred |
| 29 | Fiber-local storage | Medium | Distributed tracing (#4) | 5 |
| 32 | Atomic primitives | Low | Lock-free user code | Deferred |
| 44 | CPU affinity | Low | — | Deferred |
| 46 | Stack trace capture | Medium | Debugging (#4) | 3 |
| 47 | Panic hooks | Low | Panic handling (#9) | 3 |
| 48 | Core dumps | Low | — | Deferred |
| 50 | JIT | Very High | — | Deferred |

### Standard Library (Blood + FFI)

| # | Oversight | Complexity | Blocks | Phase |
|---|-----------|------------|--------|-------|
| 10 | Logging | Low | Observability (#4) | 3 |
| 13 | Networking | High | Self-hosted package mgmt | 5 |
| 14 | Subprocess | Medium | Self-hosted (linker) | 4 |
| 15 | Secure temp files | Low | **Security** | 1 |
| 16 | Endianness | Low | Serialization (#40) | Deferred |
| 19 | Path traversal | Low | **Security** | 1 |
| 20 | Cryptographic RNG | Medium | Security features | Deferred |
| 33 | i18n | Medium | — | Deferred |
| 40 | Serialization | Medium | Persistence, networking | 5 |
| 41 | Schema evolution | High | — | Deferred |
| 42 | BigInt | Medium | Cryptography | Deferred |
| 43 | Decimal | Medium | Financial apps | Deferred |

### Ecosystem & Packaging

| # | Oversight | Complexity | Blocks | Phase |
|---|-----------|------------|--------|-------|
| 26 | 32-bit support | Medium | Embedded/IoT | Deferred |
| 27 | Cross-compilation | Medium | — | Deferred |
| 31 | API stability | Low | Ecosystem trust | Deferred |
| 34 | Package signing | Medium | Supply chain security | Deferred |
| 35 | License compliance | Low | Enterprise adoption | Deferred |
| 36 | Vendoring | Low | Offline builds | Deferred |
| 37 | Symbol visibility | Medium | Shared libs (#55) | Deferred |
| 38 | Name mangling docs | Low | FFI interop | Deferred |
| 39 | Soname versioning | Medium | Shared libs (#55) | Deferred |
| 45 | GPU support | Very High | — | Deferred |
| 49 | Reflection | High | Serialization (#40) | Deferred |
| 52 | Plugin architecture | High | Requires #55 | Deferred |

---

## Dependency Graph

```
PHASE 1: SECURITY ──────────────────────────────────────────────────────────
│
├─► #11 UTF-8 validation (runtime)     ─┐
├─► #53 Type recursion limits (compiler) │  Must fix before ANY deployment
├─► #15 Secure temp files (stdlib)       │
└─► #19 Path traversal (stdlib)        ─┘
                                         │
PHASE 2: RUNTIME FOUNDATION ─────────────┼──────────────────────────────────
                                         │
├─► #5  Runtime configuration ◄──────────┘
│       └── enables tuning of ↓
├─► #2  Signal handling
│       └── enables ↓
├─► #3  Cancellation (HARDEST)
│       └── enables #6 timeouts
└─► #1  Memory limits
                                         │
PHASE 3: ERROR HANDLING ─────────────────┼──────────────────────────────────
                                         │
├─► #47 Panic hooks                      │
│       └── enables ↓                    │
├─► #9  Panic handling                   │
├─► #46 Stack trace capture              │
│       └── enables ↓                    │
└─► #10 Logging (stdlib)                 │
        └── feeds into #4 observability  │
                                         │
PHASE 4: SELF-HOSTING ───────────────────┼──────────────────────────────────
                                         │
├─► #22 Test framework (compiler)        │
├─► #14 Subprocess management (stdlib)   │  invoke linker/assembler
├─► #21 DWARF debug symbols (compiler)   │
└─► #24 Variance rules (compiler)        │
                                         │
PHASE 5: PRODUCTION ─────────────────────┼──────────────────────────────────
                                         │
├─► #4  Observability (runtime)          │
│       └── needs #29, #10               │
├─► #29 Fiber-local storage (runtime)    │
├─► #6  Timeout enforcement (runtime)    │  needs #3
├─► #13 Networking (stdlib)              │
└─► #40 Serialization (stdlib)           │
```

---

## Phase Details

### Phase 1: Security Fixes

> **Goal**: Eliminate security vulnerabilities before any external use.
> **Acceptance**: No known security issues remain.

| # | Task | Component | Est. | Acceptance Criteria |
|---|------|-----------|------|---------------------|
| 11 | UTF-8 validation | Runtime | 1d | All `from_utf8_unchecked()` replaced with checked variant; errors handled gracefully |
| 53 | Type recursion limits | Compiler | 2d | Configurable depth limit; clear error on excessive nesting |
| 19 | Path traversal | Stdlib | 2d | `canonicalize()` implemented; paths validated before file ops |
| 15 | Secure temp files | Stdlib | 1d | `TempFile` type uses OS secure APIs; auto-cleanup on drop |

**Files to modify**:
- `blood-runtime/src/ffi_exports.rs` — #11
- `bloodc/src/typeck/` — #53
- Stdlib path module — #19, #15

---

### Phase 2: Runtime Foundation

> **Goal**: Runtime can handle real-world operational requirements.
> **Acceptance**: Programs can be configured, interrupted, cancelled, and resource-limited.

| # | Task | Component | Est. | Acceptance Criteria |
|---|------|-----------|------|---------------------|
| 5 | Runtime configuration | Runtime | 3d | `BLOOD_*` env vars work; config validated at startup |
| 2 | Signal handling | Runtime | 1w | SIGTERM/SIGINT trigger graceful shutdown; fibers notified |
| 3 | Cancellation | Runtime | 2w | Cancellation tokens propagate; handlers clean up linear values |
| 1 | Memory limits | Runtime | 1w | Per-region quotas enforced; memory pressure callbacks fire |

**Files to modify**:
- `blood-runtime/src/lib.rs` — #5
- `blood-runtime/src/scheduler.rs` — #2, #3
- `blood-runtime/src/fiber.rs` — #3
- `blood-runtime/src/region.rs` — #1

**Key design decisions needed**:
- [ ] Cancellation semantics: cooperative vs preemptive?
- [ ] Linear value cleanup: who is responsible when cancelled?
- [ ] Memory pressure: callback or effect?

---

### Phase 3: Error Handling & Debugging

> **Goal**: Production issues can be diagnosed.
> **Acceptance**: Panics are logged with backtraces; structured logging available.

| # | Task | Component | Est. | Acceptance Criteria |
|---|------|-----------|------|---------------------|
| 47 | Panic hooks | Runtime | 2d | Hooks can be registered; called before termination |
| 46 | Stack trace capture | Runtime | 1w | `Backtrace` type available; captured on errors |
| 9 | Panic handling | Runtime | 3d | Effect handlers can catch panics; recovery possible |
| 10 | Logging | Stdlib | 3d | `Log` effect with levels; structured output option |

**Files to modify**:
- `blood-runtime/src/lib.rs` — #47
- `blood-runtime/src/fiber.rs` — #46, #9
- Stdlib logging module — #10

---

### Phase 4: Self-Hosting Prerequisites

> **Goal**: Self-hosted compiler can be built and tested.
> **Acceptance**: `blood test` works; subprocess spawning works; debug symbols generated.

| # | Task | Component | Est. | Acceptance Criteria |
|---|------|-----------|------|---------------------|
| 22 | Test framework | Compiler | 1w | `#[test]` recognized; `blood test` discovers and runs tests |
| 14 | Subprocess | Stdlib | 1w | `Process` effect works; can invoke linker |
| 21 | DWARF debug symbols | Compiler | 2w | Debug builds include DWARF; gdb/lldb can set breakpoints |
| 24 | Variance rules | Compiler | 1w | Variance inferred or annotated; documented in spec |

**Files to modify**:
- `bloodc/src/attr/` — #22
- `blood-tools/` — #22 (test runner)
- Stdlib process module — #14
- `bloodc/src/codegen/` — #21
- `bloodc/src/typeck/` — #24

---

### Phase 5: Production Readiness

> **Goal**: Production deployment is viable.
> **Acceptance**: Full observability; networking; serialization.

| # | Task | Component | Est. | Acceptance Criteria |
|---|------|-----------|------|---------------------|
| 4 | Observability | Runtime | 2w | Metrics, tracing, introspection APIs available |
| 29 | Fiber-local storage | Runtime | 1w | `FiberLocal<T>` works; context propagates through handlers |
| 6 | Timeout enforcement | Runtime | 1w | Timeouts actually cancel operations (uses #3) |
| 13 | Networking | Stdlib | 3w | DNS, TCP, TLS available |
| 40 | Serialization | Stdlib | 2w | `Serialize` effect; JSON support |

---

## Self-Hosted Compiler Path

The self-hosted compiler has specific dependencies:

```
1. Stable Runtime (Phases 1-3)
   └── Can't test compiler if runtime crashes

2. #22 Test Framework
   └── Need to test the compiler itself

3. #14 Subprocess Management
   └── Compiler must invoke linker/assembler

4. #21 Debug Symbols
   └── Need to debug the compiler when it breaks

5. #13 Networking
   └── Package resolution during compilation

6. #40 Serialization
   └── Caching compiled artifacts
```

**The bootstrap compiler (`bloodc`) handles compilation today. The self-hosted compiler cannot be developed until the runtime is production-ready.**

---

## Progress Tracking

### Phase 1: Security
- [x] #11 UTF-8 validation
- [x] #53 Type recursion limits
- [x] #19 Path traversal prevention
- [x] #15 Secure temp files

### Phase 2: Runtime Foundation
- [x] #5 Runtime configuration
- [x] #2 Signal handling
- [x] #3 Cancellation mechanism
- [x] #1 Memory limits

### Phase 3: Error Handling
- [x] #47 Panic hooks
- [x] #46 Stack trace capture
- [x] #9 Panic handling
- [x] #10 Logging infrastructure

### Phase 4: Self-Hosting
- [x] #22 Test framework
- [x] #14 Subprocess management
- [x] #21 DWARF debug symbols
- [x] #24 Variance rules

### Phase 5: Production
- [x] #4 Observability
- [x] #29 Fiber-local storage
- [x] #6 Timeout enforcement
- [x] #13 Networking
- [x] #40 Serialization

---

## Deferred Items (34 total)

These are tracked in [DESIGN_OVERSIGHTS.md](./DESIGN_OVERSIGHTS.md) but not planned for initial release:

**Type System**: #54 Coherence rules, #23 Code coverage
**Runtime**: #7 Stack overflow, #8 Determinism, #12 FD limits, #17 Priority, #18 Backpressure, #28 Leak detection, #32 Atomics, #44 CPU affinity, #48 Core dumps, #50 JIT
**Stdlib**: #16 Endianness, #20 CSPRNG, #33 i18n, #41 Schema evolution, #42 BigInt, #43 Decimal
**Tooling**: #30 Deprecation, #51 Linter
**Ecosystem**: #26 32-bit, #27 Cross-compile, #31 Stability, #34 Signing, #35 Licenses, #36 Vendoring, #37 Visibility, #38 Mangling, #39 Soname, #45 GPU, #49 Reflection, #52 Plugins, #55 Shared libs

---

## Decision Log

Record key decisions made during implementation:

| Date | Decision | Rationale |
|------|----------|-----------|
| — | — | — |

---

## Notes

- **Complexity estimates are rough** — actual effort depends on existing code structure
- **Phases can overlap** — e.g., start Phase 3 while finishing Phase 2
- **Security fixes are blocking** — nothing else ships until Phase 1 is complete
- **Self-hosting is not the priority** — runtime stability comes first
