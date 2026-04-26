# Blood Compiler Implementation Status — RETIRED

This document is **retired**. Its body content was last refreshed
2026-01-29 (v0.5.3) and predates the deep audit of 2026-04-10 plus
the ~70 sessions of subsequent work — including the self-hosted Blood
compiler shipping (this file listed it as "Planned"), the libmprompt
vertical slice, and the eq-family soundness closures of S98–S103.

A retirement banner was added 2026-04-22 (commit `3a6e772`) but the
1450-line stale body remained in tree. This revision (S104-A) replaces
the body with a redirect-only doc. Anyone landing here from a search,
an old link, or a habit should follow the table below to the current
references.

## Where to look instead

| Where | What it covers |
|-------|----------------|
| **`docs/KNOWN_LIMITATIONS.md`** | Honest enumeration of gaps between the spec and the current compiler artifact. The canonical "what won't work today" reference. |
| **`docs/spec/`** | Normative spec hierarchy. The compiler conforms to the spec; when the two disagree, the spec is authoritative (CLAUDE.md §Spec-First Principle). |
| **`src/selfhost/COMPILER_NOTES.md`** | Divergences between the bootstrap (Rust) and self-hosted (Blood) compilers, plus selfhost-specific notes. |
| **`tools/FAILURE_LOG.md`** | Fixed compiler bugs, with root causes and resolutions. |
| **`docs/planning/SPEC_WORK_PLAN.md`** | Spec-coverage audit and remediation phases (still active). |
| **`proofs/PROOF_ROADMAP.md`** | Coq formalization status (273 theorems/lemmas, 219 proved). |

## Recovering historical content

Earlier revisions are preserved in git. Useful commands:

```bash
# List every commit that touched this file
git log --follow -- docs/planning/IMPLEMENTATION_STATUS.md

# Read the last content-bearing revision (before the retirement banner add)
git show $(git log --format=%H --follow -- docs/planning/IMPLEMENTATION_STATUS.md | sed -n '3p'):docs/planning/IMPLEMENTATION_STATUS.md
```

The 2026-01-29 revision documents Phase 0–5 of the original Rust-based
compiler architecture. It has historical interest but no operational use:
the project has since self-hosted, dropped the Rust bootstrap to
recovery-only status, completed the deep audit of 2026-04-10, and shipped
the soundness work tracked in `docs/KNOWN_LIMITATIONS.md`.
