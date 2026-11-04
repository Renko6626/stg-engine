# xform rand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans inline. User has already approved the three-stage design and implementation. User subsequently authorized a stage commit and push on main (2026-11-04).

**Goal:** Support existing rand(n) in bind/spawn/effect xform expressions with deterministic shared RNG consumption.
**Architecture:** Add Rand to expression lowering/evaluation; use the existing World RNG and sampling state. No World layout changes.
**Tech Stack:** Existing Rust core/compiler and PyO3 wheel; no dependencies.
**Spec:** `docs/superpowers/specs/2026-11-04-xform-rand-design.md`

## Global Constraints

- Existing rand signature and n<=0 behavior, source evaluation order and short-circuiting.
- Bound values shared per binding; spawn values per bullet; effect values per operation entry.
- No new persistent state, RNG stream, feature branch/worktree, card migration or training. Commit/push on main subsequently authorized by the user.
- ENGINE_VER30; P4 no-op+counter and all existing capture/slot budgets retained.

## Review Focus

- Rand must not be folded even when its result is multiplied by zero.
- Multiple identical calls consume separate draws in source order.
- STEP/WAIT_SIGNAL holds and LOOP reentry retain the correct sampling cadence.
- Mutable sampling must not invalidate capture views or static xform behavior.
- Existing invalid-parameter handling and snapshot RNG continuation stay deterministic.

## Task 1: Compiler/runtime support

Files: `crates/stg-core/src/xform/expr.rs`, `world/xform_eval.rs`, `world/xform_bind.rs`, `world/xform_bind_tests.rs`; `crates/stg-ecl-compiler/src/lang/typeck/xforms.rs`, `lang/codegen/xform.rs`.
Interfaces: existing expression callback gains a bounded RNG request; existing World bind/spawn methods become mutable; no state format changes.

- [x] Add real compile+execute tests `xform_rand_samples_each_phase_on_the_shared_stream`, `xform_rand_preserves_call_order_short_circuit_and_nonpositive_bounds`, `xform_rand_step_and_signal_hold_samples_across_restore` to existing xform tests.
- [x] Run `cargo test -p stg-ecl-compiler xform_rand -- --nocapture`; expect rejection of rand before implementation.
- [x] Add runtime request/Unary Rand and mutable staged sampling; whitelist/lower rand and exclude both constant-folding paths; replace obsolete rand rejection fixture with a genuinely forbidden call.
- [x] Extend the existing LOOP reentry test with RNG consumption; rerun relevant xform tests and inspect actual failures before correcting.

## Task 2: Delivery and verification

Files: engine version/anchor, author docs, PROGRESS; training mapping/contracts/skill/memory, four wheel consumer files.

- [x] Capture baseline release golden, then bump ENGINE_VER30 and its anchor; document semantics and new example.
- [x] Run `cargo test -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl`, `cargo fmt --all -- --check`, `cargo clippy -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings`, `verify-tables` and golden comparison.
- [x] Build 0.4.4 from a recorded source snapshot+patch, synchronize training dependency/lock/Magnus wheel/hash; run existing targeted consumer checks and a real staged-random probe.
- [x] Refresh derived unit docs, update project memory and run one independent review of the complete scoped diff. Fix only concrete relevant findings; record checks and limitations here.

## Progress

- Plan/spec recorded after user approval; current engine main clean except pre-existing dist/.

- Task 1 complete: the three new compiler/runtime tests first failed specifically at the existing rand rejection, then passed; the existing LOOP test now checks two draws across reentry.
- Task 2 complete: all related Rust groups and doctests passed; fmt, scoped clippy and table verification passed. Three-scene golden before/after byte-equal, SHA256 `24d2e24f67c5f01e55029586fef81951ed6faee807b83889ce9a0831f16a3529`.
- Training adoption: 0.4.4/ENGINE_VER30, source patch SHA256 `d1bcbae87e51bd0878b50f145f4f3bb3510492c4372dd0cc0c6571d7e259f41b`, wheel SHA256 `86d661e6c1f5158afce6691adfe4108c3288d89ef7d222f499d20bb81d1fe631`; four consumer files agree. Six fixed-action replay cases byte-equal to 0.4.3; Python3.12 and Python3.11 VecEnv staged random values match ordinary ECL rand calls. Related training/transcription tests 158 passed, Python binding smoke 15 passed. Detailed commands and provenance are in the training repository `docs/practice-cards/reports/stg-rl-044-adoption.json`.
- Final review: independent read-only reviewer found no critical/important/minor issues and reran staged-rand and LOOP tests. Review scope: engine source/tests/author docs/version; consumer verification was performed by the implementer.
- Ruling: reuse the existing expression query callback for a RandRange request and copy up to eight captures to a temporary array; no new context framework or persistent state.
- Ruling: already-consumed draws remain consumed on later invalid parameters, matching ordinary argument evaluation; no transactional RNG recovery.
- Ruling: clippy covers the changed core/compiler/harness/RL packages and their targets; unrelated Godot work is excluded to respect verification scope.
- Not performed: cross-platform execution, Godot/GPU training, original-game dynamic comparisons, new throughput benchmarks, card migration, transcription dispatch, upstream release. User subsequently authorized stage commit/push on main; delivery identity is recorded in Git history. Pre-existing untracked dist/ retained. Temporary logs and source snapshot are in `/tmp/sunyunbo/stg-xform-rand-20261104/`.
