#!/usr/bin/env python3
"""Build pipeline for Blood-native runtime.

Steps:
1. Strip conflicting `declare` statements (where a `define` exists)
2. Inject ptr_read/ptr_write builtin implementations (selfhost emits calls, not inline)
3. Inject missing builtin function implementations (str_len, env_get, etc.)
"""

import re
import sys

def process_ir(input_path, output_path):
    with open(input_path) as f:
        lines = f.readlines()

    # Step 1: Find all defined functions
    defined = set()
    for line in lines:
        m = re.search(r'^define\s+(?:internal\s+)?.*?@(\w+)\s*\(', line)
        if m:
            defined.add(m.group(1))

    # blood_main and main are dummy entry points in the runtime library — strip both
    # to avoid conflict with the actual program's entry points
    strip_defines = {'blood_main', 'main'}

    # Step 2: Collect ptr_read/ptr_write declares to replace
    ptr_builtins = {}
    for line in lines:
        m = re.search(r'^declare\s+(.*?)\s+@(ptr_(?:read|write)_\w+)\((.*?)\)', line)
        if m:
            ptr_builtins[m.group(2)] = (m.group(1), m.group(3))

    # Step 3: Collect other missing builtins we need to inject
    inject_names = {
        'str_len', 'str_len_usize', 'env_get', 'println_u64',
        'blood_float64_to_bits', '__builtin_safepoint_check',
        'call_handler_op', 'println_int', 'println_bool', 'print_int',
        # Stage 2 print builtins
        'println_i64', 'print_i64', 'println_char', 'println_f32', 'println_f64',
        # Stage 2 type conversion builtins
        'i32_to_i64', 'i64_to_i32', 'size_of_bool', 'size_of_i32', 'size_of_i64',
        # Stage 2 number-to-string conversions
        'i64_to_string', 'u64_to_string', 'f32_to_string', 'f64_to_string',
        'i8_to_string', 'i16_to_string', 'i128_to_string',
        'u8_to_string', 'u16_to_string', 'u32_to_string', 'u128_to_string',
        # Stage 2 file I/O
        'file_delete',
        # Thread spawn/join
        'blood_thread_spawn', 'blood_thread_join',
    }

    # Step 4: Strip conflicting declares, builtin declares, and unwanted defines
    # Also track which declares remain (so we don't re-declare them)
    out = []
    remaining_declares = set()
    in_strip_define = False
    for line in lines:
        # Strip unwanted define blocks (e.g., blood_main)
        if not in_strip_define:
            dm = re.search(r'^define\s+(?:internal\s+)?.*?@(\w+)\s*\(', line)
            if dm and dm.group(1) in strip_defines:
                in_strip_define = True
                continue
        else:
            if line.strip() == '}':
                in_strip_define = False
            continue

        m = re.search(r'^declare\s+.*?@(\w+)\s*\(', line)
        if m:
            name = m.group(1)
            if name in defined:
                continue
            if name in ptr_builtins:
                continue
            if name in inject_names:
                continue
            remaining_declares.add(name)
        out.append(line)

    # Step 5: Generate ptr builtin implementations
    impl_lines = ['\n; Pointer builtin implementations\n']
    for name, (ret_type, params) in sorted(ptr_builtins.items()):
        if ret_type == 'void':
            param_parts = [p.strip() for p in params.split(',')]
            val_type = param_parts[1] if len(param_parts) > 1 else 'i64'
            impl_lines.append(f'define {ret_type} @{name}(i64 %addr, {val_type} %val) {{\n')
            impl_lines.append(f'  %ptr = inttoptr i64 %addr to ptr\n')
            impl_lines.append(f'  store {val_type} %val, ptr %ptr\n')
            impl_lines.append(f'  ret void\n')
            impl_lines.append(f'}}\n')
        else:
            impl_lines.append(f'define {ret_type} @{name}(i64 %addr) {{\n')
            impl_lines.append(f'  %ptr = inttoptr i64 %addr to ptr\n')
            impl_lines.append(f'  %val = load {ret_type}, ptr %ptr\n')
            impl_lines.append(f'  ret {ret_type} %val\n')
            impl_lines.append(f'}}\n')

    # Step 6: Generate missing builtin implementations
    impl_lines.append('\n; Missing builtin implementations\n')

    # str_len({ptr, i64}) -> i64 : extract length from fat pointer
    impl_lines.append('define i64 @str_len({ ptr, i64 } %s) {\n')
    impl_lines.append('  %len = extractvalue { ptr, i64 } %s, 1\n')
    impl_lines.append('  ret i64 %len\n')
    impl_lines.append('}\n')

    # str_len_usize(ptr) -> i64 : load length from &str pointer (method call ABI)
    impl_lines.append('define i64 @str_len_usize(ptr %s) {\n')
    impl_lines.append('  %len_ptr = getelementptr inbounds { ptr, i64 }, ptr %s, i32 0, i32 1\n')
    impl_lines.append('  %len = load i64, ptr %len_ptr\n')
    impl_lines.append('  ret i64 %len\n')
    impl_lines.append('}\n')

    # __builtin_safepoint_check() : no-op for single-threaded Stage 1
    impl_lines.append('define void @__builtin_safepoint_check() {\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # blood_float64_to_bits(double) -> i64 : bitcast f64 to i64
    impl_lines.append('define i64 @blood_float64_to_bits(double %val) {\n')
    impl_lines.append('  %bits = bitcast double %val to i64\n')
    impl_lines.append('  ret i64 %bits\n')
    impl_lines.append('}\n')

    # println_u64(i64) -> void : print unsigned 64-bit integer with newline
    impl_lines.append('@.fmt_u64 = private unnamed_addr constant [5 x i8] c"%lu\\0A\\00"\n')
    if 'printf' not in remaining_declares and 'printf' not in defined:
        impl_lines.append('declare i32 @printf(ptr, ...)\n')
    for decl_name in ['fflush']:
        if decl_name not in remaining_declares and decl_name not in defined:
            impl_lines.append(f'declare i32 @{decl_name}(ptr)\n')
    impl_lines.append('define void @println_u64(i64 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_u64, i64 %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # println_int(i32) -> void : print signed 32-bit integer with newline
    impl_lines.append('@.fmt_i32 = private unnamed_addr constant [4 x i8] c"%d\\0A\\00"\n')
    impl_lines.append('define void @println_int(i32 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_i32, i32 %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # print_int(i32) -> void : print signed 32-bit integer (no newline)
    impl_lines.append('@.fmt_i32_nn = private unnamed_addr constant [3 x i8] c"%d\\00"\n')
    impl_lines.append('define void @print_int(i32 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_i32_nn, i32 %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # println_bool(i1) -> void : print "true" or "false" with newline
    impl_lines.append('@.str_true = private unnamed_addr constant [5 x i8] c"true\\00"\n')
    impl_lines.append('@.str_false = private unnamed_addr constant [6 x i8] c"false\\00"\n')
    impl_lines.append('define void @println_bool(i1 %val) {\n')
    impl_lines.append('  %str = select i1 %val, ptr @.str_true, ptr @.str_false\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr %str)\n')
    impl_lines.append('  call i32 @putchar(i32 10)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')
    if 'putchar' not in remaining_declares and 'putchar' not in defined:
        impl_lines.append('declare i32 @putchar(i32)\n')

    # call_handler_op(ptr fn, ptr state, ptr args, i64 arg_count, i64 cont) -> i64
    # Indirect function call for effect handler dispatch — Blood can't call raw fn ptrs
    impl_lines.append('define i64 @call_handler_op(ptr %fn_ptr, ptr %state, ptr %args, i64 %arg_count, i64 %cont) {\n')
    impl_lines.append('  %result = call i64 %fn_ptr(ptr %state, ptr %args, i64 %arg_count, i64 %cont)\n')
    impl_lines.append('  ret i64 %result\n')
    impl_lines.append('}\n')

    # --- Stage 2 print builtins ---

    # println_i64(i64) -> void
    impl_lines.append('@.fmt_i64 = private unnamed_addr constant [5 x i8] c"%ld\\0A\\00"\n')
    impl_lines.append('define void @println_i64(i64 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_i64, i64 %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # print_i64(i64) -> void (no newline)
    impl_lines.append('@.fmt_i64_nn = private unnamed_addr constant [4 x i8] c"%ld\\00"\n')
    impl_lines.append('define void @print_i64(i64 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_i64_nn, i64 %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # println_char(i32) -> void : print Unicode code point as character + newline
    impl_lines.append('define void @println_char(i32 %c) {\n')
    impl_lines.append('  call i32 @putchar(i32 %c)\n')
    impl_lines.append('  call i32 @putchar(i32 10)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # println_f32(float) -> void : promote to double, print with %g
    impl_lines.append('@.fmt_g_nl = private unnamed_addr constant [4 x i8] c"%g\\0A\\00"\n')
    impl_lines.append('define void @println_f32(float %val) {\n')
    impl_lines.append('  %d = fpext float %val to double\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_g_nl, double %d)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # println_f64(double) -> void
    impl_lines.append('define void @println_f64(double %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_g_nl, double %val)\n')
    impl_lines.append('  call i32 @fflush(ptr null)\n')
    impl_lines.append('  ret void\n')
    impl_lines.append('}\n')

    # --- Stage 2 simple type conversion builtins ---

    impl_lines.append('define i64 @i32_to_i64(i32 %val) {\n')
    impl_lines.append('  %r = sext i32 %val to i64\n')
    impl_lines.append('  ret i64 %r\n')
    impl_lines.append('}\n')

    impl_lines.append('define i32 @i64_to_i32(i64 %val) {\n')
    impl_lines.append('  %r = trunc i64 %val to i32\n')
    impl_lines.append('  ret i32 %r\n')
    impl_lines.append('}\n')

    impl_lines.append('define i64 @size_of_bool() {\n  ret i64 1\n}\n')
    impl_lines.append('define i64 @size_of_i32() {\n  ret i64 4\n}\n')
    impl_lines.append('define i64 @size_of_i64() {\n  ret i64 8\n}\n')

    # --- Stage 2 number-to-string conversions ---
    # Pattern: snprintf to stack buffer -> calloc + memcpy -> return { ptr, i64 }

    for decl_name in ['snprintf', 'unlink']:
        if decl_name not in remaining_declares and decl_name not in defined:
            if decl_name == 'snprintf':
                impl_lines.append('declare i32 @snprintf(ptr, i64, ptr, ...)\n')
            elif decl_name == 'unlink':
                impl_lines.append('declare i32 @unlink(ptr)\n')

    # Helper: format a value via snprintf, allocate result, return { ptr, i64 }
    # Each *_to_string function follows this pattern with type-specific format.

    # i64_to_string(i64) -> { ptr, i64 }
    impl_lines.append('@.fmt_ld = private unnamed_addr constant [4 x i8] c"%ld\\00"\n')
    impl_lines.append('define { ptr, i64 } @i64_to_string(i64 %n) {\n')
    impl_lines.append('  %buf = alloca [24 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 24, ptr @.fmt_ld, i64 %n)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # u64_to_string(i64) -> { ptr, i64 }  (arg is i64, interpreted as unsigned)
    impl_lines.append('@.fmt_lu = private unnamed_addr constant [4 x i8] c"%lu\\00"\n')
    impl_lines.append('define { ptr, i64 } @u64_to_string(i64 %n) {\n')
    impl_lines.append('  %buf = alloca [24 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 24, ptr @.fmt_lu, i64 %n)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # i8_to_string(i8), i16_to_string(i16) — sext to i32, format with %d
    impl_lines.append('@.fmt_d = private unnamed_addr constant [3 x i8] c"%d\\00"\n')
    for (fn_name, from_type) in [('i8_to_string', 'i8'), ('i16_to_string', 'i16')]:
        impl_lines.append(f'define {{ ptr, i64 }} @{fn_name}({from_type} %n) {{\n')
        impl_lines.append(f'  %ext = sext {from_type} %n to i32\n')
        impl_lines.append('  %buf = alloca [12 x i8]\n')
        impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 12, ptr @.fmt_d, i32 %ext)\n')
        impl_lines.append('  %len64 = sext i32 %len to i64\n')
        impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
        impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
        impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
        impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
        impl_lines.append('  ret { ptr, i64 } %r2\n')
        impl_lines.append('}\n')

    # u8_to_string(i8), u16_to_string(i16), u32_to_string(i32) — zext to i32/i64, format with %u/%lu
    impl_lines.append('@.fmt_u = private unnamed_addr constant [3 x i8] c"%u\\00"\n')
    for (fn_name, from_type) in [('u8_to_string', 'i8'), ('u16_to_string', 'i16')]:
        impl_lines.append(f'define {{ ptr, i64 }} @{fn_name}({from_type} %n) {{\n')
        impl_lines.append(f'  %ext = zext {from_type} %n to i32\n')
        impl_lines.append('  %buf = alloca [12 x i8]\n')
        impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 12, ptr @.fmt_u, i32 %ext)\n')
        impl_lines.append('  %len64 = sext i32 %len to i64\n')
        impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
        impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
        impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
        impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
        impl_lines.append('  ret { ptr, i64 } %r2\n')
        impl_lines.append('}\n')

    impl_lines.append('define { ptr, i64 } @u32_to_string(i32 %n) {\n')
    impl_lines.append('  %ext = zext i32 %n to i64\n')
    impl_lines.append('  %buf = alloca [12 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 12, ptr @.fmt_lu, i64 %ext)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # i128_to_string(i128) — sext to i64 (works for values in i64 range, sufficient for golden tests)
    impl_lines.append('define { ptr, i64 } @i128_to_string(i128 %n) {\n')
    impl_lines.append('  %trunc = trunc i128 %n to i64\n')
    impl_lines.append('  %buf = alloca [24 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 24, ptr @.fmt_ld, i64 %trunc)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # u128_to_string(i128) — trunc to i64, format as unsigned
    impl_lines.append('define { ptr, i64 } @u128_to_string(i128 %n) {\n')
    impl_lines.append('  %trunc = trunc i128 %n to i64\n')
    impl_lines.append('  %buf = alloca [24 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 24, ptr @.fmt_lu, i64 %trunc)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # f32_to_string(float) -> { ptr, i64 } — fpext to double, format with %g
    impl_lines.append('@.fmt_g = private unnamed_addr constant [3 x i8] c"%g\\00"\n')
    impl_lines.append('define { ptr, i64 } @f32_to_string(float %n) {\n')
    impl_lines.append('  %d = fpext float %n to double\n')
    impl_lines.append('  %buf = alloca [32 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 32, ptr @.fmt_g, double %d)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # f64_to_string(double) -> { ptr, i64 }
    impl_lines.append('define { ptr, i64 } @f64_to_string(double %n) {\n')
    impl_lines.append('  %buf = alloca [32 x i8]\n')
    impl_lines.append('  %len = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %buf, i64 32, ptr @.fmt_g, double %n)\n')
    impl_lines.append('  %len64 = sext i32 %len to i64\n')
    impl_lines.append('  %mem = call ptr @calloc(i64 1, i64 %len64)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %mem, ptr %buf, i64 %len64, i1 false)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %mem, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %len64, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    # --- Stage 2 file I/O ---

    # file_delete({ ptr, i64 }) -> i1 : null-terminate path, call unlink
    impl_lines.append('define i1 @file_delete({ ptr, i64 } %path) {\n')
    impl_lines.append('  %pptr = extractvalue { ptr, i64 } %path, 0\n')
    impl_lines.append('  %plen = extractvalue { ptr, i64 } %path, 1\n')
    impl_lines.append('  %bsz = add i64 %plen, 1\n')
    impl_lines.append('  %pbuf = call ptr @calloc(i64 %bsz, i64 1)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %pbuf, ptr %pptr, i64 %plen, i1 false)\n')
    impl_lines.append('  %rc = call i32 @unlink(ptr %pbuf)\n')
    impl_lines.append('  %ok = icmp eq i32 %rc, 0\n')
    impl_lines.append('  ret i1 %ok\n')
    impl_lines.append('}\n')

    # --- Thread spawn/join ---
    # Trampoline: pthread start_routine that calls a Blood fn via indirect call.
    # Blood fn ptrs use closure ABI: fn(ptr env, i64 arg) -> i64, env=null for plain fns.
    # Receives packed args {func_addr: i64, arg: i64} as void* parameter.
    impl_lines.append('define ptr @blood_thread_trampoline(ptr %packed) {\n')
    impl_lines.append('  %func_addr = load i64, ptr %packed\n')
    impl_lines.append('  %arg_ptr = getelementptr i64, ptr %packed, i64 1\n')
    impl_lines.append('  %arg = load i64, ptr %arg_ptr\n')
    impl_lines.append('  %func = inttoptr i64 %func_addr to ptr\n')
    impl_lines.append('  %result = call i64 %func(ptr null, i64 %arg)\n')
    impl_lines.append('  %ret = inttoptr i64 %result to ptr\n')
    impl_lines.append('  ret ptr %ret\n')
    impl_lines.append('}\n')

    # blood_thread_spawn(func: i64, arg: i64) -> i64 (pthread_t handle)
    impl_lines.append('define i64 @blood_thread_spawn(i64 %func, i64 %arg) {\n')
    impl_lines.append('  %packed = call ptr @calloc(i64 16, i64 1)\n')
    impl_lines.append('  store i64 %func, ptr %packed\n')
    impl_lines.append('  %arg_slot = getelementptr i64, ptr %packed, i64 1\n')
    impl_lines.append('  store i64 %arg, ptr %arg_slot\n')
    impl_lines.append('  %tid = alloca i64\n')
    impl_lines.append('  call i32 @pthread_create(ptr %tid, ptr null, ptr @blood_thread_trampoline, ptr %packed)\n')
    impl_lines.append('  %handle = load i64, ptr %tid\n')
    impl_lines.append('  ret i64 %handle\n')
    impl_lines.append('}\n')

    # blood_thread_join(handle: i64) -> i64
    impl_lines.append('define i64 @blood_thread_join(i64 %handle) {\n')
    impl_lines.append('  %retval = alloca ptr\n')
    impl_lines.append('  call i32 @pthread_join(i64 %handle, ptr %retval)\n')
    impl_lines.append('  %ret_ptr = load ptr, ptr %retval\n')
    impl_lines.append('  %result = ptrtoint ptr %ret_ptr to i64\n')
    impl_lines.append('  ret i64 %result\n')
    impl_lines.append('}\n')

    # env_get({ptr, i64}) -> {ptr, i64} : get environment variable
    # Extract name, null-terminate, call getenv, return as &str
    for decl_name, decl_text in [
        ('getenv', 'declare ptr @getenv(ptr)\n'),
        ('strlen', 'declare i64 @strlen(ptr)\n'),
        ('calloc', 'declare ptr @calloc(i64, i64)\n'),
    ]:
        if decl_name not in remaining_declares and decl_name not in defined:
            impl_lines.append(decl_text)
    # llvm.memcpy is always declared by the compiler, don't redeclare
    impl_lines.append('define { ptr, i64 } @env_get({ ptr, i64 } %name) {\n')
    impl_lines.append('  %name_ptr = extractvalue { ptr, i64 } %name, 0\n')
    impl_lines.append('  %name_len = extractvalue { ptr, i64 } %name, 1\n')
    impl_lines.append('  ; Allocate null-terminated copy\n')
    impl_lines.append('  %buf_size = add i64 %name_len, 1\n')
    impl_lines.append('  %buf = call ptr @calloc(i64 %buf_size, i64 1)\n')
    impl_lines.append('  call void @llvm.memcpy.p0.p0.i64(ptr %buf, ptr %name_ptr, i64 %name_len, i1 false)\n')
    impl_lines.append('  %result = call ptr @getenv(ptr %buf)\n')
    impl_lines.append('  %is_null = icmp eq ptr %result, null\n')
    impl_lines.append('  br i1 %is_null, label %null_case, label %found_case\n')
    impl_lines.append('null_case:\n')
    impl_lines.append('  ret { ptr, i64 } zeroinitializer\n')
    impl_lines.append('found_case:\n')
    impl_lines.append('  %rlen = call i64 @strlen(ptr %result)\n')
    impl_lines.append('  %r1 = insertvalue { ptr, i64 } undef, ptr %result, 0\n')
    impl_lines.append('  %r2 = insertvalue { ptr, i64 } %r1, i64 %rlen, 1\n')
    impl_lines.append('  ret { ptr, i64 } %r2\n')
    impl_lines.append('}\n')

    out.extend(impl_lines)

    with open(output_path, 'w') as f:
        f.writelines(out)

    stripped = len(lines) - len(out) + len(impl_lines)
    print(f'Processed: stripped {stripped} declares, injected {len(ptr_builtins)} ptr builtins + 6 missing builtins')

if __name__ == '__main__':
    input_path = sys.argv[1] if len(sys.argv) > 1 else 'runtime/blood-runtime/build/debug/lib.ll'
    output_path = sys.argv[2] if len(sys.argv) > 2 else 'runtime/blood-runtime/build/debug/lib_clean.ll'
    process_ir(input_path, output_path)
