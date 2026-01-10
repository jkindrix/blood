# Blood Foreign Function Interface (FFI) Specification

**Version**: 0.3.0-draft
**Status**: Active Development
**Implementation**: 🔶 Scaffolded (see [IMPLEMENTATION_STATUS.md](./IMPLEMENTATION_STATUS.md))
**Last Updated**: 2026-01-10

> ⚠️ **Validation Status**: This specification describes design targets. Platform-specific considerations (§9) are validated only for x86-64 Linux. Other platforms are design targets pending validation. FFI code exists in `blood-runtime/src/ffi.rs` but integration with compiled programs is incomplete.

**Revision 0.3.0 Changes**:
- Added validation status box and implementation status link
- Clarified platform support as design targets pending validation

This document specifies Blood's Foreign Function Interface, enabling interoperability with C, C++, and other languages through a safety-preserving bridge dialect.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Bridge Dialect](#2-bridge-dialect)
3. [Type Mapping](#3-type-mapping)
4. [Calling Conventions](#4-calling-conventions)
5. [Safety Boundaries](#5-safety-boundaries)
6. [Memory Ownership](#6-memory-ownership)
7. [Effect Integration](#7-effect-integration)
8. [Error Handling](#8-error-handling)
9. [Platform-Specific Considerations](#9-platform-specific-considerations)
10. [Code Generation](#10-code-generation)

---

## 1. Overview

### 1.1 Design Goals

Blood's FFI is designed to:

1. **Preserve Safety** — Clearly delineate safe and unsafe boundaries
2. **Enable Interop** — Work with existing C/C++ ecosystems
3. **Integrate Effects** — FFI operations are effects, tracked in types
4. **Minimize Overhead** — Zero-cost when possible, explicit cost otherwise
5. **Support Gradual Migration** — Incrementally port C code to Blood

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Pointer representation at FFI boundaries
- [GRAMMAR.md](./GRAMMAR.md) — Bridge block syntax
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — FFI-related error messages
- [CONCURRENCY.md](./CONCURRENCY.md) — Fiber interaction with FFI

### 1.3 Implementation Status

The following table tracks implementation status of FFI subsystems:

| Component | Status | Location | Notes |
|-----------|--------|----------|-------|
| DynamicLibrary loader | ✅ Implemented | `blood-runtime/src/ffi.rs` | Cross-platform via libloading |
| LibraryRegistry | ✅ Implemented | `blood-runtime/src/ffi.rs` | Symbol resolution |
| FfiValue types | ✅ Implemented | `blood-runtime/src/ffi.rs` | i8-i64, u8-u64, f32, f64, ptr |
| FfiType definitions | ✅ Implemented | `blood-runtime/src/ffi.rs` | Type introspection |
| blood_ffi_* exports | ✅ Integrated | `blood-runtime/src/ffi_exports.rs` | Runtime FFI dispatch |
| Bridge block parsing | 📋 Designed | — | Grammar specified, not parsed |
| Bridge block codegen | 📋 Designed | — | Awaits bridge parsing |
| Type mapping validation | 📋 Designed | — | Per §3 specification |
| Calling conventions | 📋 Designed | — | sysv64 only for now |
| Platform validation (Linux) | ✅ Validated | `blood-runtime/tests/` | Primary development platform |
| Platform validation (macOS) | ⚠️ Partial | — | Expected to work, not CI tested |
| Platform validation (Windows) | ⚠️ Partial | — | Expected to work, not CI tested |
| WASM FFI | 📋 Designed | — | Design target only |

**Legend**: ✅ Implemented | ⚠️ Partial | 📋 Designed | ❌ Not Started

### 1.4 FFI Philosophy

Unlike languages that hide FFI complexity (risking hidden unsafety) or make it painful (discouraging legitimate use), Blood takes an explicit approach:

| Aspect | Blood Approach |
|--------|----------------|
| **Safety** | All FFI calls are `@unsafe` but encapsulated in safe wrappers |
| **Effects** | FFI implies `{FFI}` effect, subsumes `{IO}` |
| **Types** | Explicit mapping between Blood and C types |
| **Ownership** | Clear ownership transfer semantics |

### 1.5 The FFI Effect

```blood
/// FFI effect captures all foreign function interactions.
/// Subsumes IO because FFI can have arbitrary side effects.
effect FFI extends IO {
    /// Call a foreign function (internal operation)
    op call_foreign(symbol: Symbol, args: ForeignArgs) -> ForeignResult;
}
```

All foreign function calls implicitly perform the `FFI` effect.

---

## 2. Bridge Dialect

### 2.1 Bridge Blocks

Blood uses explicit bridge blocks to declare foreign interfaces:

```blood
/// Declare a bridge to a C library
bridge "C" libc {
    /// Link against libc
    #[link(name = "c")]

    /// Import declarations
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8;
    fn strlen(s: *const u8) -> usize;

    /// Constants
    const STDIN_FILENO: i32 = 0;
    const STDOUT_FILENO: i32 = 1;
    const STDERR_FILENO: i32 = 2;

    /// Types (opaque by default)
    type FILE;
    type DIR;
}
```

### 2.2 Bridge Block Syntax

```ebnf
BridgeBlock ::= 'bridge' StringLiteral Ident '{' BridgeItem* '}'

BridgeItem ::= BridgeLink
             | BridgeFn
             | BridgeConst
             | BridgeType
             | BridgeStruct
             | BridgeCallback

BridgeLink ::= '#[link(' LinkSpec ')]'
LinkSpec ::= 'name' '=' StringLiteral (',' LinkAttr)*
LinkAttr ::= 'kind' '=' ('dylib' | 'static' | 'framework')
           | 'wasm_import_module' '=' StringLiteral

BridgeFn ::= Attribute* 'fn' Ident '(' BridgeParams ')' ('->' BridgeType)? ';'
BridgeParams ::= (BridgeParam (',' BridgeParam)*)?
BridgeParam ::= Ident ':' BridgeType

BridgeConst ::= 'const' Ident ':' BridgeType '=' Literal ';'

BridgeType ::= 'type' Ident ';'                    (* Opaque type *)
             | 'type' Ident '=' BridgeTypeExpr ';' (* Type alias *)

BridgeStruct ::= '#[repr(C)]' 'struct' Ident '{' BridgeField* '}'
BridgeField ::= Ident ':' BridgeTypeExpr ','?

BridgeCallback ::= 'callback' Ident '=' 'fn' '(' BridgeParams ')' ('->' BridgeType)? ';'
```

### 2.3 Using Foreign Functions

```blood
fn allocate_buffer(size: usize) -> *mut u8 / {FFI} {
    @unsafe {
        let ptr = libc::malloc(size)
        if ptr.is_null() {
            panic!("malloc failed")
        }
        ptr
    }
}

fn safe_allocate(size: usize) -> Vec<u8> / {Allocate} {
    // Safe wrapper that doesn't expose raw pointers
    Vec::with_capacity(size)
}
```

### 2.4 Language Specifiers

```blood
// C ABI (default)
bridge "C" mylib { ... }

// C++ with name mangling
bridge "C++" cpplib {
    #[link(name = "mylib")]
    #[mangle("_ZN4Math3addEii")]
    fn add(x: i32, y: i32) -> i32;
}

// Platform-specific calling conventions
bridge "C" winapi {
    #[link(name = "kernel32")]
    #[calling_convention("stdcall")]
    fn GetLastError() -> u32;
}

// WebAssembly imports
bridge "wasm" env {
    #[link(wasm_import_module = "env")]
    fn console_log(ptr: *const u8, len: usize);
}
```

### 2.5 Inline Foreign Code (Escape Hatch)

For complex cases, Blood allows inline foreign code:

```blood
bridge "C" inline {
    #[inline_c]
    const SIGINT: i32 = """
        #include <signal.h>
        SIGINT
    """;

    #[inline_c]
    fn complex_macro(x: i32) -> i32 = """
        #include "myheader.h"
        COMPLEX_MACRO(x)
    """;
}
```

This generates a C file at compile time and links the result.

---

## 3. Type Mapping

### 3.1 Primitive Type Mapping

| Blood Type | C Type | Size | Notes |
|------------|--------|------|-------|
| `i8` | `int8_t` | 1 | Signed |
| `i16` | `int16_t` | 2 | Signed |
| `i32` | `int32_t` | 4 | Signed |
| `i64` | `int64_t` | 8 | Signed |
| `i128` | `__int128` | 16 | Platform-dependent |
| `u8` | `uint8_t` | 1 | Unsigned |
| `u16` | `uint16_t` | 2 | Unsigned |
| `u32` | `uint32_t` | 4 | Unsigned |
| `u64` | `uint64_t` | 8 | Unsigned |
| `u128` | `unsigned __int128` | 16 | Platform-dependent |
| `f32` | `float` | 4 | IEEE 754 |
| `f64` | `double` | 8 | IEEE 754 |
| `bool` | `_Bool` / `bool` | 1 | C99+ |
| `char` | `uint32_t` | 4 | Unicode scalar |
| `usize` | `size_t` | ptr | Platform-dependent |
| `isize` | `ssize_t` / `ptrdiff_t` | ptr | Platform-dependent |

### 3.2 Pointer Type Mapping

| Blood Type | C Type | Notes |
|------------|--------|-------|
| `*const T` | `const T*` | Immutable raw pointer |
| `*mut T` | `T*` | Mutable raw pointer |
| `*const u8` | `const char*` | C string (by convention) |
| `Option<*const T>` | `T*` | Nullable pointer (None = NULL) |

**Critical**: Blood's `&T` and `&mut T` references are **not** FFI-safe. They carry generation metadata. Use raw pointers for FFI.

### 3.3 Struct Mapping

```blood
bridge "C" mylib {
    /// C-compatible struct layout
    #[repr(C)]
    struct Point {
        x: f64,
        y: f64,
    }

    /// Packed struct (no padding)
    #[repr(C, packed)]
    struct PackedData {
        flag: u8,
        value: u32,
    }

    /// Aligned struct
    #[repr(C, align(16))]
    struct AlignedData {
        data: [u8; 64],
    }
}
```

### 3.4 Enum Mapping

```blood
bridge "C" mylib {
    /// C-compatible enum (repr(C) required)
    #[repr(C)]
    enum Status {
        Ok = 0,
        Error = 1,
        Pending = 2,
    }

    /// Enum with explicit discriminant type
    #[repr(u8)]
    enum Flag {
        Off = 0,
        On = 1,
    }
}
```

### 3.5 Union Mapping

```blood
bridge "C" mylib {
    #[repr(C)]
    union Value {
        i: i32,
        f: f32,
        p: *mut u8,
    }
}

// Usage requires @unsafe
fn use_union() / {FFI} {
    @unsafe {
        let mut v = Value { i: 42 };
        let f = v.f;  // Reinterpret as float
    }
}
```

### 3.6 Opaque Types

For types whose layout is unknown or should not be exposed:

```blood
bridge "C" mylib {
    /// Opaque type - only pointers allowed
    type FILE;
    type sqlite3;
    type SSL_CTX;

    fn fopen(path: *const u8, mode: *const u8) -> *mut FILE;
    fn fclose(file: *mut FILE) -> i32;
}
```

Opaque types can only be used through pointers. This prevents:
- Stack allocation of unknown-size types
- Direct field access
- Incorrect size assumptions

### 3.7 Function Pointer Mapping

```blood
bridge "C" mylib {
    /// Callback type definition
    callback Comparator = fn(*const u8, *const u8) -> i32;

    /// Function taking a callback
    fn qsort(
        base: *mut u8,
        num: usize,
        size: usize,
        compare: Comparator
    );
}

// Blood function as C callback
fn my_compare(a: *const u8, b: *const u8) -> i32 / pure {
    @unsafe {
        let a_val = *(a as *const i32);
        let b_val = *(b as *const i32);
        a_val - b_val
    }
}

fn sort_array(arr: &mut [i32]) / {FFI} {
    @unsafe {
        libc::qsort(
            arr.as_mut_ptr() as *mut u8,
            arr.len(),
            std::mem::size_of::<i32>(),
            my_compare  // Blood function as callback
        )
    }
}
```

### 3.8 String Handling

```blood
/// C string utilities
mod ffi::cstr {
    /// Owned null-terminated C string
    struct CString {
        ptr: *mut u8,
        len: usize,
    }

    impl CString {
        /// Create from Blood string (copies, adds null terminator)
        fn new(s: &str) -> CString / {Allocate} { ... }

        /// Get raw pointer for FFI
        fn as_ptr(&self) -> *const u8 / pure { self.ptr }

        /// Convert back to Blood string (validates UTF-8)
        fn to_str(&self) -> Result<&str, Utf8Error> / pure { ... }
    }

    impl Drop for CString {
        fn drop(&mut self) / pure {
            @unsafe { libc::free(self.ptr) }
        }
    }

    /// Borrowed C string slice
    struct CStr {
        ptr: *const u8,
    }

    impl CStr {
        /// Wrap a C string pointer (unsafe: must be null-terminated)
        @unsafe fn from_ptr(ptr: *const u8) -> CStr { ... }

        /// Get length (calls strlen)
        fn len(&self) -> usize / {FFI} { ... }
    }
}
```

---

## 4. Calling Conventions

### 4.1 Supported Conventions

| Convention | Platforms | Usage |
|------------|-----------|-------|
| `"C"` (cdecl) | All | Default, most C code |
| `"stdcall"` | Windows | Win32 API |
| `"fastcall"` | x86 | Performance-critical |
| `"vectorcall"` | Windows | SIMD parameters |
| `"aapcs"` | ARM | ARM procedure call standard |
| `"win64"` | Windows x64 | 64-bit Windows |
| `"sysv64"` | Unix x64 | System V AMD64 ABI |
| `"wasm"` | WebAssembly | WASM imports/exports |

### 4.2 Specifying Conventions

```blood
bridge "C" platform {
    // Default C calling convention
    fn regular_function(x: i32) -> i32;

    // Explicit calling convention
    #[calling_convention("stdcall")]
    fn win32_function(hwnd: usize, msg: u32) -> i32;

    // Platform-conditional
    #[cfg(target_os = "windows")]
    #[calling_convention("stdcall")]
    fn platform_specific() -> i32;

    #[cfg(not(target_os = "windows"))]
    fn platform_specific() -> i32;
}
```

### 4.3 Variadic Functions

```blood
bridge "C" libc {
    /// Variadic function declaration
    fn printf(format: *const u8, ...) -> i32;
}

fn example() / {FFI} {
    @unsafe {
        let fmt = CString::new("Hello, %s! Number: %d\n");
        let name = CString::new("World");
        libc::printf(fmt.as_ptr(), name.as_ptr(), 42i32);
    }
}
```

### 4.4 Inline Assembly (Low-Level Escape)

For cases requiring direct hardware access:

```blood
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) / pure {
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);

    @unsafe {
        asm!(
            "cpuid",
            inout("eax") leaf => eax,
            out("ebx") ebx,
            out("ecx") ecx,
            out("edx") edx,
        )
    }

    (eax, ebx, ecx, edx)
}
```

---

## 5. Safety Boundaries

### 5.1 The Safety Contract

Blood's FFI safety model is explicit:

```
┌─────────────────────────────────────────────────────────────┐
│                      SAFE BLOOD CODE                         │
│                                                              │
│  - Type safety guaranteed                                    │
│  - Memory safety via generations                             │
│  - Effect tracking                                           │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│                    SAFE WRAPPER LAYER                        │
│                                                              │
│  - Validates inputs before FFI                               │
│  - Validates outputs after FFI                               │
│  - Converts between Blood and C types                        │
│  - Handles ownership transfer                                │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│                    @unsafe BOUNDARY                          │
│                                                              │
│  - Explicit @unsafe blocks                                   │
│  - No automatic safety guarantees                            │
│  - Developer asserts correctness                             │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│                      FOREIGN CODE                            │
│                                                              │
│  - C/C++/other language                                      │
│  - May have arbitrary behavior                               │
│  - May corrupt memory                                        │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### 5.2 Unsafe Scope Rules

```blood
// WRONG: Implicit unsafe (rejected)
fn call_c() / {FFI} {
    libc::malloc(100)  // ERROR: FFI call requires @unsafe
}

// CORRECT: Explicit unsafe block
fn call_c() / {FFI} {
    @unsafe {
        libc::malloc(100)
    }
}

// CORRECT: Unsafe function (caller must use @unsafe)
@unsafe fn raw_malloc(size: usize) -> *mut u8 / {FFI} {
    libc::malloc(size)
}
```

### 5.3 Safe Wrapper Pattern

```blood
/// Safe wrapper for C memory allocation
fn allocate<T>(value: T) -> Box<T> / {Allocate, Error<AllocError>} {
    let size = std::mem::size_of::<T>();
    let align = std::mem::align_of::<T>();

    // Validate before FFI
    if size == 0 {
        return Err(AllocError::ZeroSize);
    }

    let ptr: *mut T = @unsafe {
        let raw = libc::aligned_alloc(align, size);
        if raw.is_null() {
            return Err(AllocError::OutOfMemory);
        }
        raw as *mut T
    };

    // Initialize and wrap in safe Box
    @unsafe {
        std::ptr::write(ptr, value);
        Box::from_raw(ptr)
    }
}
```

### 5.4 Input Validation

```blood
/// Validates a C string before passing to foreign code
fn safe_strlen(s: &str) -> usize / {FFI} {
    // Validate: must not contain interior null bytes
    if s.bytes().any(|b| b == 0) {
        panic!("String contains interior null byte");
    }

    let cstr = CString::new(s);

    @unsafe {
        libc::strlen(cstr.as_ptr())
    }
}
```

### 5.5 Output Validation

```blood
/// Reads a C string and validates UTF-8
fn read_c_string(ptr: *const u8) -> Result<String, FfiError> / {FFI} {
    if ptr.is_null() {
        return Err(FfiError::NullPointer);
    }

    @unsafe {
        let len = libc::strlen(ptr);
        let slice = std::slice::from_raw_parts(ptr, len);

        // Validate UTF-8
        match std::str::from_utf8(slice) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => Err(FfiError::InvalidUtf8(e)),
        }
    }
}
```

### 5.6 The @unsafe Audit Trail

All `@unsafe` blocks should include a safety comment:

```blood
fn copy_memory(dest: *mut u8, src: *const u8, len: usize) / {FFI} {
    @unsafe {
        // SAFETY:
        // - Caller guarantees dest has space for `len` bytes
        // - Caller guarantees src is readable for `len` bytes
        // - Caller guarantees no overlap (use memmove otherwise)
        libc::memcpy(dest, src, len);
    }
}
```

---

## 6. Memory Ownership

### 6.1 Ownership Transfer Semantics

FFI must clearly define who owns memory:

| Pattern | Blood Side | Foreign Side |
|---------|------------|--------------|
| **Borrow** | Lends pointer | Must not free or store |
| **Transfer Out** | Gives up ownership | Responsible for freeing |
| **Transfer In** | Receives ownership | No longer responsible |
| **Shared** | Reference counted | Must increment/decrement |

### 6.2 Ownership Annotations

```blood
bridge "C" mylib {
    /// Blood allocates, C uses temporarily
    fn process_data(
        #[borrow] data: *const u8,
        len: usize
    ) -> i32;

    /// Blood transfers ownership to C
    fn take_buffer(
        #[transfer] buffer: *mut u8,
        len: usize
    );

    /// C allocates, Blood takes ownership
    fn create_buffer(
        len: usize
    ) -> #[acquire] *mut u8;

    /// C allocates, returns borrowed view
    fn get_static_string() -> #[borrow] *const u8;
}
```

### 6.3 Ownership Patterns in Practice

```blood
/// Borrow pattern: Blood owns, C borrows
fn process(data: &[u8]) -> i32 / {FFI} {
    @unsafe {
        // C gets a pointer, must not store it
        mylib::process_data(data.as_ptr(), data.len())
    }
    // data still valid here
}

/// Transfer out: Blood gives ownership to C
fn give_to_c(data: Vec<u8>) / {FFI} {
    let (ptr, len, cap) = data.into_raw_parts();

    @unsafe {
        mylib::take_buffer(ptr, len);
        // DO NOT drop 'data' - C owns it now
        std::mem::forget(data);
    }
}

/// Transfer in: C gives ownership to Blood
fn receive_from_c(len: usize) -> Vec<u8> / {FFI} {
    @unsafe {
        let ptr = mylib::create_buffer(len);
        if ptr.is_null() {
            panic!("create_buffer returned null");
        }
        // Blood now owns this memory
        Vec::from_raw_parts(ptr, len, len)
    }
}
```

### 6.4 Custom Deallocators

```blood
/// Resource that must be freed with specific function
struct CResource {
    ptr: *mut c_void,
}

impl CResource {
    fn new() -> CResource / {FFI} {
        @unsafe {
            CResource { ptr: mylib::create_resource() }
        }
    }
}

impl Drop for CResource {
    fn drop(&mut self) / {FFI} {
        @unsafe {
            // Must use library's free function, not libc::free
            mylib::destroy_resource(self.ptr);
        }
    }
}
```

### 6.5 Preventing Memory Leaks

```blood
/// RAII wrapper for C resources
struct CGuard<T, F: Fn(*mut T)> {
    ptr: *mut T,
    destructor: F,
}

impl<T, F: Fn(*mut T)> CGuard<T, F> {
    fn new(ptr: *mut T, destructor: F) -> Self {
        CGuard { ptr, destructor }
    }

    fn as_ptr(&self) -> *mut T { self.ptr }
}

impl<T, F: Fn(*mut T)> Drop for CGuard<T, F> {
    fn drop(&mut self) {
        (self.destructor)(self.ptr)
    }
}

// Usage
fn use_resource() / {FFI} {
    let guard = @unsafe {
        CGuard::new(
            mylib::create_resource(),
            |p| mylib::destroy_resource(p)
        )
    };

    @unsafe {
        mylib::use_resource(guard.as_ptr());
    }
    // Automatically cleaned up
}
```

---

## 7. Effect Integration

### 7.1 FFI as an Effect

All foreign function calls are effectful:

```blood
/// The FFI effect
effect FFI extends IO {
    /// Marker that foreign code was called
    /// Used for tracking, not typically handled
}

// FFI functions implicitly have the FFI effect
fn call_foreign() / {FFI} {
    @unsafe {
        libc::printf("Hello\n".as_ptr());
    }
}
```

### 7.2 Effect Propagation

```blood
/// Safe wrapper with narrower effect
fn print_message(msg: &str) / {IO} {
    let cstr = CString::new(msg);
    @unsafe {
        libc::printf(cstr.as_ptr());
    }
    // FFI effect is "absorbed" into IO since FFI extends IO
}

/// Pure wrapper (must guarantee no side effects)
fn compute_length(s: &str) -> usize / pure {
    // ERROR: Cannot call FFI from pure context
    // libc::strlen(...)

    // Instead, use pure Blood code
    s.len()
}
```

### 7.3 Mocking FFI for Testing

```blood
/// FFI mock handler for testing
deep handler MockFFI for FFI {
    let log: Vec<FfiCall> = vec![]

    return(x) { (x, log) }

    op call_foreign(symbol, args) {
        log.push(FfiCall { symbol, args });
        resume(mock_result(symbol, args))
    }
}

fn mock_result(symbol: Symbol, args: ForeignArgs) -> ForeignResult {
    match symbol.name {
        "malloc" => ForeignResult::Pointer(0x1000),
        "strlen" => ForeignResult::Size(5),
        _ => ForeignResult::Error,
    }
}

#[test]
fn test_with_mock_ffi() {
    let (result, calls) = with MockFFI handle {
        safe_allocate(100)
    };

    assert!(calls.iter().any(|c| c.symbol.name == "malloc"));
}
```

### 7.4 Panic Safety

```blood
/// Panics must not unwind through C frames
fn call_c_with_callback(f: fn() -> i32) -> i32 / {FFI} {
    // Blood callback that might panic
    let callback = || -> i32 {
        with CatchPanic handle {
            f()
        }
    };

    @unsafe {
        // C code calls the callback
        mylib::call_with_callback(callback)
    }
}

/// Attribute to prevent unwinding
#[no_unwind]
fn c_compatible_callback(x: i32) -> i32 {
    // Compiler ensures this cannot panic
    x * 2
}
```

---

## 8. Error Handling

### 8.1 C Error Conventions

C code typically signals errors via:
1. Return codes (0 = success, negative = error)
2. `errno` global variable
3. Out parameters
4. NULL pointers

Blood provides utilities for each:

### 8.2 Return Code Handling

```blood
bridge "C" libc {
    fn open(path: *const u8, flags: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
}

/// Convert C return code to Blood Result
fn safe_open(path: &Path, flags: OpenFlags) -> Result<Fd, IoError> / {FFI} {
    let cpath = CString::new(path.to_str());

    let fd = @unsafe {
        libc::open(cpath.as_ptr(), flags.bits())
    };

    if fd < 0 {
        Err(IoError::from_errno())
    } else {
        Ok(Fd(fd))
    }
}
```

### 8.3 Errno Handling

```blood
bridge "C" libc {
    fn __errno_location() -> *mut i32;  // Linux
    // fn __error() -> *mut i32;        // macOS
    // fn _errno() -> *mut i32;         // Windows
}

fn errno() -> i32 / {FFI} {
    @unsafe {
        *libc::__errno_location()
    }
}

fn set_errno(value: i32) / {FFI} {
    @unsafe {
        *libc::__errno_location() = value;
    }
}

/// Standard errno values
enum Errno {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    // ... etc
}

impl From<Errno> for IoError {
    fn from(e: Errno) -> IoError {
        match e {
            Errno::ENOENT => IoError::NotFound,
            Errno::EACCES => IoError::PermissionDenied,
            Errno::EEXIST => IoError::AlreadyExists,
            _ => IoError::Other(e as i32),
        }
    }
}
```

### 8.4 Out-Parameter Errors

```blood
bridge "C" mylib {
    fn get_value(key: *const u8, out_value: *mut i32, out_error: *mut i32) -> bool;
}

fn safe_get_value(key: &str) -> Result<i32, MyError> / {FFI} {
    let ckey = CString::new(key);
    let mut value: i32 = 0;
    let mut error: i32 = 0;

    let success = @unsafe {
        mylib::get_value(ckey.as_ptr(), &mut value, &mut error)
    };

    if success {
        Ok(value)
    } else {
        Err(MyError::from_code(error))
    }
}
```

### 8.5 NULL Pointer Errors

```blood
fn safe_fopen(path: &str, mode: &str) -> Result<File, IoError> / {FFI} {
    let cpath = CString::new(path);
    let cmode = CString::new(mode);

    let file_ptr = @unsafe {
        libc::fopen(cpath.as_ptr(), cmode.as_ptr())
    };

    if file_ptr.is_null() {
        Err(IoError::from_errno())
    } else {
        Ok(File { ptr: file_ptr })
    }
}
```

### 8.6 Error Effect Integration

```blood
/// Convert FFI errors to Blood effects
fn read_file(path: &Path) -> Vec<u8> / {FFI, Error<IoError>} {
    let fd = safe_open(path, OpenFlags::READ)?;

    let mut buffer = Vec::with_capacity(4096);
    loop {
        let n = @unsafe {
            libc::read(fd.0, buffer.as_mut_ptr().add(buffer.len()), 4096)
        };

        if n < 0 {
            raise(IoError::from_errno());
        } else if n == 0 {
            break;
        } else {
            @unsafe { buffer.set_len(buffer.len() + n as usize) };
        }
    }

    safe_close(fd)?;
    buffer
}
```

---

## 9. Platform-Specific Considerations

### 9.1 Conditional Compilation

```blood
bridge "C" platform {
    #[cfg(target_os = "linux")]
    mod linux {
        fn epoll_create(size: i32) -> i32;
        fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: *mut EpollEvent) -> i32;
        fn epoll_wait(epfd: i32, events: *mut EpollEvent, max: i32, timeout: i32) -> i32;
    }

    #[cfg(target_os = "macos")]
    mod macos {
        fn kqueue() -> i32;
        fn kevent(kq: i32, changelist: *const Kevent, nchanges: i32,
                  eventlist: *mut Kevent, nevents: i32, timeout: *const Timespec) -> i32;
    }

    #[cfg(target_os = "windows")]
    mod windows {
        fn CreateIoCompletionPort(
            file: usize, port: usize, key: usize, threads: u32
        ) -> usize;
    }
}
```

### 9.2 Platform Abstractions

```blood
/// Cross-platform event loop
trait EventLoop {
    fn new() -> Self / {FFI};
    fn register(&mut self, fd: Fd, events: Events) -> Result<(), IoError> / {FFI};
    fn wait(&mut self, timeout: Duration) -> Vec<Event> / {FFI};
}

#[cfg(target_os = "linux")]
struct LinuxEventLoop { epfd: i32 }

#[cfg(target_os = "linux")]
impl EventLoop for LinuxEventLoop {
    fn new() -> Self / {FFI} {
        let epfd = @unsafe { platform::linux::epoll_create(1) };
        LinuxEventLoop { epfd }
    }
    // ...
}

#[cfg(target_os = "macos")]
struct MacOSEventLoop { kq: i32 }

#[cfg(target_os = "macos")]
impl EventLoop for MacOSEventLoop {
    fn new() -> Self / {FFI} {
        let kq = @unsafe { platform::macos::kqueue() };
        MacOSEventLoop { kq }
    }
    // ...
}
```

### 9.3 ABI Differences

| Platform | Pointer Size | Long Size | Default Convention |
|----------|-------------|-----------|-------------------|
| Linux x86_64 | 8 | 8 | System V AMD64 |
| Linux arm64 | 8 | 8 | AAPCS64 |
| Windows x64 | 8 | 4 | Win64 |
| macOS x64 | 8 | 8 | System V AMD64 |
| macOS arm64 | 8 | 8 | AAPCS64 |
| WASM32 | 4 | 4 | WASM |

```blood
/// Platform-specific type aliases
#[cfg(target_os = "windows")]
type c_long = i32;

#[cfg(not(target_os = "windows"))]
type c_long = i64;
```

### 9.4 WebAssembly Considerations

```blood
bridge "wasm" env {
    #[link(wasm_import_module = "env")]

    /// WASM imports must be declared
    fn console_log(ptr: *const u8, len: usize);
    fn get_time() -> f64;
}

/// WASM exports
#[export]
fn greet(name_ptr: *const u8, name_len: usize) -> *const u8 {
    // Return pointer to static or heap memory
    // (WASM linear memory is shared with host)
}

/// Memory management for WASM
#[export]
fn alloc(size: usize) -> *mut u8 {
    // Allocate in WASM linear memory
}

#[export]
fn dealloc(ptr: *mut u8, size: usize) {
    // Free from WASM linear memory
}
```

---

## 10. Code Generation

### 10.1 Compiler Output

For each bridge block, the Blood compiler generates:

1. **Header file** (optional): For C code calling Blood
2. **Binding code**: Low-level FFI calls
3. **Metadata**: For debugging and reflection

### 10.2 Generated Binding Example

```blood
// Blood source
bridge "C" example {
    fn add(a: i32, b: i32) -> i32;
}

fn use_add() / {FFI} {
    @unsafe { example::add(1, 2) }
}
```

```asm
; Generated assembly (simplified)
use_add:
    mov     edi, 1
    mov     esi, 2
    call    add@PLT          ; Call C function via PLT
    ret
```

### 10.3 Exporting Blood Functions

```blood
/// Export Blood function for C to call
#[export(name = "blood_process")]
fn process_data(ptr: *const u8, len: usize) -> i32 / pure {
    // Pure function safe to call from C
    // (no Blood effects, no panics)
    42
}
```

Generated C header:

```c
// blood_exports.h (auto-generated)
#ifndef BLOOD_EXPORTS_H
#define BLOOD_EXPORTS_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

int32_t blood_process(const uint8_t* ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif
```

### 10.4 Build System Integration

```toml
# blood.toml
[package]
name = "myproject"
version = "0.1.0"

[ffi]
# C libraries to link
links = ["z", "ssl", "crypto"]

# Header search paths
include_dirs = ["/usr/include/openssl"]

# Library search paths
lib_dirs = ["/usr/lib"]

# Generate C headers for exports
generate_headers = true
header_output = "include/myproject.h"

[ffi.platform.linux]
links = ["pthread", "dl"]

[ffi.platform.windows]
links = ["ws2_32", "userenv"]
```

---

## Appendix A: Complete Type Mapping Reference

### A.1 Primitive Types

| Blood | C | Rust | Size |
|-------|---|------|------|
| `i8` | `int8_t` | `i8` | 1 |
| `i16` | `int16_t` | `i16` | 2 |
| `i32` | `int32_t` | `i32` | 4 |
| `i64` | `int64_t` | `i64` | 8 |
| `i128` | `__int128` | `i128` | 16 |
| `u8` | `uint8_t` | `u8` | 1 |
| `u16` | `uint16_t` | `u16` | 2 |
| `u32` | `uint32_t` | `u32` | 4 |
| `u64` | `uint64_t` | `u64` | 8 |
| `u128` | `unsigned __int128` | `u128` | 16 |
| `f32` | `float` | `f32` | 4 |
| `f64` | `double` | `f64` | 8 |
| `bool` | `_Bool` | `bool` | 1 |
| `usize` | `size_t` | `usize` | ptr |
| `isize` | `ptrdiff_t` | `isize` | ptr |
| `*const T` | `const T*` | `*const T` | ptr |
| `*mut T` | `T*` | `*mut T` | ptr |
| `()` | `void` | `()` | 0 |
| `!` | (noreturn) | `!` | 0 |

### A.2 Complex Types

| Blood | C Equivalent | Notes |
|-------|--------------|-------|
| `#[repr(C)] struct` | `struct` | Same layout |
| `#[repr(C)] enum` | `enum` | Same layout |
| `#[repr(C)] union` | `union` | Same layout |
| `Option<*T>` | `T*` | None = NULL |
| `fn(...) -> T` | `T(*)(...)` | Function pointer |

---

## Appendix B: Safety Checklist

Before releasing FFI code, verify:

- [ ] All raw pointer uses are in `@unsafe` blocks
- [ ] All `@unsafe` blocks have safety comments
- [ ] Ownership semantics are documented and consistent
- [ ] Null pointers are checked before dereference
- [ ] Buffer lengths are validated
- [ ] String encoding is validated (UTF-8 for Blood, null-terminated for C)
- [ ] Error codes are converted to Blood errors
- [ ] Resources are freed with correct deallocator
- [ ] No panics can unwind through C frames
- [ ] Platform-specific code is properly gated with `#[cfg]`

---

## Appendix C: References

FFI design draws from established practices:

- [The Rustonomicon - FFI](https://doc.rust-lang.org/nomicon/ffi.html)
- [Zig C Interoperability](https://zighelp.org/chapter-4/)
- [C++/Rust FFI with cxx Bridge](https://cxx.rs/)
- [GCC Guidelines for Diagnostics](https://gcc.gnu.org/onlinedocs/gccint/Guidelines-for-Diagnostics.html)

---

*This document is part of the Blood Language Specification.*
