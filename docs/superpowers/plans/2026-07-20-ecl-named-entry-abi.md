# ECL Named Entry ABI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace declaration-order ECL script IDs with a compact, canonical named-entry ABI: one singleton `main` root, named async entries, call-only subs, typed arguments, and optional debug symbols outside the runtime image.

**Architecture:** `stg-core` owns a compact immutable `EclImage` made from contiguous code, sub metadata, parameter-type, and public-entry-name tables. `stg-ecl-compiler` assigns `SubId`/`EntryId` by lexical name order, emits `CALL`/`SPAWN` operands as canonical `SubId`, and optionally returns an `EclDebugSymbols` sidecar; `World` exposes safe root/named-entry startup APIs and snapshots the one-shot main state.

**Tech Stack:** Rust 2024, `stg-core`, `stg-ecl-compiler`, `stg-harness`, hand-written ECL frontend, stack VM, Cargo test/Clippy, deterministic golden harness.

## Global Constraints

- One non-empty `.ecl` compilation unit represents one stage and must contain exactly one zero-argument `sub main()`.
- `main` is `Root`, every `async sub` is a public `Async` entry, and every other sub is `CallOnly`.
- `main` can only be started by `World::start_main`; one `World` can successfully start it once, even if the task later ends or faults.
- All sub names share the existing case-sensitive ASCII identifier namespace.
- Names are the persistent identity. Numeric `SubId`/`EntryId` values are canonical image-local indexes and must never be persisted.
- `SubId` is assigned by all-sub lexical name order; `EntryId` is assigned by async-entry lexical name order.
- `CALL`, `SPAWN`, and `fire(..., task)` encode canonical `SubId`; only `JMP/JZ` retain absolute code-PC operands.
- Runtime `EclImage` keeps only public async names in one contiguous byte pool and keeps parameter types in one contiguous table; it has no per-sub/per-param `String` or `Vec` allocation.
- `DebugInfo::None` and `DebugInfo::Full` must produce byte-for-byte equal `EclImage` values. Full names and source locations live in an optional compiler-side `EclDebugSymbols` sidecar.
- `EclImage` remains outside `World`, rollback snapshots, and per-frame checksums. `World` stores only `ecl_main_started` and task state.
- `EclImage::empty()` remains the only image without a root.
- External file serialization, complete untrusted-bytecode validation, true content hashing, and engine/schema negotiation remain C11 work.
- Preserve the user's existing `README.md` edit and untracked `lang/const_eval.rs` / `lang/type_rules.rs`; do not stage them.

---

### Task 1: Enforce the ECL root-entry language contract

**Files:**
- Create: `crates/stg-ecl-compiler/src/lang/entryck.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/tests.rs`
- Test: `crates/stg-ecl-compiler/src/lang/mod.rs`

**Interfaces:**
- Consumes: parsed `Program { subs: Vec<SubDef>, .. }` and existing positioned `CompileError`.
- Produces: `entryck::check(&Program) -> Result<(), Vec<CompileError>>`; `lang::compile` invokes it after parse and before typecheck.
- Produces: typecheck rejects synchronous calls to the reserved root name `main`; existing async/call-only checks continue handling `spawn` and `fire` targets.

- [ ] **Step 1: Add failing full-pipeline tests for the root contract**

Add these tests to `lang/mod.rs`:

```rust
#[test]
fn compile_requires_exact_zero_arg_plain_main() {
    let cases = [
        ("async sub worker() {}", "缺少唯一根入口 'sub main()'"),
        ("async sub main() {}", "main 不能声明为 async"),
        ("sub main(x: int) {}", "main 必须是零参数"),
        ("sub main() {} sub main() {}", "sub 名称 'main' 重复"),
    ];
    for (src, needle) in cases {
        let errors = expect_compile_err(src, "root.ecl");
        assert!(errors.iter().any(|e| e.msg.contains(needle)), "{errors:?}");
    }
}

#[test]
fn compile_rejects_calling_main() {
    let errors = expect_compile_err(
        "sub main() {} sub helper() { main(); }",
        "root.ecl",
    );
    assert!(errors.iter().any(|e| e.msg.contains("main 只能作为关卡根入口启动")));
}
```

- [ ] **Step 2: Run the root tests and verify red**

Run:

```bash
cargo test -p stg-ecl-compiler lang::tests::compile_requires_exact_zero_arg_plain_main -- --exact
cargo test -p stg-ecl-compiler lang::tests::compile_rejects_calling_main -- --exact
```

Expected: both tests fail because `entryck` and the reserved-main call check do not exist.

- [ ] **Step 3: Implement the focused entry-contract pass**

Create `lang/entryck.rs` with this responsibility and shape:

```rust
use super::ast::{CompileError, Program, Span};

fn err(span: Span, msg: impl Into<String>) -> CompileError {
    CompileError {
        line: span.line,
        col: span.col,
        msg: msg.into(),
        src_line: String::new(),
    }
}

pub(super) fn check(prog: &Program) -> Result<(), Vec<CompileError>> {
    let mains: Vec<_> = prog.subs.iter().filter(|sub| sub.name == "main").collect();
    if mains.is_empty() {
        let span = prog.subs.first().map_or(Span { line: 1, col: 1 }, |sub| sub.span);
        return Err(vec![err(span, "缺少唯一根入口 'sub main()'")]);
    }
    if mains.len() != 1 {
        return Err(vec![err(mains[1].span, "sub 名称 'main' 重复")]);
    }
    let main = mains[0];
    let mut errors = Vec::new();
    if main.is_async {
        errors.push(err(main.span, "main 不能声明为 async；请写 'sub main()'"));
    }
    if !main.params.is_empty() {
        errors.push(err(main.span, "main 必须是零参数根入口"));
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
```

Register `mod entryck;` in `lang/mod.rs`, call it immediately after `parse`, and pass errors through `attach_src_lines`. In `typeck/exprs.rs::check_call`, add a `name == "main"` branch before ordinary sub-call checking that emits `main 只能作为关卡根入口启动，不能同步调用`.

- [ ] **Step 4: Add focused typecheck coverage for main references**

Add to `typeck/tests.rs`:

```rust
#[test]
fn main_cannot_be_a_sync_call_target() {
    let errors = check_err("sub main() {} sub helper() { main(); }");
    assert!(errors.iter().any(|e| e.msg.contains("main 只能作为关卡根入口启动")));
}

#[test]
fn main_cannot_be_spawned() {
    let errors = check_err("sub main() { spawn main(); }");
    assert!(errors.iter().any(|e| e.msg.contains("spawn 目标 'main' 必须声明为 async")));
}
```

- [ ] **Step 5: Run compiler tests and commit**

Run:

```bash
cargo test -p stg-ecl-compiler
```

Expected: all compiler unit tests and doctests pass. Parser/typecheck unit tests that intentionally exercise fragments remain usable because the mandatory-main pass runs only from the full `compile` pipeline.

Commit:

```bash
git add crates/stg-ecl-compiler/src/lang/entryck.rs \
  crates/stg-ecl-compiler/src/lang/mod.rs \
  crates/stg-ecl-compiler/src/lang/typeck/exprs.rs \
  crates/stg-ecl-compiler/src/lang/typeck/tests.rs
git commit -m "feat(lang): enforce singleton zero-arg main root"
```

---

### Task 2: Replace the flat script table with the compact canonical runtime image

**Files:**
- Modify: `crates/stg-core/src/ecl/image.rs`
- Modify: `crates/stg-core/src/ecl/task.rs`
- Modify: `crates/stg-core/src/ecl/vm.rs`
- Modify: `crates/stg-core/src/ecl/syscall.rs`
- Modify: `crates/stg-core/src/ecl/mod.rs`
- Modify: `crates/stg-core/src/step.rs`
- Modify: `crates/stg-ecl-compiler/src/lib.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`
- Modify: `crates/stg-harness/src/main.rs`
- Modify: `docs/ecl-ops.md`
- Test: the inline test modules in all files above

**Interfaces:**
- Produces in `stg-core::ecl::image`: `SubId`, `EntryId`, `ResolvedEntry`, `EclValueType`, `SubKind`, `RuntimeSubMeta`, `ImageParts`, `SubInit`, `EntryInit`, `ImageBuildError`, and compact `EclImage` query APIs.
- Produces in `stg-ecl-compiler`: opaque `BuilderSubRef`, two-stage `ImageBuilder::declare_sub` / `define_sub`, and `ImageBuilder::build() -> Result<EclImage, ImageBuildError>`.
- Changes bytecode ABI: `OP_CALL`, `OP_SPAWN`, and `fire` task references carry canonical `SubId` values.
- Temporarily retains a crate-visible task-root spawn helper for core/compiler tests; Task 3 replaces external startup with the final safe binding API.

- [ ] **Step 1: Write failing compact-image tests**

Replace/add tests in `ecl/image.rs` that pin the public contract:

```rust
#[test]
fn image_freezes_compact_tables_and_resolves_async_names() {
    let image = EclImage::try_from_parts(ImageParts {
        code: vec![0, 0, 0],
        subs: vec![
            SubInit::new(0, SubKind::Async, vec![EclValueType::Fx]),
            SubInit::new(1, SubKind::Root, vec![]),
            SubInit::new(2, SubKind::CallOnly, vec![]),
        ],
        entries: vec![EntryInit::new("bullet_task", 0)],
        root: Some(1),
        content_hash: 0,
    }).unwrap();

    assert_eq!(image.code(), &[0, 0, 0]);
    assert_eq!(image.resolve_entry("bullet_task").unwrap().id().get(), 0);
    assert_eq!(image.resolve_entry("main"), Err(ResolveError::RootRequiresStartMain));
    assert_eq!(image.resolve_entry("helper"), Err(ResolveError::UnknownEntry));
    assert_eq!(image.sub_meta(image.root().unwrap()).unwrap().kind(), SubKind::Root);
}

#[test]
fn runtime_records_are_compact() {
    assert_eq!(std::mem::size_of::<RuntimeSubMeta>(), 8);
    assert_eq!(std::mem::size_of::<RuntimeEntryMeta>(), 8);
}
```

Add rejection tests for duplicate/unsorted entry names, non-ASCII or invalid ECL identifiers, missing
root, root parameters, wrong entry kind, parameter-table overflow, sub count above the `u16`
domain, and `code_entry >= code.len()`. `EntryInit` supplies owned names rather than raw offsets, so
out-of-range `NameRef` values are made impossible by this API; checked pool construction still
guards `u32` offset and `u16` name-length overflow.

Pin the numeric domains in those tests: `SubId` and `EntryId` admit indexes `0..=u16::MAX`, so at
most 65,536 subs and 65,536 entries are valid. A non-empty parameter range must have a start that
fits `u16` and an exclusive end no greater than 65,536; a zero-length range stores start `0`.
`EclImage::try_from_parts` accepts the exact all-empty sentinel shape, but any other image without
one Root is `ImageBuildError::MissingRoot`.

- [ ] **Step 2: Run the image tests and verify red**

Run:

```bash
cargo test -p stg-core ecl::image::tests -- --nocapture
```

Expected: compilation fails because the compact types and constructor are not defined.

- [ ] **Step 3: Implement the compact immutable image**

Replace the public `Vec<u32>` fields with the following exact runtime layout (private fields unless marked public by an accessor):

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, crate::checksum::Checksum)]
pub struct SubId(u16);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedEntry<'a> {
    image: &'a EclImage,
    id: EntryId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    RootRequiresStartMain,
    UnknownEntry,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclValueType { Int = 0, Fx = 1, Angle = 2 }

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubKind { Root = 0, Async = 1, CallOnly = 2 }

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeSubMeta {
    code_entry: u32,
    param_start: u16,
    param_count: u8,
    kind: SubKind,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeEntryMeta {
    name_offset: u32,
    name_len: u16,
    sub: SubId,
}
```

Derive `Debug`, `PartialEq`, and `Eq` for `EclImage`, so the equality and error assertions in this
plan compile.
Use these construction signatures; raw indexes remain confined to validated image construction:

```rust
impl SubInit {
    pub fn new(code_entry: u32, kind: SubKind, params: Vec<EclValueType>) -> Self;
}
impl EntryInit {
    pub fn new(name: impl Into<String>, sub: u16) -> Self;
}
pub struct ImageParts {
    pub code: Vec<u32>,
    pub subs: Vec<SubInit>,
    pub entries: Vec<EntryInit>,
    pub root: Option<u16>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageBuildError {
    TooManySubs { actual: usize },
    TooManyEntries { actual: usize },
    CodeTooLong { words: usize },
    NamePoolTooLarge { bytes: usize },
    NameTooLong { name: String, bytes: usize },
    InvalidEntryName { name: String },
    EntriesNotStrictlySorted,
    MissingRoot,
    MultipleRoots,
    RootIndexMismatch,
    RootHasParameters,
    CodeEntryOutOfRange { sub: usize, code_entry: u32 },
    TooManyParameters { sub: usize, actual: usize },
    ParameterTableTooLarge { actual: usize },
    EntrySubOutOfRange { entry: usize, sub: u16 },
    EntryKindMismatch { entry: usize, kind: SubKind },
    MissingAsyncEntry { sub: usize },
    DuplicateAsyncEntry { sub: usize },
    DuplicateSubName { name: String },
    InvalidRootName { name: String },
    ReservedMainKind { kind: SubKind },
    InvalidBuilderRef,
    UndefinedSub { name: String },
    DuplicateDefinition { name: String },
    WrongTargetKind { target: String, expected: SubKind, actual: SubKind },
    WrongTargetArity { target: String, expected: u8, actual: u8 },
    OperandOverflow,
}
```

Implement `EclImage::try_from_parts` so it validates `ImageParts`, builds the single async-name byte pool and flattened parameter-type table, then converts all vectors to boxed slices. Check every `usize -> u16/u32/u8` conversion, including code length, name-pool offsets, per-name byte length, table counts, parameter starts/counts, and Builder operands; reject a per-sub parameter count above `LOCALS` even though `u8` could encode more. Implement `code`, `content_hash`, `sub_count`, `sub_id(raw: u16) -> Option<SubId>`, `root`, `sub_meta`, `param_types`, `resolve_entry`, `ResolvedEntry::id/sub/meta`, `RuntimeSubMeta::code_entry/kind`, `SubId::get`, and `EclImage::empty`. `sub_id` only returns an ID already valid for that image and no public startup API accepts it. Keep `content_hash` equal to the provided placeholder without hashing debug data.

For tests across sibling core modules, add only a `#[cfg(test)] pub(crate) fn test_image(code, subs, entries, root)` wrapper around `try_from_parts`; do not re-open image fields.

- [ ] **Step 4: Write failing canonical Builder tests**

Replace declaration-order tests in `stg-ecl-compiler/src/lib.rs` with:

```rust
#[test]
fn builder_assigns_ids_and_code_layout_by_name_not_declaration_order() {
    fn build(reverse: bool) -> EclImage {
        let mut ib = ImageBuilder::new();
        let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        let worker = ib.declare_sub("worker", SubKind::Async, &[EclValueType::Int]).unwrap();
        let helper = ib.declare_sub("helper", SubKind::CallOnly, &[]).unwrap();
        let order = if reverse { [worker, helper, main] } else { [main, helper, worker] };
        for id in order {
            let mut sub = SubBuilder::new();
            sub.end();
            ib.define_sub(id, sub).unwrap();
        }
        ib.build().unwrap()
    }
    assert_eq!(build(false), build(true));
}

#[test]
fn call_and_spawn_operands_are_canonical_sub_ids() {
    let mut ib = ImageBuilder::new();
    let worker = ib.declare_sub("worker", SubKind::Async, &[]).unwrap();
    let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
    let helper = ib.declare_sub("helper", SubKind::CallOnly, &[]).unwrap();

    let mut main_body = SubBuilder::new();
    main_body.call(helper);
    main_body.spawn(worker, 0);
    main_body.end();
    ib.define_sub(main, main_body).unwrap();

    for id in [worker, helper] {
        let mut body = SubBuilder::new();
        body.end();
        ib.define_sub(id, body).unwrap();
    }

    let image = ib.build().unwrap();
    let pc = image.sub_meta(image.root().unwrap()).unwrap().code_entry() as usize;
    assert_eq!(
        &image.code()[pc..pc + 5],
        &[OP_CALL as u32, 0, OP_SPAWN as u32, 2, 0],
    );
}
```

Add red tests for duplicate declaration, undefined declaration, duplicate definition, CALL to Root/Async, SPAWN to Root/CallOnly, missing main, nonzero root params, and checked overflow errors.

- [ ] **Step 5: Run Builder tests and verify red**

Run:

```bash
cargo test -p stg-ecl-compiler tests::builder_assigns_ids_and_code_layout_by_name_not_declaration_order -- --exact
cargo test -p stg-ecl-compiler tests::call_and_spawn_operands_are_canonical_sub_ids -- --exact
```

Expected: compilation fails because `BuilderSubRef` and the two-stage API do not exist.

- [ ] **Step 6: Implement the two-stage canonical Builder**

Use these core structures in `stg-ecl-compiler/src/lib.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BuilderSubRef(u32);

struct SubDecl {
    name: String,
    kind: SubKind,
    params: Vec<EclValueType>,
    body: Option<SubBuilder>,
}

pub struct ImageBuilder {
    subs: Vec<SubDecl>,
}
```

`declare_sub` validates the identifier and duplicate name, then returns an insertion-stable `BuilderSubRef`. `define_sub` fills `body` exactly once. `SubBuilder` fixups store `BuilderSubRef`, not `ScriptId`. At build time:

```rust
impl SubBuilder {
    pub fn call(&mut self, target: BuilderSubRef);
    pub fn spawn(&mut self, target: BuilderSubRef, argc: u8);
}

impl ImageBuilder {
    pub fn declare_sub(
        &mut self,
        name: &str,
        kind: SubKind,
        params: &[EclValueType],
    ) -> Result<BuilderSubRef, ImageBuildError>;
    pub fn define_sub(
        &mut self,
        sub: BuilderSubRef,
        body: SubBuilder,
    ) -> Result<(), ImageBuildError>;
    pub fn build(self) -> Result<EclImage, ImageBuildError>;
}
```

`sys_create_bullet(..., task)` stores its optional task target as `Option<BuilderSubRef>` too; it is
validated as a zero-parameter `Async` target and rewritten to canonical `SubId` by `build`.
An entirely empty Builder returns `EclImage::empty()`. Every non-empty Builder requires exactly one
Root named `main`; the name `main` is rejected on Async/CallOnly declarations, and any other Root
name is rejected.

At build time:

1. Validate every declaration is defined and root/entry kinds are valid.
2. Sort declaration indexes by `name.as_str()`.
3. Create `BuilderSubRef -> canonical u16` and `BuilderSubRef -> absolute code base` maps with checked conversions.
4. Concatenate code in canonical name order.
5. Rewrite CALL/SPAWN/fire target operands to canonical `SubId`; add bases only to local JMP/JZ operands.
6. Create `SubInit`, async `EntryInit`, and root index; call `EclImage::try_from_parts`.

Change `build` to return `Result<EclImage, ImageBuildError>`. Do not expose an unchecked build path.

- [ ] **Step 7: Migrate language codegen to predeclare all sub symbols**

In `lang/codegen.rs`, replace `name_to_id: BTreeMap<String, ScriptId>` with `name_to_ref: BTreeMap<String, BuilderSubRef>`. Before generating any body:

```rust
let mut ib = ImageBuilder::new();
let mut name_to_ref = BTreeMap::new();
for sub in &ti.subs {
    let kind = if sub.name == "main" {
        SubKind::Root
    } else if sub.is_async {
        SubKind::Async
    } else {
        SubKind::CallOnly
    };
    let params: Vec<EclValueType> = sub.params.iter().map(|(_, ty)| match ty {
        Ty::Int => EclValueType::Int,
        Ty::Fx => EclValueType::Fx,
        Ty::Angle => EclValueType::Angle,
    }).collect();
    let id = ib
        .declare_sub(&sub.name, kind, &params)
        .map_err(|error| image_error_at(sub.span, error))?;
    name_to_ref.insert(sub.name.clone(), id);
}
```

Define `image_error_at(span: Span, error: ImageBuildError) -> Vec<CompileError>` using the same
`CompileError { line, col, msg, src_line: String::new() }` shape as Task 1, and use
`.map_err(|error| image_error_at(sub.span, error))?` above. Generate and `define_sub` bodies in any
iteration order; Builder canonicalization determines final code layout. Remove the old reverse
`call_style` inference: kind now comes directly from Root/Async/CallOnly, so main/async bodies end
with `END` and call-only bodies end with `RET`. Map whole-image `build()` failures at
`Span { line: 1, col: 1 }`.

- [ ] **Step 8: Migrate VM, syscall, task, core tests, compiler tests, and harness accessors**

Make these exact semantic changes:

- `Task.script` becomes `SubId`; task zero state remains valid because dead slots may contain `SubId(0)`.
- Scheduler resolves `task.script` with `ecl.sub_meta` and reads `ecl.code()`.
- `OP_CALL` reads a raw operand, resolves a SubId, requires `CallOnly`, pushes return PC, and jumps to `code_entry`.
- `OP_SPAWN` resolves a SubId, requires `Async`, requires exact argc, copies args to child locals, and spawns next frame.
- `SYS_CREATE_BULLET` task target requires a zero-parameter `Async` SubId before creating the bullet.
- Fault events convert `SubId::get()` to `i32` explicitly.
- Replace every direct `EclImage { ... }`, `.code`, `.subs`, and `.entry(raw)` use with `test_image` or public accessors.
- Update compiler/harness tests to use `image.root()` or `resolve_entry`, never `subs[0]` or declaration position.
- Update `docs/ecl-ops.md`: CALL operand is SubId; SPAWN operands are SubId and argc.

Keep a `pub(crate)` core helper that starts a known Root/Async SubId with raw args for VM tests. Do not leave a public raw-u16 startup API.

- [ ] **Step 9: Run the cross-crate ABI gate and commit**

Run:

```bash
cargo test -p stg-core ecl
cargo test -p stg-ecl-compiler
cargo test -p stg-harness
cargo clippy -p stg-core -p stg-ecl-compiler -p stg-harness --all-targets -- -D warnings
```

Expected: all selected tests pass and Clippy emits no warnings. Confirm with `rg` that production code has no `ScriptId`, public `EclImage` fields, or declaration-position main lookup.

Commit only the files listed for Task 2:

```bash
git add crates/stg-core/src/ecl crates/stg-core/src/step.rs \
  crates/stg-ecl-compiler/src/lib.rs crates/stg-ecl-compiler/src/lang/codegen.rs \
  crates/stg-ecl-compiler/src/lang/mod.rs crates/stg-harness/src/main.rs docs/ecl-ops.md
git commit -m "feat(ecl): add compact canonical named image ABI"
```

---

### Task 3: Add safe named-entry binding and snapshot the one-shot main state

**Files:**
- Create: `crates/stg-core/src/ecl/binding.rs`
- Modify: `crates/stg-core/src/ecl/mod.rs`
- Modify: `crates/stg-core/src/step.rs`
- Modify: `crates/stg-core/src/ecl/image.rs`
- Modify: `crates/stg-core/src/ecl/task.rs`
- Modify: `crates/stg-ecl-compiler/src/lib.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`
- Modify: `crates/stg-harness/src/main.rs`
- Test: `crates/stg-core/src/ecl/binding.rs`
- Test: affected inline tests in core/compiler/harness

**Interfaces:**
- Consumes: Task 2 `ResolvedEntry<'a>`, compact parameter types, `SubId`, and internal root-task spawn primitive.
- Produces: `EclArg`, `EclOwner`, `TaskStartError`, `World::start_main`, `World::spawn_entry`, and `World::spawn_entry_named`.
- Produces: checksummed/snapshotted `World.ecl_main_started: u8`.
- Removes: transitional/raw `World::spawn_task` entry points.

- [ ] **Step 1: Write failing binding and singleton tests**

Create `ecl/binding.rs` tests covering these exact behaviors:

```rust
#[test]
fn start_main_is_stage_owned_and_once_per_world_lifetime() {
    let image = root_and_async_image();
    let mut world = World::new(1);
    let task = world.start_main(&image).unwrap();
    let spawned = &world.tasks.slots[task as usize];
    assert_eq!(
        (spawned.owner_kind, spawned.owner_index, spawned.owner_gen),
        (OWNER_STAGE, 0, 0),
    );
    assert_eq!(world.start_main(&image), Err(TaskStartError::MainAlreadyStarted));
    assert_eq!(world.body.diag.contract_viol, 1);
    assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
}

#[test]
fn main_started_roundtrips_through_snapshot_and_checksum() {
    let image = root_and_async_image();
    let mut source = World::new(1);
    source.start_main(&image).unwrap();
    let mut copy = World::new(2);
    source.copy_into(&mut copy);
    assert_eq!(source.checksum(), copy.checksum());
    assert_eq!(copy.start_main(&image), Err(TaskStartError::MainAlreadyStarted));
}

#[test]
fn named_entry_checks_types_and_fast_entry_checks_only_arity() {
    let image = root_and_async_image();
    let entry = image.resolve_entry("worker").unwrap();
    let mut world = World::new(1);
    assert!(matches!(
        world.spawn_entry_named(&image, "worker", &[EclArg::Int(1)], EclOwner::Stage),
        Err(TaskStartError::WrongArgType { index: 0, .. })
    ));
    assert!(world.spawn_entry(entry, &[Fx::ONE.raw()], EclOwner::Stage).is_ok());
}

#[test]
fn main_cannot_restart_after_end_or_fault() {
    for first_word in [OP_END as u32, u8::MAX as u32] {
        let image = root_only_image(first_word);
        let mut world = World::new(1);
        world.start_main(&image).unwrap();
        for _ in 0..2 {
            step(&mut world, &TABLES_V0, &image, &InputFrame::default());
        }
        assert_eq!(world.tasks.iter_alive().count(), 0);
        assert_eq!(world.start_main(&image), Err(TaskStartError::MainAlreadyStarted));
    }
}
```

Also test unknown/root names, wrong raw argc, stale enemy/bullet owners, invalid owner kind being unrepresentable through the public enum, and pool-full counters.

Add `#[cfg(test)] EclImage::test_resolved_entry(raw: u16) -> ResolvedEntry<'_>` in `image.rs` and
use it only in this module's tests to exercise `TaskStartError::InvalidEntryId`. This preserves the
production invariant that callers cannot construct either `EntryId` or `ResolvedEntry` from a raw
number. Fill the task pool in a test through the private spawn primitive, then assert the next public
spawn returns `PoolFull`, increments only `diag.pool_full[POOL_TASK]`, and sets
`STATUS_POOL_FULL`. Add a root-specific pool-full test that asserts `ecl_main_started` remains zero,
frees one test task, retries `start_main`, and then observes the flag become one. For every other binding error, snapshot `tasks.iter_alive().count()` before the
call and assert it is unchanged, `contract_viol` increments exactly once, and `last_status` becomes
`STATUS_BAD_ARGS`.

- [ ] **Step 2: Run binding tests and verify red**

Run:

```bash
cargo test -p stg-core ecl::binding::tests -- --nocapture
```

Expected: compilation fails because the module and APIs do not exist.

- [ ] **Step 3: Implement typed binding values, owners, and errors**

Use these public shapes in `ecl/binding.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclArg { Int(i32), Fx(Fx), Angle(Angle) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclOwner {
    Stage,
    Enemy(EnemyHandle),
    Bullet(BulletHandle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskStartError {
    NoRoot,
    MainAlreadyStarted,
    UnknownEntry,
    RootRequiresStartMain,
    InvalidEntryId,
    WrongArgCount { expected: u8, actual: usize },
    WrongArgType { index: u8, expected: EclValueType, actual: EclValueType },
    InvalidOwner,
    PoolFull,
}
```

Implement `EclArg::value_type/raw`, `EclOwner::validate(&WorldBody)`, and one private `World::spawn_resolved_sub` that validates entry range, argc, and owner; writes arguments to child locals in declaration order; and maps pool exhaustion to diagnostics plus `TaskStartError::PoolFull`. `EclOwner::Enemy` validates with `body.enemies.get(handle)`, `EclOwner::Bullet` with `body.bullets.get(handle)`, and `Stage` maps to `(OWNER_STAGE, 0, 0)`.

- [ ] **Step 4: Implement the three startup APIs and main state**

Add `pub(crate) ecl_main_started: u8` to `World`. Copy it in `copy_into`; the derive automatically includes it in checksum. Implement:

```rust
pub fn start_main(&mut self, image: &EclImage) -> Result<u16, TaskStartError>;
pub fn spawn_entry(
    &mut self,
    entry: ResolvedEntry<'_>,
    args: &[i32],
    owner: EclOwner,
) -> Result<u16, TaskStartError>;
pub fn spawn_entry_named(
    &mut self,
    image: &EclImage,
    name: &str,
    args: &[EclArg],
    owner: EclOwner,
) -> Result<u16, TaskStartError>;
```

`start_main` checks `root`, rejects a set flag, uses stage owner and no args, and sets the flag only after successful task allocation. Binding contract errors increment `contract_viol` once and set `STATUS_BAD_ARGS`; pool full uses the existing task-pool counter and `STATUS_POOL_FULL`. Pure `image.resolve_entry` never mutates World.

- [ ] **Step 5: Remove raw startup and migrate all call sites**

Replace stage startup with `start_main`. Replace external async startup with `resolve_entry` once plus `spawn_entry`, or `spawn_entry_named` in low-frequency tests. Keep opcode/syscall task creation on the private resolved-SubId primitive. Remove the transitional public/raw startup method and update compiler/harness helpers accordingly.

- [ ] **Step 6: Run snapshot, binding, and workspace tests; commit**

Run:

```bash
cargo test -p stg-core ecl::binding
cargo test -p stg-core step
cargo test -p stg-ecl-compiler
cargo test -p stg-harness
```

Expected: all tests pass, including checksum equality after `copy_into` and permanent rejection after main ends/faults.

Commit:

```bash
git add crates/stg-core/src/ecl/binding.rs crates/stg-core/src/ecl/mod.rs \
  crates/stg-core/src/ecl/image.rs crates/stg-core/src/ecl/task.rs crates/stg-core/src/step.rs \
  crates/stg-ecl-compiler/src/lib.rs crates/stg-ecl-compiler/src/lang/codegen.rs \
  crates/stg-harness/src/main.rs
git commit -m "feat(ecl): add safe named startup and singleton main"
```

---

### Task 4: Produce an explicit optional debug-symbol sidecar

**Files:**
- Create: `crates/stg-ecl-compiler/src/lang/debug.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`
- Test: `crates/stg-ecl-compiler/src/lang/mod.rs`
- Test: `crates/stg-ecl-compiler/src/lang/debug.rs`

**Interfaces:**
- Consumes: canonical `SubId` ordering and final code ranges from Task 2.
- Produces: `DebugInfo`, `CompileOptions`, `CompiledEcl`, `EclDebugSymbols`, `DebugSubMeta`, `DebugParamMeta`, `PcSourceSpan`, and `compile_with_options`.
- Preserves: `compile(src, file) -> Result<EclImage, Vec<CompileError>>` as the `DebugInfo::None` convenience API.

- [ ] **Step 1: Write failing sidecar separation tests**

Add to `lang/mod.rs`:

```rust
#[test]
fn debug_mode_does_not_change_runtime_image() {
    let src = "sub main() { helper(1); } sub helper(x: int) {}";
    let none = compile_with_options(src, "stage.ecl", CompileOptions { debug_info: DebugInfo::None }).unwrap();
    let full = compile_with_options(src, "stage.ecl", CompileOptions { debug_info: DebugInfo::Full }).unwrap();
    assert_eq!(none.image, full.image);
    assert!(none.debug.is_none());
    assert!(full.debug.is_some());
}

#[test]
fn full_debug_symbols_keep_call_only_and_parameter_names() {
    let src = "sub main() { helper(1); } sub helper(value: int) {}";
    let out = compile_with_options(src, "stage.ecl", CompileOptions { debug_info: DebugInfo::Full }).unwrap();
    let debug = out.debug.unwrap();
    let helper = debug.symbol("helper").unwrap();
    assert_eq!(helper.kind(), SubKind::CallOnly);
    assert_eq!(debug.param_name(helper, 0), Some("value"));
    assert_eq!(debug.source_at(helper.pc_start()).unwrap().file(), "stage.ecl");
}
```

- [ ] **Step 2: Run debug tests and verify red**

Run:

```bash
cargo test -p stg-ecl-compiler lang::tests::debug_mode_does_not_change_runtime_image -- --exact
cargo test -p stg-ecl-compiler lang::tests::full_debug_symbols_keep_call_only_and_parameter_names -- --exact
```

Expected: compilation fails because the options and sidecar types do not exist.

- [ ] **Step 3: Implement compact debug tables outside stg-core**

In `lang/debug.rs`, use one `Box<[u8]>` string pool and boxed record slices. Define:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugInfo { None, Full }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions { pub debug_info: DebugInfo }

pub struct CompiledEcl {
    pub image: EclImage,
    pub debug: Option<EclDebugSymbols>,
}
```

`DebugSubMeta` stores canonical `SubId`, a pooled full sub name, kind, parameter range, and absolute `[pc_start, pc_end)`. `DebugParamMeta` stores a pooled parameter name and `EclValueType`. `PcSourceSpan` stores the sub code range plus a pooled file name and the `SubDef.span` line/column. Obtain each canonical ID with `image.sub_id(index as u16)`; derive its start from `sub_meta`, its end from the next canonical sub's `code_entry`, and the last end from `image.code().len()`. This first version deliberately maps each PC to the owning sub declaration span; statement-level mappings are a later refinement, not an omitted step.

- [ ] **Step 4: Wire explicit compile options without changing image bytes**

Refactor the pipeline so parsing/typechecking/slots/codegen run once. `compile_with_options` always constructs the same `EclImage`; only after image construction does `DebugInfo::Full` build the sidecar from `Program`, `TypedInfo`, canonical sub ordering, and image code ranges. `compile` calls `compile_with_options` with `None` and returns `.image`.

Do not use `cfg!(debug_assertions)` or Cargo features to choose symbol output.

- [ ] **Step 5: Run compiler tests, assert no debug strings in the runtime API, and commit**

Run:

```bash
cargo test -p stg-ecl-compiler
cargo clippy -p stg-ecl-compiler --all-targets -- -D warnings
rg -n "DebugSubMeta|DebugParamMeta|PcSourceSpan" crates/stg-core/src && exit 1 || true
```

Expected: tests and Clippy pass; the final search prints no stg-core matches.

Commit:

```bash
git add crates/stg-ecl-compiler/src/lang/debug.rs \
  crates/stg-ecl-compiler/src/lang/mod.rs \
  crates/stg-ecl-compiler/src/lang/codegen.rs
git commit -m "feat(ecl): emit optional compiler debug-symbol sidecar"
```

---

### Task 5: Integrate the named ABI, update documentation, and verify determinism

**Files:**
- Modify: `crates/stg-harness/src/main.rs`
- Modify: `crates/stg-harness/scenes/rainbow.ecl` only if the canonical language rules expose a real source error; do not reorder it merely to influence IDs
- Modify: `crates/stg-ecl-compiler/README.md`
- Modify: `docs/ecl-lang.md`
- Modify: `docs/ecl-ops.md`
- Modify: `docs/follow-ups.md`
- Modify: `PROGRESS.md`
- Test: `crates/stg-harness/src/main.rs`

**Interfaces:**
- Consumes: final runtime image, binding APIs, compiler options, and debug sidecar from Tasks 1-4.
- Produces: author-facing semantics, bytecode reference, dogfood harness coverage, and recorded migration status.

- [ ] **Step 1: Add/adjust harness assertions for named main and async resolution**

Replace declaration-position assertions with behavior like:

```rust
let image = compile_rainbow_image();
assert_eq!(image.sub_count(), 3);
assert!(image.root().is_some());
assert!(image.resolve_entry("patrol").is_ok());
assert!(image.resolve_entry("timer_ui").is_ok());
assert_eq!(image.resolve_entry("main"), Err(ResolveError::RootRequiresStartMain));
```

Start the scene with `world.start_main(&image)`. Add a compile-twice test that permutes only sub declaration order in a small fixture and asserts equal `EclImage` values.

- [ ] **Step 2: Run harness tests and the golden scene twice**

Run:

```bash
cargo test -p stg-harness
tmp_a=$(mktemp)
tmp_b=$(mktemp)
cargo run -p stg-harness -- golden --out "$tmp_a"
cargo run -p stg-harness -- golden --out "$tmp_b"
diff -u "$tmp_a" "$tmp_b"
```

Expected: harness tests pass; both golden runs succeed and `diff` is empty. Inspect the ECL scene event/count assertions before accepting checksum changes caused by canonical SubId/PC layout.

- [ ] **Step 3: Update the authoritative documentation**

Document these exact points:

- `ecl-lang.md`: mandatory `sub main()`, singleton lifecycle, all async subs are public named entries, ordinary subs are call-only, persistent references use names.
- `ecl-ops.md`: CALL/SPAWN operands are SubId; Root/Async/CallOnly checks and new binding error boundary.
- compiler README: `compile` returns stripped runtime image; `compile_with_options(DebugInfo::Full)` returns a separate sidecar.
- `follow-ups.md`: mark named entry ABI complete; keep C11 serialization/hash/full validator open.
- `PROGRESS.md`: add one milestone-history line and update the current-state paragraph without claiming C11 complete.

- [ ] **Step 4: Run the full local quality gate**

Run:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out /tmp/stg-ecl-named-entry-checksums.txt
git diff --check
```

Expected: all workspace tests pass, Clippy has zero warnings, golden exits successfully, and diff check reports no whitespace errors.

- [ ] **Step 5: Audit final API and forbidden legacy assumptions**

Run:

```bash
rg -n "ScriptId|\.subs\b|\.code\b|spawn_task\(|script 0|第一个.*main|最后一个.*main" \
  crates/stg-core crates/stg-ecl-compiler crates/stg-harness docs/ecl-lang.md docs/ecl-ops.md
```

Expected: no production use of `ScriptId`, public image fields, raw `spawn_task`, or declaration-position main assumptions. Any matches must be historical migration comments that are rewritten or removed before commit.

- [ ] **Step 6: Commit integration and documentation**

```bash
git add crates/stg-harness/src/main.rs crates/stg-harness/scenes/rainbow.ecl \
  crates/stg-ecl-compiler/README.md docs/ecl-lang.md docs/ecl-ops.md \
  docs/follow-ups.md PROGRESS.md
git commit -m "docs(ecl): finalize named entry ABI integration"
```

- [ ] **Step 7: Record remote three-platform verification as a handoff gate**

After pushing the implementation branch and opening its PR, require the existing CI jobs to pass:

```text
lint
test
golden (Linux / macOS / Windows)
determinism-gate
```

Do not merge solely from local golden output; the design's final acceptance criterion is byte-identical checksum streams across all three CI platforms.
