/*
 * p15_mprompt_handlers.c — Integration test for Blood's mprompt handler wrappers.
 *
 * Exercises the full handler pattern through Blood's rt_mprompt_shim API:
 *   Test 1: Basic yield + resume (resumable handler)
 *   Test 2: Abort (handler doesn't resume)
 *   Test 3: Nested handlers (two prompts, yield to inner)
 *   Test 4: Deep yield (yield from nested function calls)
 *   Test 5: Handler dispatch trampolines (blood_yield_to_handler)
 *
 * Returns 0 on success, negative code on failure.
 */

#include <stdint.h>
#include <stdio.h>

/* Declarations for Blood's mprompt shim (rt_mprompt_shim.c) */
extern void  blood_mprompt_init(void);
extern void* blood_prompt_create(void);
extern void* blood_prompt_enter(void* prompt, void* (*body_fn)(void*, void*), void* arg);
extern void* blood_prompt_run(void* (*body_fn)(void*, void*), void* arg);
extern void* blood_yield(void* prompt, void* (*yield_fn)(void*, void*), void* arg);
extern void* blood_resume(void* resume, void* arg);
extern void* blood_resume_tail(void* resume, void* arg);
extern void  blood_resume_drop(void* resume);
extern void* blood_yield_to_handler(
    void* prompt,
    int64_t (*handler_op_fn)(void* state, void* args, int64_t arg_count, int64_t cont),
    void* state, void* args, int64_t arg_count, int32_t is_abort
);


/* ========================================================================
 * Test 1: Basic yield + resume
 *
 * Pattern: body yields value 10, handler receives it, resumes with 42.
 * Body returns yielded + resumed = 52.
 * ======================================================================== */

static void* test1_yield_fn(void* resume, void* arg) {
    int64_t yield_val = (int64_t)(intptr_t)arg;
    if (yield_val != 10) return (void*)(intptr_t)(-100);
    /* Resume with 42 */
    return blood_resume(resume, (void*)(intptr_t)42);
}

static void* test1_body(void* prompt, void* arg) {
    /* Yield with value 10 */
    void* resumed = blood_yield(prompt, test1_yield_fn, (void*)(intptr_t)10);
    int64_t resumed_val = (int64_t)(intptr_t)resumed;
    /* Return 10 + resumed value */
    return (void*)(intptr_t)(10 + resumed_val);
}

static int64_t test1_basic_resume(void) {
    void* result = blood_prompt_run(test1_body, NULL);
    int64_t r = (int64_t)(intptr_t)result;
    if (r != 52) return -1;
    return 0;
}


/* ========================================================================
 * Test 2: Abort (handler doesn't resume)
 *
 * Pattern: body yields, handler drops continuation, returns abort value.
 * blood_prompt_run returns the abort value (99), not the body's return.
 * ======================================================================== */

static void* test2_yield_fn(void* resume, void* arg) {
    /* Abort: drop continuation, return abort value */
    blood_resume_drop(resume);
    return (void*)(intptr_t)99;
}

static void* test2_body(void* prompt, void* arg) {
    /* Yield — handler will abort, so this return is never reached */
    blood_yield(prompt, test2_yield_fn, NULL);
    /* Should NOT reach here */
    return (void*)(intptr_t)(-200);
}

static int64_t test2_abort(void) {
    void* result = blood_prompt_run(test2_body, NULL);
    int64_t r = (int64_t)(intptr_t)result;
    if (r != 99) return -2;
    return 0;
}


/* ========================================================================
 * Test 3: Nested handlers
 *
 * Pattern: outer prompt installs, inner prompt installs inside outer body.
 * Inner body yields to inner prompt. Outer body yields to outer prompt.
 * Both resume correctly.
 * ======================================================================== */

static void* outer_prompt_g = NULL;

static void* test3_inner_yield_fn(void* resume, void* arg) {
    /* Inner handler: resume with 100 */
    return blood_resume(resume, (void*)(intptr_t)100);
}

static void* test3_inner_body(void* inner_prompt, void* arg) {
    void* resumed = blood_yield(inner_prompt, test3_inner_yield_fn, (void*)(intptr_t)50);
    return resumed;  /* 100 */
}

static void* test3_outer_yield_fn(void* resume, void* arg) {
    /* Outer handler: resume with the value + 1000 */
    int64_t val = (int64_t)(intptr_t)arg;
    return blood_resume(resume, (void*)(intptr_t)(val + 1000));
}

static void* test3_outer_body(void* outer_prompt, void* arg) {
    outer_prompt_g = outer_prompt;

    /* Inner handler scope */
    void* inner_result = blood_prompt_run(test3_inner_body, NULL);
    int64_t inner_val = (int64_t)(intptr_t)inner_result;  /* 100 */

    /* Now yield to outer handler with inner result */
    void* outer_resumed = blood_yield(outer_prompt, test3_outer_yield_fn, (void*)(intptr_t)inner_val);
    int64_t outer_val = (int64_t)(intptr_t)outer_resumed;  /* 100 + 1000 = 1100 */

    return (void*)(intptr_t)outer_val;
}

static int64_t test3_nested(void) {
    void* result = blood_prompt_run(test3_outer_body, NULL);
    int64_t r = (int64_t)(intptr_t)result;
    if (r != 1100) return -3;
    return 0;
}


/* ========================================================================
 * Test 4: Deep yield (yield from nested function calls)
 *
 * Pattern: body calls helper1 → helper2 → yield.
 * The entire call chain is captured and restored on resume.
 * ======================================================================== */

static void* deep_prompt_g = NULL;

static int64_t test4_helper2(void) {
    /* Yield from deep in the call stack */
    void* resumed = blood_yield(deep_prompt_g, test1_yield_fn, (void*)(intptr_t)10);
    return (int64_t)(intptr_t)resumed;
}

static int64_t test4_helper1(void) {
    int64_t val = test4_helper2();
    return val + 1;  /* 42 + 1 = 43 */
}

static void* test4_body(void* prompt, void* arg) {
    deep_prompt_g = prompt;
    int64_t val = test4_helper1();
    return (void*)(intptr_t)val;  /* 43 */
}

static int64_t test4_deep_yield(void) {
    void* result = blood_prompt_run(test4_body, NULL);
    int64_t r = (int64_t)(intptr_t)result;
    if (r != 43) return -4;
    return 0;
}


/* ========================================================================
 * Test 5: Handler dispatch trampolines (blood_yield_to_handler)
 *
 * Tests the high-level API that codegen will use: blood_yield_to_handler
 * with Blood's handler op calling convention.
 * ======================================================================== */

/* Simulated Blood handler op: resumable.
 * Receives cont as an opaque resume handle, calls blood_resume through it. */
static int64_t test5_resumable_op(void* state, void* args, int64_t arg_count, int64_t cont) {
    /* cont is the resume handle (mp_resume_t* cast to i64) */
    if (cont == 0) return -500;  /* should have a resume handle */
    int64_t* arg_arr = (int64_t*)args;
    int64_t input = arg_arr[0];  /* first arg */
    /* Resume with input * 2 */
    void* result = blood_resume((void*)(intptr_t)cont, (void*)(intptr_t)(input * 2));
    return (int64_t)(intptr_t)result;
}

/* Simulated Blood handler op: abort (never resumes). */
static int64_t test5_abort_op(void* state, void* args, int64_t arg_count, int64_t cont) {
    /* cont should be 0 (abort trampoline drops it) */
    if (cont != 0) return -501;
    return 777;  /* abort value */
}

static void* test5_prompt_g = NULL;

static void* test5_body_resume(void* prompt, void* arg) {
    test5_prompt_g = prompt;
    int64_t op_args[1] = { 21 };
    void* result = blood_yield_to_handler(
        prompt,
        test5_resumable_op,
        NULL,        /* state */
        (void*)op_args,
        1,           /* arg_count */
        0            /* is_abort = false */
    );
    int64_t r = (int64_t)(intptr_t)result;
    /* 21 * 2 = 42 */
    return (void*)(intptr_t)(r + 1);  /* 43 */
}

static void* test5_body_abort(void* prompt, void* arg) {
    test5_prompt_g = prompt;
    int64_t op_args[1] = { 0 };
    blood_yield_to_handler(
        prompt,
        test5_abort_op,
        NULL,
        (void*)op_args,
        1,
        1            /* is_abort = true */
    );
    /* Should NOT reach here */
    return (void*)(intptr_t)(-502);
}

static int64_t test5_dispatch_trampolines(void) {
    /* Test resumable dispatch */
    void* result1 = blood_prompt_run(test5_body_resume, NULL);
    int64_t r1 = (int64_t)(intptr_t)result1;
    if (r1 != 43) return -5;

    /* Test abort dispatch */
    void* result2 = blood_prompt_run(test5_body_abort, NULL);
    int64_t r2 = (int64_t)(intptr_t)result2;
    if (r2 != 777) return -6;

    return 0;
}


/* ========================================================================
 * Test 6: Separate create + enter (blood_prompt_create + blood_prompt_enter)
 *
 * Validates the two-step API that will be used by codegen (create prompt,
 * store in evidence, then enter).
 * ======================================================================== */

static void* test6_body(void* prompt, void* arg) {
    void* resumed = blood_yield(prompt, test1_yield_fn, (void*)(intptr_t)10);
    int64_t r = (int64_t)(intptr_t)resumed;
    return (void*)(intptr_t)(r + 8);  /* 42 + 8 = 50 */
}

static int64_t test6_separate_create_enter(void) {
    void* prompt = blood_prompt_create();
    void* result = blood_prompt_enter(prompt, test6_body, NULL);
    int64_t r = (int64_t)(intptr_t)result;
    if (r != 50) return -7;
    return 0;
}


/* ========================================================================
 * Entry point — called from Blood
 * ======================================================================== */

int64_t mprompt_handler_test(void) {
    int64_t rc;

    rc = test1_basic_resume();
    if (rc != 0) { fprintf(stderr, "FAIL: test1_basic_resume (%ld)\n", (long)rc); return rc; }

    rc = test2_abort();
    if (rc != 0) { fprintf(stderr, "FAIL: test2_abort (%ld)\n", (long)rc); return rc; }

    rc = test3_nested();
    if (rc != 0) { fprintf(stderr, "FAIL: test3_nested (%ld)\n", (long)rc); return rc; }

    rc = test4_deep_yield();
    if (rc != 0) { fprintf(stderr, "FAIL: test4_deep_yield (%ld)\n", (long)rc); return rc; }

    rc = test5_dispatch_trampolines();
    if (rc != 0) { fprintf(stderr, "FAIL: test5_dispatch_trampolines (%ld)\n", (long)rc); return rc; }

    rc = test6_separate_create_enter();
    if (rc != 0) { fprintf(stderr, "FAIL: test6_separate_create_enter (%ld)\n", (long)rc); return rc; }

    return 0;  /* all passed */
}
