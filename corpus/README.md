# Blood corpus

Real programs written in Blood to push the language and compilers forward.

Every project targets specific compiler stress points. If something breaks, that's the
point — each failure is a concrete bug report with a minimal reproduction path.

This corpus is a regression signal the golden tests cannot provide. Goldens test the
compiler against what it already does; these test it against what someone actually tried
to write. The two link failures that seeded `run_corpus.sh` were green across all 713
goldens.

## Status

Measured, not remembered:

```bash
./run_corpus.sh          # compile every project with the current first_gen
```

Last measured 2026-09-16: **28 of 30 compile**. `brainfuck` and `sortbench` fail at link
against phantom builtins (`print_char`, `print_u64`) — see `tools/builtin-parity.sh`.

Do not hand-maintain a pass/fail table here. It will go stale, and a stale table is worse
than no table: the entry claiming "trait methods + effect annotations rejected — CRITICAL"
sat here for five months after the bug was fixed.

## Inherited claims, unverified (March 2026)

Carried over from the pre-rescue worklog. Each needs a probe before it is trusted or
discarded; none has been re-tested against the current compiler.

| Claim | Compiler | Source |
|---|---|---|
| Vec mutation in handler op body segfaults | Selfhost | effects |
| Vec index inside handler op body crashes codegen | Bootstrap | effects |
| Bootstrap default method codegen crash | Bootstrap | morse |
| `&mut T` does not coerce to `&T` | All | json |
| `read_line()` thread-local buffer aliasing | Runtime | guess |
| `read_int()` EOF indistinguishable from 0 | Runtime | guess |

Disproved on 2026-09-16: *"Trait methods + effect annotations rejected — CRITICAL"*. Traits
and effects compose; a trait method carrying `/ {Log}` dispatches through a deep handler
correctly.

### Fixed

| Bug | Fix | Source |
|---|---|---|
| Multi-param generic field types swapped | 3bd4abf + 2c10700 | collections, json |
| Multiple handler sites in one fn → LLVM crash | c923eb2 + 1cd24d5 | stresstest |
| Filter closure `\|x\| x > 10` produces wrong results | b415772 | pipeline |
| Trait dispatch emits undefined `@fn_4294967294` | 1cd24d5 | pipeline |
| Module import breaks handler type inference | 55914da | json, stresstest |
| CallFinallyClause missing abort labels | e11abb2 | sortbench |
| Perform divergence type mismatch | 990b73c | calc |
| `*out = val` dereference codegen | e11abb2 | sortbench |

## What we build and why

Projects are designed to stress the boundaries between features — not to validate what already works. Priorities:

1. **Multi-param generics** — `Entry<K, V>`, nested inside `Vec`, inside recursive enums
2. **Multi-module compilation** — `mod` imports with cross-module type inference
3. **Effect handler composition** — 3+ nested handlers, effects across recursive calls
4. **Recursive data structures** — `Box<T>`, self-referential enums, generic trees
5. **Programs at scale** — 500+ lines combining all of the above

## What we don't build

- Single-file utilities under 300 lines (diminishing returns)
- Programs that avoid generics/effects/traits
- Programs that work around known limitations instead of testing them

## Tracking

`WORKLOG.md` — chronological record of all projects, bugs found, and compiler status per generation.
