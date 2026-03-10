# Dispatch Gap Tests

Tests that pass with the bootstrap compiler (blood-rust) but fail with the selfhost (first_gen).
These document the DIS-01 cluster gap — the selfhost's numeric specificity dispatch vs the
spec's pairwise subtype-based dispatch.

**Purpose:** After DIS-01 is implemented, move these into `tests/golden/` as regression tests.

**To run with bootstrap:**
```bash
for f in tests/dispatch/*.blood; do
    src/bootstrap/target/release/blood run "$f" 2>&1 | tail -3
done
```

## Test Coverage

| Test | Dispatch Feature | Why Selfhost Fails |
|------|-----------------|-------------------|
| generic_multi | Arity-based overloading | Selfhost picks wrong overload |
| generic_return | Generic method with `T` return | Selfhost compile failure |
| generic_vs_concrete | Concrete > generic specificity | Selfhost picks wrong overload |
| mixed_specificity | Multi-param specificity | Selfhost picks wrong overload |
| option_param | Multiple param type overloads | Selfhost picks wrong overload |
| ref_vs_val | `&T` vs `T` dispatch | Selfhost compile failure |
| struct_param | Struct type discrimination | Selfhost picks wrong overload |
| three_overloads | 3+ overloads same method | Selfhost picks wrong overload |
| trait_impl | Trait dispatch on different types | Selfhost picks wrong overload |
