// Minimal C runtime stub - real runtime provided by Rust blood-runtime
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <unistd.h>

// Forward declaration - provided by compiled Blood code
extern int blood_main(void);

// Forward declaration - provided by Rust blood-runtime
extern void blood_init_args(int argc, const char** argv);

// 64MB default stack size for compiled Blood programs
#define BLOOD_STACK_SIZE (64 * 1024 * 1024)

int main(int argc, const char** argv) {
    // The initial thread's stack is fixed at execve() time, so setrlimit()
    // alone won't help. If the stack is too small, raise the limit and
    // re-exec to get a new stack from the kernel.
    struct rlimit rl;
    if (getrlimit(RLIMIT_STACK, &rl) == 0 && rl.rlim_cur < BLOOD_STACK_SIZE) {
        rl.rlim_cur = BLOOD_STACK_SIZE;
        if (setrlimit(RLIMIT_STACK, &rl) == 0) {
            execv("/proc/self/exe", (char* const*)argv);
            // execv only returns on failure — fall through to normal execution
        }
    }

    // Initialize command-line arguments before calling Blood main
    blood_init_args(argc, argv);
    return blood_main();
}
