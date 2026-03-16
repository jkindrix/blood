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

    # blood_main is a dummy entry point in the runtime library — strip it
    # to avoid conflict with the actual program's blood_main
    strip_defines = {'blood_main'}

    # Step 2: Collect ptr_read/ptr_write declares to replace
    ptr_builtins = {}
    for line in lines:
        m = re.search(r'^declare\s+(.*?)\s+@(ptr_(?:read|write)_\w+)\((.*?)\)', line)
        if m:
            ptr_builtins[m.group(2)] = (m.group(1), m.group(3))

    # Step 3: Collect other missing builtins we need to inject
    inject_names = {
        'str_len', 'str_len_usize', 'env_get', 'println_u64',
        'blood_float64_to_bits', '__builtin_safepoint_check'
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
    impl_lines.append('define void @println_u64(i64 %val) {\n')
    impl_lines.append('  call i32 (ptr, ...) @printf(ptr @.fmt_u64, i64 %val)\n')
    impl_lines.append('  ret void\n')
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
