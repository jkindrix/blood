// Minimal C runtime stub - real runtime provided by Rust blood-runtime
#include <stdio.h>
#include <stdlib.h>

// Forward declaration - provided by compiled Blood code
extern int blood_main(void);

// Forward declaration - provided by Rust blood-runtime
extern void blood_init_args(int argc, const char** argv);

int main(int argc, const char** argv) {
    // Initialize command-line arguments before calling Blood main
    blood_init_args(argc, argv);
    return blood_main();
}
