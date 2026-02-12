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
        let i8_ty = Type::i8();
        let i16_ty = Type::i16();
        let i32_ty = Type::i32();
        let i64_ty = Type::i64();
        let i128_ty = Type::i128();
        let u8_ty = Type::u8();
        let u16_ty = Type::u16();
        let u32_ty = Type::u32();
        let u64_ty = Type::u64();
        let u128_ty = Type::u128();
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

        // === Region Memory Management ===

        // region_create(initial_size: u64, max_size: u64) -> u64 - create a region, returns region_id
        self.register_builtin_fn_aliased(
            "region_create", "blood_region_create",
            vec![u64_ty.clone(), u64_ty.clone()], u64_ty.clone(),
        );

        // region_destroy(region_id: u64) -> () - destroy a region and free all its memory
        self.register_builtin_fn_aliased(
            "region_destroy", "blood_region_destroy",
            vec![u64_ty.clone()], unit_ty.clone(),
        );

        // region_activate(region_id: u64) -> () - route String/Vec/Box allocations to this region
        self.register_builtin_fn_aliased(
            "region_activate", "blood_region_activate",
            vec![u64_ty.clone()], unit_ty.clone(),
        );

        // region_deactivate() -> () - revert to global allocation
        self.register_builtin_fn_aliased(
            "region_deactivate", "blood_region_deactivate",
            vec![], unit_ty.clone(),
        );

        // region_alloc(region_id: u64, size: u64, align: u64) -> u64 - allocate from a region
        self.register_builtin_fn_aliased(
            "region_alloc", "blood_region_alloc",
            vec![u64_ty.clone(), u64_ty.clone(), u64_ty.clone()], u64_ty.clone(),
        );

        // region_exit_scope(region_id: u64) -> i32 - exit a region's lexical scope
        self.register_builtin_fn_aliased(
            "region_exit_scope", "blood_region_exit_scope",
            vec![u64_ty.clone()], i32_ty.clone(),
        );

        // region_used(region_id: u64) -> u64 - get current used bytes in region
        self.register_builtin_fn_aliased(
            "region_used", "blood_region_used",
            vec![u64_ty.clone()], u64_ty.clone(),
        );

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

        // str_concat(&str, &str) -> &str - concatenate two strings (returns newly allocated BloodStr)
        self.register_builtin_fn_aliased("str_concat", "blood_str_concat", vec![ref_str_ty.clone(), ref_str_ty.clone()], ref_str_ty.clone());

        // === Input Functions ===

        // read_line() -> String - read a line from stdin (returns owned string)
        self.register_builtin_fn("read_line", vec![], Type::string());

        // read_int() -> i32 - read an integer from stdin
        self.register_builtin_fn("read_int", vec![], i32_ty.clone());

        // === Conversion Functions ===
        // Note: These return &str (BloodStr = {ptr, len}) in the runtime,
        // not owned String (which would be {ptr, len, capacity}).

        // Signed integer to string conversions
        // i8_to_string(i8) -> &str - convert 8-bit integer to string
        self.register_builtin_fn("i8_to_string", vec![i8_ty.clone()], ref_str_ty.clone());

        // i16_to_string(i16) -> &str - convert 16-bit integer to string
        self.register_builtin_fn("i16_to_string", vec![i16_ty.clone()], ref_str_ty.clone());

        // int_to_string(i32) -> &str - convert integer to string
        self.register_builtin_fn("int_to_string", vec![i32_ty.clone()], ref_str_ty.clone());

        // i64_to_string(i64) -> &str - convert 64-bit integer to string
        self.register_builtin_fn("i64_to_string", vec![i64_ty.clone()], ref_str_ty.clone());

        // i128_to_string(i128) -> &str - convert 128-bit integer to string
        self.register_builtin_fn("i128_to_string", vec![i128_ty.clone()], ref_str_ty.clone());

        // Unsigned integer to string conversions
        // u8_to_string(u8) -> &str - convert unsigned 8-bit integer to string
        self.register_builtin_fn("u8_to_string", vec![u8_ty.clone()], ref_str_ty.clone());

        // u16_to_string(u16) -> &str - convert unsigned 16-bit integer to string
        self.register_builtin_fn("u16_to_string", vec![u16_ty.clone()], ref_str_ty.clone());

        // u32_to_string(u32) -> &str - convert unsigned 32-bit integer to string
        self.register_builtin_fn("u32_to_string", vec![u32_ty.clone()], ref_str_ty.clone());

        // u64_to_string(u64) -> &str - convert unsigned 64-bit integer to string
        self.register_builtin_fn("u64_to_string", vec![u64_ty.clone()], ref_str_ty.clone());

        // u128_to_string(u128) -> &str - convert unsigned 128-bit integer to string
        self.register_builtin_fn("u128_to_string", vec![u128_ty.clone()], ref_str_ty.clone());

        // bool_to_string(bool) -> &str - convert boolean to string
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], ref_str_ty.clone());

        // char_to_string(char) -> &str - convert character to string
        self.register_builtin_fn("char_to_string", vec![char_ty.clone()], ref_str_ty.clone());

        // f32_to_string(f32) -> &str - convert f32 to string
        self.register_builtin_fn("f32_to_string", vec![f32_ty.clone()], ref_str_ty.clone());

        // f64_to_string(f64) -> &str - convert f64 to string
        self.register_builtin_fn("f64_to_string", vec![f64_ty.clone()], ref_str_ty.clone());

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

        // file_open(&str, &str) -> i64 - open a file, returns fd or -1 on error
        // Mode: "r" (read), "w" (write), "a" (append), "rw" (read/write), "rw+" (read/write/create)
        self.register_builtin_fn("file_open", vec![ref_str_ty.clone(), ref_str_ty.clone()], i64_ty.clone());

        // file_read(fd: i64, buf: u64, count: u64) -> i64 - read from fd, returns bytes read or -1
        self.register_builtin_fn("file_read", vec![i64_ty.clone(), u64_ty.clone(), u64_ty.clone()], i64_ty.clone());

        // file_write(fd: i64, buf: u64, count: u64) -> i64 - write to fd, returns bytes written or -1
        self.register_builtin_fn("file_write", vec![i64_ty.clone(), u64_ty.clone(), u64_ty.clone()], i64_ty.clone());

        // file_close(fd: i64) -> i32 - close fd, returns 0 on success or -1 on error
        self.register_builtin_fn("file_close", vec![i64_ty.clone()], i32_ty.clone());

        // file_read_to_string(&str) -> &str - read entire file as string
        self.register_builtin_fn("file_read_to_string", vec![ref_str_ty.clone()], ref_str_ty.clone());

        // file_write_string(&str, &str) -> bool - write string to file (creates/truncates)
        self.register_builtin_fn("file_write_string", vec![ref_str_ty.clone(), ref_str_ty.clone()], bool_ty.clone());

        // file_append_string(&str, &str) -> bool - append string to file (creates if needed)
        self.register_builtin_fn("file_append_string", vec![ref_str_ty.clone(), ref_str_ty.clone()], bool_ty.clone());

        // file_exists(&str) -> bool - check if file exists
        self.register_builtin_fn("file_exists", vec![ref_str_ty.clone()], bool_ty.clone());

        // file_delete(&str) -> bool - delete a file
        self.register_builtin_fn("file_delete", vec![ref_str_ty.clone()], bool_ty.clone());

        // file_size(&str) -> i64 - get file size in bytes, returns -1 on error
        self.register_builtin_fn("file_size", vec![ref_str_ty.clone()], i64_ty.clone());

        // system(&str) -> i32 - execute a shell command and return exit code
        self.register_builtin_fn("system", vec![ref_str_ty.clone()], i32_ty.clone());

        // === Command-Line Argument Functions ===

        // args_count() -> i32 - get number of command-line arguments
        self.register_builtin_fn("args_count", vec![], i32_ty.clone());

        // args_get(i32) -> &str - get argument at index (returns empty string if out of bounds)
        self.register_builtin_fn("args_get", vec![i32_ty.clone()], ref_str_ty.clone());

        // args_join() -> &str - get all arguments as a single space-separated string
        self.register_builtin_fn("args_join", vec![], ref_str_ty.clone());

        // === OS Thread Primitives ===

        // thread_spawn(fn_ptr: u64, arg: u64) -> u64 - spawn OS thread, returns handle
        self.register_builtin_fn_aliased(
            "thread_spawn", "blood_thread_spawn",
            vec![u64_ty.clone(), u64_ty.clone()], u64_ty.clone(),
        );

        // thread_join(handle: u64) -> u64 - join thread, returns 0 on success
        self.register_builtin_fn_aliased(
            "thread_join", "blood_thread_join",
            vec![u64_ty.clone()], u64_ty.clone(),
        );
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
            const_generics: Vec::new(),
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

            // Parse string to f64, returns 0.0 on parse failure
            self.register_builtin_fn("parse_f64", vec![ref_str_ty.clone()], f64_ty.clone());

            // Parse string to i64 with given radix, returns 0 on parse failure
            self.register_builtin_fn("parse_i64_radix", vec![ref_str_ty.clone(), u32_ty.clone()], i64_ty.clone());

            // Parse string to u8 with given radix, returns None on parse failure
            let option_u8 = Type::adt(option_def_id, vec![u8_ty.clone()]);
            self.register_builtin_fn("parse_u8_radix", vec![ref_str_ty.clone(), u32_ty.clone()], option_u8);

            // Parse string to u32 with given radix, returns None on parse failure
            let option_u32 = Type::adt(option_def_id, vec![u32_ty.clone()]);
            self.register_builtin_fn("parse_u32_radix", vec![ref_str_ty.clone(), u32_ty.clone()], option_u32);

            // Convert u32 code point to char (returns U+FFFD for invalid code points)
            let char_ty = Type::char();
            self.register_builtin_fn("char_from_u32", vec![u32_ty.clone()], char_ty.clone());
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

        self.box_def_id = Some(box_def_id);

        // Register Iter<T> as a built-in struct (iterator over T)
        let iter_def_id = self.resolver.define_item(
            "Iter".to_string(),
            hir::DefKind::Struct,
            Span::dummy(),
        ).expect("BUG: Iter builtin registration failed");
        self.resolver.define_type("Iter".to_string(), iter_def_id, Span::dummy())
            .expect("BUG: Iter type registration failed");

        // Iter has one type parameter T (the item type)
        let iter_t_var_id = TyVarId(self.next_type_param_id);
        self.next_type_param_id += 1;

        // Register struct info (opaque struct with no exposed fields)
        self.struct_defs.insert(iter_def_id, StructInfo {
            name: "Iter".to_string(),
            fields: vec![],  // opaque - no exposed fields
            generics: vec![iter_t_var_id],
        });

        self.iter_def_id = Some(iter_def_id);
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

        // str.char_at(&self, index: usize) -> Option<char>
        let option_char = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::char()],
        );
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "char_at",
            false,
            vec![Type::reference(Type::str(), false), usize_ty.clone()],
            option_char.clone(),
            "str_char_at",
        );

        // &str.char_at(&self, index: usize) -> Option<char>
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "char_at",
            false,
            vec![Type::reference(Type::str(), false), usize_ty.clone()],
            option_char.clone(),
            "str_char_at",
        );

        // str.char_at_index(&self, index: usize) -> Option<char>
        // Character-based (not byte-based) indexing
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "char_at_index",
            false,
            vec![Type::reference(Type::str(), false), usize_ty.clone()],
            option_char.clone(),
            "str_char_at_index",
        );

        // &str.char_at_index(&self, index: usize) -> Option<char>
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "char_at_index",
            false,
            vec![Type::reference(Type::str(), false), usize_ty.clone()],
            option_char.clone(),
            "str_char_at_index",
        );

        // str.len_chars(&self) -> usize - count of UTF-8 characters (not bytes)
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "len_chars",
            false,
            vec![Type::reference(Type::str(), false)],
            usize_ty.clone(),
            "str_len_chars",
        );

        // &str.len_chars(&self) -> usize - count of UTF-8 characters (not bytes)
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "len_chars",
            false,
            vec![Type::reference(Type::str(), false)],
            usize_ty.clone(),
            "str_len_chars",
        );

        // str.as_bytes(&self) -> &[u8]
        let byte_slice_ty = Type::reference(Type::slice(Type::u8()), false);
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "as_bytes",
            false,
            vec![Type::reference(Type::str(), false)],
            byte_slice_ty.clone(),
            "str_as_bytes",
        );

        // &str.as_bytes(&self) -> &[u8]
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "as_bytes",
            false,
            vec![Type::reference(Type::str(), false)],
            byte_slice_ty.clone(),
            "str_as_bytes",
        );

        // str.chars(&self) -> Vec<char>
        // Returns characters as a Vec (for iteration)
        let vec_def_id = self.vec_def_id.expect("BUG: vec_def_id not set");
        let vec_char_ty = Type::adt(vec_def_id, vec![Type::char()]);
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "chars",
            false,
            vec![Type::reference(Type::str(), false)],
            vec_char_ty.clone(),
            "str_chars",
        );

        // &str.chars(&self) -> Vec<char>
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "chars",
            false,
            vec![Type::reference(Type::str(), false)],
            vec_char_ty,
            "str_chars",
        );

        // str.to_uppercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "to_uppercase",
            false,
            vec![Type::reference(Type::str(), false)],
            string_ty.clone(),
            "str_to_uppercase",
        );

        // &str.to_uppercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "to_uppercase",
            false,
            vec![Type::reference(Type::str(), false)],
            string_ty.clone(),
            "str_to_uppercase",
        );

        // str.to_lowercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "to_lowercase",
            false,
            vec![Type::reference(Type::str(), false)],
            string_ty.clone(),
            "str_to_lowercase",
        );

        // &str.to_lowercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "to_lowercase",
            false,
            vec![Type::reference(Type::str(), false)],
            string_ty.clone(),
            "str_to_lowercase",
        );

        // str.replace(&self, from: &str, to: &str) -> String
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "replace",
            false,
            vec![
                Type::reference(Type::str(), false),
                Type::reference(Type::str(), false),
                Type::reference(Type::str(), false),
            ],
            string_ty.clone(),
            "str_replace",
        );

        // &str.replace(&self, from: &str, to: &str) -> String
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "replace",
            false,
            vec![
                Type::reference(Type::str(), false),
                Type::reference(Type::str(), false),
                Type::reference(Type::str(), false),
            ],
            string_ty.clone(),
            "str_replace",
        );

        // str.is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "is_empty",
            false,
            vec![Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_is_empty",
        );

        // &str.is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "is_empty",
            false,
            vec![Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_is_empty",
        );

        // str.contains(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "contains",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_contains",
        );

        // &str.contains(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "contains",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_contains",
        );

        // str.starts_with(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "starts_with",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_starts_with",
        );

        // &str.starts_with(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "starts_with",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_starts_with",
        );

        // str.ends_with(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "ends_with",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_ends_with",
        );

        // &str.ends_with(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "ends_with",
            false,
            vec![Type::reference(Type::str(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "str_ends_with",
        );

        // str.trim(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::Str,
            "trim",
            false,
            vec![Type::reference(Type::str(), false)],
            Type::reference(Type::str(), false),
            "str_trim",
        );

        // &str.trim(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::StrRef,
            "trim",
            false,
            vec![Type::reference(Type::str(), false)],
            Type::reference(Type::str(), false),
            "str_trim",
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
            "char_to_string_owned",
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

        // String.char_at(&self, index: usize) -> Option<char>
        // Helper for accessing character at byte index (alternative to chars().nth())
        let option_char = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::char()],
        );
        self.register_builtin_method(
            BuiltinMethodType::String,
            "char_at",
            false,
            vec![Type::reference(string_ty.clone(), false), usize_ty.clone()],
            option_char,
            "string_char_at",
        );

        // String.len_chars(&self) -> usize - count of UTF-8 characters (not bytes)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "len_chars",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            usize_ty.clone(),
            "string_len_chars",
        );

        // String.as_bytes(&self) -> &[u8]
        self.register_builtin_method(
            BuiltinMethodType::String,
            "as_bytes",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            byte_slice_ty,  // reuse from str method above
            "string_as_bytes",
        );

        // String.contains(&self, pattern: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::String,
            "contains",
            false,
            vec![Type::reference(string_ty.clone(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "string_contains",
        );

        // String.starts_with(&self, prefix: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::String,
            "starts_with",
            false,
            vec![Type::reference(string_ty.clone(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "string_starts_with",
        );

        // String.ends_with(&self, suffix: &str) -> bool
        self.register_builtin_method(
            BuiltinMethodType::String,
            "ends_with",
            false,
            vec![Type::reference(string_ty.clone(), false), Type::reference(Type::str(), false)],
            bool_ty.clone(),
            "string_ends_with",
        );

        // String.find(&self, pattern: &str) -> Option<usize>
        let option_usize = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![usize_ty.clone()],
        );
        self.register_builtin_method(
            BuiltinMethodType::String,
            "find",
            false,
            vec![Type::reference(string_ty.clone(), false), Type::reference(Type::str(), false)],
            option_usize.clone(),
            "string_find",
        );

        // String.rfind(&self, pattern: &str) -> Option<usize>
        self.register_builtin_method(
            BuiltinMethodType::String,
            "rfind",
            false,
            vec![Type::reference(string_ty.clone(), false), Type::reference(Type::str(), false)],
            option_usize,
            "string_rfind",
        );

        // String.trim(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::String,
            "trim",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            Type::reference(Type::str(), false),
            "string_trim",
        );

        // String.trim_start(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::String,
            "trim_start",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            Type::reference(Type::str(), false),
            "string_trim_start",
        );

        // String.trim_end(&self) -> &str
        self.register_builtin_method(
            BuiltinMethodType::String,
            "trim_end",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            Type::reference(Type::str(), false),
            "string_trim_end",
        );

        // String.to_uppercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::String,
            "to_uppercase",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            string_ty.clone(),
            "string_to_uppercase",
        );

        // String.to_lowercase(&self) -> String
        self.register_builtin_method(
            BuiltinMethodType::String,
            "to_lowercase",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            string_ty.clone(),
            "string_to_lowercase",
        );

        // String.replace(&self, from: &str, to: &str) -> String
        self.register_builtin_method(
            BuiltinMethodType::String,
            "replace",
            false,
            vec![
                Type::reference(string_ty.clone(), false),
                Type::reference(Type::str(), false),
                Type::reference(Type::str(), false),
            ],
            string_ty.clone(),
            "string_replace",
        );

        // String.split(&self, pat: &str) -> Vec<String>
        // Splits string by pattern delimiter
        // Note: Returns Vec until Iterator trait is implemented
        let vec_def_id_early = self.vec_def_id.expect("vec_def_id must be set");
        let vec_string = Type::adt(vec_def_id_early, vec![string_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "split",
            false,
            vec![
                Type::reference(string_ty.clone(), false),
                Type::reference(Type::str(), false),
            ],
            vec_string.clone(),
            "string_split",
        );

        // String.split_whitespace(&self) -> Vec<String>
        // Splits string by whitespace
        self.register_builtin_method(
            BuiltinMethodType::String,
            "split_whitespace",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            vec_string.clone(),
            "string_split_whitespace",
        );

        // String.lines(&self) -> Vec<String>
        // Splits string by line endings
        self.register_builtin_method(
            BuiltinMethodType::String,
            "lines",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            vec_string.clone(),
            "string_lines",
        );

        // String.chars(&self) -> Vec<char>
        // Returns characters as a Vec
        let vec_char = Type::adt(vec_def_id_early, vec![Type::char()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "chars",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            vec_char,
            "string_chars",
        );

        // String.substring(&self, start: usize, end: usize) -> String
        // Extracts a substring from byte indices [start, end)
        self.register_builtin_method(
            BuiltinMethodType::String,
            "substring",
            false,
            vec![
                Type::reference(string_ty.clone(), false),
                usize_ty.clone(),
                usize_ty.clone(),
            ],
            string_ty.clone(),
            "string_substring",
        );

        // String.bytes(&self) -> Vec<u8>
        // Returns bytes as a Vec
        let vec_u8 = Type::adt(vec_def_id_early, vec![Type::u8()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "bytes",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            vec_u8,
            "string_bytes",
        );

        // String.parse<T>(&self) -> Result<T, ParseError>
        // Note: For now, we register specific parse methods for common types
        // Full generic parse requires FromStr trait

        // String.parse_i32(&self) -> Result<i32, String>
        // Returns Result with error message on failure
        let result_def_id_early = self.result_def_id.expect("result_def_id must be set");
        let parse_i32_result = Type::adt(result_def_id_early, vec![Type::i32(), string_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "parse_i32",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            parse_i32_result,
            "string_parse_i32",
        );

        // String.parse_i64(&self) -> Result<i64, String>
        let parse_i64_result = Type::adt(result_def_id_early, vec![Type::i64(), string_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "parse_i64",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            parse_i64_result,
            "string_parse_i64",
        );

        // String.parse_f64(&self) -> Result<f64, String>
        let parse_f64_result = Type::adt(result_def_id_early, vec![Type::f64(), string_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "parse_f64",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            parse_f64_result,
            "string_parse_f64",
        );

        // String.parse_bool(&self) -> Result<bool, String>
        let parse_bool_result = Type::adt(result_def_id_early, vec![Type::bool(), string_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::String,
            "parse_bool",
            false,
            vec![Type::reference(string_ty.clone(), false)],
            parse_bool_result,
            "string_parse_bool",
        );

        // === Option methods ===
        // Note: Option<T>.unwrap(self) -> T requires generic handling,
        // which is done in find_builtin_method with type substitution.
        // We register a placeholder here for method discovery.

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
        // This unwraps the Option, propagating None via early return
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "try_",
            false,
            vec![option_t.clone()],
            t_ty.clone(),
            "option_try",
        );

        // Option<T>.expect(self, msg: &str) -> T
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "expect",
            false,
            vec![option_t.clone(), Type::reference(Type::str(), false)],
            t_ty.clone(),
            "option_expect",
        );

        // Option<T>.unwrap_or(self, default: T) -> T
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "unwrap_or",
            false,
            vec![option_t.clone(), t_ty.clone()],
            t_ty.clone(),
            "option_unwrap_or",
        );

        // Option<T>.ok_or(self, err: E) -> Result<T, E>
        // Note: E is a fresh type parameter
        let e_var_id = TyVarId(9002);  // synthetic placeholder for E
        let e_ty_opt = Type::new(TypeKind::Param(e_var_id));
        let result_te_opt = Type::adt(
            self.result_def_id.expect("BUG: result_def_id not set"),
            vec![t_ty.clone(), e_ty_opt.clone()]
        );
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "ok_or",
            false,
            vec![option_t.clone(), e_ty_opt.clone()],
            result_te_opt.clone(),
            "option_ok_or",
        );

        // Option<T>.and(self, other: Option<U>) -> Option<U>
        // Note: U is a fresh type parameter that must be inferred from the argument
        let u_var_id = TyVarId(9003);  // synthetic placeholder for U
        let u_ty = Type::new(TypeKind::Param(u_var_id));
        let option_u = Type::adt(option_def_id, vec![u_ty.clone()]);
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "and",
            false,
            vec![option_t.clone(), option_u.clone()],
            option_u.clone(),
            "option_and",
            vec![u_var_id],  // U is a method-level type parameter
        );

        // Option<T>.or(self, other: Option<T>) -> Option<T>
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "or",
            false,
            vec![option_t.clone(), option_t.clone()],
            option_t.clone(),
            "option_or",
        );

        // Option<T>.xor(self, other: Option<T>) -> Option<T>
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "xor",
            false,
            vec![option_t.clone(), option_t.clone()],
            option_t.clone(),
            "option_xor",
        );

        // Option<T>.as_ref(&self) -> Option<&T>
        let option_ref_t_opt = Type::adt(option_def_id, vec![Type::reference(t_ty.clone(), false)]);
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "as_ref",
            false,
            vec![Type::reference(option_t.clone(), false)],
            option_ref_t_opt,
            "option_as_ref",
        );

        // Option<T>.as_mut(&mut self) -> Option<&mut T>
        let option_ref_mut_t = Type::adt(option_def_id, vec![Type::reference(t_ty.clone(), true)]);
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "as_mut",
            false,
            vec![Type::reference(option_t.clone(), true)],
            option_ref_mut_t,
            "option_as_mut",
        );

        // Option<T>.take(&mut self) -> Option<T>
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "take",
            false,
            vec![Type::reference(option_t.clone(), true)],
            option_t.clone(),
            "option_take",
        );

        // Option<T>.replace(&mut self, value: T) -> Option<T>
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "replace",
            false,
            vec![Type::reference(option_t.clone(), true), t_ty.clone()],
            option_t.clone(),
            "option_replace",
        );

        // === Option closure-accepting methods ===

        // Option<T>.map<U>(self, f: fn(T) -> U) -> Option<U>
        // Applies f to the contained value if Some, otherwise returns None
        let fn_t_to_u = Type::function(vec![t_ty.clone()], u_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "map",
            false,
            vec![option_t.clone(), fn_t_to_u],
            option_u.clone(),
            "option_map",
            vec![u_var_id],
        );

        // Option<T>.and_then<U>(self, f: fn(T) -> Option<U>) -> Option<U>
        // Returns None if Option is None, otherwise calls f with the wrapped value
        let fn_t_to_option_u = Type::function(vec![t_ty.clone()], option_u.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "and_then",
            false,
            vec![option_t.clone(), fn_t_to_option_u],
            option_u.clone(),
            "option_and_then",
            vec![u_var_id],
        );

        // Option<T>.filter(self, predicate: fn(&T) -> bool) -> Option<T>
        // Returns None if Option is None, otherwise calls predicate with the wrapped value
        let fn_ref_t_to_bool = Type::function(vec![Type::reference(t_ty.clone(), false)], bool_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "filter",
            false,
            vec![option_t.clone(), fn_ref_t_to_bool],
            option_t.clone(),
            "option_filter",
        );

        // Option<T>.map_or<U>(self, default: U, f: fn(T) -> U) -> U
        // Applies f to the contained value if Some, otherwise returns default
        let fn_t_to_u_map_or = Type::function(vec![t_ty.clone()], u_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "map_or",
            false,
            vec![option_t.clone(), u_ty.clone(), fn_t_to_u_map_or],
            u_ty.clone(),
            "option_map_or",
            vec![u_var_id],
        );

        // Option<T>.map_or_else<U>(self, default: fn() -> U, f: fn(T) -> U) -> U
        // Applies f to the contained value if Some, otherwise computes default
        let fn_void_to_u = Type::function(vec![], u_ty.clone());
        let fn_t_to_u_map_or_else = Type::function(vec![t_ty.clone()], u_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "map_or_else",
            false,
            vec![option_t.clone(), fn_void_to_u.clone(), fn_t_to_u_map_or_else],
            u_ty.clone(),
            "option_map_or_else",
            vec![u_var_id],
        );

        // Option<T>.ok_or_else<E>(self, err: fn() -> E) -> Result<T, E>
        // Transforms the Option<T> into a Result<T, E>, using err() to provide Err value
        let fn_void_to_e = Type::function(vec![], e_ty_opt.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Option,
            "ok_or_else",
            false,
            vec![option_t.clone(), fn_void_to_e],
            result_te_opt.clone(),
            "option_ok_or_else",
            vec![e_var_id],
        );

        // Option<T>.or_else(self, f: fn() -> Option<T>) -> Option<T>
        // Returns self if Some, otherwise calls f and returns result
        let fn_void_to_option_t = Type::function(vec![], option_t.clone());
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "or_else",
            false,
            vec![option_t.clone(), fn_void_to_option_t],
            option_t.clone(),
            "option_or_else",
        );

        // Option<T>.unwrap_or_else(self, f: fn() -> T) -> T
        // Returns the contained value or computes it from f
        let fn_void_to_t = Type::function(vec![], t_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "unwrap_or_else",
            false,
            vec![option_t.clone(), fn_void_to_t],
            t_ty.clone(),
            "option_unwrap_or_else",
        );

        // Option<T>.unwrap_or_default(self) -> T where T: Default
        // Returns the contained value or the default for T
        self.register_builtin_method(
            BuiltinMethodType::Option,
            "unwrap_or_default",
            false,
            vec![option_t.clone()],
            t_ty.clone(),
            "option_unwrap_or_default",
        );

        // === Vec methods ===

        // Vec<T>::new() -> Vec<T> (static method)
        let vec_t = Type::adt(vec_def_id, vec![t_ty.clone()]);
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
            vec![Type::reference(t_ty.clone(), false)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "get",
            false,
            vec![Type::reference(vec_t.clone(), false), usize_ty.clone()],
            option_ref_t,
            "vec_get",
        );

        // Vec<T>.pop(&mut self) -> Option<T>
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "pop",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            option_t.clone(),
            "vec_pop",
        );

        // Vec<T>.capacity(&self) -> usize
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "capacity",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            usize_ty.clone(),
            "vec_capacity",
        );

        // Vec<T>.clear(&mut self)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "clear",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_clear",
        );

        // Vec<T>.first(&self) -> Option<&T>
        let option_ref_t_first = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty.clone(), false)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "first",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            option_ref_t_first,
            "vec_first",
        );

        // Vec<T>.last(&self) -> Option<&T>
        let option_ref_t_last = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty.clone(), false)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "last",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            option_ref_t_last,
            "vec_last",
        );

        // Vec<T>.reverse(&mut self)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "reverse",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_reverse",
        );

        // Vec<T>.contains(&self, elem: &T) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "contains",
            false,
            vec![Type::reference(vec_t.clone(), false), Type::reference(t_ty.clone(), false)],
            bool_ty.clone(),
            "vec_contains",
        );

        // Vec<T>::with_capacity(capacity: usize) -> Vec<T> (static method)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "with_capacity",
            true,
            vec![usize_ty.clone()],
            vec_t.clone(),
            "vec_with_capacity",
        );

        // Vec<T>.insert(&mut self, index: usize, element: T)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "insert",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone(), t_ty.clone()],
            Type::unit(),
            "vec_insert",
        );

        // Vec<T>.remove(&mut self, index: usize) -> T
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "remove",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone()],
            t_ty.clone(),
            "vec_remove",
        );

        // Vec<T>.swap_remove(&mut self, index: usize) -> T
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "swap_remove",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone()],
            t_ty.clone(),
            "vec_swap_remove",
        );

        // Vec<T>.truncate(&mut self, len: usize)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "truncate",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone()],
            Type::unit(),
            "vec_truncate",
        );

        // Vec<T>.first_mut(&mut self) -> Option<&mut T>
        let option_ref_mut_t = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty.clone(), true)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "first_mut",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            option_ref_mut_t.clone(),
            "vec_first_mut",
        );

        // Vec<T>.last_mut(&mut self) -> Option<&mut T>
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "last_mut",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            option_ref_mut_t.clone(),
            "vec_last_mut",
        );

        // Vec<T>.get_mut(&mut self, index: usize) -> Option<&mut T>
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "get_mut",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone()],
            option_ref_mut_t,
            "vec_get_mut",
        );

        // Vec<T>.reserve(&mut self, additional: usize)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "reserve",
            false,
            vec![Type::reference(vec_t.clone(), true), usize_ty.clone()],
            Type::unit(),
            "vec_reserve",
        );

        // Vec<T>.shrink_to_fit(&mut self)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "shrink_to_fit",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_shrink_to_fit",
        );

        // Vec<T>.as_slice(&self) -> &[T]
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "as_slice",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            Type::reference(Type::slice(t_ty.clone()), false),
            "vec_as_slice",
        );

        // Vec<T>.as_mut_slice(&mut self) -> &mut [T]
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "as_mut_slice",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::reference(Type::slice(t_ty.clone()), true),
            "vec_as_mut_slice",
        );

        // Vec<T>.append(&mut self, other: &mut Vec<T>)
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "append",
            false,
            vec![Type::reference(vec_t.clone(), true), Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_append",
        );

        // Vec<T>.extend_from_slice(&mut self, other: &[T])
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "extend_from_slice",
            false,
            vec![Type::reference(vec_t.clone(), true), Type::reference(Type::slice(t_ty.clone()), false)],
            Type::unit(),
            "vec_extend_from_slice",
        );

        // Vec<T>.dedup(&mut self) - removes consecutive duplicates
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "dedup",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_dedup",
        );

        // Vec<T>.retain(&mut self, f: F) where F: FnMut(&T) -> bool
        // Retains only elements for which the predicate returns true
        let retain_pred_ty = Type::function(vec![Type::reference(t_ty.clone(), false)], Type::bool());
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "retain",
            false,
            vec![Type::reference(vec_t.clone(), true), retain_pred_ty],
            Type::unit(),
            "vec_retain",
        );

        // Vec<T>.sort(&mut self) where T: Ord
        // Sorts the vector in ascending order
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "sort",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            Type::unit(),
            "vec_sort",
        );

        // Vec<T>.sort_by(&mut self, compare: F) where F: FnMut(&T, &T) -> Ordering
        // Sorts using a custom comparator
        // Note: Ordering is represented as i32 (-1, 0, 1) at runtime
        let sort_cmp_ty = Type::function(
            vec![Type::reference(t_ty.clone(), false), Type::reference(t_ty.clone(), false)],
            Type::i32(),  // Ordering encoded as i32
        );
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "sort_by",
            false,
            vec![Type::reference(vec_t.clone(), true), sort_cmp_ty],
            Type::unit(),
            "vec_sort_by",
        );

        // Vec<T>.binary_search(&self, x: &T) -> Result<usize, usize> where T: Ord
        // Binary search for an element, returns Ok(index) if found or Err(insert_position)
        let result_def_id = self.result_def_id.expect("result_def_id must be set");
        let binary_search_result = Type::adt(result_def_id, vec![usize_ty.clone(), usize_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "binary_search",
            false,
            vec![Type::reference(vec_t.clone(), false), Type::reference(t_ty.clone(), false)],
            binary_search_result,
            "vec_binary_search",
        );

        // Vec<T>.iter(&self) -> Iter<T>
        // Returns an iterator over the vector's elements
        let iter_def_id = self.iter_def_id.expect("iter_def_id must be set");
        let iter_t = Type::adt(iter_def_id, vec![t_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "iter",
            false,
            vec![Type::reference(vec_t.clone(), false)],
            iter_t.clone(),
            "vec_iter",
        );

        // Vec<T>.iter_mut(&mut self) -> Iter<&mut T>
        // Returns a mutable iterator over the vector's elements
        let iter_mut_t = Type::adt(iter_def_id, vec![Type::reference(t_ty.clone(), true)]);
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "iter_mut",
            false,
            vec![Type::reference(vec_t.clone(), true)],
            iter_mut_t,
            "vec_iter_mut",
        );

        // Vec<T>.into_iter(self) -> Iter<T>
        // Consumes the vector and returns an iterator that owns the values
        self.register_builtin_method(
            BuiltinMethodType::Vec,
            "into_iter",
            false,
            vec![vec_t.clone()],
            iter_t.clone(),
            "vec_into_iter",
        );

        // === Iterator methods ===

        // Iter<T>.next(&mut self) -> Option<T>
        // Returns the next element or None if the iterator is exhausted
        let option_def_id = self.option_def_id.expect("option_def_id must be set");
        let option_t = Type::adt(option_def_id, vec![t_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "next",
            false,
            vec![Type::reference(iter_t.clone(), true)],
            option_t.clone(),
            "iter_next",
        );

        // Iter<T>.count(self) -> usize
        // Consumes the iterator and counts the number of elements
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "count",
            false,
            vec![iter_t.clone()],
            usize_ty.clone(),
            "iter_count",
        );

        // Iter<T>.last(self) -> Option<T>
        // Consumes the iterator and returns the last element
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "last",
            false,
            vec![iter_t.clone()],
            option_t.clone(),
            "iter_last",
        );

        // Iter<T>.nth(&mut self, n: usize) -> Option<T>
        // Returns the nth element of the iterator
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "nth",
            false,
            vec![Type::reference(iter_t.clone(), true), usize_ty.clone()],
            option_t.clone(),
            "iter_nth",
        );

        // Iter<T>.take(self, n: usize) -> Iter<T>
        // Creates an iterator that yields the first n elements
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "take",
            false,
            vec![iter_t.clone(), usize_ty.clone()],
            iter_t.clone(),
            "iter_take",
        );

        // Iter<T>.skip(self, n: usize) -> Iter<T>
        // Creates an iterator that skips the first n elements
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "skip",
            false,
            vec![iter_t.clone(), usize_ty.clone()],
            iter_t.clone(),
            "iter_skip",
        );

        // Iter<T>.collect(self) -> Vec<T>
        // Collects all elements into a vector
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "collect",
            false,
            vec![iter_t.clone()],
            vec_t.clone(),
            "iter_collect",
        );

        // Iter<T>.any<F>(self, f: F) -> bool where F: FnMut(T) -> bool
        // Returns true if any element satisfies the predicate
        let any_pred_ty = Type::function(vec![t_ty.clone()], bool_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "any",
            false,
            vec![iter_t.clone(), any_pred_ty.clone()],
            bool_ty.clone(),
            "iter_any",
        );

        // Iter<T>.all<F>(self, f: F) -> bool where F: FnMut(T) -> bool
        // Returns true if all elements satisfy the predicate
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "all",
            false,
            vec![iter_t.clone(), any_pred_ty.clone()],
            bool_ty.clone(),
            "iter_all",
        );

        // Iter<T>.find<F>(&mut self, f: F) -> Option<T> where F: FnMut(&T) -> bool
        // Finds the first element matching the predicate
        let find_pred_ty = Type::function(vec![Type::reference(t_ty.clone(), false)], bool_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "find",
            false,
            vec![Type::reference(iter_t.clone(), true), find_pred_ty.clone()],
            option_t.clone(),
            "iter_find",
        );

        // Iter<T>.position<F>(&mut self, f: F) -> Option<usize> where F: FnMut(T) -> bool
        // Returns the index of the first element matching the predicate
        let option_usize = Type::adt(option_def_id, vec![usize_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "position",
            false,
            vec![Type::reference(iter_t.clone(), true), any_pred_ty.clone()],
            option_usize,
            "iter_position",
        );

        // Iter<T>.sum(self) -> T where T: Add + Default
        // Sums all elements in the iterator
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "sum",
            false,
            vec![iter_t.clone()],
            t_ty.clone(),
            "iter_sum",
        );

        // Iter<T>.product(self) -> T where T: Mul + Default
        // Multiplies all elements in the iterator
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "product",
            false,
            vec![iter_t.clone()],
            t_ty.clone(),
            "iter_product",
        );

        // Iter<T>.max(self) -> Option<T> where T: Ord
        // Returns the maximum element
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "max",
            false,
            vec![iter_t.clone()],
            option_t.clone(),
            "iter_max",
        );

        // Iter<T>.min(self) -> Option<T> where T: Ord
        // Returns the minimum element
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "min",
            false,
            vec![iter_t.clone()],
            option_t.clone(),
            "iter_min",
        );

        // Iter<T>.enumerate(self) -> Iter<(usize, T)>
        // Returns an iterator that yields (index, element) pairs
        let tuple_usize_t = Type::tuple(vec![usize_ty.clone(), t_ty.clone()]);
        let iter_enumerate = Type::adt(iter_def_id, vec![tuple_usize_t]);
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "enumerate",
            false,
            vec![iter_t.clone()],
            iter_enumerate,
            "iter_enumerate",
        );

        // Iter<T>.rev(self) -> Iter<T>
        // Reverses the iterator
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "rev",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_rev",
        );

        // Iter<T>.cloned(self) -> Iter<T> where T: Clone
        // Creates an iterator that clones all elements
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "cloned",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_cloned",
        );

        // Iter<T>.copied(self) -> Iter<T> where T: Copy
        // Creates an iterator that copies all elements
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "copied",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_copied",
        );

        // Iter<T>.peekable(self) -> Iter<T>
        // Creates a peekable iterator
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "peekable",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_peekable",
        );

        // Iter<T>.fuse(self) -> Iter<T>
        // Creates a fused iterator that stops after first None
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "fuse",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_fuse",
        );

        // === Closure-based Iterator methods ===
        // Type variable U for map operations
        let iter_u_var_id = TyVarId(9010);  // synthetic placeholder for U in Iterator context
        let iter_u_ty = Type::new(TypeKind::Param(iter_u_var_id));
        let iter_u = Type::adt(iter_def_id, vec![iter_u_ty.clone()]);

        // Iter<T>.for_each<F>(self, f: F) where F: FnMut(T)
        // Calls a closure on each element
        let foreach_fn_ty = Type::function(vec![t_ty.clone()], Type::unit());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "for_each",
            false,
            vec![iter_t.clone(), foreach_fn_ty],
            Type::unit(),
            "iter_for_each",
        );

        // Iter<T>.map<U>(self, f: fn(T) -> U) -> Iter<U>
        // Applies f to each element, producing an iterator of results
        let map_fn_ty = Type::function(vec![t_ty.clone()], iter_u_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "map",
            false,
            vec![iter_t.clone(), map_fn_ty],
            iter_u.clone(),
            "iter_map",
            vec![iter_u_var_id],
        );

        // Iter<T>.filter<F>(self, predicate: F) -> Iter<T> where F: FnMut(&T) -> bool
        // Filters elements based on a predicate
        let filter_fn_ty = Type::function(vec![Type::reference(t_ty.clone(), false)], bool_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "filter",
            false,
            vec![iter_t.clone(), filter_fn_ty.clone()],
            iter_t.clone(),
            "iter_filter",
        );

        // Iter<T>.filter_map<U>(self, f: fn(T) -> Option<U>) -> Iter<U>
        // Applies f and filters based on Some/None, unwrapping Some values
        let option_u = Type::adt(option_def_id, vec![iter_u_ty.clone()]);
        let filter_map_fn_ty = Type::function(vec![t_ty.clone()], option_u.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "filter_map",
            false,
            vec![iter_t.clone(), filter_map_fn_ty],
            iter_u.clone(),
            "iter_filter_map",
            vec![iter_u_var_id],
        );

        // Iter<T>.flat_map<U>(self, f: fn(T) -> Iter<U>) -> Iter<U>
        // Maps each element to an iterator and flattens
        let flat_map_fn_ty = Type::function(vec![t_ty.clone()], iter_u.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "flat_map",
            false,
            vec![iter_t.clone(), flat_map_fn_ty],
            iter_u.clone(),
            "iter_flat_map",
            vec![iter_u_var_id],
        );

        // Iter<T>.fold<B>(self, init: B, f: fn(B, T) -> B) -> B
        // Folds every element into an accumulator
        let fold_b_var_id = TyVarId(9011);
        let fold_b_ty = Type::new(TypeKind::Param(fold_b_var_id));
        let fold_fn_ty = Type::function(vec![fold_b_ty.clone(), t_ty.clone()], fold_b_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "fold",
            false,
            vec![iter_t.clone(), fold_b_ty.clone(), fold_fn_ty],
            fold_b_ty.clone(),
            "iter_fold",
            vec![fold_b_var_id],
        );

        // Iter<T>.reduce(self, f: fn(T, T) -> T) -> Option<T>
        // Reduces elements to a single one using f
        let reduce_fn_ty = Type::function(vec![t_ty.clone(), t_ty.clone()], t_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "reduce",
            false,
            vec![iter_t.clone(), reduce_fn_ty],
            option_t.clone(),
            "iter_reduce",
        );

        // Iter<T>.inspect<F>(self, f: F) -> Iter<T> where F: FnMut(&T)
        // Calls f on each element for side effects, returns original iterator
        let inspect_fn_ty = Type::function(vec![Type::reference(t_ty.clone(), false)], Type::unit());
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "inspect",
            false,
            vec![iter_t.clone(), inspect_fn_ty],
            iter_t.clone(),
            "iter_inspect",
        );

        // Iter<T>.take_while<F>(self, predicate: F) -> Iter<T> where F: FnMut(&T) -> bool
        // Takes elements while predicate returns true
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "take_while",
            false,
            vec![iter_t.clone(), filter_fn_ty.clone()],
            iter_t.clone(),
            "iter_take_while",
        );

        // Iter<T>.skip_while<F>(self, predicate: F) -> Iter<T> where F: FnMut(&T) -> bool
        // Skips elements while predicate returns true
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "skip_while",
            false,
            vec![iter_t.clone(), filter_fn_ty.clone()],
            iter_t.clone(),
            "iter_skip_while",
        );

        // Iter<T>.partition<F>(self, f: F) -> (Vec<T>, Vec<T>) where F: FnMut(&T) -> bool
        // Partitions into two collections based on predicate
        let partition_result = Type::tuple(vec![vec_t.clone(), vec_t.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "partition",
            false,
            vec![iter_t.clone(), filter_fn_ty.clone()],
            partition_result,
            "iter_partition",
        );

        // Iter<T>.zip<U>(self, other: Iter<U>) -> Iter<(T, U)>
        // Zips two iterators together into pairs
        let tuple_t_u = Type::tuple(vec![t_ty.clone(), iter_u_ty.clone()]);
        let iter_tuple_t_u = Type::adt(iter_def_id, vec![tuple_t_u]);
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "zip",
            false,
            vec![iter_t.clone(), iter_u.clone()],
            iter_tuple_t_u,
            "iter_zip",
            vec![iter_u_var_id],
        );

        // Iter<T>.chain(self, other: Iter<T>) -> Iter<T>
        // Chains two iterators together
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "chain",
            false,
            vec![iter_t.clone(), iter_t.clone()],
            iter_t.clone(),
            "iter_chain",
        );

        // Iter<T>.cycle(self) -> Iter<T>
        // Creates an iterator that cycles forever
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "cycle",
            false,
            vec![iter_t.clone()],
            iter_t.clone(),
            "iter_cycle",
        );

        // Iter<T>.step_by(self, step: usize) -> Iter<T>
        // Creates an iterator that yields every nth element
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "step_by",
            false,
            vec![iter_t.clone(), usize_ty.clone()],
            iter_t.clone(),
            "iter_step_by",
        );

        // Iter<T>.flatten(self) -> Iter<U> where T: IntoIterator<Item = U>
        // Flattens nested iterators
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "flatten",
            false,
            vec![iter_t.clone()],
            iter_u.clone(),
            "iter_flatten",
            vec![iter_u_var_id],
        );

        // Iter<T>.unzip(self) -> (Vec<A>, Vec<B>) where T = (A, B)
        // Converts an iterator of pairs into a pair of collections
        let unzip_a_var_id = TyVarId(9012);
        let unzip_b_var_id = TyVarId(9013);
        let unzip_a_ty = Type::new(TypeKind::Param(unzip_a_var_id));
        let unzip_b_ty = Type::new(TypeKind::Param(unzip_b_var_id));
        let vec_def_id = self.vec_def_id.expect("vec_def_id must be set");
        let vec_a = Type::adt(vec_def_id, vec![unzip_a_ty.clone()]);
        let vec_b = Type::adt(vec_def_id, vec![unzip_b_ty.clone()]);
        let unzip_result = Type::tuple(vec![vec_a, vec_b]);
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "unzip",
            false,
            vec![iter_t.clone()],
            unzip_result,
            "iter_unzip",
            vec![unzip_a_var_id, unzip_b_var_id],
        );

        // Iter<T>.max_by<F>(self, compare: F) -> Option<T> where F: FnMut(&T, &T) -> i32
        // Returns max element using a comparison function (i32 represents Ordering)
        let cmp_fn_ty = Type::function(
            vec![Type::reference(t_ty.clone(), false), Type::reference(t_ty.clone(), false)],
            Type::i32(),
        );
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "max_by",
            false,
            vec![iter_t.clone(), cmp_fn_ty.clone()],
            option_t.clone(),
            "iter_max_by",
        );

        // Iter<T>.min_by<F>(self, compare: F) -> Option<T> where F: FnMut(&T, &T) -> i32
        // Returns min element using a comparison function
        self.register_builtin_method(
            BuiltinMethodType::Iterator,
            "min_by",
            false,
            vec![iter_t.clone(), cmp_fn_ty.clone()],
            option_t.clone(),
            "iter_min_by",
        );

        // Iter<T>.max_by_key<B, F>(self, f: F) -> Option<T> where F: FnMut(&T) -> B, B: Ord
        // Returns max element by key computed with f
        let key_fn_ty = Type::function(vec![Type::reference(t_ty.clone(), false)], iter_u_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "max_by_key",
            false,
            vec![iter_t.clone(), key_fn_ty.clone()],
            option_t.clone(),
            "iter_max_by_key",
            vec![iter_u_var_id],
        );

        // Iter<T>.min_by_key<B, F>(self, f: F) -> Option<T> where F: FnMut(&T) -> B, B: Ord
        // Returns min element by key computed with f
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Iterator,
            "min_by_key",
            false,
            vec![iter_t.clone(), key_fn_ty.clone()],
            option_t.clone(),
            "iter_min_by_key",
            vec![iter_u_var_id],
        );

        // === Box methods ===

        let box_def_id = self.box_def_id.expect("BUG: box_def_id not set");

        // Box<T>::new(value: T) -> Box<T> (static method)
        let box_t = Type::adt(box_def_id, vec![t_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "new",
            true,
            vec![t_ty.clone()],  // Takes value: T
            box_t.clone(),       // Returns Box<T>
            "box_new",
        );

        // Box<T>::as_ref(self) -> &T
        // Note: Box is special - it's already a pointer at runtime, so we take it by value
        // The runtime function just returns the same pointer (identity)
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "as_ref",
            false,
            vec![box_t.clone()],  // Takes Box<T> by value (which is just a pointer)
            Type::reference(t_ty.clone(), false),         // Returns &T
            "box_as_ref",
        );

        // Box<T>::as_mut(self) -> &mut T
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "as_mut",
            false,
            vec![box_t.clone()],  // Takes Box<T> by value
            Type::reference(t_ty.clone(), true),         // Returns &mut T
            "box_as_mut",
        );

        // Box<T>.into_inner(self) -> T
        // Consumes the box and returns the inner value
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "into_inner",
            false,
            vec![box_t.clone()],  // Takes Box<T> by value (consumes it)
            t_ty.clone(),         // Returns T
            "box_into_inner",
        );

        // Box<T>.into_raw(self) -> *mut T
        // Consumes the box and returns a raw pointer
        let raw_ptr_t = Type::new(TypeKind::Ptr { inner: t_ty.clone(), mutable: true });
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "into_raw",
            false,
            vec![box_t.clone()],  // Takes Box<T> by value (consumes it)
            raw_ptr_t.clone(),    // Returns *mut T
            "box_into_raw",
        );

        // Box<T>::from_raw(ptr: *mut T) -> Box<T>
        // Creates a Box from a raw pointer (static method)
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "from_raw",
            true,                 // Static method
            vec![raw_ptr_t],      // Takes *mut T
            box_t.clone(),        // Returns Box<T>
            "box_from_raw",
        );

        // Box<T>.leak(self) -> &'static mut T
        // Leaks the box and returns a mutable reference
        self.register_builtin_method(
            BuiltinMethodType::Box,
            "leak",
            false,
            vec![box_t.clone()],                        // Takes Box<T> by value
            Type::reference(t_ty.clone(), true),        // Returns &mut T (static lifetime implied)
            "box_leak",
        );

        // === Slice [T] methods ===

        // [T].len(&self) -> usize
        let slice_t = Type::slice(t_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "len",
            false,
            vec![Type::reference(slice_t.clone(), false)],
            usize_ty.clone(),
            "slice_len",
        );

        // [T].is_empty(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "is_empty",
            false,
            vec![Type::reference(slice_t.clone(), false)],
            bool_ty.clone(),
            "slice_is_empty",
        );

        // [T].first(&self) -> Option<&T>
        let slice_option_ref_t = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty.clone(), false)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "first",
            false,
            vec![Type::reference(slice_t.clone(), false)],
            slice_option_ref_t.clone(),
            "slice_first",
        );

        // [T].last(&self) -> Option<&T>
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "last",
            false,
            vec![Type::reference(slice_t.clone(), false)],
            slice_option_ref_t.clone(),
            "slice_last",
        );

        // [T].get(&self, index: usize) -> Option<&T>
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "get",
            false,
            vec![Type::reference(slice_t.clone(), false), usize_ty.clone()],
            slice_option_ref_t,
            "slice_get",
        );

        // [T].contains(&self, elem: &T) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "contains",
            false,
            vec![Type::reference(slice_t.clone(), false), Type::reference(t_ty.clone(), false)],
            bool_ty.clone(),
            "slice_contains",
        );

        // [T].get_mut(&mut self, index: usize) -> Option<&mut T>
        let slice_option_ref_mut_t = Type::adt(
            self.option_def_id.expect("BUG: option_def_id not set"),
            vec![Type::reference(t_ty.clone(), true)],
        );
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "get_mut",
            false,
            vec![Type::reference(slice_t.clone(), true), usize_ty.clone()],
            slice_option_ref_mut_t,
            "slice_get_mut",
        );

        // [T].starts_with(&self, needle: &[T]) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "starts_with",
            false,
            vec![Type::reference(slice_t.clone(), false), Type::reference(slice_t.clone(), false)],
            bool_ty.clone(),
            "slice_starts_with",
        );

        // [T].ends_with(&self, needle: &[T]) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "ends_with",
            false,
            vec![Type::reference(slice_t.clone(), false), Type::reference(slice_t.clone(), false)],
            bool_ty.clone(),
            "slice_ends_with",
        );

        // [T].reverse(&mut self)
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "reverse",
            false,
            vec![Type::reference(slice_t.clone(), true)],
            Type::unit(),
            "slice_reverse",
        );

        // [T].split_at(&self, mid: usize) -> (&[T], &[T])
        // Splits slice at index, returns tuple of two slices
        let slice_ref = Type::reference(slice_t.clone(), false);
        let split_result = Type::tuple(vec![slice_ref.clone(), slice_ref.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "split_at",
            false,
            vec![Type::reference(slice_t.clone(), false), usize_ty.clone()],
            split_result,
            "slice_split_at",
        );

        // [T].split_at_mut(&mut self, mid: usize) -> (&mut [T], &mut [T])
        let slice_mut_ref = Type::reference(slice_t.clone(), true);
        let split_mut_result = Type::tuple(vec![slice_mut_ref.clone(), slice_mut_ref.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "split_at_mut",
            false,
            vec![Type::reference(slice_t.clone(), true), usize_ty.clone()],
            split_mut_result,
            "slice_split_at_mut",
        );

        // [T].sort(&mut self) where T: Ord
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "sort",
            false,
            vec![Type::reference(slice_t.clone(), true)],
            Type::unit(),
            "slice_sort",
        );

        // [T].sort_by(&mut self, compare: F) where F: FnMut(&T, &T) -> Ordering
        let slice_sort_cmp_ty = Type::function(
            vec![Type::reference(t_ty.clone(), false), Type::reference(t_ty.clone(), false)],
            Type::i32(),  // Ordering encoded as i32
        );
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "sort_by",
            false,
            vec![Type::reference(slice_t.clone(), true), slice_sort_cmp_ty],
            Type::unit(),
            "slice_sort_by",
        );

        // [T].binary_search(&self, x: &T) -> Result<usize, usize> where T: Ord
        let slice_binary_search_result = Type::adt(
            self.result_def_id.expect("result_def_id must be set"),
            vec![usize_ty.clone(), usize_ty.clone()]
        );
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "binary_search",
            false,
            vec![Type::reference(slice_t.clone(), false), Type::reference(t_ty.clone(), false)],
            slice_binary_search_result,
            "slice_binary_search",
        );

        // [T].copy_from_slice(&mut self, src: &[T]) where T: Copy
        // Copies elements from src into self
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "copy_from_slice",
            false,
            vec![Type::reference(slice_t.clone(), true), Type::reference(slice_t.clone(), false)],
            Type::unit(),
            "slice_copy_from_slice",
        );

        // [T].swap(&mut self, a: usize, b: usize)
        // Swaps elements at indices a and b
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "swap",
            false,
            vec![Type::reference(slice_t.clone(), true), usize_ty.clone(), usize_ty.clone()],
            Type::unit(),
            "slice_swap",
        );

        // [T].iter(&self) -> Iter<&T>
        // Returns an iterator over references to the slice's elements
        let iter_def_id_slice = self.iter_def_id.expect("iter_def_id must be set");
        let iter_ref_t = Type::adt(iter_def_id_slice, vec![Type::reference(t_ty.clone(), false)]);
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "iter",
            false,
            vec![Type::reference(slice_t.clone(), false)],
            iter_ref_t.clone(),
            "slice_iter",
        );

        // [T].iter_mut(&mut self) -> Iter<&mut T>
        // Returns a mutable iterator over the slice's elements
        let iter_mut_ref_t = Type::adt(iter_def_id_slice, vec![Type::reference(t_ty.clone(), true)]);
        self.register_builtin_method(
            BuiltinMethodType::Slice,
            "iter_mut",
            false,
            vec![Type::reference(slice_t.clone(), true)],
            iter_mut_ref_t,
            "slice_iter_mut",
        );

        // === Result<T, E> methods ===

        let result_def_id = self.result_def_id
            .expect("BUG: result_def_id not set before register_builtin_methods");

        // Use fresh type variable for the error type E
        let e_var_id = TyVarId(9001);  // synthetic placeholder
        let e_ty = Type::new(TypeKind::Param(e_var_id));

        // Result<T, E>
        let result_te = Type::adt(result_def_id, vec![t_ty.clone(), e_ty.clone()]);

        // Result<T, E>.is_ok(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "is_ok",
            false,
            vec![Type::reference(result_te.clone(), false)],
            bool_ty.clone(),
            "result_is_ok",
        );

        // Result<T, E>.is_err(&self) -> bool
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "is_err",
            false,
            vec![Type::reference(result_te.clone(), false)],
            bool_ty.clone(),
            "result_is_err",
        );

        // Result<T, E>.unwrap(self) -> T
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "unwrap",
            false,
            vec![result_te.clone()],
            t_ty.clone(),
            "result_unwrap",
        );

        // Result<T, E>.unwrap_err(self) -> E
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "unwrap_err",
            false,
            vec![result_te.clone()],
            e_ty.clone(),
            "result_unwrap_err",
        );

        // Result<T, E>.try_(self) -> T (for ? operator desugaring)
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "try_",
            false,
            vec![result_te.clone()],
            t_ty.clone(),
            "result_try",
        );

        // Result<T, E>.ok(self) -> Option<T>
        let option_t = Type::adt(option_def_id, vec![t_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "ok",
            false,
            vec![result_te.clone()],
            option_t.clone(),
            "result_ok",
        );

        // Result<T, E>.err(self) -> Option<E>
        let option_e = Type::adt(option_def_id, vec![e_ty.clone()]);
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "err",
            false,
            vec![result_te.clone()],
            option_e,
            "result_err",
        );

        // Result<T, E>.expect(self, msg: &str) -> T
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "expect",
            false,
            vec![result_te.clone(), Type::reference(Type::str(), false)],
            t_ty.clone(),
            "result_expect",
        );

        // Result<T, E>.expect_err(self, msg: &str) -> E
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "expect_err",
            false,
            vec![result_te.clone(), Type::reference(Type::str(), false)],
            e_ty.clone(),
            "result_expect_err",
        );

        // Result<T, E>.unwrap_or(self, default: T) -> T
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "unwrap_or",
            false,
            vec![result_te.clone(), t_ty.clone()],
            t_ty.clone(),
            "result_unwrap_or",
        );

        // Result<T, E>.and(self, other: Result<U, E>) -> Result<U, E>
        // Note: U is a fresh type parameter that must be inferred from the argument
        let u_var_id_res = TyVarId(9004);  // synthetic placeholder for U
        let u_ty_res = Type::new(TypeKind::Param(u_var_id_res));
        let result_ue = Type::adt(result_def_id, vec![u_ty_res.clone(), e_ty.clone()]);
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "and",
            false,
            vec![result_te.clone(), result_ue.clone()],
            result_ue.clone(),
            "result_and",
            vec![u_var_id_res],  // U is a method-level type parameter
        );

        // Result<T, E>.or(self, other: Result<T, F>) -> Result<T, F>
        // Note: F is a fresh type parameter that must be inferred from the argument
        let f_var_id = TyVarId(9005);  // synthetic placeholder for F
        let f_ty = Type::new(TypeKind::Param(f_var_id));
        let result_tf = Type::adt(result_def_id, vec![t_ty.clone(), f_ty.clone()]);
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "or",
            false,
            vec![result_te.clone(), result_tf.clone()],
            result_tf.clone(),
            "result_or",
            vec![f_var_id],  // F is a method-level type parameter
        );

        // Result<T, E>.as_ref(&self) -> Result<&T, &E>
        let result_ref_t_ref_e = Type::adt(
            result_def_id,
            vec![Type::reference(t_ty.clone(), false), Type::reference(e_ty.clone(), false)]
        );
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "as_ref",
            false,
            vec![Type::reference(result_te.clone(), false)],
            result_ref_t_ref_e,
            "result_as_ref",
        );

        // Result<T, E>.as_mut(&mut self) -> Result<&mut T, &mut E>
        let result_ref_mut_t_ref_mut_e = Type::adt(
            result_def_id,
            vec![Type::reference(t_ty.clone(), true), Type::reference(e_ty.clone(), true)]
        );
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "as_mut",
            false,
            vec![Type::reference(result_te.clone(), true)],
            result_ref_mut_t_ref_mut_e,
            "result_as_mut",
        );

        // === Result closure-accepting methods ===

        // Type variable U for map operations
        let u_var_id_res = TyVarId(9006);  // synthetic placeholder for U in Result context
        let u_ty_res = Type::new(TypeKind::Param(u_var_id_res));
        let result_ue = Type::adt(result_def_id, vec![u_ty_res.clone(), e_ty.clone()]);

        // Result<T, E>.map<U>(self, f: fn(T) -> U) -> Result<U, E>
        // Applies f to the Ok value if present
        let fn_t_to_u_result = Type::function(vec![t_ty.clone()], u_ty_res.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "map",
            false,
            vec![result_te.clone(), fn_t_to_u_result],
            result_ue.clone(),
            "result_map",
            vec![u_var_id_res],
        );

        // Type variable F for map_err operations
        let f_var_id = TyVarId(9007);  // synthetic placeholder for F
        let f_ty = Type::new(TypeKind::Param(f_var_id));
        let result_tf = Type::adt(result_def_id, vec![t_ty.clone(), f_ty.clone()]);

        // Result<T, E>.map_err<F>(self, f: fn(E) -> F) -> Result<T, F>
        // Applies f to the Err value if present
        let fn_e_to_f = Type::function(vec![e_ty.clone()], f_ty.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "map_err",
            false,
            vec![result_te.clone(), fn_e_to_f],
            result_tf,
            "result_map_err",
            vec![f_var_id],
        );

        // Result<T, E>.and_then<U>(self, f: fn(T) -> Result<U, E>) -> Result<U, E>
        // Returns Err if self is Err, otherwise calls f with Ok value
        let fn_t_to_result_ue = Type::function(vec![t_ty.clone()], result_ue.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "and_then",
            false,
            vec![result_te.clone(), fn_t_to_result_ue],
            result_ue.clone(),
            "result_and_then",
            vec![u_var_id_res],
        );

        // Result<T, E>.or_else<F>(self, f: fn(E) -> Result<T, F>) -> Result<T, F>
        // Returns Ok if self is Ok, otherwise calls f with Err value
        let result_tf_for_or_else = Type::adt(result_def_id, vec![t_ty.clone(), f_ty.clone()]);
        let fn_e_to_result_tf = Type::function(vec![e_ty.clone()], result_tf_for_or_else.clone());
        self.register_builtin_method_with_generics(
            BuiltinMethodType::Result,
            "or_else",
            false,
            vec![result_te.clone(), fn_e_to_result_tf],
            result_tf_for_or_else,
            "result_or_else",
            vec![f_var_id],
        );

        // Result<T, E>.unwrap_or_else(self, f: fn(E) -> T) -> T
        // Returns Ok value or computes it from Err using f
        let fn_e_to_t = Type::function(vec![e_ty.clone()], t_ty.clone());
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "unwrap_or_else",
            false,
            vec![result_te.clone(), fn_e_to_t],
            t_ty.clone(),
            "result_unwrap_or_else",
        );

        // Result<T, E>.unwrap_or_default(self) -> T where T: Default
        // Returns Ok value or the default for T
        self.register_builtin_method(
            BuiltinMethodType::Result,
            "unwrap_or_default",
            false,
            vec![result_te.clone()],
            t_ty.clone(),
            "result_unwrap_or_default",
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
        self.register_builtin_method_with_generics(
            type_match,
            name,
            is_static,
            inputs,
            output,
            runtime_name,
            Vec::new(),
        )
    }

    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
    fn register_builtin_method_with_generics(
        &mut self,
        type_match: super::BuiltinMethodType,
        name: &str,
        is_static: bool,
        inputs: Vec<Type>,
        output: Type,
        runtime_name: &str,
        generics: Vec<TyVarId>,
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
                super::BuiltinMethodType::Box => "Box",
                super::BuiltinMethodType::Result => "Result",
                super::BuiltinMethodType::Slice => "Slice",
                super::BuiltinMethodType::Iterator => "Iterator",
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
            generics,
            const_generics: Vec::new(),
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
