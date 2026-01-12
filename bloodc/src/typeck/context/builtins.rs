//! Built-in function registration for the type checker.

use crate::hir::{self, Type};
use crate::span::Span;

use super::TypeContext;

impl<'a> TypeContext<'a> {
    /// Register built-in runtime functions.
    pub(crate) fn register_builtins(&mut self) {
        let unit_ty = Type::unit();
        let bool_ty = Type::bool();
        let i32_ty = Type::i32();
        let i64_ty = Type::i64();
        let string_ty = Type::string();
        let never_ty = Type::never();

        // === I/O Functions ===

        // print(String) -> () - convenience function (maps to runtime print_str)
        self.register_builtin_fn_aliased("print", "print_str", vec![string_ty.clone()], unit_ty.clone());

        // println(String) -> () - convenience function (prints string + newline, maps to runtime println_str)
        self.register_builtin_fn_aliased("println", "println_str", vec![string_ty.clone()], unit_ty.clone());

        // print_int(i32) -> ()
        self.register_builtin_fn("print_int", vec![i32_ty.clone()], unit_ty.clone());

        // println_int(i32) -> ()
        self.register_builtin_fn("println_int", vec![i32_ty.clone()], unit_ty.clone());

        // print_str(String) -> () - legacy name, same as print
        self.register_builtin_fn("print_str", vec![string_ty.clone()], unit_ty.clone());

        // println_str(String) -> () - legacy name, same as println
        self.register_builtin_fn("println_str", vec![string_ty.clone()], unit_ty.clone());

        // print_char(i32) -> ()  (char as i32 for now)
        self.register_builtin_fn("print_char", vec![i32_ty.clone()], unit_ty.clone());

        // print_newline() -> () - prints just a newline
        self.register_builtin_fn("print_newline", vec![], unit_ty.clone());

        // print_bool(bool) -> ()
        self.register_builtin_fn("print_bool", vec![bool_ty.clone()], unit_ty.clone());

        // println_bool(bool) -> ()
        self.register_builtin_fn("println_bool", vec![bool_ty.clone()], unit_ty.clone());

        // === Control Flow / Assertions ===

        // panic(String) -> !
        self.register_builtin_fn("panic", vec![string_ty.clone()], never_ty.clone());

        // assert(bool) -> ()
        self.register_builtin_fn("assert", vec![bool_ty.clone()], unit_ty.clone());

        // assert_eq(i32, i32) -> ()
        self.register_builtin_fn("assert_eq_int", vec![i32_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // assert_eq(bool, bool) -> ()
        self.register_builtin_fn("assert_eq_bool", vec![bool_ty.clone(), bool_ty.clone()], unit_ty.clone());

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

        // === Conversion Functions ===

        // int_to_string(i32) -> String
        self.register_builtin_fn("int_to_string", vec![i32_ty.clone()], string_ty.clone());

        // bool_to_string(bool) -> String
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], string_ty.clone());

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
