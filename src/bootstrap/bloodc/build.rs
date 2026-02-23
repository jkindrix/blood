//! Build script for bloodc.
//!
//! Computes a hash of codegen-related source files to detect when the code
//! generation logic changes. This hash is used to invalidate the build cache
//! when codegen changes would produce incompatible artifacts.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn main() {
    // Directories containing codegen-related source files
    let codegen_dirs = ["src/codegen", "src/mir"];

    // Collect all .rs files in codegen directories, sorted for determinism
    let mut files: BTreeSet<String> = BTreeSet::new();

    for dir in &codegen_dirs {
        collect_rs_files(Path::new(dir), &mut files);
    }

    // Compute combined hash of all file contents
    let mut hasher = blake3::Hasher::new();

    // Include ABI version in the hash as well
    // This is defined in src/codegen/mod.rs and should be bumped for intentional breaks
    hasher.update(b"CODEGEN_ABI_VERSION:1:");

    for file in &files {
        if let Ok(contents) = fs::read(file) {
            // Include filename to detect renames/moves
            hasher.update(file.as_bytes());
            hasher.update(&contents);
        }
    }

    let hash = hasher.finalize();
    // Use first 16 hex chars (64 bits) - enough to detect changes
    let hash_short = &hash.to_hex()[..16];

    println!("cargo:rustc-env=CODEGEN_HASH={}", hash_short);

    // Re-run if any codegen source file changes
    for file in &files {
        println!("cargo:rerun-if-changed={}", file);
    }

    // Also re-run if build.rs itself changes
    println!("cargo:rerun-if-changed=build.rs");
}

/// Recursively collect all .rs files in a directory.
fn collect_rs_files(dir: &Path, files: &mut BTreeSet<String>) {
    if !dir.exists() {
        return;
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                if let Some(path_str) = path.to_str() {
                    files.insert(path_str.to_string());
                }
            }
        }
    }
}
