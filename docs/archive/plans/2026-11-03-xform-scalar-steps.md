# Xform Scalar Steps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add six scalar bullet STEP operations using the existing fixed-order runtime.

**Architecture:** Extend the existing operation map, two-slot scratch and STEP interpreter. Preserve existing ordering and mode setters without conflict handling or new state.

**Tech Stack:** Existing Rust workspace and ECL compiler, integer fixed-point math only.

**Spec:** `docs/archive/specs/2026-11-03-xform-scalar-steps-design.md`

## Global Constraints

- New ops: 44 STEP_ANG_VEL, 45 STEP_ACCEL, 70 STEP_VX, 71 STEP_VY, 72 STEP_AX, 73 STEP_AY.
- Two physical slots per STEP; ENGINE_VER 28→29, no World layout changes.
- Fixed ascending slot writes; conflicting inputs unsupported. No cancellation, arbitration, queues or new state.
- Existing phase order, pure dynamic expressions and mode setter semantics remain intact.

## Review Focus

- Signed angular velocity must interpolate as a scalar, including across zero.
- Single-axis operations preserve the other vector component.
- Zero-duration STEP writes immediately; normal STEP starts ticking next frame.
- New operations participate in scratch sizing, dynamic easing validation and stage sampling.
- Existing speed/angle STEP behavior and unchanged content must continue to pass existing tests.

### Task 1: Runtime and compiler

**Files:** `crates/stg-core/src/{xform.rs,lib.rs,world/transform.rs,world/xform_bind.rs}`, `crates/stg-ecl-compiler/src/lang/{xform_map.rs,typeck/xforms.rs,codegen/xform.rs}`.

**Interfaces:** Six ECL operations `(target, timing)` lower to the op IDs above. Internal STEP classification is shared by ARITY validation and runtime dispatch; existing slot ABI stays unchanged.

- [x] Add a real compiler/runtime test and confirm unknown operations before implementation; add scalar behavior cases to existing transform tests.
- [x] Extend op constants/ARITY/implemented list, compiler names/types and runtime easing checks.
- [x] Extend STEP start/read/write and scalar interpolation; keep shortest-arc logic exclusive to STEP_ANGLE.
- [x] Run targeted tests, then existing core/compiler/harness/RL tests and fmt/clippy. No duplicate conflict tests.

### Task 2: Documentation and verification

**Files:** `docs/xform-ops.md`, `docs/ecl-lang/4-bullets.md`, `PROGRESS.md`, this plan.

- [x] Document new signatures, signed angular velocity, axis preservation, mode behavior and unsupported conflicts with an ECL example.
- [x] Run table verification and local golden; review final diff without expanding scope to unsupported combinations.
- [x] Record actual results and limitations; leave a reviewable local change without publishing or merging.

## Execution record

- User selected fixed slot-order writes and explicitly rejected conflict handling. This supersedes the earlier takeover proposal. Implementation performed inline on `codex/xform-scalar-steps`; no dependencies or World fields added.
- Before implementation: `cargo test -p stg-core world::transform::tests` passed all 31 existing cases. The new `cargo test -p stg-ecl-compiler xform_scalar_steps_compile_and_run_static_and_dynamic_targets` failed on unknown `step_ax`; after implementation it passed. Two added core behavior tests cover all six operations, signed angular velocity, nonlinear sampling, single-axis preservation, zero duration and exact final values.
- `cargo test -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl` ran core: 813 passed; only the existing version anchor failed on expected28/actual29. Updated the anchor and its explanation for the intentional new opcode contract; `cargo test -p stg-core engine_ver_anchored --quiet` passed. The behavior tests were not rerun unnecessarily after this test-only correction.
- `cargo test -p stg-ecl-compiler -p stg-harness -p stg-rl --quiet`: compiler 345 unit cases and 4/3/1 integration cases passed; harness 52/54 passed, with two local-port cases blocked by sandbox `PermissionDenied`. Approved sandbox-external `cargo test -p stg-harness viewer::tests:: --quiet` passed all six viewer cases.
- Since the combined command stopped at harness, `cargo test -p stg-rl --quiet` ran separately: all five suites passed (5/16/12/4/12). The existing frozen-op numbering assertion was extended for the new IDs and its targeted command passed.
- `cargo clippy -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings` passed. `cargo fmt --all -- --check` and `git diff --check` passed.
- `cargo run -p stg-harness -- verify-tables` passed. `cargo run -p stg-harness -- golden --out /tmp/sunyunbo/xform-scalar-steps-golden.txt` completed all three local scenarios. No cross-platform comparison, Godot runtime, wheel build or browser interaction was performed.
- A read-only independent review found no Important/Critical issues; unsupported conflict combinations were explicitly excluded. Initial delivery left changes local and uncommitted; the user subsequently authorized committing, pushing and merging into `main`. Existing `dist/` artifacts remain untouched.
