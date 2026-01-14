// Blood Runtime Library - C Entry Point
// Compile with: cc -c runtime.c -o runtime.o
//
// This minimal C runtime provides ONLY the main() entry point and
// a few utility functions. All other runtime functionality is provided
// by the Rust blood-runtime crate (libblood_runtime.a).
//
// To use: Link your Blood program with BOTH runtime.o AND libblood_runtime.a
//   cc program.o runtime.o -lblood_runtime -lpthread -ldl -lm -o program

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

// ============================================================================
// BloodStr - Fat Pointer String Type
// ============================================================================

// Blood's str type is a fat pointer: {ptr, len}
// This struct must match the LLVM type {i8*, i64}
typedef struct {
    char* ptr;
    int64_t len;
} BloodStr;

// ============================================================================
// String Functions (not provided by Rust runtime)
// ============================================================================

// Simple memory allocation for string operations
static void* string_alloc(size_t size) {
    void* ptr = malloc(size);
    if (ptr == NULL) {
        fprintf(stderr, "blood: out of memory\n");
        exit(1);
    }
    return ptr;
}

// NOTE: blood_str_concat is now implemented in the Rust runtime (ffi_exports.rs)
// to properly handle the BloodStr fat pointer type.

// Convert integer to string
char* blood_int_to_str(int32_t n) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%d", n);
    char* result = (char*)string_alloc(strlen(buffer) + 1);
    strcpy(result, buffer);
    return result;
}

// Convert 64-bit integer to string
char* blood_i64_to_str(int64_t n) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%ld", n);
    char* result = (char*)string_alloc(strlen(buffer) + 1);
    strcpy(result, buffer);
    return result;
}

// Convert float to string
char* blood_f64_to_str(double n) {
    char buffer[64];
    snprintf(buffer, sizeof(buffer), "%g", n);
    char* result = (char*)string_alloc(strlen(buffer) + 1);
    strcpy(result, buffer);
    return result;
}

// Get string length
size_t blood_str_len(const char* s) {
    return s ? strlen(s) : 0;
}

// Compare strings (returns 0 if equal, <0 if a<b, >0 if a>b)
int blood_str_cmp(const char* a, const char* b) {
    if (a == NULL && b == NULL) return 0;
    if (a == NULL) return -1;
    if (b == NULL) return 1;
    return strcmp(a, b);
}

// Free a string allocated by blood_str_* functions
void blood_str_free(char* s) {
    free(s);
}

// NOTE: int_to_string and bool_to_string are now implemented in the Rust runtime
// (ffi_exports.rs) to return BloodStr fat pointers.

// ============================================================================
// Additional I/O Functions (supplements to Rust runtime)
// ============================================================================

// Print a character (no newline)
void print_char(int32_t c) {
    printf("%c", (char)c);
    fflush(stdout);
}

// Print a character with newline
void println_char(int32_t c) {
    printf("%c\n", (char)c);
    fflush(stdout);
}

// Print a newline
void println(void) {
    printf("\n");
    fflush(stdout);
}

// Note: println_i64 is provided by the Rust runtime (libblood_runtime.a)

// Print a 64-bit integer (no newline) - NOT in Rust runtime
void print_i64(int64_t n) {
    printf("%ld", n);
    fflush(stdout);
}

// Print an unsigned 64-bit integer (no newline)
void print_u64(uint64_t n) {
    printf("%lu", n);
    fflush(stdout);
}

// Print an unsigned 64-bit integer with newline
void println_u64(uint64_t n) {
    printf("%lu\n", n);
    fflush(stdout);
}

// Print a double (no newline)
void print_f64(double n) {
    printf("%g", n);
    fflush(stdout);
}

// Print a double with newline
void println_f64(double n) {
    printf("%g\n", n);
    fflush(stdout);
}

// Print a float (no newline)
void print_f32(float n) {
    printf("%g", (double)n);
    fflush(stdout);
}

// Print a float with newline
void println_f32(float n) {
    printf("%g\n", (double)n);
    fflush(stdout);
}

// Print a double with specified decimal precision (no newline)
void print_f64_prec(double n, int32_t precision) {
    printf("%.*f", precision, n);
    fflush(stdout);
}

// Print a double with specified decimal precision (with newline)
void println_f64_prec(double n, int32_t precision) {
    printf("%.*f\n", precision, n);
    fflush(stdout);
}

// Print a float with specified decimal precision (no newline)
void print_f32_prec(float n, int32_t precision) {
    printf("%.*f", precision, (double)n);
    fflush(stdout);
}

// Print a float with specified decimal precision (with newline)
void println_f32_prec(float n, int32_t precision) {
    printf("%.*f\n", precision, (double)n);
    fflush(stdout);
}

// Print a boolean
void print_bool(int b) {
    printf("%s", b ? "true" : "false");
    fflush(stdout);
}

// Print a boolean with newline
void println_bool(int b) {
    printf("%s\n", b ? "true" : "false");
    fflush(stdout);
}

// ============================================================================
// Assertions
// ============================================================================

void blood_assert(int32_t condition) {
    if (!condition) {
        fprintf(stderr, "BLOOD ASSERTION FAILED\n");
        abort();
    }
}

void blood_assert_eq_int(int32_t a, int32_t b) {
    if (a != b) {
        fprintf(stderr, "BLOOD ASSERTION FAILED: %d != %d\n", a, b);
        abort();
    }
}

void blood_assert_eq_bool(int32_t a, int32_t b) {
    if (a != b) {
        fprintf(stderr, "BLOOD ASSERTION FAILED: %s != %s\n",
                a ? "true" : "false", b ? "true" : "false");
        abort();
    }
}

void blood_unreachable(void) {
    fprintf(stderr, "BLOOD RUNTIME ERROR: Unreachable code was reached!\n");
    abort();
}

// ============================================================================
// Backward Compatibility - Simple Allocation Functions
// These are NOT the main memory management functions (see Rust runtime).
// They're provided for legacy code that uses simpler allocation patterns.
// ============================================================================

void* blood_alloc_simple(size_t size) {
    void* ptr = malloc(size);
    if (ptr == NULL) {
        fprintf(stderr, "blood: out of memory\n");
        exit(1);
    }
    return ptr;
}

void* blood_realloc(void* ptr, size_t size) {
    void* new_ptr = realloc(ptr, size);
    if (new_ptr == NULL && size > 0) {
        fprintf(stderr, "blood: out of memory\n");
        exit(1);
    }
    return new_ptr;
}

void blood_free_simple(void* ptr) {
    free(ptr);
}

// ============================================================================
// Memory Intrinsics for Vec/Collections
// ============================================================================

// Copy n bytes from src to dest, return dest
void* blood_memcpy(void* dest, const void* src, size_t n) {
    return memcpy(dest, src, n);
}

// Read i32 from memory address
int32_t ptr_read_i32(uint64_t ptr) {
    return *(int32_t*)(uintptr_t)ptr;
}

// Write i32 to memory address
void ptr_write_i32(uint64_t ptr, int32_t value) {
    *(int32_t*)(uintptr_t)ptr = value;
}

// Read i64 from memory address
int64_t ptr_read_i64(uint64_t ptr) {
    return *(int64_t*)(uintptr_t)ptr;
}

// Write i64 to memory address
void ptr_write_i64(uint64_t ptr, int64_t value) {
    *(int64_t*)(uintptr_t)ptr = value;
}

// Read u64 from memory address
uint64_t ptr_read_u64(uint64_t ptr) {
    return *(uint64_t*)(uintptr_t)ptr;
}

// Write u64 to memory address
void ptr_write_u64(uint64_t ptr, uint64_t value) {
    *(uint64_t*)(uintptr_t)ptr = value;
}

// Read u8 from memory address
uint8_t ptr_read_u8(uint64_t ptr) {
    return *(uint8_t*)(uintptr_t)ptr;
}

// Write u8 to memory address
void ptr_write_u8(uint64_t ptr, uint8_t value) {
    *(uint8_t*)(uintptr_t)ptr = value;
}

// Read f64 from memory address
double ptr_read_f64(uint64_t ptr) {
    return *(double*)(uintptr_t)ptr;
}

// Write f64 to memory address
void ptr_write_f64(uint64_t ptr, double value) {
    *(double*)(uintptr_t)ptr = value;
}

// ============================================================================
// Entry Point
// ============================================================================

// The Blood main function - defined by compiled Blood programs
// Returns int32_t (i32) as the exit code
extern int32_t blood_main(void);

// Rust runtime initialization (from libblood_runtime.a)
extern int blood_runtime_init(void);
extern void blood_runtime_shutdown(void);

int main(int argc, char** argv) {
    (void)argc;
    (void)argv;

    // Initialize the Rust runtime
    blood_runtime_init();

    // Call the Blood program's main function and capture return value
    int32_t result = blood_main();

    // Shutdown the runtime
    blood_runtime_shutdown();

    return result;
}
