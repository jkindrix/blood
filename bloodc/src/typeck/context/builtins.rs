//! Built-in function registration for the type checker.

use crate::hir::{self, Type};
use crate::span::Span;

use super::TypeContext;

impl<'a> TypeContext<'a> {
    /// Register built-in runtime functions.
    pub(crate) fn register_builtins(&mut self) {
        let unit_ty = Type::unit();
        let bool_ty = Type::bool();
        let char_ty = Type::char();
        let u8_ty = Type::u8();
        let i32_ty = Type::i32();
        let i64_ty = Type::i64();
        let u64_ty = Type::u64();
        let f32_ty = Type::f32();
        let f64_ty = Type::f64();
        let str_ty = Type::str();  // str slice for string literals
        let _string_ty = Type::string();  // owned String (for functions that return owned strings)
        let never_ty = Type::never();

        // === I/O Functions ===

        // print(str) -> () - convenience function (maps to runtime print_str)
        self.register_builtin_fn_aliased("print", "print_str", vec![str_ty.clone()], unit_ty.clone());

        // println(str) -> () - convenience function (prints string + newline, maps to runtime println_str)
        self.register_builtin_fn_aliased("println", "println_str", vec![str_ty.clone()], unit_ty.clone());

        // print_int(i32) -> ()
        self.register_builtin_fn("print_int", vec![i32_ty.clone()], unit_ty.clone());

        // println_int(i32) -> ()
        self.register_builtin_fn("println_int", vec![i32_ty.clone()], unit_ty.clone());

        // print_str(str) -> () - legacy name, same as print
        self.register_builtin_fn("print_str", vec![str_ty.clone()], unit_ty.clone());

        // println_str(str) -> () - legacy name, same as println
        self.register_builtin_fn("println_str", vec![str_ty.clone()], unit_ty.clone());

        // print_char(char) -> ()
        self.register_builtin_fn("print_char", vec![char_ty.clone()], unit_ty.clone());

        // println_char(char) -> ()
        self.register_builtin_fn("println_char", vec![char_ty.clone()], unit_ty.clone());

        // print_newline() -> () - prints just a newline
        self.register_builtin_fn("print_newline", vec![], unit_ty.clone());

        // print_bool(bool) -> ()
        self.register_builtin_fn("print_bool", vec![bool_ty.clone()], unit_ty.clone());

        // println_bool(bool) -> ()
        self.register_builtin_fn("println_bool", vec![bool_ty.clone()], unit_ty.clone());

        // print_f64(f64) -> ()
        self.register_builtin_fn("print_f64", vec![f64_ty.clone()], unit_ty.clone());

        // println_f64(f64) -> ()
        self.register_builtin_fn("println_f64", vec![f64_ty.clone()], unit_ty.clone());

        // print_i64(i64) -> () - 64-bit integer output
        self.register_builtin_fn("print_i64", vec![i64_ty.clone()], unit_ty.clone());

        // println_i64(i64) -> () - 64-bit integer output with newline
        self.register_builtin_fn("println_i64", vec![i64_ty.clone()], unit_ty.clone());

        // print_u64(u64) -> () - unsigned 64-bit integer output
        self.register_builtin_fn("print_u64", vec![u64_ty.clone()], unit_ty.clone());

        // println_u64(u64) -> () - unsigned 64-bit integer output with newline
        self.register_builtin_fn("println_u64", vec![u64_ty.clone()], unit_ty.clone());

        // print_f32(f32) -> ()
        self.register_builtin_fn("print_f32", vec![f32_ty.clone()], unit_ty.clone());

        // println_f32(f32) -> ()
        self.register_builtin_fn("println_f32", vec![f32_ty.clone()], unit_ty.clone());

        // === Control Flow / Assertions ===

        // panic(str) -> !
        self.register_builtin_fn("panic", vec![str_ty.clone()], never_ty.clone());

        // assert(bool) -> () - maps to blood_assert in runtime
        self.register_builtin_fn_aliased("assert", "blood_assert", vec![bool_ty.clone()], unit_ty.clone());

        // assert_eq_int(i32, i32) -> () - maps to blood_assert_eq_int in runtime
        self.register_builtin_fn_aliased("assert_eq_int", "blood_assert_eq_int", vec![i32_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // assert_eq_bool(bool, bool) -> () - maps to blood_assert_eq_bool in runtime
        self.register_builtin_fn_aliased("assert_eq_bool", "blood_assert_eq_bool", vec![bool_ty.clone(), bool_ty.clone()], unit_ty.clone());

        // unreachable() -> !
        self.register_builtin_fn("unreachable", vec![], never_ty.clone());

        // todo() -> !
        self.register_builtin_fn("todo", vec![], never_ty.clone());

        // === Memory Functions ===

        // size_of_i32() -> i64
        self.register_builtin_fn("size_of_i32", vec![], i64_ty.clone());

        // size_of_i64() -> i64
        self.register_builtin_fn("size_of_i64", vec![], i64_ty.clone());

        // size_of_bool() -> i64
        self.register_builtin_fn("size_of_bool", vec![], i64_ty.clone());

        // === Dynamic Memory Allocation ===
        // These use u64 for pointer addresses (void* on 64-bit systems)

        // alloc(size: u64) -> u64 - allocate memory, returns address (0 on failure)
        self.register_builtin_fn_aliased("alloc", "blood_alloc_simple", vec![u64_ty.clone()], u64_ty.clone());

        // realloc(ptr: u64, size: u64) -> u64 - reallocate memory, returns new address
        self.register_builtin_fn_aliased("realloc", "blood_realloc", vec![u64_ty.clone(), u64_ty.clone()], u64_ty.clone());

        // free(ptr: u64) -> () - free allocated memory
        self.register_builtin_fn_aliased("free", "blood_free_simple", vec![u64_ty.clone()], unit_ty.clone());

        // memcpy(dest: u64, src: u64, n: u64) -> u64 - copy n bytes, returns dest
        self.register_builtin_fn_aliased("memcpy", "blood_memcpy", vec![u64_ty.clone(), u64_ty.clone(), u64_ty.clone()], u64_ty.clone());

        // ptr_read_i32(ptr: u64) -> i32 - read i32 from memory address
        self.register_builtin_fn("ptr_read_i32", vec![u64_ty.clone()], i32_ty.clone());

        // ptr_write_i32(ptr: u64, value: i32) -> () - write i32 to memory address
        self.register_builtin_fn("ptr_write_i32", vec![u64_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // ptr_read_i64(ptr: u64) -> i64 - read i64 from memory address
        self.register_builtin_fn("ptr_read_i64", vec![u64_ty.clone()], i64_ty.clone());

        // ptr_write_i64(ptr: u64, value: i64) -> () - write i64 to memory address
        self.register_builtin_fn("ptr_write_i64", vec![u64_ty.clone(), i64_ty.clone()], unit_ty.clone());

        // ptr_read_u64(ptr: u64) -> u64 - read u64 from memory address
        self.register_builtin_fn("ptr_read_u64", vec![u64_ty.clone()], u64_ty.clone());

        // ptr_write_u64(ptr: u64, value: u64) -> () - write u64 to memory address
        self.register_builtin_fn("ptr_write_u64", vec![u64_ty.clone(), u64_ty.clone()], unit_ty.clone());

        // ptr_read_u8(ptr: u64) -> u8 - read u8 from memory address
        self.register_builtin_fn("ptr_read_u8", vec![u64_ty.clone()], u8_ty.clone());

        // ptr_write_u8(ptr: u64, value: u8) -> () - write u8 to memory address
        self.register_builtin_fn("ptr_write_u8", vec![u64_ty.clone(), u8_ty.clone()], unit_ty.clone());

        // === String Operations ===

        // str_len(str) -> i64 - get string length in bytes
        self.register_builtin_fn("str_len", vec![str_ty.clone()], i64_ty.clone());

        // str_eq(str, str) -> bool - compare two strings for equality
        self.register_builtin_fn("str_eq", vec![str_ty.clone(), str_ty.clone()], bool_ty.clone());

        // str_concat(str, str) -> str - concatenate two strings (returns newly allocated string)
        self.register_builtin_fn_aliased("str_concat", "blood_str_concat", vec![str_ty.clone(), str_ty.clone()], str_ty.clone());

        // === Input Functions ===

        // read_line() -> str - read a line from stdin
        self.register_builtin_fn("read_line", vec![], str_ty.clone());

        // read_int() -> i32 - read an integer from stdin
        self.register_builtin_fn("read_int", vec![], i32_ty.clone());

        // === Conversion Functions ===

        // int_to_string(i32) -> str - convert integer to string
        self.register_builtin_fn("int_to_string", vec![i32_ty.clone()], str_ty.clone());

        // bool_to_string(bool) -> str - convert boolean to string
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], str_ty.clone());

        // i32_to_i64(i32) -> i64
        self.register_builtin_fn("i32_to_i64", vec![i32_ty.clone()], i64_ty.clone());

        // i64_to_i32(i64) -> i32
        self.register_builtin_fn("i64_to_i32", vec![i64_ty.clone()], i32_ty.clone());
    }

    /// Register a single built-in function.
    pub(crate) fn register_builtin_fn(&mut self, name: &str, inputs: Vec<Type>, output: Type) {
        self.register_builtin_fn_aliased(name, name, inputs, output);
    }

    /// Register a builtin function with a user-facing name that maps to a different runtime name.
    /// E.g., `println(String)` maps to runtime function `println_str`.
    pub(crate) fn register_builtin_fn_aliased(&mut self, user_name: &str, runtime_name: &str, inputs: Vec<Type>, output: Type) {
        let def_id = self.resolver.define_item(
            user_name.to_string(),
            hir::DefKind::Fn,
            Span::dummy(),
        ).expect("BUG: builtin registration failed - this indicates a name collision in builtin definitions");

        self.fn_sigs.insert(def_id, hir::FnSig {
            inputs,
            output,
            is_const: false,
            is_async: false,
            is_unsafe: false,
            generics: Vec::new(),
        });

        // Track runtime function name for codegen to resolve runtime function calls
        self.builtin_fns.insert(def_id, runtime_name.to_string());
    }
}
