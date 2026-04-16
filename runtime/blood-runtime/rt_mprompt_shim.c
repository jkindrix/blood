/*
 * rt_mprompt_shim.c — C trampoline for libmprompt integration.
 *
 * Blood can't emit C-compatible function pointer callbacks directly.
 * This shim wraps libmprompt's multi-prompt API with Blood-callable
 * extern functions. All pointers are void* for ABI simplicity.
 *
 * Layers:
 *   1. Core API (blood_prompt_*, blood_yield, blood_resume_*)
 *      Thin wrappers around mp_prompt/mp_yield/mp_resume.
 *
 *   2. Handler dispatch trampolines (blood_dispatch_*)
 *      Generic yield functions that bridge mprompt's callback API
 *      to Blood's handler op calling convention. Used by codegen
 *      in Step 3 — tested here via C integration tests.
 *
 * Build:
 *   clang-18 -c -O2 -fPIC \
 *     -I vendor/libmprompt/include \
 *     runtime/blood-runtime/rt_mprompt_shim.c \
 *     -o build/rt_mprompt_shim.o
 *
 * Link (alongside runtime):
 *   clang-18 ... build/rt_mprompt_shim.o vendor/libmprompt/build/libmprompt.a ...
 *
 * Copyright: follows Blood project license.
 */

#include "mprompt.h"
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

/* ========================================================================
 * 1. Initialization
 * ======================================================================== */

static bool blood_mprompt_initialized = false;

/*
 * Auto-initialize libmprompt on program startup.
 *
 * Constructor priority 200 runs before Blood's handler registration
 * (which uses default priority = 65535). This ensures gstacks are
 * ready before any effect handler can fire.
 */
__attribute__((constructor(200)))
static void blood_mprompt_auto_init(void) {
    if (!blood_mprompt_initialized) {
        mp_config_t cfg = mp_config_default();
        /*
         * Use overcommit on Linux: reserves virtual address space for
         * gstacks without committing physical pages until touched.
         * Trades off gpool reuse for simpler address-space management.
         * Appropriate for Blood's single-threaded model.
         */
        cfg.stack_use_overcommit = true;
        mp_init(&cfg);
        blood_mprompt_initialized = true;
    }
}

/* Explicit initialization — callable from Blood if startup ordering requires it. */
void blood_mprompt_init(void) {
    blood_mprompt_auto_init();
}


/* ========================================================================
 * 2. Core prompt/yield/resume API
 *
 * These are the low-level building blocks. Codegen (Step 3) will emit
 * calls to these functions for NTR handler scopes.
 * ======================================================================== */

/*
 * Create a prompt marker without entering it.
 *
 * Used when the prompt needs to be stored (e.g., in the evidence vector)
 * before the body starts executing.
 *
 * Returns: opaque prompt pointer (mp_prompt_t*)
 */
void* blood_prompt_create(void) {
    return (void*)mp_prompt_create();
}

/*
 * Enter a prompt: run body_fn(prompt, arg) on a fresh gstack.
 *
 * Returns when:
 *   - body_fn returns normally → returns body_fn's return value
 *   - A handler aborts (mp_yield without resume) → returns the abort value
 *
 * @param prompt   Prompt marker from blood_prompt_create()
 * @param body_fn  Function to run on the new gstack.
 *                 Signature: void* (*)(mp_prompt_t* prompt, void* arg)
 * @param arg      Argument passed to body_fn
 * @return         body_fn's return value or abort value
 */
void* blood_prompt_enter(void* prompt, void* (*body_fn)(void*, void*), void* arg) {
    return mp_prompt_enter((mp_prompt_t*)prompt, (mp_start_fun_t*)body_fn, arg);
}

/*
 * Combined create + enter: creates a prompt and runs body_fn on a fresh gstack.
 * Convenience for cases where the prompt doesn't need to be stored beforehand.
 */
void* blood_prompt_run(void* (*body_fn)(void*, void*), void* arg) {
    return mp_prompt((mp_start_fun_t*)body_fn, arg);
}

/*
 * Yield to a prompt: suspends the current gstack and calls
 * yield_fn(resume, arg) on the prompt's parent stack.
 *
 * The entire call chain between the matching blood_prompt_enter and this
 * call is captured as a continuation (the resume handle).
 *
 * @param prompt    Target prompt marker (identifies which handler to yield to)
 * @param yield_fn  Function to run on the parent stack.
 *                  Signature: void* (*)(mp_resume_t* resume, void* arg)
 * @param arg       Argument passed to yield_fn
 * @return          Value passed to blood_resume() when the body is resumed
 */
void* blood_yield(void* prompt, void* (*yield_fn)(void*, void*), void* arg) {
    return mp_yield((mp_prompt_t*)prompt, (mp_yield_fun_t*)yield_fn, arg);
}

/*
 * Resume a suspended continuation with a value.
 *
 * Single-shot: the resume handle is consumed. Calling twice panics
 * (libmprompt enforces this).
 *
 * @param resume  Resume handle from mp_yield's yield_fn
 * @param arg     Value to return from blood_yield in the body
 * @return        Value returned by the body (if it returns to this prompt)
 */
void* blood_resume(void* resume, void* arg) {
    return mp_resume((mp_resume_t*)resume, arg);
}

/*
 * Resume in tail position: used when resume is the last action in a yield_fn.
 * Optimizes away an extra stack frame.
 */
void* blood_resume_tail(void* resume, void* arg) {
    return mp_resume_tail((mp_resume_t*)resume, arg);
}

/*
 * Drop a continuation without resuming.
 *
 * Used for abort-style handlers (Cancel, Error, StaleReference) that
 * never continue the body. The body's gstack is freed.
 */
void blood_resume_drop(void* resume) {
    mp_resume_drop((mp_resume_t*)resume);
}


/* ========================================================================
 * 3. Handler dispatch trampolines
 *
 * These are generic yield functions passed to mp_yield by the codegen.
 * They bridge mprompt's (resume, arg) callback convention to Blood's
 * handler op calling convention:
 *
 *   Blood handler op: i64 (*)(ptr state, ptr args, i64 arg_count, i64 cont)
 *   mprompt yield fn: void* (*)(mp_resume_t* resume, void* arg)
 *
 * The environment struct packs the Blood handler info so the trampoline
 * can unpack and dispatch.
 * ======================================================================== */

/*
 * Environment passed from the perform site to the yield trampoline.
 * Allocated on the body's gstack (before mp_yield), so it's valid
 * when the trampoline reads it on the parent stack.
 *
 * For abort trampolines: the env is copied out before mp_resume_drop
 * destroys the gstack (following mpeff.c's pattern).
 */
typedef struct {
    int64_t (*handler_op_fn)(void* state, void* args, int64_t arg_count, int64_t cont);
    void*   state;
    void*   args;
    int64_t arg_count;
} blood_yield_env_t;

/*
 * Resumable dispatch: handler op receives the resume handle (as i64)
 * and can call blood_resume() through it.
 *
 * Used for: MPE_OP_SCOPED_ONCE, MPE_OP_ONCE (in mpeff terms)
 * Blood equivalent: NTR handlers that may resume.
 */
static void* blood_dispatch_resumable(mp_resume_t* resume, void* earg) {
    blood_yield_env_t* env = (blood_yield_env_t*)earg;
    int64_t result = env->handler_op_fn(
        env->state, env->args, env->arg_count, (int64_t)(intptr_t)resume
    );
    return (void*)(intptr_t)result;
}

/*
 * Abort dispatch: drops the continuation immediately, then calls
 * handler op with cont=0 (handler cannot resume).
 *
 * IMPORTANT: copies env before mp_resume_drop because drop destroys
 * the gstack where env lives (same pattern as mpeff.c line 294).
 *
 * Used for: MPE_OP_ABORT, MPE_OP_NEVER (in mpeff terms)
 * Blood equivalent: Cancel, Error, StaleReference handlers.
 */
static void* blood_dispatch_abort(mp_resume_t* resume, void* earg) {
    blood_yield_env_t env_copy;
    memcpy(&env_copy, earg, sizeof(blood_yield_env_t));
    mp_resume_drop(resume);
    int64_t result = env_copy.handler_op_fn(
        env_copy.state, env_copy.args, env_copy.arg_count, 0
    );
    return (void*)(intptr_t)result;
}

/*
 * High-level yield-to-handler: packs the Blood handler info and yields
 * through the appropriate trampoline.
 *
 * This is what blood_perform_ntr() in the runtime will call (Step 3).
 *
 * @param prompt          Target prompt (from evidence vector)
 * @param handler_op_fn   Blood handler op function pointer
 * @param state           Handler state pointer
 * @param args            Packed argument array
 * @param arg_count       Number of arguments
 * @param is_abort        true → handler never resumes (use abort trampoline)
 * @return                Handler's return value (i64 cast to void*)
 */
void* blood_yield_to_handler(
    void* prompt,
    int64_t (*handler_op_fn)(void* state, void* args, int64_t arg_count, int64_t cont),
    void* state,
    void* args,
    int64_t arg_count,
    int32_t is_abort
) {
    blood_yield_env_t env = { handler_op_fn, state, args, arg_count };
    if (is_abort) {
        return mp_yield(
            (mp_prompt_t*)prompt,
            (mp_yield_fun_t*)&blood_dispatch_abort,
            &env
        );
    } else {
        return mp_yield(
            (mp_prompt_t*)prompt,
            (mp_yield_fun_t*)&blood_dispatch_resumable,
            &env
        );
    }
}
