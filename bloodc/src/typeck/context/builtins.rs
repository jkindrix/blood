//! Built-in function registration for the type checker.

use crate::hir::{self, Type};
use crate::hir::ty::{TyVarId, TypeKind};
use crate::span::Span;

use super::{TypeContext, EnumInfo, VariantInfo, FieldInfo, StructInfo};

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

        // eprint(str) -> () - print to stderr (maps to runtime eprint_str)
        self.register_builtin_fn_aliased("eprint", "eprint_str", vec![str_ty.clone()], unit_ty.clone());

        // eprintln(str) -> () - print to stderr with newline (maps to runtime eprintln_str)
        self.register_builtin_fn_aliased("eprintln", "eprintln_str", vec![str_ty.clone()], unit_ty.clone());

        // eprint_str(str) -> () - print to stderr
        self.register_builtin_fn("eprint_str", vec![str_ty.clone()], unit_ty.clone());

        // eprintln_str(str) -> () - print to stderr with newline
        self.register_builtin_fn("eprintln_str", vec![str_ty.clone()], unit_ty.clone());

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

        // print_f64_prec(f64, i32) -> () - print with specified decimal precision
        self.register_builtin_fn("print_f64_prec", vec![f64_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // println_f64_prec(f64, i32) -> () - print with specified decimal precision and newline
        self.register_builtin_fn("println_f64_prec", vec![f64_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // print_f32_prec(f32, i32) -> () - print with specified decimal precision
        self.register_builtin_fn("print_f32_prec", vec![f32_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // println_f32_prec(f32, i32) -> () - print with specified decimal precision and newline
        self.register_builtin_fn("println_f32_prec", vec![f32_ty.clone(), i32_ty.clone()], unit_ty.clone());

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

        // ptr_read_f64(ptr: u64) -> f64 - read f64 from memory address
        self.register_builtin_fn("ptr_read_f64", vec![u64_ty.clone()], f64_ty.clone());

        // ptr_write_f64(ptr: u64, value: f64) -> () - write f64 to memory address
        self.register_builtin_fn("ptr_write_f64", vec![u64_ty.clone(), f64_ty.clone()], unit_ty.clone());

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

        // i64_to_string(i64) -> str - convert 64-bit integer to string
        self.register_builtin_fn("i64_to_string", vec![i64_ty.clone()], str_ty.clone());

        // u64_to_string(u64) -> str - convert unsigned 64-bit integer to string
        self.register_builtin_fn("u64_to_string", vec![u64_ty.clone()], str_ty.clone());

        // bool_to_string(bool) -> str - convert boolean to string
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], str_ty.clone());

        // char_to_string(char) -> str - convert character to string
        self.register_builtin_fn("char_to_string", vec![char_ty.clone()], str_ty.clone());

        // f32_to_string(f32) -> str - convert f32 to string
        self.register_builtin_fn("f32_to_string", vec![f32_ty.clone()], str_ty.clone());

        // f64_to_string(f64) -> str - convert f64 to string
        self.register_builtin_fn("f64_to_string", vec![f64_ty.clone()], str_ty.clone());

        // i32_to_i64(i32) -> i64
        self.register_builtin_fn("i32_to_i64", vec![i32_ty.clone()], i64_ty.clone());

        // i64_to_i32(i64) -> i32
        self.register_builtin_fn("i64_to_i32", vec![i64_ty.clone()], i32_ty.clone());

        // === Math Functions (LLVM intrinsics for performance) ===
        // These map to LLVM intrinsics which compile to hardware instructions

        // sqrt(f64) -> f64 - square root
        self.register_builtin_fn("sqrt", vec![f64_ty.clone()], f64_ty.clone());

        // sqrt_f32(f32) -> f32 - square root for f32
        self.register_builtin_fn("sqrt_f32", vec![f32_ty.clone()], f32_ty.clone());

        // abs(f64) -> f64 - absolute value
        self.register_builtin_fn("abs", vec![f64_ty.clone()], f64_ty.clone());

        // abs_f32(f32) -> f32 - absolute value for f32
        self.register_builtin_fn("abs_f32", vec![f32_ty.clone()], f32_ty.clone());

        // floor(f64) -> f64 - round down
        self.register_builtin_fn("floor", vec![f64_ty.clone()], f64_ty.clone());

        // ceil(f64) -> f64 - round up
        self.register_builtin_fn("ceil", vec![f64_ty.clone()], f64_ty.clone());

        // sin(f64) -> f64 - sine
        self.register_builtin_fn("sin", vec![f64_ty.clone()], f64_ty.clone());

        // cos(f64) -> f64 - cosine
        self.register_builtin_fn("cos", vec![f64_ty.clone()], f64_ty.clone());

        // exp(f64) -> f64 - exponential
        self.register_builtin_fn("exp", vec![f64_ty.clone()], f64_ty.clone());

        // log(f64) -> f64 - natural logarithm
        self.register_builtin_fn("log", vec![f64_ty.clone()], f64_ty.clone());

        // pow(f64, f64) -> f64 - power
        self.register_builtin_fn("pow", vec![f64_ty.clone(), f64_ty.clone()], f64_ty.clone());
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

    /// Register built-in types like Option<T> and Result<T, E>.
    pub(crate) fn register_builtin_types(&mut self) {
        // Register Option<T> as a built-in enum
        let option_def_id = self.resolver.define_item(
            "Option".to_string(),
            hir::DefKind::Enum,
            Span::dummy(),
        ).expect("BUG: Option builtin registration failed");
        self.resolver.define_type("Option".to_string(), option_def_id, Span::dummy())
            .expect("BUG: Option type registration failed");

        // Option has one type parameter T
        let t_var_id = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;
        let t_type = Type::new(TypeKind::Param(t_var_id));

        // Create None variant (unit variant)
        // Use define_item (not define_namespaced_item) to make None accessible globally
        let none_def_id = self.resolver.define_item(
            "None".to_string(),
            hir::DefKind::Variant,
            Span::dummy(),
        ).expect("BUG: None variant registration failed");
        if let Some(def_info) = self.resolver.def_info.get_mut(&none_def_id) {
            def_info.parent = Some(option_def_id);
        }

        // Create Some(T) variant
        // Use define_item (not define_namespaced_item) to make Some accessible globally
        let some_def_id = self.resolver.define_item(
            "Some".to_string(),
            hir::DefKind::Variant,
            Span::dummy(),
        ).expect("BUG: Some variant registration failed");
        if let Some(def_info) = self.resolver.def_info.get_mut(&some_def_id) {
            def_info.parent = Some(option_def_id);
        }

        // Register enum info
        self.enum_defs.insert(option_def_id, EnumInfo {
            name: "Option".to_string(),
            variants: vec![
                VariantInfo {
                    name: "None".to_string(),
                    fields: vec![],
                    index: 0,
                    def_id: none_def_id,
                },
                VariantInfo {
                    name: "Some".to_string(),
                    fields: vec![
                        FieldInfo {
                            name: "0".to_string(),
                            ty: t_type.clone(),
                            index: 0,
                        },
                    ],
                    index: 1,
                    def_id: some_def_id,
                },
            ],
            generics: vec![t_var_id],
        });

        self.option_def_id = Some(option_def_id);

        // Register Result<T, E> as a built-in enum
        let result_def_id = self.resolver.define_item(
            "Result".to_string(),
            hir::DefKind::Enum,
            Span::dummy(),
        ).expect("BUG: Result builtin registration failed");
        self.resolver.define_type("Result".to_string(), result_def_id, Span::dummy())
            .expect("BUG: Result type registration failed");

        // Result has two type parameters: T and E
        let t_var_id2 = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;
        let t_type2 = Type::new(TypeKind::Param(t_var_id2));

        let e_var_id = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;
        let e_type = Type::new(TypeKind::Param(e_var_id));

        // Create Ok(T) variant
        // Use define_item (not define_namespaced_item) to make Ok accessible globally
        let ok_def_id = self.resolver.define_item(
            "Ok".to_string(),
            hir::DefKind::Variant,
            Span::dummy(),
        ).expect("BUG: Ok variant registration failed");
        if let Some(def_info) = self.resolver.def_info.get_mut(&ok_def_id) {
            def_info.parent = Some(result_def_id);
        }

        // Create Err(E) variant
        // Use define_item (not define_namespaced_item) to make Err accessible globally
        let err_def_id = self.resolver.define_item(
            "Err".to_string(),
            hir::DefKind::Variant,
            Span::dummy(),
        ).expect("BUG: Err variant registration failed");
        if let Some(def_info) = self.resolver.def_info.get_mut(&err_def_id) {
            def_info.parent = Some(result_def_id);
        }

        // Register enum info
        self.enum_defs.insert(result_def_id, EnumInfo {
            name: "Result".to_string(),
            variants: vec![
                VariantInfo {
                    name: "Ok".to_string(),
                    fields: vec![
                        FieldInfo {
                            name: "0".to_string(),
                            ty: t_type2.clone(),
                            index: 0,
                        },
                    ],
                    index: 0,
                    def_id: ok_def_id,
                },
                VariantInfo {
                    name: "Err".to_string(),
                    fields: vec![
                        FieldInfo {
                            name: "0".to_string(),
                            ty: e_type.clone(),
                            index: 0,
                        },
                    ],
                    index: 1,
                    def_id: err_def_id,
                },
            ],
            generics: vec![t_var_id2, e_var_id],
        });

        self.result_def_id = Some(result_def_id);

        // Register Vec<T> as a built-in struct (opaque for now)
        let vec_def_id = self.resolver.define_item(
            "Vec".to_string(),
            hir::DefKind::Struct,
            Span::dummy(),
        ).expect("BUG: Vec builtin registration failed");
        self.resolver.define_type("Vec".to_string(), vec_def_id, Span::dummy())
            .expect("BUG: Vec type registration failed");

        // Vec has one type parameter T
        let vec_t_var_id = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;

        // Register struct info (opaque struct with no exposed fields)
        self.struct_defs.insert(vec_def_id, StructInfo {
            name: "Vec".to_string(),
            fields: vec![],  // opaque - no exposed fields
            generics: vec![vec_t_var_id],
        });

        // Register Box<T> as a built-in struct (opaque for now)
        let box_def_id = self.resolver.define_item(
            "Box".to_string(),
            hir::DefKind::Struct,
            Span::dummy(),
        ).expect("BUG: Box builtin registration failed");
        self.resolver.define_type("Box".to_string(), box_def_id, Span::dummy())
            .expect("BUG: Box type registration failed");

        // Box has one type parameter T
        let box_t_var_id = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;

        // Register struct info (opaque struct with no exposed fields)
        self.struct_defs.insert(box_def_id, StructInfo {
            name: "Box".to_string(),
            fields: vec![],  // opaque - no exposed fields
            generics: vec![box_t_var_id],
        });
    }
}
