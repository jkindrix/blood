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
        let _u32_ty = Type::u32();
        let i32_ty = Type::i32();
        let i64_ty = Type::i64();
        let u64_ty = Type::u64();
        let _usize_ty = Type::usize();
        let f32_ty = Type::f32();
        let f64_ty = Type::f64();
        let str_ty = Type::str();  // str slice
        let ref_str_ty = Type::reference(str_ty.clone(), false);  // &str for string literals
        let _string_ty = Type::string();  // owned String (for functions that return owned strings)
        let never_ty = Type::never();

        // === I/O Functions ===

        // print(&str) -> () - convenience function (maps to runtime print_str)
        self.register_builtin_fn_aliased("print", "print_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // println(&str) -> () - convenience function (prints string + newline, maps to runtime println_str)
        self.register_builtin_fn_aliased("println", "println_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // print_int(i32) -> ()
        self.register_builtin_fn("print_int", vec![i32_ty.clone()], unit_ty.clone());

        // println_int(i32) -> ()
        self.register_builtin_fn("println_int", vec![i32_ty.clone()], unit_ty.clone());

        // print_str(&str) -> () - legacy name, same as print
        self.register_builtin_fn("print_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // println_str(&str) -> () - legacy name, same as println
        self.register_builtin_fn("println_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // eprint(&str) -> () - print to stderr (maps to runtime eprint_str)
        self.register_builtin_fn_aliased("eprint", "eprint_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // eprintln(&str) -> () - print to stderr with newline (maps to runtime eprintln_str)
        self.register_builtin_fn_aliased("eprintln", "eprintln_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // eprint_str(&str) -> () - print to stderr
        self.register_builtin_fn("eprint_str", vec![ref_str_ty.clone()], unit_ty.clone());

        // eprintln_str(&str) -> () - print to stderr with newline
        self.register_builtin_fn("eprintln_str", vec![ref_str_ty.clone()], unit_ty.clone());

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
        self.register_builtin_fn("panic", vec![ref_str_ty.clone()], never_ty.clone());

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

        // str_len(&str) -> i64 - get string length in bytes
        self.register_builtin_fn("str_len", vec![ref_str_ty.clone()], i64_ty.clone());

        // str_eq(&str, &str) -> bool - compare two strings for equality
        self.register_builtin_fn("str_eq", vec![ref_str_ty.clone(), ref_str_ty.clone()], bool_ty.clone());

        // str_concat(&str, &str) -> String - concatenate two strings (returns newly allocated string)
        self.register_builtin_fn_aliased("str_concat", "blood_str_concat", vec![ref_str_ty.clone(), ref_str_ty.clone()], Type::string());

        // === Input Functions ===

        // read_line() -> String - read a line from stdin (returns owned string)
        self.register_builtin_fn("read_line", vec![], Type::string());

        // read_int() -> i32 - read an integer from stdin
        self.register_builtin_fn("read_int", vec![], i32_ty.clone());

        // === Conversion Functions ===

        // int_to_string(i32) -> String - convert integer to string
        self.register_builtin_fn("int_to_string", vec![i32_ty.clone()], Type::string());

        // i64_to_string(i64) -> String - convert 64-bit integer to string
        self.register_builtin_fn("i64_to_string", vec![i64_ty.clone()], Type::string());

        // u64_to_string(u64) -> String - convert unsigned 64-bit integer to string
        self.register_builtin_fn("u64_to_string", vec![u64_ty.clone()], Type::string());

        // bool_to_string(bool) -> String - convert boolean to string
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], Type::string());

        // char_to_string(char) -> String - convert character to string
        self.register_builtin_fn("char_to_string", vec![char_ty.clone()], Type::string());

        // f32_to_string(f32) -> String - convert f32 to string
        self.register_builtin_fn("f32_to_string", vec![f32_ty.clone()], Type::string());

        // f64_to_string(f64) -> String - convert f64 to string
        self.register_builtin_fn("f64_to_string", vec![f64_ty.clone()], Type::string());

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

        // === File I/O Functions ===
        // These map to the __builtin_* functions used in std::fs

        // File operations using file descriptors (i64)
        // __builtin_file_open(path: &str, flags: i32, mode: u32) -> Result<i64, Error>
        // Returns fd on success (simplified to i64 for bootstrap)
        self.register_builtin_fn("__builtin_file_open", vec![ref_str_ty.clone(), i32_ty.clone(), Type::u32()], i64_ty.clone());

        // __builtin_file_read(fd: i64, buf: &mut [u8]) -> Result<usize, Error>
        // Returns bytes read (simplified to i64)
        self.register_builtin_fn("__builtin_file_read", vec![i64_ty.clone(), u64_ty.clone(), u64_ty.clone()], i64_ty.clone());

        // __builtin_file_write(fd: i64, buf: &[u8]) -> Result<usize, Error>
        // Returns bytes written (simplified to i64)
        self.register_builtin_fn("__builtin_file_write", vec![i64_ty.clone(), u64_ty.clone(), u64_ty.clone()], i64_ty.clone());

        // __builtin_file_close(fd: i64) -> Result<(), Error>
        // Returns 0 on success, -1 on error
        self.register_builtin_fn("__builtin_file_close", vec![i64_ty.clone()], i32_ty.clone());

        // __builtin_file_flush(fd: i64) -> Result<(), Error>
        self.register_builtin_fn("__builtin_file_flush", vec![i64_ty.clone()], i32_ty.clone());

        // __builtin_file_seek(fd: i64, offset: i64, whence: i32) -> Result<u64, Error>
        // Returns new position
        self.register_builtin_fn("__builtin_file_seek", vec![i64_ty.clone(), i64_ty.clone(), i32_ty.clone()], u64_ty.clone());

        // __builtin_file_sync_all(fd: i64) -> Result<(), Error>
        self.register_builtin_fn("__builtin_file_sync_all", vec![i64_ty.clone()], i32_ty.clone());

        // __builtin_file_sync_data(fd: i64) -> Result<(), Error>
        self.register_builtin_fn("__builtin_file_sync_data", vec![i64_ty.clone()], i32_ty.clone());

        // __builtin_file_set_len(fd: i64, len: u64) -> Result<(), Error>
        self.register_builtin_fn("__builtin_file_set_len", vec![i64_ty.clone(), u64_ty.clone()], i32_ty.clone());

        // Path-based file operations
        // __builtin_remove_file(path: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_remove_file", vec![ref_str_ty.clone()], i32_ty.clone());

        // __builtin_rename(from: &str, to: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_rename", vec![ref_str_ty.clone(), ref_str_ty.clone()], i32_ty.clone());

        // Directory operations
        // __builtin_create_dir(path: &str, mode: u32) -> Result<(), Error>
        self.register_builtin_fn("__builtin_create_dir", vec![ref_str_ty.clone(), Type::u32()], i32_ty.clone());

        // __builtin_create_dir_all(path: &str, mode: u32) -> Result<(), Error>
        self.register_builtin_fn("__builtin_create_dir_all", vec![ref_str_ty.clone(), Type::u32()], i32_ty.clone());

        // __builtin_remove_dir(path: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_remove_dir", vec![ref_str_ty.clone()], i32_ty.clone());

        // __builtin_remove_dir_all(path: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_remove_dir_all", vec![ref_str_ty.clone()], i32_ty.clone());

        // __builtin_read_dir(path: &str) -> Result<DirHandle, Error>
        // Returns directory handle (i64)
        self.register_builtin_fn("__builtin_read_dir", vec![ref_str_ty.clone()], i64_ty.clone());

        // __builtin_close_dir(handle: i64) -> Result<(), Error>
        self.register_builtin_fn("__builtin_close_dir", vec![i64_ty.clone()], i32_ty.clone());

        // __builtin_current_dir() -> Result<String, Error>
        self.register_builtin_fn("__builtin_current_dir", vec![], Type::string());

        // __builtin_set_current_dir(path: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_set_current_dir", vec![ref_str_ty.clone()], i32_ty.clone());

        // Symlinks and hard links
        // __builtin_hard_link(original: &str, link: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_hard_link", vec![ref_str_ty.clone(), ref_str_ty.clone()], i32_ty.clone());

        // __builtin_symlink(original: &str, link: &str) -> Result<(), Error>
        self.register_builtin_fn("__builtin_symlink", vec![ref_str_ty.clone(), ref_str_ty.clone()], i32_ty.clone());

        // __builtin_read_link(path: &str) -> Result<String, Error>
        self.register_builtin_fn("__builtin_read_link", vec![ref_str_ty.clone()], Type::string());

        // __builtin_canonicalize(path: &str) -> Result<String, Error>
        self.register_builtin_fn("__builtin_canonicalize", vec![ref_str_ty.clone()], Type::string());

        // Permissions
        // __builtin_set_permissions(path: &str, mode: u32) -> Result<(), Error>
        self.register_builtin_fn("__builtin_set_permissions", vec![ref_str_ty.clone(), Type::u32()], i32_ty.clone());
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

        // === Parsing Functions (registered after Option is available) ===
        {
            let f64_ty = Type::f64();
            let i64_ty = Type::i64();
            let u8_ty = Type::u8();
            let u32_ty = Type::u32();
            let ref_str_ty = Type::reference(Type::str(), false);

            // Parse string to f64, returns None on parse failure
            let option_f64 = Type::adt(option_def_id, vec![f64_ty.clone()]);
            self.register_builtin_fn("parse_f64", vec![ref_str_ty.clone()], option_f64);

            // Parse string to i64 with given radix, returns None on parse failure
            let option_i64 = Type::adt(option_def_id, vec![i64_ty.clone()]);
            self.register_builtin_fn("parse_i64_radix", vec![ref_str_ty.clone(), u32_ty.clone()], option_i64);

            // Parse string to u8 with given radix, returns None on parse failure
            let option_u8 = Type::adt(option_def_id, vec![u8_ty.clone()]);
            self.register_builtin_fn("parse_u8_radix", vec![ref_str_ty.clone(), u32_ty.clone()], option_u8);

            // Parse string to u32 with given radix, returns None on parse failure
            let option_u32 = Type::adt(option_def_id, vec![u32_ty.clone()]);
            self.register_builtin_fn("parse_u32_radix", vec![ref_str_ty.clone(), u32_ty.clone()], option_u32);

            // Convert u32 code point to char, returns None if invalid
            let char_ty = Type::char();
            let option_char = Type::adt(option_def_id, vec![char_ty.clone()]);
            self.register_builtin_fn("char_from_u32", vec![u32_ty.clone()], option_char);
        }

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

        self.vec_def_id = Some(vec_def_id);

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

    /// Register built-in methods for primitive and builtin types.
    ///
    /// This registers methods like `str.to_string()`, `char.is_whitespace()`,
    /// `String.len()`, `Option.unwrap()`, etc. that are available without
    /// importing the standard library.
    pub(crate) fn register_builtin_methods(&mut self) {
        use super::BuiltinMethodType;

        let _unit_ty = Type::unit();
        let bool_ty = Type::bool();
        let usize_ty = Type::usize();
        let string_ty = Type::string();

        // === str methods ===

        // str.to_string(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "to_string",
            false,  // not static, has self
            vec![Type::reference(Type::str(), false)],  // &self
            string_ty.clone(),
            "str_to_string",
        );

        // &str.to_string(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "to_string",
            false,
            vec![Type::reference(Type::str(), false)],
            string_ty.clone(),
            "str_to_string",
        );

        // str.len(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "len",
            false,
            vec![Type::reference(Type::str(), false)],
            usize_ty.clone(),
            "str_len_usize",
        );

        // &str.len(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "len",
            false,
            vec![Type::reference(Type::str(), false)],
            usize_ty.clone(),
            "str_len_usize",
        );

        // === char methods ===

        // char.is_whitespace(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_whitespace",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_whitespace",
        );

        // char.is_alphabetic(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_alphabetic",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_alphabetic",
        );

        // char.is_alphanumeric(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_alphanumeric",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_alphanumeric",
        );

        // char.is_ascii_digit(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_ascii_digit",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_ascii_digit",
        );

        // char.is_ascii_hexdigit(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_ascii_hexdigit",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_ascii_hexdigit",
        );

        // char.is_ascii_uppercase(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_ascii_uppercase",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_ascii_uppercase",
        );

        // char.is_ascii_lowercase(self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "is_ascii_lowercase",
            false,
            vec![Type::char()],
            bool_ty.clone(),
            "char_is_ascii_lowercase",
        );

        // char.to_string(self) -> String
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "to_string",
            false,
            vec![Type::char()],
            string_ty.clone(),
            "char_to_string",
        );

        // char.len_utf8(self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::Char,
            "len_utf8",
            false,
            vec![Type::char()],
            usize_ty.clone(),
            "char_len_utf8",
        );

        // === String methods ===

        // String.len(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::String,
            "len",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            usize_ty.clone(),
            "string_len",
        );

        // String.is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::String,
            "is_empty",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            bool_ty.clone(),
            "string_is_empty",
        );

        // String::new() -> String (static method)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "new",
            true,  // static method
            vec![],
            string_ty.clone(),
            "string_new",
        );

        // String.push(&mut self, ch: char)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "push",
            false,
            vec![Type::reference(string_ty.clone(), true), Type::char()],
            Type::unit(),
            "string_push",
        );

        // String.push_str(&mut self, s: &str)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "push_str",
            false,
            vec![Type::reference(string_ty.clone(), true), Type::reference(Type::str(), false)],
            Type::unit(),
            "string_push_str",
        );

        // String.as_str(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::String,
            "as_str",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            Type::reference(Type::str(), false),
            "string_as_str",
        );

        // String.clear(&mut self)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "clear",
            false,
            vec![Type::reference(string_ty.clone(), true)],
            Type::unit(),
            "string_clear",
        );

        // String.char_at(&self, index: usize) -> char
        // Access character at byte index (panics on out of bounds)
        // Blood code typically does bounds check before calling this
        self.register_builtin_method(
            BuiltinMethodType::String,
            "char_at",
            false,
            vec![Type::reference(string_ty.clone(), false), usize_ty.clone()],
            Type::char(),
            "string_char_at",
        );

        // String.substring(&self, start: usize, end: usize) -> String
        // Extract a substring from start to end (exclusive)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "substring",
            false,
            vec![Type::reference(string_ty.clone(), false), usize_ty.clone(), usize_ty.clone()],
            string_ty.clone(),
            "string_substring",
        );

        // === Option methods ===
        // Note: Option<T>.unwrap(self) -> T requires generic handling,
        // which is done in find_builtin_method with type substitution.

        // Get the Option and Vec DefIds
        let option_def_id = self.option_def_id
            .expect("BUG: option_def_id not set before register_builtin_methods");
        let vec_def_id = self.vec_def_id
            .expect("BUG: vec_def_id not set before register_builtin_methods");

        // Option<T>.unwrap(self) -> T
        // The actual return type is determined by the type argument
        let t_var_id = TyVarId(9000);  // synthetic placeholder
        let t_ty = Type::new(TypeKind::Param(t_var_id));
        let option_t = Type::adt(option_def_id, vec![t_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "unwrap",
            false,
            vec![option_t.clone()],
            t_ty.clone(),
            "option_unwrap",
        );

        // Option<T>.is_some(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "is_some",
            false,
            vec![Type::reference(option_t.clone(), false)],
            bool_ty.clone(),
            "option_is_some",
        );

        // Option<T>.is_none(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "is_none",
            false,
            vec![Type::reference(option_t.clone(), false)],
            bool_ty.clone(),
            "option_is_none",
        );

        // Option<T>.try_(self) -> T (for ? operator desugaring)
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "try_",
            false,
            vec![option_t],
            t_ty.clone(),
            "option_try",
        );

        // === Vec methods ===
        let vec_t = Type::adt(vec_def_id, vec![t_ty.clone()]);

        // Vec<T>::new() -> Vec<T> (static method)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "new",
            true,
            vec![],
            vec_t.clone(),
            "vec_new",
        );

        // Vec<T>.len(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "len",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            usize_ty.clone(),
            "vec_len",
        );

        // Vec<T>.is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "is_empty",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            bool_ty.clone(),
            "vec_is_empty",
        );

        // Vec<T>.push(&mut self, value: T)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "push",
            false,
            vec![Type::reference(vec_t.clone(), true), t_ty.clone()],
            Type::unit(),
            "vec_push",
        );

        // Vec<T>.get(&self, index: usize) -> Option<&T>
        let option_ref_t = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty, false)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "get",
            false,
            vec![Type::reference(vec_t, false), usize_ty.clone()],
            option_ref_t,
            "vec_get",
        );

        // =========================================================================
        // Array [T; N] methods
        // =========================================================================

        // Array types in Blood are growable (like Vec in semantic terms)
        // [T; N].len(&self) -> usize
        let array_elem_t = self.unifier.fresh_var();
        let array_ty = Type::array(array_elem_t.clone(), 0); // Size 0 as placeholder
        self.register_builtin_method(
            BuiltinMethodType::Array,
            "len",
            false,
            vec![Type::reference(array_ty.clone(), false)],
            usize_ty.clone(),
            "array_len",
        );

        // [T; N].push(&mut self, value: T)
        self.register_builtin_method(
            BuiltinMethodType::Array,
            "push",
            false,
            vec![Type::reference(array_ty.clone(), true), array_elem_t.clone()],
            Type::unit(),
            "array_push",
        );

        // [T; N].is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Array,
            "is_empty",
            false,
            vec![Type::reference(array_ty.clone(), false)],
            bool_ty.clone(),
            "array_is_empty",
        );

        // =========================================================================
        // Slice [T] methods
        // =========================================================================

        let slice_elem_t = self.unifier.fresh_var();
        let slice_ty = Type::slice(slice_elem_t.clone());

        // [T].len(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "len",
            false,
            vec![Type::reference(slice_ty.clone(), false)],
            usize_ty.clone(),
            "slice_len",
        );

        // [T].push(&mut self, value: T)
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "push",
            false,
            vec![Type::reference(slice_ty.clone(), true), slice_elem_t.clone()],
            Type::unit(),
            "slice_push",
        );

        // [T].is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "is_empty",
            false,
            vec![Type::reference(slice_ty.clone(), false)],
            bool_ty.clone(),
            "slice_is_empty",
        );

        // =========================================================================
        // Range<T> methods
        // =========================================================================

        // Range<T>.contains(&self, item: &T) -> bool
        // Note: For simplicity, we use a fresh type variable for T.
        // The actual implementation will be specialized per element type.
        let range_elem_t = self.unifier.fresh_var();
        let range_ty = Type::new(hir::TypeKind::Range {
            element: range_elem_t.clone(),
            inclusive: false,
        });
        self.register_builtin_method(
            BuiltinMethodType::Range,
            "contains",
            false,
            vec![Type::reference(range_ty.clone(), false), Type::reference(range_elem_t.clone(), false)],
            bool_ty.clone(),
            "range_contains",
        );

        // RangeInclusive<T>.contains(&self, item: &T) -> bool
        let range_incl_elem_t = self.unifier.fresh_var();
        let range_incl_ty = Type::new(hir::TypeKind::Range {
            element: range_incl_elem_t.clone(),
            inclusive: true,
        });
        self.register_builtin_method(
            BuiltinMethodType::RangeInclusive,
            "contains",
            false,
            vec![Type::reference(range_incl_ty.clone(), false), Type::reference(range_incl_elem_t.clone(), false)],
            bool_ty.clone(),
            "range_inclusive_contains",
        );
    }

    /// Register a single builtin method.
    fn register_builtin_method(
        &mut self,
        type_match: super::BuiltinMethodType,
        name: &str,
        is_static: bool,
        inputs: Vec<Type>,
        output: Type,
        runtime_name: &str,
    ) {
        // Create a DefId for this method
        let method_name = format!("__builtin_{}_{}",
            match &type_match {
                super::BuiltinMethodType::Str => "str",
                super::BuiltinMethodType::StrRef => "str_ref",
                super::BuiltinMethodType::Char => "char",
                super::BuiltinMethodType::String => "String",
                super::BuiltinMethodType::Option => "Option",
                super::BuiltinMethodType::Vec => "Vec",
                super::BuiltinMethodType::Range => "Range",
                super::BuiltinMethodType::RangeInclusive => "RangeInclusive",
                super::BuiltinMethodType::Slice => "Slice",
                super::BuiltinMethodType::Array => "Array",
            },
            name
        );

        let def_id = self.resolver.define_item(
            method_name,
            hir::DefKind::Fn,
            Span::dummy(),
        ).expect("BUG: builtin method registration failed");

        // Register the function signature
        self.fn_sigs.insert(def_id, hir::FnSig {
            inputs,
            output,
            is_const: false,
            is_async: false,
            is_unsafe: false,
            generics: Vec::new(),
        });

        // Track runtime function name
        self.builtin_fns.insert(def_id, runtime_name.to_string());

        // Add to builtin methods list
        self.builtin_methods.push(super::BuiltinMethodInfo {
            type_match,
            name: name.to_string(),
            def_id,
            is_static,
            runtime_name: runtime_name.to_string(),
        });
    }
}
