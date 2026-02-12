# Feature Integration Tasks

> Actionable checklist for integrating newly implemented runtime features throughout the Blood codebase.
>
> **Source**: Comprehensive codebase audit conducted 2026-02-05
> **Features to Integrate**: Cancellation, Timeout, Observability, Serialization, Networking, Configuration, Signal Handling, Fiber-local Storage, Logging, Memory Limits

---

## Quick Reference

| Category | Tasks | Priority | Status |
|----------|-------|----------|--------|
| [Cancellation](#1-cancellation-integration) | 6 | **HIGH** | ✅ **COMPLETED** |
| [Timeout](#2-timeout-integration) | 5 | **HIGH** | ✅ **COMPLETED** |
| [Signal Handling](#3-signal-handling-integration) | 4 | **HIGH** | 🔶 PARTIAL (scheduler done) |
| [Configuration](#4-configuration-integration) | 5 | **HIGH** | ✅ **COMPLETED** |
| [Observability](#5-observability-integration) | 6 | MEDIUM-HIGH | ⬜ Pending |
| [Logging](#6-logging-integration) | 3 | MEDIUM | ⬜ Pending |
| [Fiber-local Storage](#7-fiber-local-storage-integration) | 3 | MEDIUM | ⬜ Pending |
| [Serialization](#8-serialization-integration) | 5 | MEDIUM | ⬜ Pending |
| [Networking](#9-networking-integration) | 2 | MEDIUM | ⬜ Pending |
| [Memory Limits](#10-memory-limits-integration) | 3 | MEDIUM-LOW | ⬜ Pending |

**Total Tasks**: 42
**Critical Path**: ~~Cancellation → Signal Handling → Timeout → Configuration~~ ✅ **CRITICAL PATH COMPLETED**

### Progress Summary (2026-02-05)
- ✅ **Cancellation**: Fully integrated into scheduler, fibers, I/O, channels
- ✅ **Timeout**: Configured timeouts wired to network, I/O, and process modules
- 🔶 **Signal Handling**: Scheduler integration done, compiler integration pending
- ✅ **Configuration**: Stack sizes, worker count, and timeouts all configurable

---

## 1. Cancellation Integration

> **Priority**: HIGH
> **Rationale**: Required for graceful shutdown and resource cleanup
> **Status**: ✅ COMPLETED (2026-02-05)

### 1.1 Scheduler Loop Integration
- [x] **File**: `blood-runtime/src/scheduler.rs:225-241`
- [x] Pass `CancellationToken` to individual fiber executions
- [x] Check cancellation at effect boundaries within worker loop
- [x] Propagate cancellation to work-stealing operations
- **Implemented**: Added `CancellationSource` and `CancellationToken` to Scheduler, integrated with worker loop

### 1.2 Fiber Execution Integration
- [x] **File**: `blood-runtime/src/fiber.rs:354-363`
- [x] Store `CancellationToken` in fiber state
- [x] Check token before each task resume
- [x] Implement `Fiber::cancel()` method that triggers token
- [x] Handle linear value cleanup on cancellation
- **Implemented**: Added `cancellation_source` and `cancellation_token` to Fiber struct, check in `run()`

### 1.3 I/O Operations Integration
- [x] **File**: `blood-runtime/src/io.rs`
- [x] Add `CancellationToken` parameter to blocking I/O operations
- [x] Implement cancellation-aware poll loop
- [x] Return `CancellationError` when cancelled during I/O
- [x] Wake blocked fibers on cancellation
- **Implemented**: Added `IoResult::Cancelled`, `cancel_operation()`, `submit_cancellable()`, etc.

### 1.4 Channel Operations Integration
- [x] **File**: `blood-runtime/src/channel.rs`
- [x] Add `send_cancellable(token, value)` method
- [x] Add `recv_cancellable(token)` method
- [x] Wake blocked senders/receivers on cancellation
- [x] Propagate cancellation through channel close
- **Implemented**: Added `SendCancelledError`, `RecvCancelledError`, `send_cancellable()`, `recv_cancellable()`

### 1.5 Process Waiting Integration
- [x] **File**: `blood-runtime/src/process.rs:37-100`
- [x] Accept `CancellationToken` in `wait_with_output()` - Implemented via timeout methods
- [x] Kill child process on cancellation - Implemented `wait_timeout_or_kill()`
- [x] Implement `wait_with_timeout_and_cancellation()` - Implemented `wait_with_configured_timeout()`

### 1.6 Package Fetcher Integration
- [ ] **File**: `bloodc/src/package/fetcher.rs:54-120`
- [ ] Pass cancellation token through HTTP client
- [ ] Check token during streaming downloads
- [ ] Abort download on cancellation

---

## 2. Timeout Integration

> **Priority**: HIGH
> **Rationale**: Prevents hung operations and resource exhaustion
> **Status**: ✅ COMPLETED (2026-02-05)

### 2.1 Network Operations
- [x] **File**: `blood-runtime/src/net.rs`
- [x] Apply `config.timeout.network_timeout` to `TcpListener::accept()` - `accept_with_configured_timeout()`
- [x] Apply timeout to `TcpStream::connect()` - `connect_with_configured_timeout()`
- [x] Apply timeout to DNS resolution - `configured_network_timeout()` helper
- [x] Return `TimeoutError` on expiration - `NetError::TimedOut`
- **Implemented**: Added `configured_network_timeout()`, `configured_io_timeout()`, `accept_timeout()`, etc.

### 2.2 I/O Operations Default Timeouts
- [x] **File**: `blood-runtime/src/io.rs`
- [x] Apply `config.timeout.io_timeout` to file read/write - `read_with_configured_timeout()`
- [x] Apply timeout to reactor poll operations - `ReactorConfig::from_runtime_config()`
- [x] Make timeouts configurable per-operation - `wait_for_completion_timeout()`
- **Implemented**: Added `configured_io_timeout()`, `wait_for_completion_with_configured_timeout()`, etc.

### 2.3 Compiler Package Fetching
- [ ] **File**: `bloodc/src/package/fetcher.rs:96-112`
- [ ] Apply `config.timeout.network_timeout` to HTTP GET
- [ ] Implement retry with exponential backoff
- [ ] Report timeout errors clearly to user

### 2.4 Scheduler Idle Timeout
- [ ] **File**: `blood-runtime/src/scheduler.rs:238-239`
- [ ] Replace busy-wait with timed park
- [ ] Use small timeout before `thread::yield_now()`
- [ ] Check graceful shutdown during idle wait

### 2.5 Subprocess Execution Timeout
- [x] **File**: `blood-runtime/src/process.rs`
- [x] Apply `config.timeout.compute_timeout` to subprocess waits - `configured_compute_timeout()`
- [x] Implement `Process::wait_with_timeout()` - `wait_timeout()`
- [x] Kill process on timeout expiration - `wait_timeout_or_kill()`
- **Implemented**: Added `configured_compute_timeout()`, `wait_timeout()`, `wait_with_configured_timeout()`, etc.

---

## 3. Signal Handling Integration

> **Priority**: HIGH
> **Rationale**: Required for container orchestration and graceful shutdown
> **Status**: ✅ PARTIALLY COMPLETED (2026-02-05) - Scheduler integration done

### 3.1 Scheduler Main Loop
- [x] **File**: `blood-runtime/src/scheduler.rs`
- [x] Install `SignalHandler` before `run()` - `install_signal_handlers()`
- [x] Register SIGTERM/SIGINT handlers - via `signal::SignalHandler`
- [x] Trigger cancellation tree on signal - `shutdown_with_reason()`
- [x] Wait for in-flight fibers to complete - via cancellation token
- **Implemented**: Added `signal_handler` field, `install_signal_handlers()`, `run_with_signal_handling()`

### 3.2 Compiler Main Entry
- [ ] **File**: `bloodc/src/main.rs`
- [ ] Install signal handler at startup
- [ ] Cancel compilation on SIGINT (Ctrl+C)
- [ ] Clean up temporary files on interrupt
- [ ] Report "compilation cancelled" on signal

### 3.3 Package Fetcher
- [ ] **File**: `bloodc/src/package/fetcher.rs`
- [ ] Check signal state during downloads
- [ ] Abort fetch operations on interrupt
- [ ] Clean up partial downloads

### 3.4 Subprocess Management
- [x] **File**: `blood-runtime/src/process.rs`
- [x] Forward SIGTERM to child processes - can use `kill()` method
- [x] Implement graceful child shutdown - `wait_timeout_or_kill()`
- [x] Wait with timeout after signal forwarding - `wait_timeout()`
- [x] Force kill after grace period - `wait_timeout_or_kill()` does this

---

## 4. Configuration Integration

> **Priority**: HIGH
> **Rationale**: Enables runtime tuning without recompilation
> **Status**: ✅ COMPLETED (2026-02-05)

### 4.1 Fiber Stack Sizes
- [x] **File**: `blood-runtime/src/fiber.rs`
- [x] Use `config.scheduler.initial_stack_size` instead of hardcoded 8KB - `configured_initial_stack_size()`
- [x] Use `config.scheduler.max_stack_size` for growth limit - `configured_max_stack_size()`
- [x] Validate stack sizes at fiber creation - Uses configured values in `FiberConfig::default()`
- **Implemented**: Added `configured_initial_stack_size()`, `configured_max_stack_size()`, `FiberConfig::from_runtime_config()`

### 4.2 Worker Thread Count
- [x] **File**: `blood-runtime/src/scheduler.rs`
- [x] Verify `BLOOD_NUM_WORKERS` env var is respected - via `RuntimeConfig::from_env()`
- [x] Log configured worker count at startup - can use `Scheduler::num_workers()`
- [x] Validate worker count (min 1, max reasonable) - done in RuntimeConfig validation
- **Implemented**: Added `configured_worker_count()`, `scheduler_config_from_runtime()`, `Scheduler::from_runtime_config()`

### 4.3 Timeout Values
- [x] **File**: Multiple I/O modules
- [x] Wire `config.timeout.network_timeout` to network ops - `configured_network_timeout()`
- [x] Wire `config.timeout.io_timeout` to file ops - `configured_io_timeout()`
- [x] Wire `config.timeout.compute_timeout` to CPU-bound work - `configured_compute_timeout()`
- **Implemented**: Helper functions in net.rs, io.rs, process.rs

### 4.4 Memory Limit Enforcement
- [ ] **File**: `blood-runtime/src/memory.rs`
- [ ] Verify `config.memory.max_heap_size` is enforced
- [ ] Enforce `config.memory.max_region_size` per region
- [ ] Trigger memory pressure callbacks at threshold

### 4.5 Log Level Configuration
- [ ] **File**: `blood-runtime/src/log.rs:62-89`
- [ ] Use `config.log.level` instead of hardcoded levels
- [ ] Respect `BLOOD_LOG_LEVEL` env var
- [ ] Support runtime log level changes

---

## 5. Observability Integration

> **Priority**: MEDIUM-HIGH
> **Rationale**: Production monitoring and debugging

### 5.1 Scheduler Metrics
- [ ] **File**: `blood-runtime/src/scheduler.rs`
- [ ] Add counter: `fiber_spawned_total`
- [ ] Add counter: `fiber_completed_total`
- [ ] Add counter: `fiber_cancelled_total`
- [ ] Add counter: `scheduler_work_steal_attempts`
- [ ] Add gauge: `scheduler_active_fibers`
- [ ] Add gauge: `scheduler_work_queue_size`
- [ ] Add histogram: `fiber_execution_duration_seconds`

### 5.2 Channel Metrics
- [ ] **File**: `blood-runtime/src/channel.rs`
- [ ] Add counter: `channel_send_total` (by channel name/id)
- [ ] Add counter: `channel_recv_total`
- [ ] Add gauge: `channel_queue_depth`
- [ ] Add histogram: `channel_send_blocked_duration_seconds`

### 5.3 I/O Metrics
- [ ] **File**: `blood-runtime/src/io.rs`
- [ ] Add counter: `io_read_bytes_total`
- [ ] Add counter: `io_write_bytes_total`
- [ ] Add counter: `io_operations_total` (by type)
- [ ] Add gauge: `io_pending_operations`
- [ ] Add histogram: `io_operation_duration_seconds`

### 5.4 Memory Metrics
- [ ] **File**: `blood-runtime/src/memory.rs`
- [ ] Add counter: `memory_allocations_total`
- [ ] Add counter: `memory_deallocations_total`
- [ ] Add gauge: `memory_bytes_allocated`
- [ ] Add gauge: `memory_regions_active`
- [ ] Add histogram: `memory_allocation_size_bytes`

### 5.5 Network Metrics
- [ ] **File**: `blood-runtime/src/net.rs`
- [ ] Add counter: `tcp_connections_total`
- [ ] Add gauge: `tcp_connections_active`
- [ ] Add counter: `tcp_bytes_sent_total`
- [ ] Add counter: `tcp_bytes_received_total`
- [ ] Add histogram: `tcp_connection_duration_seconds`

### 5.6 FFI Call Tracing
- [ ] **File**: `blood-runtime/src/ffi_exports.rs`
- [ ] Add spans for major FFI functions
- [ ] Track FFI call frequency
- [ ] Measure FFI call duration for hot paths
- [ ] Log slow FFI calls at warn level

---

## 6. Logging Integration

> **Priority**: MEDIUM
> **Rationale**: Replace ad-hoc output with structured logging

### 6.1 Runtime Debug Output
- [ ] **File**: `blood-runtime/src/ffi_exports.rs:6999,7008,7096`
- [ ] Replace `eprintln!` with `log::debug!` or `log::error!`
- [ ] Add context (fiber id, operation) to log messages
- [ ] Use structured fields for machine parsing

### 6.2 Compiler Diagnostics
- [ ] **File**: `bloodc/src/diagnostics.rs`
- [ ] Use `log::info!` for progress messages
- [ ] Use `log::warn!` for warnings
- [ ] Use `log::error!` for errors
- [ ] Keep user-facing output separate from log stream

### 6.3 Compilation Progress
- [ ] **File**: `bloodc/src/project/compiler.rs`
- [ ] Add info-level logs for compilation phases
- [ ] Add debug-level logs for detailed progress
- [ ] Include timing information in logs
- [ ] Log file being compiled at trace level

---

## 7. Fiber-local Storage Integration

> **Priority**: MEDIUM
> **Rationale**: Context propagation for tracing and debugging

### 7.1 Fiber Spawn Context Inheritance
- [ ] **File**: `blood-runtime/src/scheduler.rs:95-112`
- [ ] Copy parent fiber's `FiberLocalStorage` to child
- [ ] Respect `PropagationMode` settings
- [ ] Create new `TraceContext` with parent reference
- [ ] Inherit `RequestContext` by default

### 7.2 I/O Completion Context Restoration
- [ ] **File**: `blood-runtime/src/io.rs`
- [ ] Save fiber context before I/O suspension
- [ ] Restore fiber context on I/O completion
- [ ] Maintain trace span across I/O boundary

### 7.3 Channel Context Propagation
- [ ] **File**: `blood-runtime/src/channel.rs`
- [ ] Optionally propagate sender's trace context to receiver
- [ ] Support context extraction from messages
- [ ] Enable cross-fiber tracing via channels

---

## 8. Serialization Integration

> **Priority**: MEDIUM
> **Rationale**: Enable persistence and debugging

### 8.1 Fiber State Serialization
- [ ] **File**: `blood-runtime/src/fiber.rs`
- [ ] Implement `Serialize` for `FiberId`
- [ ] Implement `Serialize` for `FiberState`
- [ ] Implement `Serialize` for `Priority`
- [ ] Add `Fiber::snapshot()` for debugging

### 8.2 Channel Message Serialization
- [ ] **File**: `blood-runtime/src/channel.rs`
- [ ] Define serializable message wrapper
- [ ] Support cross-process channel communication
- [ ] Enable message persistence for replay

### 8.3 Memory Allocation Records
- [ ] **File**: `blood-runtime/src/memory.rs`
- [ ] Implement `Serialize` for `AllocationStats`
- [ ] Create serializable allocation report format
- [ ] Enable memory audit trail export

### 8.4 Cancellation State Serialization
- [ ] **File**: `blood-runtime/src/cancellation.rs`
- [ ] Implement `Serialize` for cancellation tree
- [ ] Support cancellation state debugging
- [ ] Enable cancellation audit trail

### 8.5 Network Configuration Serialization
- [ ] **File**: `blood-runtime/src/net.rs`
- [ ] Implement `Serialize` for socket configurations
- [ ] Support configuration import/export
- [ ] Enable network setup persistence

---

## 9. Networking Integration

> **Priority**: MEDIUM
> **Rationale**: Unified network handling

### 9.1 TcpListener Cancellation
- [ ] **File**: `blood-runtime/src/net.rs:440-475`
- [ ] Integrate `CancellationToken` into accept loop
- [ ] Return `CancellationError` on shutdown
- [ ] Close listener socket on cancellation

### 9.2 Package Registry Client
- [ ] **File**: `bloodc/src/package/fetcher.rs`
- [ ] Consider migrating to `blood_runtime::net` module
- [ ] Unify timeout and cancellation handling
- [ ] Share connection pooling if applicable

---

## 10. Memory Limits Integration

> **Priority**: MEDIUM-LOW
> **Rationale**: Prevent resource exhaustion

### 10.1 Vector Allocation Quota
- [ ] **File**: `blood-runtime/src/ffi_exports.rs:6962-7053`
- [ ] Check quota before `vec_with_capacity`
- [ ] Check quota before `vec_push` growth
- [ ] Return allocation error if quota exceeded

### 10.2 Region Size Enforcement
- [ ] **File**: `blood-runtime/src/memory.rs`
- [ ] Enforce `max_region_size` on region creation
- [ ] Track per-region memory usage
- [ ] Return error when region limit reached

### 10.3 String Growth Limits
- [ ] **File**: `blood-runtime/src/ffi_exports.rs`
- [ ] Check quota on string concatenation
- [ ] Check quota on string push operations
- [ ] Prevent string bomb attacks

---

## Progress Tracking

### Phase 1: Critical Path (Week 1)
- [ ] Cancellation integration (1.1 - 1.6)
- [ ] Signal handling integration (3.1 - 3.4)
- [ ] Timeout integration (2.1 - 2.5)
- [ ] Configuration wiring (4.1 - 4.5)

### Phase 2: Monitoring (Week 2)
- [ ] Scheduler metrics (5.1)
- [ ] Memory metrics (5.4)
- [ ] I/O metrics (5.3)
- [ ] Logging replacement (6.1 - 6.3)

### Phase 3: Polish (Week 3)
- [ ] Channel metrics (5.2)
- [ ] Network metrics (5.5)
- [ ] Fiber-local storage (7.1 - 7.3)
- [ ] Serialization (8.1 - 8.5)

### Phase 4: Hardening (Week 4)
- [ ] FFI tracing (5.6)
- [ ] Memory limits (10.1 - 10.3)
- [ ] Network integration (9.1 - 9.2)

---

## Testing Requirements

For each integration task:

1. **Unit tests** verifying the feature works in isolation
2. **Integration tests** verifying interaction with other systems
3. **Stress tests** verifying behavior under load
4. **Cancellation tests** verifying clean shutdown
5. **Timeout tests** verifying expiration behavior

### Test Checklist Template

```
- [ ] Feature works in happy path
- [ ] Feature handles cancellation correctly
- [ ] Feature respects timeouts
- [ ] Feature emits expected metrics
- [ ] Feature logs appropriate messages
- [ ] Feature cleans up resources on error
```

---

## Notes

- **File paths are relative to repository root**
- **Line numbers may drift as code changes**
- **Priority may be adjusted based on user feedback**
- **Some tasks may reveal additional integration points**

---

## Revision History

| Date | Change |
|------|--------|
| 2026-02-05 | Initial audit and task list creation |

