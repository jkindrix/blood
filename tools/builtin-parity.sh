#!/usr/bin/env bash
# Detect drift between the builtins the compiler REGISTERS and the symbols the
# runtime EXPORTS.
#
# Why this exists: hir_lower_builtin.blood can register a builtin the runtime
# does not implement. The type checker accepts the call, codegen emits it, and
# the program dies at `ld' with "undefined reference" and no source location.
# 17 of 160 builtins were in that state when this check was written -- including
# every float-printing function and all file-handle I/O. Two corpus programs
# found it; all 713 golden tests were green.
#
# This is a DRIFT gate, not a correctness gate. It fails when a builtin appears
# that the baseline has never classified. It does not re-probe the baseline --
# see tools/builtin-probe.sh for that.
#
# Exit: 0 no drift | 1 new unclassified builtin | 2 setup error

set -u
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REG_SRC="$REPO_ROOT/src/selfhost/hir_lower_builtin.blood"
RUNTIME_A="${BLOOD_RUST_RUNTIME:-}"
# Fall back to the committed bootstrap archive: this gate must run on a fresh
# clone with nothing built.
for cand in "$RUNTIME_A" "$REPO_ROOT/src/selfhost/build/libblood_runtime.a" \
            "$REPO_ROOT/bootstrap/libblood_runtime_blood.a"; do
    [ -n "$cand" ] && [ -f "$cand" ] && { RUNTIME_A="$cand"; break; }
done
BASELINE="$REPO_ROOT/tools/builtin-parity.baseline"
GREP="$(type -P grep)"   # shell `grep' here is a ugrep wrapper that skips gitignored files

[ -f "$REG_SRC" ]   || { echo "builtin-parity: missing $REG_SRC" >&2; exit 2; }
[ -f "$RUNTIME_A" ] || { echo "builtin-parity: missing runtime archive $RUNTIME_A" >&2; exit 2; }
[ -f "$BASELINE" ]  || { echo "builtin-parity: missing $BASELINE" >&2; exit 2; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

"$GREP" -hoE 'register_builtin_fn\(ctx, &mut items, "[A-Za-z0-9_]+"' "$REG_SRC" \
    | sed 's/.*"\(.*\)"/\1/' | sort -u > "$tmp/registered"
nm -g "$RUNTIME_A" 2>/dev/null | awk '$2=="T"{print $3}' | sort -u > "$tmp/rtsyms"

n_reg=$(wc -l < "$tmp/registered")
n_sym=$(wc -l < "$tmp/rtsyms")

# Positive control: without it, an empty match set looks identical to a clean run.
if ! "$GREP" -qx 'println_str' "$tmp/registered" || ! "$GREP" -qx 'println_str' "$tmp/rtsyms"; then
    echo "builtin-parity: CONTROL FAILED -- println_str must appear in both sets." >&2
    echo "  registered=$n_reg runtime=$n_sym  (extraction is broken, not the tree)" >&2
    exit 2
fi

# A builtin is resolved if the runtime exports it directly, exports it under the
# blood_ prefix, or codegen/MIR references it by name (intrinsic lowering).
: > "$tmp/suspects"
while read -r n; do
    "$GREP" -qx "$n" "$tmp/rtsyms"        && continue
    "$GREP" -qx "blood_$n" "$tmp/rtsyms"  && continue
    if "$GREP" -ql "\"$n\"" "$REPO_ROOT"/src/selfhost/codegen*.blood \
                           "$REPO_ROOT"/src/selfhost/mir_*.blood 2>/dev/null; then continue; fi
    echo "$n" >> "$tmp/suspects"
done < "$tmp/registered"

"$GREP" -v '^#' "$BASELINE" | awk 'NF{print $1}' | sort -u > "$tmp/known"
sort -u "$tmp/suspects" > "$tmp/suspects_sorted"

new=$(comm -23 "$tmp/suspects_sorted" "$tmp/known")
gone=$(comm -13 "$tmp/suspects_sorted" "$tmp/known")
n_phantom=$("$GREP" -v '^#' "$BASELINE" | awk '$2=="PHANTOM"' | wc -l)

echo "builtin-parity: $n_reg registered, $n_sym runtime symbols, $(wc -l < "$tmp/suspects_sorted") unresolved"
echo "                $n_phantom known PHANTOM (typechecks, fails at link)"

rc=0
if [ -n "$new" ]; then
    echo
    echo "DRIFT: builtin(s) registered with no runtime symbol and no baseline entry:"
    printf '  %s\n' $new
    echo
    echo "Each either needs a runtime implementation, or must be rejected at typeck"
    echo "with a diagnostic. Probe it, then add it to tools/builtin-parity.baseline."
    rc=1
fi
if [ -n "$gone" ]; then
    echo
    echo "RESOLVED (baseline is now stale -- remove these entries):"
    printf '  %s\n' $gone
fi
exit $rc
