# Blood Design Oversights

This document catalogs design oversights discovered during development - features or considerations that were never addressed because the question was never asked, not because a deliberate decision was made.

These are distinct from:
- **Intentional omissions** (documented decisions to exclude something)
- **Deferred features** (planned but not yet implemented)
- **Known limitations** (documented constraints)

---

## Summary

| # | Area | Status | Design Decision |
|---|------|--------|-----------------|
| 1 | Memory Limits | ❌ Missing | Never considered |
| 2 | Signal Handling | ❌ Missing | Never considered |
| 3 | Graceful Shutdown / Cancellation | ❌ Missing | Never considered |
| 4 | Debugging / Observability | ❌ Missing | Never considered |
| 5 | Runtime Configuration | ❌ Missing | Never considered |
| 6 | Timeouts / Deadlines | 🔶 Partial | Spec exists, no implementation |
| 7 | Stack Overflow (Handlers) | 🔶 Partial | Fiber limits exist, handler limits missing |
| 8 | Determinism / Replay Mode | 🔶 Partial | Compile-time OK, runtime non-deterministic |
| 9 | Error Recovery / Panic Handling | 🔶 Partial | Effects designed, panic handling missing |
| 10 | Logging / Tracing | 🟡 Minimal | Effects support it, no stdlib |
| 11 | UTF-8 Validation in FFI | 🔴 **VULN** | Never considered |
| 12 | File Descriptor Limits | ❌ Missing | Never considered |
| 13 | Networking Stack (DNS/TLS/IPv6) | ❌ Missing | Never considered |
| 14 | Subprocess Management | ❌ Missing | Never considered |
| 15 | Secure Temp Files | ❌ Missing | Never considered |
| 16 | Endianness / Byte Order | ❌ Missing | Never considered |
| 17 | Priority Inversion | 🔶 Partial | Priority exists, inversion not handled |
| 18 | Connection Pooling / Backpressure | ❌ Missing | Never considered |
| 19 | Path Traversal Prevention | 🔶 Partial | Spec exists, no implementation |
| 20 | Cryptographic RNG | 🔶 Partial | RNG exists, not cryptographically secure |
| 21 | DWARF / Debug Symbols | ❌ Missing | Never considered |
| 22 | Language-Level Test Framework | 🔶 Partial | Effects can mock, no `#[test]` |
| 23 | Code Coverage | ❌ Missing | Never considered |
| 24 | Variance Rules | ❌ Missing | Never considered |
| 25 | Fuzzing Infrastructure | 🟡 Minimal | Basic property tests only |
| 26 | 32-bit Architecture Support | ❌ Missing | Never considered |
| 27 | Cross-Compilation | ❌ Missing | Never considered |
| 28 | Memory Leak Detection | ❌ Missing | Never considered |
| 29 | Thread-Local / Fiber-Local Storage | ❌ Missing | Never considered |
| 30 | Deprecation Mechanism | ❌ Missing | Never considered |
| 31 | API Stability Guarantees | ❌ Missing | Never considered |
| 32 | Atomic Primitives for Users | ❌ Missing | Used internally only |
| 33 | Internationalization / Localization | ❌ Missing | Never considered |
| 34 | Package Signing | ❌ Missing | Never considered |
| 35 | License Compliance Tracking | ❌ Missing | Never considered |
| 36 | Dependency Vendoring | ❌ Missing | Never considered |
| 37 | Symbol Visibility | ❌ Missing | Never considered |
| 38 | Name Mangling Scheme | ❌ Missing | Never considered |
| 39 | Shared Library Versioning (soname) | ❌ Missing | Never considered |
| 40 | Runtime Serialization | ❌ Missing | Never considered |
| 41 | Schema Evolution / Migration | ❌ Missing | Never considered |
| 42 | Arbitrary Precision Integers | ❌ Missing | Never considered |
| 43 | Decimal Arithmetic | ❌ Missing | Never considered |
| 44 | CPU Affinity / Deterministic Scheduling | ❌ Missing | Never considered |
| 45 | GPU / Accelerator Support | ❌ Missing | Never considered |
| 46 | Stack Trace Capture | ❌ Missing | Never considered |
| 47 | Panic Hooks | ❌ Missing | Never considered |
| 48 | Core Dump Support | ❌ Missing | Never considered |
| 49 | Runtime Reflection | ❌ Missing | Never considered |
| 50 | JIT / Runtime Code Generation | ❌ Missing | Never considered |
| 51 | Linter Tool | ❌ Missing | Never considered |
| 52 | Plugin Architecture | ❌ Missing | Never considered |
| 53 | Type Recursion Limits | ❌ Missing | Never considered |
| 54 | Implementation Coherence Rules | ❌ Missing | Never considered |
| 55 | Compile to Shared Library | ❌ Missing | Never considered |

Legend: ❌ Never considered | 🔶 Partially addressed | 🟡 Minimal | 🟢 Well-designed | 🔴 **Vulnerability**

---

## Oversight #1: No Memory Limits or Resource Management

**Discovery Date**: 2026-02-04

**The Problem**: Blood has no mechanism to limit memory consumption. Programs will consume all available RAM until the OOM killer intervenes.

**What's Missing**:
- No maximum heap size configuration
- No per-region capacity limits
- No memory pressure callbacks or handlers
- No watchdog for runaway allocation
- No graceful degradation on memory exhaustion
- No resource quotas (per-fiber, per-region, per-program)

**Evidence This Was Never Considered**:
1. Original MEMORY_MODEL.md (Jan 9, 2026) describes three memory tiers focused entirely on *safety* (preventing use-after-free) with zero mention of *resource limits*
2. ADR-008 (Tiered Memory Model) discusses allocation strategies but not bounds
3. Research phase examined Vale, Hylo, Koka - all focused on safety/effects, none on resource management
4. No conversation in project history ever asked "should Blood have memory limits?"
5. OOM only appeared as a bug report during self-hosting, treated as allocation strategy problem, not missing design feature

**Why This Matters**:
- Production reliability: Long-running servers can exhaust memory
- Denial of service: Malicious input can cause resource exhaustion
- Embedded/constrained systems: Cannot deploy without resource bounds
- Predictability: No way to enforce memory budgets

**Potential Solutions**:
- Add optional capacity limits to `Region::allocate`
- Implement `MemoryPressure` effect for graceful handling
- Add per-fiber or per-region quotas to runtime configuration
- Install pre-OOM handler hooks

---

## Oversight #2: No Signal Handling

**Discovery Date**: 2026-02-04

**The Problem**: Blood has no mechanism to handle OS signals (SIGTERM, SIGINT, SIGHUP). Programs cannot respond to external interrupts or perform graceful shutdown.

**What's Missing**:
- No signal handler registration or installation infrastructure
- No graceful shutdown mechanism triggered by OS signals
- No resource cleanup on SIGTERM
- Fiber scheduler has no emergency stop mechanism for signal arrival
- No signal-safe event channels to notify running fibers

**Evidence This Was Never Considered**:
- Zero references to `SIGTERM`, `SIGINT`, `SIGHUP` in runtime code
- FFI spec mentions `SIGINT` in C code examples but provides no Blood-level signal handling API
- No ADR or specification discusses signal handling
- CONCURRENCY.md assumes programs run until natural completion

**Why This Matters**:
- Container orchestration (Docker, Kubernetes) signals SIGTERM before killing processes
- Server applications require graceful restarts
- Interactive CLI tools need Ctrl+C handling
- Daemon processes need SIGHUP for config reload

**Potential Solutions**:
- Add `Signal` effect for handling OS signals
- Implement signal-safe channel to fiber scheduler
- Add shutdown hooks to runtime initialization
- Define signal-to-effect mapping in FFI layer

---

## Oversight #3: No Graceful Shutdown or Fiber Cancellation

**Discovery Date**: 2026-02-04

**The Problem**: There is no way to cancel in-flight fibers or operations. Fibers run to completion cooperatively with no preemption possible.

**What's Missing**:
- No cancellation tokens or context that propagate through fiber hierarchies
- No mechanism to abort in-flight operations at effect points
- No handler cleanup/unwinding on fiber cancellation
- No resource cleanup guarantees (linear type consumption) on cancellation
- Fiber states include `Cancelled` but no mechanism to trigger it

**Evidence This Was Never Considered**:
- CONCURRENCY.md specifies structured concurrency but cancellation semantics are implicit
- No abort/kill method on fibers or tasks
- ROADMAP.md doesn't mention cancellation as a planned feature
- Effects can't be interrupted (deep handlers persist through all resumes)
- The interaction between cancellation and mutable effect state was never designed

**Why This Matters**:
- Server shutdown requires ability to stop in-flight requests
- Request timeouts need to cancel work, not just ignore results
- Resource limits require ability to terminate runaway computations
- Without cancellation, systems leak resources and hang during shutdown

**Potential Solutions**:
- Add `Cancel` effect with propagation semantics
- Implement cancellation token pattern (context propagation through effects)
- Define handler cleanup behavior on cancellation (how linear values are consumed)
- Add timeout effect that actually cancels operations

---

## Oversight #4: No Debugging or Observability Infrastructure

**Discovery Date**: 2026-02-04

**The Problem**: The runtime has no hooks for debuggers, profilers, or introspection. Developers must rely on println debugging.

**What's Missing**:
- No runtime introspection API to query fiber states, priorities, stack usage
- No debugger hooks for setting breakpoints, inspecting registers
- No performance profiling infrastructure (CPU flame graphs, allocation tracking)
- No distributed tracing support (OpenTelemetry-like)
- No fiber-local storage for context propagation
- No telemetry export (unlike Go's runtime)

**Evidence This Was Never Considered**:
- DEBUGGING_GUIDE.md only addresses language-level debugging ("use println and test")
- No performance profiling hooks or instrumentation points
- No tracing infrastructure for fiber scheduling decisions
- Specification focuses on correctness and performance, not observability

**Why This Matters**:
- Production systems require visibility into runtime behavior
- Debugging distributed issues is impossible without tracing
- Performance optimization requires profiling data
- Incident response needs introspection capabilities

**Potential Solutions**:
- Add runtime introspection API (fiber list, states, stack traces)
- Implement debugger protocol (DAP) support
- Add profiling hooks (allocation, CPU, blocking)
- Integrate OpenTelemetry-style tracing
- Add fiber-local storage for request context

---

## Oversight #5: No Runtime Configuration Management

**Discovery Date**: 2026-02-04

**The Problem**: Scheduler has compile-time defaults with no way to configure at runtime.

**What's Missing**:
- No environment variable configuration (`BLOOD_NUM_WORKERS`, `BLOOD_STACK_SIZE`)
- No configuration file support (TOML/YAML)
- No command-line flag handling for configuration
- No dynamic reconfiguration (without restart)
- No per-fiber resource limits

**Evidence This Was Never Considered**:
- `SchedulerConfig` in lib.rs has hardcoded defaults:
  - `num_workers`: runtime CPU count
  - `initial_stack_size`: 8 KB
  - `max_stack_size`: 1 MB
- FFI passes config to `init_with()` but applications don't expose configuration
- No ADR discusses configuration management

**Why This Matters**:
- Containerized applications need to adapt to cgroup resource limits
- Different workloads need different tuning (compute vs I/O bound)
- Development vs production may need different settings
- Cannot optimize without ability to tune parameters

**Potential Solutions**:
- Support environment variable configuration
- Add configuration file parsing
- Expose configuration through CLI flags
- Document tunable parameters and their effects

---

## Oversight #6: Incomplete Timeout/Deadline Implementation

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Specified but not implemented

**The Problem**: Timeouts are specified in documentation but not enforced at runtime.

**What Exists**:
- CONCURRENCY.md specifies `select_timeout` for fiber select operations
- ERROR_HANDLING.md shows `Timeout` error variant in examples
- WCET_REALTIME.md documents timing bounds

**What's Missing**:
- Runtime enforcement of timeouts (specification only)
- Timeout effect that expires operations after duration
- Deadline tracking through continuation snapshots
- Automatic cancellation machinery when deadlines exceeded
- Per-fiber timeout contexts

**Evidence**:
- ROADMAP.md mentions "Add timeout, cycle detection" as future work
- No deadline propagation across effect boundaries designed
- Interaction between timeouts and ADR-004 (continuation snapshots) not analyzed

**Why This Matters**:
- Network services require request timeouts
- Distributed systems need deadline propagation
- Without enforcement, slow operations block entire systems

---

## Oversight #7: Incomplete Stack Overflow Protection

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Fiber limits exist, handler limits missing

**The Problem**: Fiber stacks have limits, but compiler-generated code (handlers, matches) can still overflow.

**What Exists**:
- Fiber stack has `max_stack_size` configuration (1MB default)
- Fiber stack grows on-demand with 2x growth factor
- SECURITY_MODEL.md mentions "stack overflow protection"

**What's Missing**:
- No checks for handler recursion depth (deep handlers can nest unboundedly)
- No validation when continuation snapshots captured at deep handler nesting
- No limits on pattern match depth
- No protection against quadratic stack growth in certain effect patterns
- WCET_REALTIME.md references "max_recursion_depth" but not enforced

**Evidence**:
- DEBUGGING_GUIDE.md treats stack overflow as user error to fix, not runtime protection
- Interaction between deep handlers and stack exhaustion never analyzed

**Why This Matters**:
- Malicious or buggy input can trigger unbounded recursion
- Handler nesting can grow without bound
- Crashes rather than graceful degradation

---

## Oversight #8: No Determinism Mode for Testing

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Compile-time reproducibility, runtime non-deterministic

**The Problem**: No way to reproduce exact execution traces for debugging concurrency issues.

**What Exists**:
- Content-addressed code (ADR-003) ensures reproducible compilation
- Hash functions (BLAKE3) are deterministic

**What's Missing**:
- Documented sources of runtime non-determinism
- Seed parameter for deterministic scheduling
- Reproducible I/O completion ordering option
- Determinism mode that logs all non-deterministic decisions

**Sources of Runtime Non-determinism**:
- Fiber scheduler uses work-stealing with implementation-dependent ordering
- HashMap (ADR-029) uses Robin Hood hashing without documented seed control
- I/O reactor completion order is inherently non-deterministic
- Thread scheduling order varies across runs

**Why This Matters**:
- Reproducing concurrency bugs is nearly impossible
- Critical systems need execution trace replay
- Testing concurrent code requires determinism

---

## Oversight #9: Incomplete Panic/Error Recovery

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Effects designed well, panic handling missing

**The Problem**: Error effects exist but interaction with panic and handler failures was not designed.

**What Exists**:
- ERROR_HANDLING.md documents `Result<T, E>` and effect `Error<E>`
- Well-designed error effect system

**What's Missing**:
- No panic handler or catch-panic effect
- Rust `panic!()` in FFI code terminates the process
- No structured error recovery (circuit breakers, retries)
- Handler failure recovery (what happens if handler panics?)
- Linear values cannot be recovered if effect handler panics before resume
- One-shot continuations prevent retry logic

**Why This Matters**:
- Fault-tolerant systems need structured error recovery
- Cannot implement retries, bulkheads, or graceful degradation
- Single panic crashes entire program

---

## Oversight #10: No Standard Logging Infrastructure

**Discovery Date**: 2026-02-04

**Status**: 🟡 Minimal - Effects support logging, no stdlib

**The Problem**: Developers must implement their own Log effect; no standard approach exists.

**What Exists**:
- EFFECTS_TUTORIAL.md shows debug effect examples
- Effects CAN implement logging
- SPECIFICATION.md lists "logging" as an effect use case

**What's Missing**:
- No built-in `Log` effect in stdlib
- No structured logging support (JSON, metrics, tracing)
- No async log flushing
- No log level configuration
- No integration with common logging systems

**Why This Matters**:
- Applications can't easily emit observability data
- No standard way to configure log levels
- Each application reinvents logging

---

## Oversight #11: UTF-8 Validation Vulnerability in FFI

**Discovery Date**: 2026-02-04

**Status**: 🔴 **SECURITY VULNERABILITY**

**The Problem**: FFI string handling in blood-runtime (the Rust implementation) uses `from_utf8_unchecked()` extensively without validating that incoming data is valid UTF-8. This is undefined behavior if FFI provides invalid UTF-8.

**What's Missing**:
- No validation that FFI strings are valid UTF-8
- No safe fallback for invalid UTF-8 data
- No documented UTF-8 contract for FFI boundary

**Evidence** (in blood-runtime, which is Rust code):
- `blood-runtime/src/ffi_exports.rs` uses `std::str::from_utf8_unchecked()` 12+ times
- Lines 1075, 1114, 1159, and many others use unchecked conversion
- No `from_utf8()` (checked) variant found in FFI code
- STDLIB.md defines String as UTF-8 but doesn't specify FFI validation

**Why This Matters**:
- Undefined behavior if C code passes invalid UTF-8
- Memory safety guarantees violated
- Potential for crashes, data corruption, or security exploits
- The runtime's safety model depends on strings being valid UTF-8

**Potential Solutions**:
- Replace `from_utf8_unchecked()` with `from_utf8()` in blood-runtime and handle errors
- Add validation layer at FFI boundary
- Document UTF-8 requirement and validate in debug builds
- Consider lossy UTF-8 conversion for robustness

---

## Oversight #12: No File Descriptor Limits

**Discovery Date**: 2026-02-04

**The Problem**: No tracking or limiting of file descriptors. Programs can exhaust OS fd limits without warning.

**What's Missing**:
- No fd tracking or counting
- No OS limit querying (`getrlimit`/`setrlimit`)
- No graceful degradation when approaching fd limits
- No connection pooling to reuse fds

**Evidence**:
- `blood-runtime/src/io.rs` uses raw `Fd(i32)` with no validation
- Each file open creates new fd without reuse
- STDLIB.md defines `Fd` type but doesn't specify limits

**Why This Matters**:
- Programs can hit "too many open files" errors unexpectedly
- No way to diagnose fd leaks
- Server applications need fd budgeting
- Cannot implement connection pooling without fd awareness

**Potential Solutions**:
- Track open fd count in runtime
- Query and respect OS limits
- Add `FdPool` for connection reuse
- Implement fd exhaustion effect/callback

---

## Oversight #13: No Networking Stack

**Discovery Date**: 2026-02-04

**The Problem**: Blood has raw I/O primitives but no networking abstractions. DNS, TLS, and IP address handling are completely absent.

**What's Missing**:
- No DNS resolution (sync or async)
- No TLS/SSL support or certificate validation
- No IPv4/IPv6 address types
- No socket address abstraction
- No TCP/UDP high-level APIs
- No HTTP client/server

**Evidence**:
- STDLIB.md §7 defines only `Fd` and `File`, no networking types
- `blood-runtime/src/io.rs` has raw fd operations only
- No `Address`, `IpAddr`, `SocketAddr` types anywhere
- No DNS resolver in runtime

**Why This Matters**:
- Cannot build networked applications without FFI
- No secure communication (TLS) possible
- Must implement entire network stack in user code
- IPv6 transition impossible without address types

**Potential Solutions**:
- Add `net` module to stdlib with address types
- Implement async DNS resolver
- Integrate TLS library (rustls/native-tls)
- Define `TcpStream`, `UdpSocket` effects

---

## Oversight #14: No Subprocess Management

**Discovery Date**: 2026-02-04

**The Problem**: Blood can spawn fibers but cannot spawn OS processes. No way to execute external commands.

**What's Missing**:
- No `Process` or `Command` types
- No ability to exec/spawn child processes
- No stdin/stdout/stderr piping
- No process environment manipulation
- No wait/kill for child processes

**Evidence**:
- STDLIB.md has no process types
- `blood-runtime/src/` has no subprocess handling
- CONCURRENCY.md discusses fiber spawning only
- No process spawning capability at Blood level

**Why This Matters**:
- Cannot build shell tools or command runners
- Cannot integrate with external tools (git, compilers, etc.)
- Cannot implement build systems
- Severely limits practical utility

**Potential Solutions**:
- Add `Process` effect for spawning
- Implement `Command` builder pattern
- Define stdio piping semantics
- Add process wait/signal handling

---

## Oversight #15: No Secure Temporary File API

**Discovery Date**: 2026-02-04

**The Problem**: No safe way to create temporary files. Risk of predictable temp file attacks.

**What's Missing**:
- No `mkstemp()` equivalent
- No temp directory abstraction
- No automatic cleanup on scope exit
- No secure random filename generation

**Evidence**:
- STDLIB.md has no mention of temp files
- `blood-runtime/src/io.rs` has no temp file functions
- No `tempfile` or `tempdir` in any module

**Why This Matters**:
- Predictable temp file names enable symlink attacks
- Race conditions in temp file creation
- No automatic cleanup leads to temp file leaks
- Security-sensitive operations need secure temp files

**Potential Solutions**:
- Add `TempFile` type with automatic cleanup
- Use OS-provided secure temp file creation
- Implement scoped temp directories
- Integrate with linear types for cleanup guarantee

---

## Oversight #16: Endianness Not Specified

**Discovery Date**: 2026-02-04

**The Problem**: No byte order specification for serialization or cross-platform data exchange.

**What's Missing**:
- No endianness documentation
- No `to_le_bytes()` / `to_be_bytes()` in stdlib
- No platform-neutral serialization
- No bigint/littleint types

**Evidence**:
- STDLIB.md contains no byte-order specification
- `simd.rs` hash functions assume specific byte order
- No endian-aware I/O functions
- Cross-platform binary compatibility not addressed

**Why This Matters**:
- Binary data broken between platforms (x86 vs ARM)
- Network protocols require specific byte orders
- File format compatibility issues
- Cannot implement portable serialization

**Potential Solutions**:
- Document assumed endianness (likely little-endian)
- Add byte-order conversion functions to stdlib
- Specify network byte order for protocols
- Consider platform-neutral serialization format

---

## Oversight #17: Priority Inversion Not Handled

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Priority exists, inversion not prevented

**The Problem**: Fiber priority levels exist but the scheduler ignores them. No priority inheritance or inversion prevention.

**What Exists**:
- `Priority::Low, Normal, High, Critical` defined in fiber.rs
- `priority` field in FiberConfig
- `with_priority()` method to set priority

**What's Missing**:
- Scheduler doesn't use priority in scheduling decisions
- No priority inheritance when high-priority waits on low-priority
- No priority ceiling protocol
- No priority boosting for lock holders

**Evidence**:
- Priority field exists but scheduler ignores it
- Work-stealing scheduler treats all fibers equally
- CONCURRENCY.md §1.5 mentions priority but not inversion

**Why This Matters**:
- Real-time systems cannot guarantee latency
- High-priority work can be starved
- Priority inversion causes unpredictable delays
- Cannot implement deadline-driven scheduling

**Potential Solutions**:
- Implement priority-aware scheduling
- Add priority inheritance for mutexes
- Consider priority ceiling protocol
- Document priority semantics

---

## Oversight #18: No Connection Pooling or Backpressure

**Discovery Date**: 2026-02-04

**The Problem**: No resource pooling and minimal flow control. Each operation creates new resources; no reuse.

**What's Missing**:
- No connection pool abstraction
- No fd reuse mechanism
- No adaptive backpressure
- No per-fiber rate limiting
- I/O reactor uses fixed 100ms timeout

**Evidence**:
- `blood-runtime/src/io.rs` line 225: `poll_timeout: Duration::from_millis(100)` hardcoded
- Each file operation creates new `Fd` without reuse
- No bounded queue or resource limits
- Channels have capacity but no broader flow control

**Why This Matters**:
- Database connections exhausted quickly
- No way to throttle overloaded systems
- Memory grows unbounded under load
- Cannot implement circuit breakers

**Potential Solutions**:
- Add `Pool<T>` abstraction to stdlib
- Implement adaptive backpressure in I/O reactor
- Add rate limiting effect
- Support bounded concurrency patterns

---

## Oversight #19: Path Traversal Prevention Incomplete

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Specified but not implemented

**The Problem**: Path manipulation functions exist but canonicalization is missing from implementation.

**What Exists**:
- STDLIB.md §7.1: `starts_with()`, `strip_prefix()` exist
- ACTION_ITEMS_V1_COMPLETED.md mentions `canonicalize` as design item

**What's Missing**:
- No `canonicalize()` in runtime code
- No path validation before file operations
- No symlink resolution control
- No jail/chroot abstraction

**Evidence**:
- No `canonicalize()` function in Blood stdlib
- `ffi_exports.rs` doesn't show path validation
- File operations accept paths without sanitization

**Why This Matters**:
- Path traversal attacks (`../../../etc/passwd`)
- Symlink following can escape intended directories
- Cannot safely handle user-provided paths
- Required for any file-handling server

**Potential Solutions**:
- Implement `canonicalize()` in stdlib
- Add path validation before all file operations
- Implement `SafePath` type that's pre-validated
- Add chroot/jail abstraction for sandboxing

---

## Oversight #20: No Cryptographically Secure RNG

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - RNG exists, not cryptographically secure

**The Problem**: Random number generation uses non-cryptographic algorithm. Cannot safely generate keys, tokens, or secrets.

**What Exists**:
- STDLIB.md §10: `random_float()` effect defined
- STDLIB.md §11.2: Handler using `rng.next_f64()`

**What's Missing**:
- No CSPRNG (cryptographically secure PRNG)
- No `random_bytes()` for key generation
- No entropy source specification
- RNG implementation appears to be XorShift-like (predictable)

**Evidence**:
- Handler example uses non-cryptographic RNG
- No mention of `/dev/urandom` or OS entropy
- No crypto module in stdlib

**Why This Matters**:
- Cannot safely generate session tokens
- Cannot generate encryption keys
- Predictable RNG enables attacks
- Security-sensitive applications impossible

**Potential Solutions**:
- Add `SecureRandom` effect using OS entropy
- Implement `random_bytes(n)` for key material
- Document which RNG is cryptographic vs fast
- Consider separate `crypto` stdlib module

---

## Oversight #21: No DWARF Debug Symbol Generation

**Discovery Date**: 2026-02-04

**The Problem**: No debug symbol table generation. Debuggers cannot set breakpoints, inspect variables, or step through Blood code.

**What's Missing**:
- No DWARF debug info generation
- No source maps linking binary to source
- No integration with LLVM debug metadata
- No way to debug Blood programs with gdb/lldb

**Evidence**:
- No `debug_info` or `source_map` functions in codebase
- Diagnostics.rs tracks line/column for errors but not debug symbols
- No LLVM debug intrinsics used in codegen
- No debugger integration documented

**Why This Matters**:
- Cannot debug Blood programs with standard tools
- Production crash debugging impossible
- IDE debugging features unavailable
- Core dump analysis limited to assembly

**Potential Solutions**:
- Generate DWARF debug info in codegen
- Integrate with LLVM DIBuilder
- Support debug vs release build modes
- Add source map generation for stack traces

---

## Oversight #22: No Language-Level Test Framework

**Discovery Date**: 2026-02-04

**Status**: 🔶 Partial - Mocking via effects, but no test discovery

**The Problem**: While effects enable mocking, there's no `#[test]` annotation or test discovery mechanism at the Blood language level.

**What Exists**:
- Rust tests exist for the compiler (`parser_tests.rs`, etc.) - this is appropriate since bloodc is in Rust
- Effects provide natural mocking mechanism
- Handlers enable dependency injection for testing

**What's Missing**:
- No Blood-level `#[test]` annotation
- No test discovery or runner
- No assertion effects in stdlib
- No test isolation guarantees
- STDLIB.md makes no mention of testing

**Why This Matters**:
- No standard way to write tests in Blood
- Test frameworks must be built from scratch
- No integration with CI/CD tools
- Cannot test Blood libraries with Blood tests

**Potential Solutions**:
- Add `#[test]` attribute with test discovery
- Implement `blood test` command
- Add assertion effects to stdlib (e.g., `effect Assert { op assert(condition: bool, message: String); }`)
- Support test filtering and reporting

---

## Oversight #23: No Code Coverage Infrastructure

**Discovery Date**: 2026-02-04

**The Problem**: No instrumentation for code coverage. Cannot measure test coverage.

**What's Missing**:
- No coverage instrumentation in codegen
- No branch coverage tracking
- No integration with coverage tools
- No coverage reporting

**Evidence**:
- No coverage-related code in bloodc/src/
- No llvm coverage intrinsics used
- Must use external Rust tools via FFI

**Why This Matters**:
- Cannot measure test effectiveness
- No way to find untested code paths
- CI/CD coverage gates impossible
- Code quality metrics unavailable

**Potential Solutions**:
- Add LLVM coverage instrumentation
- Integrate with llvm-cov
- Generate coverage reports (lcov, html)
- Support `blood test --coverage`

---

## Oversight #24: Type Variance Rules Not Specified

**Discovery Date**: 2026-02-04

**The Problem**: No documentation of covariance/contravariance rules for generic types. Variance may be incorrect or inconsistent.

**What's Missing**:
- No variance rules in TYPE_INFERENCE.md
- No covariance/contravariance specification
- No variance testing found
- Generic type parameters exist but variance not designed

**Evidence**:
- No mention of "variance" in entire codebase
- No variance annotations on type parameters
- Generic function parameters exist but variance undocumented
- Type system may have implicit variance bugs

**Why This Matters**:
- Subtyping may be unsound
- Generic containers may allow invalid coercions
- Type safety could be violated
- Variance bugs are subtle and hard to find

**Potential Solutions**:
- Specify variance rules for generic parameters
- Add variance inference or explicit annotations (e.g., `struct Container<+T>` for covariant, `<-T>` for contravariant)
- Test variance corner cases
- Document variance in TYPE_INFERENCE.md spec

---

## Oversight #25: Minimal Fuzzing Infrastructure

**Discovery Date**: 2026-02-04

**Status**: 🟡 Minimal - Basic property tests, no comprehensive fuzzing

**The Problem**: Only basic property testing exists. No fuzzing harness generation or structured fuzzing support.

**What Exists**:
- `blood-runtime/tests/property_tests.rs` exists
- Some property-based testing infrastructure

**What's Missing**:
- No libfuzzer or AFL integration
- No fuzzing harness generator
- No corpus management
- No coverage-guided fuzzing
- No documented fuzzing workflow

**Why This Matters**:
- Security vulnerabilities may go undiscovered
- Parser/compiler bugs harder to find
- Cannot systematically test edge cases
- Security audit tooling limited

**Potential Solutions**:
- Add libfuzzer integration
- Generate fuzzing harnesses for parser/compiler
- Document fuzzing workflow
- Integrate with OSS-Fuzz

---

## Oversight #26: No 32-bit Architecture Support

**Discovery Date**: 2026-02-04

**The Problem**: Blood assumes 64-bit architecture. Cannot target i386, armv7, or other 32-bit platforms.

**What's Missing**:
- No 32-bit target guards in blood-runtime
- Memory model assumes 64-bit pointers implicitly
- SIMD support documented for x86_64 and aarch64 only
- No 32-bit-specific testing or CI

**Evidence**:
- Cargo.toml contains no target configurations for 32-bit
- 128-bit Blood pointers (generational references) require 64-bit alignment assumptions
- SIMD code paths in runtime only for 64-bit architectures

**Why This Matters**:
- Cannot deploy on embedded 32-bit systems
- IoT devices often use 32-bit ARM
- Legacy system support impossible
- WebAssembly is effectively 32-bit memory model

**Potential Solutions**:
- Add 32-bit configuration guards in blood-runtime
- Test on 32-bit targets in CI
- Consider alternative generational reference layouts for 32-bit (e.g., split pointer/generation)
- Document platform requirements

---

## Oversight #27: No Cross-Compilation Support

**Discovery Date**: 2026-02-04

**The Problem**: No documented support for compiling Blood code targeting different platforms than the host.

**What's Missing**:
- No cross-compilation documentation
- No `--target` flag for compiler
- No sysroot or target specification
- No build scripts for cross targets
- ROADMAP.md doesn't list cross-compilation

**Evidence**:
- GETTING_STARTED.md has no cross-compilation section
- No target configuration examples in docs
- Compiler assumes host == target

**Why This Matters**:
- Cannot build from Linux for Windows (or vice versa)
- Cannot build for embedded from desktop
- CI/CD requires native builds on each platform
- Mobile development impossible

**Potential Solutions**:
- Add `--target` flag to compiler
- Document sysroot configuration
- Provide pre-built target specs
- Add cross-compilation CI jobs

---

## Oversight #28: No Memory Leak Detection

**Discovery Date**: 2026-02-04

**The Problem**: No infrastructure for detecting memory leaks. Allocation tracking exists but not for leak detection.

**What's Missing**:
- No leak tracking hooks in allocator
- No integration with valgrind/ASAN
- No leak report at program exit
- No allocation/deallocation tracking for debugging

**Evidence**:
- SlotRegistry tracks allocations but not for leak detection
- No memcheck or valgrind integration documented
- No ASAN support in build configuration

**Why This Matters**:
- Memory leaks go undetected until OOM
- Long-running servers accumulate leaks
- Debug builds can't identify leak sources
- No way to verify memory cleanup

**Potential Solutions**:
- Add leak tracking mode to allocator
- Integrate with ASAN/MSAN/LSAN
- Report leaked allocations at exit
- Add `--detect-leaks` flag to runtime

---

## Oversight #29: No Thread-Local or Fiber-Local Storage

**Discovery Date**: 2026-02-04

**The Problem**: No way to store per-fiber or per-thread context. Cannot propagate request context through call chains.

**What's Missing**:
- No fiber-local storage API for Blood
- No context propagation mechanism
- No way to store request ID, user context, etc.
- No implicit context threading through effect handlers

**Evidence**:
- No fiber-local storage in fiber.rs
- CONCURRENCY.md doesn't specify context storage
- No context type in stdlib

**Why This Matters**:
- Cannot implement request tracing
- Cannot propagate user context through handlers
- Logging cannot include request context
- Distributed tracing impossible without context

**Potential Solutions**:
- Add `FiberLocal<T>` type to Blood stdlib
- Implement context propagation effect (e.g., `effect Context<T> { op get() -> T; }`)
- Support structured concurrency context inheritance
- Add request context storage to fiber state in runtime

---

## Oversight #30: No Deprecation Mechanism

**Discovery Date**: 2026-02-04

**The Problem**: No way to mark APIs as deprecated with warnings and migration guidance.

**What's Missing**:
- No `#[deprecated]` attribute
- No deprecation timeline policy
- No migration guides for breaking changes
- No compiler warnings for deprecated usage

**Evidence**:
- No deprecation attribute in stdlib
- SECURITY_ADVISORY.md discusses yanking but not deprecation
- No version-to-version migration docs

**Why This Matters**:
- Users have no warning before breaking changes
- Cannot gracefully evolve APIs
- No upgrade path guidance
- Ecosystem stability affected

**Potential Solutions**:
- Add `#[deprecated(since = "1.0", note = "...")]` attribute
- Implement deprecation warnings in compiler
- Document deprecation policy
- Generate migration guides

---

## Oversight #31: No API Stability Guarantees

**Discovery Date**: 2026-02-04

**The Problem**: No formal compatibility policy for language, stdlib, or runtime APIs.

**What's Missing**:
- No documented stability guarantees
- No API compatibility promise
- FFI boundary stability not specified
- Runtime FFI exports have no versioning
- No distinction between stable/unstable APIs

**Evidence**:
- Version 0.2.0 in workspace (pre-1.0)
- No stability annotations on functions
- runtime FFI exports (blood_scheduler_*, etc.) unversioned

**Why This Matters**:
- Users can't rely on APIs not changing
- No SemVer guarantees for library users
- FFI consumers may break without warning
- Cannot build stable ecosystem

**Potential Solutions**:
- Document stability policy for 1.0
- Mark APIs as stable/unstable
- Version FFI exports
- Provide compatibility testing

---

## Oversight #32: No Atomic Primitives for Users

**Discovery Date**: 2026-02-04

**The Problem**: Atomic operations used internally in blood-runtime but not exposed to Blood language users.

**What's Missing**:
- No `Atomic<T>` types in Blood stdlib
- No atomic load/store/CAS operations at Blood level
- No memory ordering specification for Blood code
- Lock-free algorithms impossible for Blood users

**Evidence**:
- blood-runtime uses Rust's atomics extensively (appropriate for runtime)
- No atomic types in STDLIB.md for Blood
- Atomic primitives used internally but not exposed via FFI or Blood stdlib

**Why This Matters**:
- Cannot implement lock-free data structures in Blood
- Performance-critical code needs atomics
- Must use Mutex for simple counters
- Cannot port lock-free algorithms to Blood

**Potential Solutions**:
- Add `Atomic<U32>`, `Atomic<U64>`, `Atomic<Usize>`, `Atomic<Bool>` to Blood stdlib
- Define memory ordering semantics (Relaxed, Acquire, Release, SeqCst)
- Document lock-free patterns in Blood
- Consider `AtomicRef<T>` interaction with generational references

---

## Oversight #33: No Internationalization or Localization

**Discovery Date**: 2026-02-04

**The Problem**: All error messages, documentation, and tooling assume English. No infrastructure for translation or locale awareness.

**What's Missing**:
- No error message localization (all hardcoded English)
- No message catalogs or translation system
- No locale-aware number/date formatting
- No right-to-left (RTL) text support
- No Unicode normalization (NFC/NFD)
- No combining character handling

**Evidence**:
- `bloodc/src/diagnostics.rs` - error messages directly embedded
- `blood-tools/fmt/src/formatter.rs` - assumes LTR text
- No `.po`, `.mo`, or other translation files
- No locale detection or configuration

**Why This Matters**:
- Non-English speakers get poor error messages
- Cannot build internationalized applications easily
- RTL language support broken
- Global adoption limited

**Potential Solutions**:
- Externalize all user-facing strings
- Add locale detection and configuration
- Support ICU or similar i18n library
- Add RTL support to formatter

---

## Oversight #34: No Package Signing

**Discovery Date**: 2026-02-04

**The Problem**: Packages distributed without cryptographic signatures. No way to verify package authenticity or integrity.

**What's Missing**:
- No cryptographic signatures on packages
- No trusted key infrastructure
- No signature verification in package manager
- Security Advisory System has no signature requirements
- PACKAGE_MANIFEST.md has no signature section

**Evidence**:
- PACKAGE_REGISTRY.md doesn't specify signature verification
- No GPG, Sigstore, or similar integration
- No public key distribution mechanism

**Why This Matters**:
- Supply chain attacks possible
- Cannot verify package hasn't been tampered
- No trust chain for dependencies
- Enterprise security requirements unmet

**Potential Solutions**:
- Integrate Sigstore or GPG signing
- Add signature verification to package manager
- Establish key distribution infrastructure
- Document signing requirements for registry

---

## Oversight #35: No License Compliance Tracking

**Discovery Date**: 2026-02-04

**The Problem**: No tooling to track, audit, or enforce license compliance across dependencies.

**What's Missing**:
- No license conflict detection
- No SBOM (Software Bill of Materials) generation
- No license audit tooling
- Manifest supports `license` field but no enforcement
- No transitive license tracking

**Evidence**:
- No license-related commands in tooling
- No license database or compatibility matrix
- PACKAGE_MANIFEST.md mentions license but doesn't enforce

**Why This Matters**:
- Legal compliance risk for users
- Enterprise adoption blocked
- Open source license violations possible
- Cannot generate compliance reports

**Potential Solutions**:
- Add `blood license audit` command
- Implement license compatibility checking
- Generate SBOM in standard formats (SPDX, CycloneDX)
- Warn on incompatible license combinations

---

## Oversight #36: No Dependency Vendoring

**Discovery Date**: 2026-02-04

**The Problem**: Cannot vendor dependencies for offline builds or reproducibility.

**What's Missing**:
- No `--vendor` flag in tooling
- No `[vendor]` section in manifest
- Cannot copy dependencies into project
- Offline builds may fail

**Evidence**:
- No vendoring mentioned in PACKAGE_MANIFEST.md
- No vendor directory convention
- No checksum verification for vendored deps

**Why This Matters**:
- Air-gapped environments can't build
- Registry unavailability breaks builds
- Reproducibility depends on external services
- Enterprise security policies may require vendoring

**Potential Solutions**:
- Add `blood vendor` command
- Support `vendor/` directory convention
- Include checksums for verification
- Document offline build workflow

---

## Oversight #37: No Symbol Visibility Specification

**Discovery Date**: 2026-02-04

**The Problem**: No distinction between public and private symbols in compiled output. All symbols potentially visible.

**What's Missing**:
- No visibility/export declarations on FFI functions
- No public/private symbol distinction
- No hidden visibility for internal symbols
- VFT (Virtual Function Table) has no versioning

**Evidence**:
- FFI exports (`blood_ffi_*`) have no visibility attributes
- No `#[visibility(hidden)]` or similar annotation
- `bloodc/src/content/vft.rs` has no versioning

**Why This Matters**:
- Symbol collisions between libraries
- Internal implementation exposed
- Cannot hide implementation details
- ABI surface larger than necessary

**Potential Solutions**:
- Add visibility attributes to functions
- Default to hidden, explicit export
- Version VFT for ABI stability
- Document symbol visibility rules

---

## Oversight #38: No Name Mangling Scheme Documentation

**Discovery Date**: 2026-02-04

**The Problem**: No documented scheme for how Blood function names become symbols in compiled code.

**What's Missing**:
- No mangling scheme specification
- Effect types and symbol naming interaction unclear
- Generic instantiation naming not documented
- No demangling tool

**Evidence**:
- No mangling documentation in specs
- `blood-tools/ucm/src/hash.rs` uses content hash, not mangled names
- FFI uses direct symbol names

**Why This Matters**:
- Cannot predict symbol names for FFI
- Debugging harder without demangle
- Linker errors confusing
- Interop with other languages difficult

**Potential Solutions**:
- Document mangling scheme (like Itanium C++ ABI)
- Provide `blood demangle` tool
- Consider stable symbol naming for FFI
- Document generic instantiation naming

---

## Oversight #39: No Shared Library Versioning (soname)

**Discovery Date**: 2026-02-04

**The Problem**: No soname or symbol versioning for shared libraries. Cannot safely upgrade libraries.

**What's Missing**:
- No soname specification for compiled libraries
- No symbol versioning scheme
- No version scripts for linker
- Cannot have multiple library versions coexist

**Evidence**:
- No `-Wl,-soname` or equivalent in codegen
- No `.symver` directives
- No library versioning documentation

**Why This Matters**:
- Library upgrades break applications
- Cannot maintain ABI compatibility
- No graceful deprecation of symbols
- Package managers can't manage versions

**Potential Solutions**:
- Generate soname from package version
- Support symbol versioning
- Document ABI stability policy
- Add version scripts to build

---

## Oversight #40: No Runtime Serialization Framework

**Discovery Date**: 2026-02-04

**The Problem**: No built-in way to serialize Blood values at runtime. Must use FFI for all serialization.

**What's Missing**:
- No `Serialize`/`Deserialize` effects or interfaces
- No JSON, CBOR, MessagePack support
- No binary serialization format
- AST serialization exists but not runtime values

**Evidence**:
- STDLIB.md has no serialization effects
- `bloodc/src/ast.rs` uses serde for compile-time only (appropriate since bloodc is Rust)
- No runtime serialization in blood-runtime

**Why This Matters**:
- Cannot save/load application state
- Network protocols require manual encoding
- Database storage requires FFI
- Configuration files need custom parsing

**Potential Solutions**:
- Add `Serialize` effect to stdlib (e.g., `effect Serialize<T> { op to_bytes(value: T) -> Bytes; }`)
- Support common formats (JSON, CBOR)
- Define binary format for Blood values
- Consider code generation for serialization implementations

---

## Oversight #41: No Schema Evolution or Migration

**Discovery Date**: 2026-02-04

**The Problem**: No strategy for evolving data formats or migrating between versions.

**What's Missing**:
- No versioning for serialized data
- No migration tooling
- No backwards compatibility for stored data
- `STORAGE_VERSION` handles format but not data migration

**Evidence**:
- `bloodc/src/content/storage.rs` has version but no migration
- No schema evolution documentation
- Breaking changes between versions not documented

**Why This Matters**:
- Cannot upgrade without data loss
- Long-lived data becomes inaccessible
- No forward compatibility
- Database migrations impossible

**Potential Solutions**:
- Define schema versioning strategy
- Add migration effect/framework
- Document compatibility guarantees
- Support gradual schema evolution

---

## Oversight #42: No Arbitrary Precision Integers

**Discovery Date**: 2026-02-04

**The Problem**: Only fixed-width integers (i8-i128). Cannot represent integers larger than 128 bits.

**What's Missing**:
- No `BigInt` type
- No arbitrary precision arithmetic
- Overflow behavior inconsistent
- Cannot represent very large numbers

**Evidence**:
- STDLIB.md defines only bounded integers
- No bigint library in stdlib
- Cryptographic applications limited

**Why This Matters**:
- Cryptography often needs large integers
- Financial calculations may overflow
- Scientific computing limited
- UUID/hash handling awkward

**Potential Solutions**:
- Add `BigInt` to stdlib
- Define overflow semantics clearly
- Support efficient bigint operations
- Consider GMP or similar integration

---

## Oversight #43: No Decimal Arithmetic

**Discovery Date**: 2026-02-04

**The Problem**: No decimal type for precise decimal math. Floating-point used for all non-integer numbers.

**What's Missing**:
- No `Decimal` type
- No fixed-point arithmetic
- No precision/scale control
- Floating-point representation for all decimals

**Evidence**:
- STDLIB.md has only f32/f64
- No decimal library
- Financial calculations impossible without precision loss

**Why This Matters**:
- Financial applications have precision requirements
- Currency calculations wrong with floats
- Tax/accounting systems can't use Blood
- Regulatory compliance impossible

**Potential Solutions**:
- Add `Decimal` type to stdlib
- Support configurable precision/scale
- Consider IEEE 754 decimal formats
- Add rounding mode control

---

## Oversight #44: No CPU Affinity or Deterministic Scheduling

**Discovery Date**: 2026-02-04

**The Problem**: Cannot pin fibers to CPUs or guarantee deterministic scheduling order.

**What's Missing**:
- No CPU affinity control for fibers
- No deterministic scheduling mode
- No NUMA awareness
- Work-stealing is inherently non-deterministic

**Evidence**:
- WCET_REALTIME.md specifies bounds but no enforcement
- Scheduler uses work-stealing (non-deterministic)
- No `pthread_setaffinity_np` exposed to users
- DESIGN_OVERSIGHTS.md #8 notes determinism gap

**Why This Matters**:
- Real-time systems need determinism
- Performance optimization needs affinity
- NUMA systems need locality
- Bug reproduction needs deterministic replay

**Potential Solutions**:
- Add fiber affinity configuration
- Implement deterministic scheduling mode
- Add NUMA-aware allocation
- Support scheduling replay for debugging

---

## Oversight #45: No GPU or Accelerator Support

**Discovery Date**: 2026-02-04

**The Problem**: No support for GPU compute, tensor operations, or hardware accelerators.

**What's Missing**:
- No CUDA, OpenCL, or Vulkan compute support
- No tensor/matrix primitives
- No GPU memory management
- SIMD limited to CPU only
- No heterogeneous compute framework

**Evidence**:
- `blood-runtime/src/simd.rs` is CPU SIMD only
- No GPU-related code in codebase
- No accelerator documentation
- STDLIB.md has no compute primitives

**Why This Matters**:
- ML/AI workloads need GPU
- Scientific computing limited
- Graphics applications impossible
- Cannot compete with modern languages

**Potential Solutions**:
- Add compute effect for GPU dispatch
- Support SPIR-V or similar IR
- Define tensor types in stdlib
- Consider SYCL or similar abstraction

---

## Oversight #46: No Stack Trace Capture

**Discovery Date**: 2026-02-04

**The Problem**: No stack trace capture on errors or panics. Cannot diagnose where errors originated.

**What's Missing**:
- No backtrace capture in Error effects
- Runtime panics terminate without context
- No backtrace integration in Blood language
- FFI panics leave no trace
- Error propagation chains lost

**Evidence**:
- No backtrace-related code in blood-runtime
- DIAGNOSTICS.md specifies compile-time locations only
- Unchecked operations in runtime panic with no context

**Why This Matters**:
- Production crash debugging impossible
- Cannot trace error origins
- FFI boundary errors untraceable
- Support tickets lack context

**Potential Solutions**:
- Add `Backtrace` type to Blood stdlib
- Capture stack trace in Error effect handlers
- Add `with_backtrace()` method to error types
- Include trace in error output (integrate with runtime's backtrace capture)

---

## Oversight #47: No Panic Hooks

**Discovery Date**: 2026-02-04

**The Problem**: No way to intercept panics. Single panic crashes entire program.

**What's Missing**:
- No panic handler registration at Blood language level
- No global panic hook mechanism
- Effect handler panics abort unrecoverably
- Cannot log panics before termination
- No structured recovery from panics

**Evidence**:
- No panic hook setup in blood-runtime/src/lib.rs (the Rust runtime)
- CONCURRENCY.md doesn't address panic recovery
- No `CatchPanic` effect

**Why This Matters**:
- Cannot log errors before crash
- Cannot notify monitoring systems
- Cannot attempt graceful degradation
- Testing panic behavior impossible

**Potential Solutions**:
- Add `Panic` effect for intercepting panics (e.g., `effect Panic { op on_panic(info: PanicInfo); }`)
- Implement `catch_panic` as an effect handler pattern
- Log panics before termination via runtime hooks
- Support panic recovery in effect handlers

---

## Oversight #48: No Core Dump Support

**Discovery Date**: 2026-02-04

**The Problem**: No crash dump generation for post-mortem debugging.

**What's Missing**:
- No core dump generation on crash
- No minidump format support
- No crash reporting integration
- No post-mortem debugger support

**Evidence**:
- No dump-related code in codebase
- No crash reporting documentation
- Relies on OS-level core dumps only

**Why This Matters**:
- Cannot debug production crashes
- No crash analytics
- Post-mortem analysis limited
- Cannot reproduce crash conditions

**Potential Solutions**:
- Generate minidumps on panic
- Integrate crash reporting (Sentry, Crashpad)
- Include Blood-specific state in dumps
- Document core dump analysis

---

## Oversight #49: No Runtime Reflection

**Discovery Date**: 2026-02-04

**The Problem**: Cannot inspect types or struct fields at runtime. No reflection API.

**What's Missing**:
- No `TypeInfo` API for introspection
- Cannot query type layout at runtime
- No field name/offset queries
- No dynamic struct field access
- No effect/handler enumeration
- Generics info not available at runtime

**Evidence**:
- Type tags exist for dispatch but not introspection
- STDLIB.md has no reflection types
- No mirror/reflect module

**Why This Matters**:
- Serialization frameworks need reflection
- ORM mapping impossible
- Test fixture generation blocked
- Generic code patterns limited

**Potential Solutions**:
- Add `TypeInfo` type to stdlib
- Support field enumeration via `effect Reflect { op fields(type: TypeId) -> List<FieldInfo>; }`
- Provide layout queries
- Consider compile-time code generation for reflection metadata

---

## Oversight #50: No JIT or Runtime Code Generation

**Discovery Date**: 2026-02-04

**The Problem**: Compilation is always ahead-of-time. Cannot generate code at runtime.

**What's Missing**:
- No JIT compilation
- No runtime code generation
- No eval() or similar
- Cannot compile strings to code
- No hot patching

**Evidence**:
- ROADMAP.md has no JIT work
- Compiler outputs static binaries only
- No LLVM JIT integration

**Why This Matters**:
- Cannot implement DSLs with runtime compilation
- No hot code replacement
- Performance-critical inner loops can't be specialized
- REPL limited to interpreter

**Potential Solutions**:
- Add LLVM ORC JIT integration
- Support runtime compilation effect
- Enable hot code swapping
- Document JIT use cases

---

## Oversight #51: No Linter Tool

**Discovery Date**: 2026-02-04

**The Problem**: No static analysis linter. Warning codes exist but no tool to check them.

**What's Missing**:
- No `blood lint` command
- No custom lint rules
- No lint configuration
- Warning codes (W01xx-W08xx) unused
- No dedicated linter tool for Blood code

**Evidence**:
- DIAGNOSTICS.md specifies warnings but no linter
- TOOLING.md has no linting section
- No `blood-tools/lint/` directory

**Why This Matters**:
- Code quality not enforced
- Best practices not checked
- Potential bugs not flagged
- CI/CD lint gates impossible

**Potential Solutions**:
- Implement `blood lint` command
- Define default lint rules (unused variables, unreachable code, effect handler patterns)
- Support custom lint configuration via manifest
- Integrate with IDE via LSP diagnostics

---

## Oversight #52: No Plugin Architecture

**Discovery Date**: 2026-02-04

**The Problem**: Cannot load code at runtime. No plugin system for extensibility.

**What's Missing**:
- No plugin loading mechanism
- No module descriptor format
- No plugin versioning
- No sandbox for plugins
- Cannot discover plugins dynamically

**Evidence**:
- No plugin documentation
- No dynamic loading API
- FFI allows calling out but not loading in

**Why This Matters**:
- Cannot build extensible applications
- No plugin ecosystems
- Feature flags require recompilation
- Cannot implement editors/IDEs in Blood

**Potential Solutions**:
- Design plugin ABI
- Add plugin discovery
- Implement sandboxing
- Support plugin versioning

---

## Oversight #53: No Type Recursion Limits

**Discovery Date**: 2026-02-04

**The Problem**: No limits on type nesting depth. Adversarial types can DoS compiler.

**What's Missing**:
- No maximum type depth enforced
- No protection against infinite type expansion
- Recursive generics can expand unboundedly
- No `max_type_depth` configuration
- Compiler can OOM on deep types

**Evidence**:
- TYPE_INFERENCE.md has no depth limits
- No recursion depth checks in typeck
- Test suite doesn't test depth limits

**Why This Matters**:
- Compiler DoS vulnerability
- Compile times can explode
- Memory exhaustion on pathological types
- Generic specialization combinatorial explosion

**Potential Solutions**:
- Add configurable type depth limit
- Error on excessive nesting
- Warn on deep generic instantiation
- Document limits

---

## Oversight #54: No Implementation Coherence Rules

**Discovery Date**: 2026-02-04

**The Problem**: No rules preventing overlapping or conflicting effect handler implementations.

**What's Missing**:
- No coherence rules for effect implementations
- No rules preventing conflicting handlers from different packages
- Can define blanket handlers for all types without restriction
- Handler precedence undefined when multiple handlers could apply
- Cross-package handler conflicts not prevented

**Evidence**:
- DISPATCH.md mentions E0234 (overlap) but no prevention mechanism
- No coherence specification for effect handlers
- No RFC or ADR on handler coherence

**Why This Matters**:
- Type dispatch soundness at risk
- Handler conflicts cause confusing errors
- Library ecosystem conflicts inevitable as packages grow
- Cannot guarantee deterministic effect dispatch

**Potential Solutions**:
- Specify coherence rules for effect handlers (e.g., require handlers to be defined in same package as either effect or type)
- Detect overlapping handler implementations at compile time
- Document allowed handler implementation patterns
- Add coherence checking pass to typeck

---

## Oversight #55: Cannot Compile to Shared Library

**Discovery Date**: 2026-02-04

**The Problem**: Can only compile to static binaries. Cannot produce .so/.dll/.dylib.

**What's Missing**:
- No shared library output mode
- Cannot produce `.so` / `.dylib` / `.dll`
- Only static linking of Blood code
- No dynamic library target type
- FFI.md specifies consuming DLLs, not producing

**Evidence**:
- Compiler outputs `.o` files, links to binary
- No library output type concept
- No `--output-type=dylib` flag or equivalent

**Why This Matters**:
- Cannot distribute Blood libraries for FFI consumption
- Cannot build plugins in Blood
- Cannot integrate Blood into larger systems
- No dynamic loading of Blood code

**Potential Solutions**:
- Add `--output-type=dylib` flag to compiler
- Generate proper shared library symbols with visibility control
- Support symbol export annotations (e.g., `#[export]`)
- Document shared library ABI and calling conventions

---

## Priority for Addressing

### 🔴 Security Vulnerabilities (fix immediately)
1. **#11 UTF-8 Validation** - Unchecked conversion in blood-runtime is UB with invalid input
2. **#19 Path Traversal** - No canonicalization enables directory escape
3. **#15 Temp Files** - Predictable names enable symlink attacks
4. **#53 Type Recursion** - Compiler DoS via pathological types
5. **#54 Coherence Rules** - Type dispatch soundness at risk

### Critical (blocks production use)
4. Signal handling (SIGTERM, graceful shutdown)
5. Cancellation mechanism
6. Memory limits / resource quotas
7. Runtime configuration
8. File descriptor limits

### Important (needed for real applications)
9. Networking stack (DNS, TLS, addresses)
10. Subprocess management
11. Debugging/introspection APIs
12. Timeout enforcement
13. Logging infrastructure
14. Cryptographic RNG

### Medium (operational concerns)
15. Connection pooling / backpressure
16. Priority inversion handling
17. Endianness specification
18. Panic recovery patterns
19. Variance rules specification

### Medium (ecosystem maturity)
20. Package signing (#34)
21. License compliance (#35)
22. Symbol visibility (#37)
23. Shared library versioning (#39)
24. Runtime serialization (#40)

### Important (debugging & tooling)
25. Stack trace capture (#46)
26. Panic hooks (#47)
27. Linter tool (#51)
28. Compile to shared library (#55)

### Lower (nice-to-have)
29. Handler stack depth limits
30. Determinism mode for testing
31. DWARF debug symbols
32. Language-level test framework
33. Code coverage
34. Fuzzing infrastructure
35. 32-bit architecture support
36. Cross-compilation support
37. Memory leak detection
38. Fiber-local storage
39. Deprecation mechanism
40. API stability guarantees
41. Atomic primitives for users
42. Internationalization (#33)
43. Dependency vendoring (#36)
44. Name mangling docs (#38)
45. Schema evolution (#41)
46. Arbitrary precision (#42)
47. Decimal arithmetic (#43)
48. CPU affinity (#44)
49. GPU support (#45)
50. Core dump support (#48)
51. Runtime reflection (#49)
52. JIT support (#50)
53. Plugin architecture (#52)

---

## Conclusion

**55 design oversights identified.** The Blood design is exceptionally thorough for *language semantics* (memory safety, effects, type system), but has significant gaps in:

### Security (3 vulnerabilities)
- **#11 UTF-8 validation** - undefined behavior at FFI boundary
- **#19 Path traversal** - no protection against `../` attacks
- **#15 Predictable temp files** - enables symlink attacks
- **#20 Non-cryptographic RNG** - cannot generate secure tokens

### Runtime Operations (8 missing)
- **#1 Memory limits** - no resource bounds
- **#2 Signal handling** - no SIGTERM/SIGINT
- **#3 Cancellation** - fibers cannot be stopped
- **#4 Observability** - no debugging/profiling
- **#5 Configuration** - hardcoded defaults
- **#12 FD limits** - no fd tracking
- **#17 Priority inversion** - priorities ignored
- **#18 Backpressure** - no flow control

### Completeness (4 missing)
- **#13 Networking** - no DNS/TLS/addresses
- **#14 Subprocesses** - cannot run commands
- **#21 Debug symbols** - no DWARF generation
- **#22 Test framework** - no language-level testing

### Type System (2 gaps)
- **#24 Variance** - covariance/contravariance unspecified
- Potential soundness issues in generic code

### Platform & Portability (3 gaps)
- **#26 32-bit support** - cannot target embedded/IoT
- **#27 Cross-compilation** - no `--target` flag
- No ARM32, RISC-V, or legacy platform support

### Developer Experience (5 gaps)
- **#22 Test framework** - no language-level testing
- **#23 Code coverage** - no instrumentation
- **#28 Leak detection** - no debugging tools
- **#29 Fiber-local storage** - no context propagation
- **#30-31 Deprecation/stability** - no API guarantees

### Ecosystem & Packaging (4 gaps)
- **#34 Package signing** - no cryptographic verification
- **#35 License compliance** - no SBOM or audit
- **#36 Dependency vendoring** - no offline builds
- **#33 Internationalization** - English-only errors/tooling

### Binary Compatibility (3 gaps)
- **#37 Symbol visibility** - all symbols exposed
- **#38 Name mangling** - undocumented scheme
- **#39 Soname versioning** - no library versioning

### Data Handling (4 gaps)
- **#40 Serialization** - no runtime serialize/deserialize
- **#41 Schema evolution** - no migration support
- **#42 BigInt** - no arbitrary precision
- **#43 Decimal** - no financial arithmetic

### Hardware & Performance (2 gaps)
- **#44 CPU affinity** - no deterministic scheduling
- **#45 GPU support** - no accelerator compute

### Error Handling & Debugging (3 gaps)
- **#46 Stack traces** - no backtrace capture
- **#47 Panic hooks** - no interception
- **#48 Core dumps** - no crash dumps

### Reflection & Metaprogramming (2 gaps)
- **#49 Runtime reflection** - no field inspection
- **#50 JIT** - no runtime code generation

### Tooling Gaps (2 gaps)
- **#51 Linter** - no static analysis tool
- **#52 Plugin architecture** - no extensibility

### Type System Soundness (2 gaps)
- **#53 Type recursion limits** - compiler DoS possible
- **#54 Coherence rules** - overlapping effect handler implementations allowed

### Deployment (1 gap)
- **#55 Shared libraries** - cannot produce .so/.dll

### Partially Addressed (8 items)
- Timeouts, stack overflow, determinism, panic recovery, logging, path canonicalization, CSPRNG, fuzzing

**Production readiness**: The security vulnerabilities (#11, #15, #19, #53) and operational gaps (#2, #3, #5) must be addressed before production use. The core language is well-designed, but the runtime is incomplete for real-world deployment.

**Total**: 55 oversights documented, 4 security-related issues, 6 critical blockers for production.

---

## How to Use This Document

When an oversight is identified:
1. Add a new section with discovery date
2. Describe what's missing
3. Document evidence it was never considered (vs. intentionally omitted)
4. Explain why it matters
5. Sketch potential solutions

Oversights may eventually become:
- ADRs (if we decide to address them)
- Documented non-goals (if we decide against addressing them)
- Backlog items (if we defer the decision)
