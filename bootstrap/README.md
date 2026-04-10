# Bootstrap Seed Compiler

`seed` is a prebuilt Blood compiler binary (x86-64 Linux) used to bootstrap the self-hosted compiler from a clean checkout.

**How it was built:**
```
build blood_runtime
RUNTIME_A=runtime/blood-runtime/build/debug/libblood_runtime_blood.a build second_gen
RUNTIME_A=runtime/blood-runtime/build/debug/libblood_runtime_blood.a build third_gen
# Verified: second_gen = third_gen (byte-identical)
# Shipped: second_gen (compiled by the selfhost, 0 Rust symbols)
```

**Contents:**
- `seed` — the compiler binary (self-hosting fixed point, byte-identical when compiling itself)
- `libblood_runtime_blood.a` — the Blood runtime archive (needed for linking compiled programs)
- `seed.meta` — provenance metadata (commit, date, size, hash)

Both `seed` and `libblood_runtime_blood.a` are updated together by `./build_selfhost.sh gate`.

**Properties:**
- Self-hosting fixed point (compiling itself produces a byte-identical binary)
- Linked against Blood runtime (zero Rust dependencies)
- x86-64 Linux only

**To update the seed:**
```bash
cd src/selfhost
./build_selfhost.sh gate  # builds all, verifies, updates bootstrap/seed + runtime
```

Note: `RUNTIME_A` defaults to the Blood runtime (`libblood_runtime_blood.a`), so the above commands produce a Rust-free binary automatically. If `RUNTIME_A` is overridden to point at the Rust runtime, the seed would contain Rust symbols — don't do that.
