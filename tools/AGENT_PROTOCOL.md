# Agent Convergence Guardrails

**Purpose:** Prevent AI agent sessions from entering unproductive loops, ensure incremental progress is captured, and provide clear stopping criteria.

---

## Time-Box Rules

| Phase | Max Duration | Action on Timeout |
|-------|-------------|-------------------|
| Investigation & planning | 15 minutes | Commit findings, propose plan |
| Single feature implementation | 30 minutes | Commit progress, reassess |
| Bug investigation | 20 minutes | Log findings to FAILURE_LOG.md |
| Full ground-truth test run | 10 minutes | If stuck, check env vars and paths |

**Wall-clock awareness:** AI agents lack clocks. Use these proxies:
- **3+ failed attempts** at the same approach = time to reassess
- **5+ tool calls** without measurable progress = stop and log state
- **Compilation loop** (edit → compile → same error → edit) repeating 3+ times = step back and analyze

---

## Mandatory Commit Intervals

### Commit Triggers

Commit immediately after:
1. Any test count improvement (even +1 pass)
2. Completing a self-contained code change
3. Fixing a bug (with description of root cause)
4. Before switching to a different subsystem
5. Before any speculative or experimental change

### Commit Message Requirements

```
<type>: <concise description> (<score>/<total>)

- What changed and why
- Which tests are affected
```

**Type prefixes:** `fix:`, `feat:`, `refactor:`, `test:`, `docs:`

### What NOT to Commit

- Incomplete changes that break compilation
- Debug prints or temporary scaffolding
- Speculative changes that haven't been tested

---

## Stop-and-Yield Criteria

**Stop working and yield to the user when ANY of these occur:**

### Hard Stops

1. **Same error 3 times:** If the same compilation error or test failure persists after 3 different fix attempts, stop. The approach is wrong.

2. **Score regression:** If ground-truth score drops below the saved baseline, immediately:
   - Run `./tools/track-regression.sh` to identify regressed tests
   - Revert the offending change
   - Log the regression in FAILURE_LOG.md
   - Yield with analysis

3. **Segfault in self-compilation:** Don't debug blindly. Run:
   ```bash
   ./tools/asan-selfcompile.sh --test "./first_gen check test.blood"
   ```
   If ASan doesn't help within 2 attempts, yield.

4. **Unknown feature required:** If a fix requires implementing a feature that doesn't exist in first_gen (closures, effect handlers, etc.), stop. These are tracked in TASKS.md of the main repo.

### Soft Stops

1. **Diminishing returns:** If 3+ consecutive changes each improve score by only +1, consider yielding with a summary of remaining failures.

2. **Yak shaving detected:** If fixing test A requires fixing B which requires fixing C, stop at the second dependency. Log the chain and yield.

3. **Scope creep:** If the current task has expanded beyond the original goal, commit what's done and yield with a scope assessment.

---

## Progress Reporting

### Session Start Protocol

At the beginning of every agent session:
1. Run `./tools/track-regression.sh --show` to see current baseline
2. Run `./tools/track-regression.sh` to verify no pre-existing regressions
3. Read `tools/FAILURE_LOG.md` active issues section
4. State the session goal explicitly

### Session End Protocol

Before ending a session:
1. Commit all changes
2. Run `./tools/track-regression.sh` and report any delta
3. If score improved, run `./tools/track-regression.sh --save` to update baseline
4. Update `tools/FAILURE_LOG.md` if any issues were resolved or discovered
5. Provide a summary:
   ```
   ## Session Summary
   Goal: <what was attempted>
   Result: <what was achieved>
   Score: <before> → <after> (delta: <+/-N>)
   Commits: <list of commits>
   Remaining: <next steps>
   ```

### Mid-Session Checkpoints

After every significant change:
1. Run `./tools/track-regression.sh` (compare, don't save)
2. If regressions: revert and reassess
3. If improvements: commit and optionally save baseline

---

## Tool Usage Requirements

### Before Changing Codegen

```bash
# 1. Check current baseline
./tools/track-regression.sh --show

# 2. Make your change

# 3. Rebuild first_gen
cd blood-std/std/compiler && blood build main.blood --no-cache
cp main first_gen

# 4. Check for regressions
./tools/track-regression.sh

# 5. If no regressions, commit and save
git add -A && git commit -m "fix: description (N/317)"
./tools/track-regression.sh --save
```

### When Investigating a Failure

```bash
# 1. Identify which phase diverges
./tools/phase-compare.sh path/to/failing_test.blood

# 2. If behavior diverges, compare outputs
./tools/difftest.sh path/to/failing_test.blood

# 3. If compile fails, minimize the test case
./tools/minimize.sh path/to/failing_test.blood

# 4. If crash, check memory safety
./tools/asan-selfcompile.sh --test "./first_gen build test.blood"
```

### When Suspecting Memory Issues

```bash
# Compare memory usage between compilers
./tools/memprofile.sh --compare path/to/test.blood

# If test compiler uses >2x reference memory, investigate
./tools/memprofile.sh --sample path/to/test.blood
```

---

## Failure Log Requirements

### When to Update FAILURE_LOG.md

Update the failure log when:
- A new bug is discovered (add to Active Issues)
- A bug is fixed (move to Resolved Issues table)
- A new root cause pattern is identified (add to Common Root Causes)

### Entry Format

```markdown
### ISSUE-NNN: Short description

**Severity:** critical | high | medium | low
**Symptom:** What the user/test observes
**Root cause:** (if known) Technical explanation
**Affected tests:** List of ground-truth tests
**Status:** investigating | workaround | fixed
```

---

## Anti-Patterns to Avoid

1. **The Shotgun Approach:** Making multiple changes at once, then running tests. Make ONE change, test, commit.

2. **The Hope Strategy:** "This might fix it" without understanding why. Understand the root cause first.

3. **The Rabbit Hole:** Spending an entire session on one obscure test when other easier wins are available. Prioritize by impact.

4. **The Silent Regression:** Making a change that fixes 1 test but breaks 2 others. Always run the full regression tracker.

5. **The Uncommitted Marathon:** Working for an extended period without committing. Commit every improvement.

6. **The Config Amnesia:** Forgetting to set `BLOOD_RUNTIME` and `BLOOD_RUST_RUNTIME` before testing. The tools handle this, but manual testing requires it.
