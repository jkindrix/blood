# Blood Compiler Bootstrap Makefile
#
# Key targets:
#   make build        - Build bootstrap compiler (cargo build --release)
#   make bootstrap    - Full bootstrap cycle (build + compile self-hosted)
#   make self-host    - Compile self-hosted compiler with itself
#   make test         - Run all tests (cargo test + ground-truth)
#   make ground-truth - Run ground-truth test suite only
#   make clean        - Clean build artifacts
#
# Configuration:
#   BLOOD_REPO  - Path to the blood language repo (default: ~/blood)
#   JOBS        - Parallel jobs for cargo (default: auto)

BLOOD_REPO  ?= $(dir $(abspath $(lastword $(MAKEFILE_LIST))))/..
BOOTSTRAP   := ./target/release/blood
SELF_HOSTED := $(BLOOD_REPO)/blood-std/std/compiler/main.blood
STDLIB_PATH := $(BLOOD_REPO)/blood-std/std

# Build the bootstrap compiler from Rust source
build:
	cargo build --release -p bloodc
	@echo "Bootstrap compiler built: $(BOOTSTRAP)"

# Stage 1: Compile the self-hosted compiler using the bootstrap compiler
bootstrap: build
	@echo "=== Stage 1: Compiling self-hosted compiler with bootstrap ==="
	@test -f "$(SELF_HOSTED)" || { echo "Error: $(SELF_HOSTED) not found. Set BLOOD_REPO."; exit 1; }
	$(BOOTSTRAP) build "$(SELF_HOSTED)" --stdlib-path "$(STDLIB_PATH)"
	@echo "Stage 1 complete: $(SELF_HOSTED:.blood=)"

# Stage 2: Compile the self-hosted compiler with itself
self-host: bootstrap
	@echo "=== Stage 2: Compiling self-hosted compiler with itself ==="
	$(SELF_HOSTED:.blood=) build "$(SELF_HOSTED)" --stdlib-path "$(STDLIB_PATH)"
	@echo "Stage 2 complete (self-hosted)."

# Run all tests: cargo tests + ground-truth suite
test: build
	cargo test --workspace
	../tests/ground-truth/run_tests.sh $(BOOTSTRAP)

# Run only the ground-truth test suite
ground-truth: build
	../tests/ground-truth/run_tests.sh $(BOOTSTRAP)

# Run only cargo tests
cargo-test:
	cargo test --workspace

# Clean all build artifacts
clean:
	cargo clean
	find . -name '*.blood_objs' -type d -exec rm -rf {} + 2>/dev/null || true
	@echo "Clean complete."

.PHONY: build bootstrap self-host test ground-truth cargo-test clean
