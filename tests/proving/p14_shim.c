/*
 * p14_shim.c — C callback shim for libmprompt smoke test.
 *
 * Blood can't create function pointers for mp_start_fun_t / mp_yield_fun_t
 * callbacks directly. This shim exercises the full prompt/yield/resume cycle
 * and exposes a single entry point callable from Blood.
 */
#include "mprompt.h"
#include <stdint.h>

static int64_t yielded_value = 0;
static int64_t resumed_value = 0;

/* Yield function: receives the resume handle, stores value, resumes */
static void* my_yield_fun(mp_resume_t* resume, void* arg) {
    int64_t yield_val = (int64_t)arg;
    yielded_value = yield_val;
    /* Resume the captured continuation with value 42 */
    void* result = mp_resume(resume, (void*)42);
    return result;
}

/* Start function: runs under a fresh prompt, yields once, returns */
static void* my_start_fun(mp_prompt_t* prompt, void* arg) {
    /* Yield with value 10 */
    void* resume_val = mp_yield(prompt, my_yield_fun, (void*)10);
    resumed_value = (int64_t)resume_val;
    /* Return yielded + resumed */
    return (void*)(yielded_value + resumed_value);
}

/* Called from Blood — exercises the full prompt/yield/resume cycle.
 * Returns 0 on success, negative error codes on failure. */
int64_t mprompt_smoke_test(void) {
    void* result = mp_prompt(my_start_fun, (void*)0);
    int64_t r = (int64_t)result;
    /* Expected: yielded_value=10, resumed_value=42, result=52 */
    if (yielded_value != 10) return -1;
    if (resumed_value != 42) return -2;
    if (r != 52) return -3;
    return 0;  /* success */
}
