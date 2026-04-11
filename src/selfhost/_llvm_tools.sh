#!/bin/bash
# _llvm_tools.sh — LLVM toolchain detection helper.
#
# Source this file from build_selfhost.sh, debug_test.sh, and other shell
# scripts that invoke the LLVM toolchain. It defines a `detect_llvm_tool`
# function and exports LLC/CLANG/OPT/FILECHECK/LLVM_AS/LLVM_EXTRACT/LLVM_LINK
# environment variables pointing at the best available binary for each tool.
#
# Probing order: versioned binaries (llc-19, llc-18, llc-17) first, then the
# unversioned fallback (llc). Respects environment overrides so users with
# non-standard installations can force a specific binary.

# Only define the function + detect once per shell invocation.
if [ -z "${BLOOD_LLVM_TOOLS_LOADED:-}" ]; then
    BLOOD_LLVM_TOOLS_LOADED=1

    detect_llvm_tool() {
        # $1 = tool name (llc, clang, opt, FileCheck, llvm-as, llvm-extract, llvm-link)
        # $2 = env var override (may be empty)
        local tool="$1" override="${2:-}"
        if [ -n "$override" ]; then
            command -v "$override" >/dev/null 2>&1 && { printf '%s' "$override"; return 0; }
            printf 'ERROR: %s override %q not found in PATH\n' "$tool" "$override" >&2
            return 1
        fi
        local v
        for v in 19 18 17; do
            if command -v "${tool}-${v}" >/dev/null 2>&1; then
                printf '%s-%s' "$tool" "$v"
                return 0
            fi
        done
        if command -v "$tool" >/dev/null 2>&1; then
            printf '%s' "$tool"
            return 0
        fi
        printf 'ERROR: no %s binary found (tried %s-19, %s-18, %s-17, %s)\n' \
            "$tool" "$tool" "$tool" "$tool" "$tool" >&2
        return 1
    }

    # Required tools — fail hard if missing.
    LLC="$(detect_llvm_tool llc "${LLC:-}")" || { printf 'ERROR: llc not found\n' >&2; exit 1; }
    CLANG="$(detect_llvm_tool clang "${CLANG:-}")" || { printf 'ERROR: clang not found\n' >&2; exit 1; }
    OPT="$(detect_llvm_tool opt "${OPT:-}")" || { printf 'ERROR: opt not found\n' >&2; exit 1; }
    # Optional tools — fall back to the llc version suffix so commands that
    # require them fail with a clear error when actually invoked.
    if [ -z "${FILECHECK:-}" ]; then
        FILECHECK="$(detect_llvm_tool FileCheck "" 2>/dev/null || echo FileCheck-18)"
    fi
    if [ -z "${LLVM_AS:-}" ]; then
        LLVM_AS="$(detect_llvm_tool llvm-as "" 2>/dev/null || echo llvm-as-18)"
    fi
    if [ -z "${LLVM_EXTRACT:-}" ]; then
        LLVM_EXTRACT="$(detect_llvm_tool llvm-extract "" 2>/dev/null || echo llvm-extract-18)"
    fi
    if [ -z "${LLVM_LINK:-}" ]; then
        LLVM_LINK="$(detect_llvm_tool llvm-link "" 2>/dev/null || echo llvm-link-18)"
    fi
    export LLC CLANG OPT FILECHECK LLVM_AS LLVM_EXTRACT LLVM_LINK
fi
