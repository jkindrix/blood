# Rust Prototype Standard Library

These files were written in Rust syntax during the bootstrap phase and have not been ported to Blood syntax. They represent design intent for the standard library but cannot compile with the Blood compiler.

**Common issues:**
- `::` path separator (Blood uses `.`)
- `Vec::new()` (should be `Vec.new()`)
- `if let` / `while let` patterns (not implemented in Blood)
- `use crate::` imports (Blood uses `mod` + `use`)
- `T::default()` associated function calls

**To port a file:** Replace `::` with `.`, replace Rust-specific patterns with Blood equivalents, verify with `first_gen check`. See the files remaining in `stdlib/` for examples of working Blood code.

**Not needed for compilation:** The selfhost compiler has its own implementations of HashMap, Vec, String, and other core types built into the runtime. The stdlib is purely for user programs.
