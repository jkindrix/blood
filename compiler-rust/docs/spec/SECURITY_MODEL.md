# Blood Security Model

**Version**: 1.0
**Status**: Authoritative
**Last Updated**: 2026-01-13

## Overview

This document describes Blood's security model: the guarantees it provides, the threats it mitigates, the trust boundaries it enforces, and the known limitations of its security properties. Blood's security approach combines compile-time verification with runtime checks, providing defense-in-depth against common vulnerability classes.

## Threat Model

### Assumed Threat Actors

Blood's security model addresses threats from:

1. **Developer Errors**: Accidental bugs from programmers
2. **Malicious Input**: Attackers providing crafted inputs
3. **Untrusted Libraries**: Third-party code with potential vulnerabilities

Blood does NOT protect against:

1. **Compromised Compiler**: If the Blood compiler is malicious, all bets are off
2. **Compromised Hardware**: Side-channel attacks, hardware backdoors
3. **Malicious `unsafe` Code**: Code explicitly bypassing safety checks
4. **Social Engineering**: Attacks targeting developers directly

### Security Goals

| Goal | Mechanism | Strength |
|------|-----------|----------|
| Memory Safety | Generational references | Runtime (high confidence) |
| Type Safety | Static type system | Compile-time (sound) |
| Effect Safety | Algebraic effect system | Compile-time (sound) |
| Capability Control | Effect-based permissions | Compile-time (sound) |
| Data Isolation | Affine types | Compile-time (sound) |

## Memory Safety Model

### Generational References

Blood's primary memory safety mechanism is **generational references**: 128-bit pointers that include a generation counter.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Generational Reference (128 bits)            │
├─────────────────────────────────┬───────────────────────────────┤
│     Memory Address (64 bits)    │   Generation Counter (64 bits)│
└─────────────────────────────────┴───────────────────────────────┘
```

#### How It Works

1. When memory is allocated, a generation counter is assigned
2. The reference stores both the address and the generation
3. When memory is freed, the allocation's generation is incremented
4. On every access, the reference's generation is compared to the allocation's current generation
5. If they differ, the access is detected as use-after-free

```blood
fn demonstrate_safety() {
    let ptr: &i32 = Box::new(42);  // Generation = G
    drop(ptr);                      // Allocation's generation becomes G+1
    // *ptr;                        // Would detect: ref has G, alloc has G+1
}
```

#### Security Properties

| Property | Guarantee | Detection |
|----------|-----------|-----------|
| Use-After-Free | Detected | Runtime |
| Double-Free | Detected | Runtime |
| Dangling Pointers | Detected | Runtime |
| Buffer Overflows | Bounds-checked | Runtime |
| Null Dereference | Type system prevents | Compile-time |

### Memory Tiers

Blood uses a tiered memory system with different safety properties:

#### Tier 1: Stack Memory

- **Properties**: Fastest, no generation checks needed for local scope
- **Safety**: Automatic lifetime management, stack overflow protection
- **Risk**: None (within language rules)

```blood
fn tier1_example() {
    let x = 42;  // Tier 1: stack allocated
    // Lifetime bound to function scope
}
```

#### Tier 2: Heap Memory

- **Properties**: Generation-checked, manually dropped
- **Safety**: Use-after-free detected at runtime
- **Risk**: Runtime overhead, panic on violation

```blood
fn tier2_example() {
    let ptr = Box::new(42);  // Tier 2: heap with generation
    drop(ptr);               // Explicit deallocation
}
```

#### Tier 3: Static Memory

- **Properties**: Lives for program duration
- **Safety**: No lifetime issues (never deallocated)
- **Risk**: Memory cannot be reclaimed

```blood
static GLOBAL: i32 = 42;  // Tier 3: static memory
```

### Bounds Checking

All array and slice accesses are bounds-checked:

```blood
fn bounds_example(arr: [i32; 10], idx: usize) -> i32 {
    arr[idx]  // Runtime bounds check
}
```

Bounds checks can be elided by the optimizer when:
- Index is provably within bounds (loop bounds, assertions)
- Compiler can prove access is safe

### Type Safety

Blood's type system is **sound**: well-typed programs cannot exhibit undefined behavior from type errors.

#### Type System Properties

1. **No Implicit Casts**: All type conversions are explicit
2. **No Null Pointers**: Option type used for optional values
3. **No Uninitialized Memory**: All variables must be initialized
4. **No Invalid Enum Variants**: Exhaustive pattern matching enforced

```blood
fn type_safety_example() {
    let x: i32 = 42;
    // let y: String = x;  // Compile error: type mismatch

    let opt: Option<i32> = Some(42);
    match opt {
        Some(n) => println!("{}", n),
        None => println!("nothing"),
    }  // Must handle all variants
}
```

## Effect System Security

### Capability Model

Blood's effect system provides capability-based security: code can only perform operations it has explicitly declared.

```blood
// This function can ONLY read files - cannot write, network, etc.
fn read_config(path: String) -> String with FileRead {
    do FileRead.read(path)
}

// This function has NO capabilities - pure computation only
fn compute(x: i32) -> i32 {
    x * 2
}
```

### Effect Tracking

| Effect Category | What It Controls | Example Operations |
|-----------------|------------------|-------------------|
| `IO` | General I/O | Printing, reading stdin |
| `FileSystem` | File operations | Read, write, delete files |
| `Net` | Network access | HTTP requests, sockets |
| `Time` | Time access | Getting current time |
| `Random` | Non-determinism | Random number generation |
| `Async` | Concurrency | Spawning fibers, channels |

### Security Properties

#### Principle of Least Privilege

Functions declare exactly what capabilities they need:

```blood
// Good: minimal capabilities
fn process_data(data: String) -> Result<Value, Error> with Parse {
    do Parse.json(data)
}

// Suspicious: too many capabilities
fn suspicious(data: String) -> Result<Value, Error> with Parse, Net, FileSystem {
    // Why does parsing need network and filesystem?
    do Parse.json(data)
}
```

#### Effect Containment

Effects must be handled, preventing capability leakage:

```blood
fn main() with IO {
    // FileSystem effect MUST be handled before program exit
    with handler FileSandbox("/safe/dir") {
        read_config("config.json");
    }
}
```

#### Effect Auditing

Effect signatures enable security audits:

```bash
# Find all functions with network access
blood audit --effect Net

# Find all unhandled effects in main
blood audit --unhandled main.blood
```

### Handler Security

Effect handlers control how capabilities are implemented:

```blood
// Sandboxed file handler - restricts to safe directory
handler SandboxedFS: FileSystem {
    allowed_path: String,

    fn read(path: String) -> Vec<u8> {
        if !path.starts_with(&self.allowed_path) {
            panic!("Access denied: outside sandbox");
        }
        resume(sys_read(&path))
    }

    fn write(path: String, data: Vec<u8>) {
        if !path.starts_with(&self.allowed_path) {
            panic!("Access denied: outside sandbox");
        }
        resume(sys_write(&path, data))
    }
}
```

## FFI Security Boundary

### Trust Boundary

The FFI (Foreign Function Interface) is a trust boundary where Blood's safety guarantees cannot be enforced:

```
┌────────────────────────────────────────────┐
│              Safe Blood Code               │
│   - Type safety enforced                   │
│   - Memory safety checked                  │
│   - Effects tracked                        │
├────────────────────────────────────────────┤
│              unsafe { FFI }                │ ← Trust Boundary
├────────────────────────────────────────────┤
│            External C/Rust Code            │
│   - No Blood safety guarantees             │
│   - Must be manually audited               │
└────────────────────────────────────────────┘
```

### FFI Requirements

All FFI code requires `unsafe` blocks:

```blood
extern "C" {
    fn external_function(ptr: *mut u8, len: usize) -> i32;
}

fn use_external() {
    unsafe {
        // Blood cannot verify safety of this call
        let result = external_function(ptr, len);
    }
}
```

### FFI Audit Checklist

When auditing FFI code, verify:

- [ ] **Buffer sizes**: Are lengths correct and bounds-checked?
- [ ] **Pointer validity**: Are pointers valid for the duration of the call?
- [ ] **Ownership**: Is ownership transfer clear and correct?
- [ ] **Thread safety**: Is the external function thread-safe?
- [ ] **Error handling**: Are all error cases handled?
- [ ] **Resource cleanup**: Are resources freed appropriately?

### FFI Best Practices

```blood
// Good: Thin safe wrapper around unsafe FFI
fn read_file_safe(path: &str) -> Result<Vec<u8>, IoError> {
    let c_path = CString::new(path).map_err(|_| IoError::InvalidPath)?;

    unsafe {
        let fd = libc_open(c_path.as_ptr(), O_RDONLY);
        if fd < 0 {
            return Err(IoError::OpenFailed);
        }

        // Ensure cleanup on all paths
        defer { libc_close(fd); }

        let size = get_file_size(fd)?;
        let mut buffer = Vec::with_capacity(size);

        let read = libc_read(fd, buffer.as_mut_ptr(), size);
        if read < 0 {
            return Err(IoError::ReadFailed);
        }

        buffer.set_len(read as usize);
        Ok(buffer)
    }
}
```

## Concurrency Security

### Data Race Prevention

Blood's affine type system prevents data races by ensuring unique ownership:

```blood
fn no_data_races() with Async {
    let data = vec![1, 2, 3];

    // Move into fiber - original binding invalidated
    spawn {
        process(data);  // data owned by this fiber
    }

    // data cannot be accessed here - compile error
}
```

### Channel Safety

Communication between fibers uses typed channels:

```blood
fn channel_example() with Async {
    let (tx, rx) = channel::<Message>();

    spawn {
        tx.send(Message::Hello);  // Type-safe send
    }

    match rx.recv() {
        Message::Hello => handle_hello(),
        Message::Goodbye => handle_goodbye(),
    }  // Exhaustive handling required
}
```

### Shared State

Mutable shared state requires explicit synchronization:

```blood
// Arc for shared ownership
let shared = Arc::new(Mutex::new(data));

spawn {
    let guard = shared.lock();
    guard.modify();
    // Lock automatically released
}
```

## Input Validation

### Untrusted Input

Blood encourages explicit handling of untrusted input:

```blood
effect Validate {
    fn reject(reason: String) -> !;  // Never returns normally
    fn sanitize(input: String) -> String;
}

fn process_user_input(input: String) -> Result<Data, Error> with Validate {
    // Length check
    if input.len() > MAX_INPUT {
        do Validate.reject("Input too large");
    }

    // Sanitize
    let sanitized = do Validate.sanitize(input);

    // Parse with error handling
    parse_data(sanitized)
}
```

### Type-Safe Parsing

Blood's type system enables type-safe parsing:

```blood
// Newtype for validated email
struct Email(String);

impl Email {
    fn parse(s: String) -> Option<Email> {
        if is_valid_email(&s) {
            Some(Email(s))
        } else {
            None
        }
    }
}

// Function only accepts validated emails
fn send_email(to: Email) with Net {
    // to is guaranteed to be valid
}
```

## Security Limitations

### Known Limitations

1. **Runtime Checks**: Memory safety relies on runtime checks, not compile-time proofs
   - Overhead in performance-critical code
   - Checks can be disabled in release builds (user responsibility)

2. **Unsafe Escape Hatch**: `unsafe` blocks bypass all safety checks
   - FFI code must be manually audited
   - Unsafe code can introduce any vulnerability

3. **Timing Attacks**: Blood does not protect against timing side channels
   - Cryptographic code should use constant-time algorithms
   - Not suitable for timing-sensitive security code without care

4. **Resource Exhaustion**: No built-in protection against DoS
   - Memory allocation can be exhausted
   - CPU can be consumed by infinite loops
   - Application must implement resource limits

5. **Error Messages**: Detailed error messages may leak information
   - Don't expose internal details in production
   - Use generic error messages for security-sensitive operations

### What Blood Cannot Prevent

| Vulnerability | Why Blood Cannot Prevent |
|---------------|-------------------------|
| Logic errors | Programmer must implement correct logic |
| Cryptographic flaws | Requires domain expertise, not type safety |
| SQL injection | Depends on library design, not language |
| Social engineering | Human problem, not technical |
| Physical access | Out of scope |

## Security Recommendations

### For Library Authors

1. **Minimize Unsafe**: Keep `unsafe` blocks small and well-documented
2. **Document Effects**: Clearly document what capabilities your library requires
3. **Validate Input**: Never trust input from external sources
4. **Handle Errors**: Use Result types, never panic on invalid input
5. **Audit Dependencies**: Review the security of your dependencies

### For Application Developers

1. **Principle of Least Privilege**: Request only necessary effects
2. **Sandbox External Code**: Use restricted handlers for untrusted operations
3. **Keep Safety Checks On**: Don't disable generation checks without good reason
4. **Review Unsafe Code**: Audit all `unsafe` blocks in dependencies
5. **Update Dependencies**: Keep dependencies updated for security patches

### For Security Auditors

1. **Search for `unsafe`**: All security-relevant code is in `unsafe` blocks
2. **Review Effect Usage**: Check that capabilities match documented behavior
3. **Examine FFI Boundaries**: Focus on data marshaling and buffer handling
4. **Test with Fuzzing**: Use Blood's fuzz testing support
5. **Check Error Handling**: Ensure errors don't leak sensitive information

## Comparison with Other Languages

| Feature | Blood | Rust | Go | C |
|---------|-------|------|-----|---|
| Memory safety | Runtime checks | Compile-time | GC | None |
| Null safety | Type system | Type system | Nil checks | None |
| Data race prevention | Affine types | Ownership | Channels | None |
| Capability tracking | Effect system | None | None | None |
| Unsafe escape | `unsafe` | `unsafe` | CGO | Default |

## Security Checklist

### Code Review Checklist

- [ ] All `unsafe` blocks are justified and minimal
- [ ] FFI code validates all inputs
- [ ] Buffer sizes are checked before use
- [ ] Error messages don't leak sensitive information
- [ ] Effects match documented capabilities
- [ ] Resource limits are enforced
- [ ] Input validation is comprehensive

### Deployment Checklist

- [ ] Generation checks enabled in production (unless performance-critical)
- [ ] Logging doesn't include sensitive data
- [ ] Error messages are generic for users
- [ ] Dependencies are audited and up-to-date
- [ ] File system sandboxing is configured
- [ ] Network access is restricted appropriately

## Reporting Security Issues

See [SECURITY.md](/SECURITY.md) for responsible disclosure policy.

## Version History

| Version | Changes |
|---------|---------|
| 1.0 | Initial security model documentation |

---

*This security model is a living document and will be updated as Blood evolves.*
