# 外接前收口刀实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地六路审阅裁定的"外接前必修":快照防漏哨兵 + D6 硬面四项封口 + 场界常量放行 +
`spawn_entry*` coherence 守卫 + `ENGINE_VER` + 弹 setter 陷阱文档。

**Architecture:** 权威 spec = `docs/superpowers/specs/2026-07-23-pre-bridge-sealing-design.md`。
三刀:① 快照防漏(纯测试+文档)→ ② D6 封口+读口+场界(可见性)→ ③ 守卫+ENGINE_VER+文档。
纯可见性/守卫/测试/文档刀,**行为零变化,金向量与基线逐位全等**(回归闸恢复 byte-diff)。

**Tech Stack:** Rust 1.92(workspace 钉死),零新依赖。

## Global Constraints

- `stg-core` 内禁 f32/f64/时钟/宿主 RNG/无序容器;零新增依赖(CI `cargo tree` 防火墙)。
- **金向量逐位不变**:本刀无新增 World 字段、无演化路径改动——golden 输出与基线 byte-diff
  必须全等;不等即回归,立刻停下排查。金向量脚本/场景一字不动。
- P4-b 惯用法内联:`contract_viol.wrapping_add(1)` + `last_status`,不 panic。
- 每任务收尾全绿:`cargo test --workspace` + `cargo fmt --all -- --check` +
  `cargo clippy --workspace --all-targets -- -D warnings`。
- commit 结尾附:`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- `.superpowers/` 不入库(commit 用 `git add -A ':!.superpowers'`)。
- 注释/文档一律中文,风格随文件内邻居。
- 契约值:`ENGINE_VER = 1`;新读口名 `frame()`/`frame_events()`/`tasks()`(A9 契约名);
  守卫复用既有 `TaskStartError::TableImageMismatch`,语义与 `start_main` 逐字一致。

---

### Task 1: 快照防漏——哨兵尺寸测试 + 七字段判别式拷贝测试

**Files:**
- Modify: `crates/stg-core/src/step.rs`(tests mod 追加)
- Modify: `docs/checksum-mechanism.md`(37 行附近"CI 守卫"承诺改口)
- Modify: `CLAUDE.md`(「改动前自检清单」第 2 条)

**Interfaces:**
- Consumes: 既有 `World::new`/`copy_into`/`checksum`、`set_player_power`、`Pcg32::next_u32`
  (rng.rs:29,pub)、`push_event`;既有拷贝测试样式(`snapshot_covers_tasks_pool` 等)。
- Produces: 无新 API(纯测试+文档)。

- [ ] **Step 1: 金向量基线**

```bash
mkdir -p .superpowers && cargo run -q -p stg-harness -- golden --out .superpowers/golden-pre-sealing.txt && wc -l .superpowers/golden-pre-sealing.txt
```

- [ ] **Step 2: 写七字段拷贝测试(先行,全部应直接绿——它们测的是既有 copy_into 行)**

`crates/stg-core/src/step.rs` 既有 `mod tests` 末尾追加:

```rust
    // ── 快照防漏(2026-07-23 审阅 §1):此前无判别式拷贝覆盖的字段,逐一 mutate 非默认值
    //    → copy_into → 命中;从 copy_into 删对应行即红。 ────────────────────────────

    #[test]
    fn snapshot_covers_frame() {
        let mut w = World::new(3);
        w.body.frame = 777;
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.frame, 777, "copy_into 漏拷 frame 即红");
    }

    #[test]
    fn snapshot_covers_rng_state() {
        let mut w = World::new(3);
        let _ = w.body.rng.next_u32(); // 状态偏离种子初值
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "rng 状态必须随快照(I3;漏拷即红)");
    }

    #[test]
    fn snapshot_covers_players() {
        let mut w = World::new(3);
        w.body.set_player_power(0, 123);
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.players()[0].power, 123, "players 数组漏拷即红");
    }

    #[test]
    fn snapshot_covers_diag_and_last_status() {
        let mut w = World::new(3);
        w.body.set_player_power(9, 0); // OOB → contract_viol+1 + BAD_ARGS
        assert!(w.body.diag.contract_viol > 0, "前置:计数已非零");
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.diag.contract_viol, w.body.diag.contract_viol);
        assert_eq!(snap.body.last_status, w.body.last_status);
    }

    #[test]
    fn snapshot_covers_ecl_main_started_and_tables_hash() {
        let mut w = World::new(3);
        w.ecl_main_started = true;
        w.tables_hash = 0xDEAD_BEEF;
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert!(snap.ecl_main_started, "ecl_main_started 漏拷即红");
        assert_eq!(snap.tables_hash, 0xDEAD_BEEF, "tables_hash 漏拷即红");
    }
```

- [ ] **Step 3: 写哨兵测试(两段式:先占位取真值,再钉死)**

同一 tests mod 追加(`EXPECTED` 先全填 0):

```rust
    /// 快照防漏哨兵(checksum-mechanism.md 承诺的二线防护,实现形态=本测试):
    /// **本断言红了 ⇒ 你增/删/改了 World 字段** ⇒ 依次核对
    /// ① `copy_into` 逐字段清单(手写,漏拷编译不报错——这正是本哨兵存在的原因)
    /// ② checksum(新字段默认入;skip 须理由) ③ D10 容量预算,然后才允许更新下方数字。
    /// `phase_guard` 仅 debug 存在 ⇒ 双值。
    #[test]
    fn world_size_sentinel_guards_copy_into_field_list() {
        let sizes = (
            core::mem::size_of::<crate::world::WorldBody>(),
            core::mem::size_of::<World>(),
        );
        #[cfg(debug_assertions)]
        const EXPECTED: (usize, usize) = (0, 0);
        #[cfg(not(debug_assertions))]
        const EXPECTED: (usize, usize) = (0, 0);
        assert_eq!(sizes, EXPECTED, "先按测试文档注释核对三件套,再更新哨兵数字");
    }
```

- [ ] **Step 4: 取真值钉死**

```bash
cargo test -p stg-core world_size_sentinel 2>&1 | grep -o "([0-9]*, [0-9]*)" | head -2
cargo test -p stg-core --release world_size_sentinel 2>&1 | grep -o "([0-9]*, [0-9]*)" | head -2
```

把 debug/release 实测值分别填进两个 `EXPECTED`,重跑两条命令确认双绿。

- [ ] **Step 5: 全量确认绿**

Run: `cargo test -p stg-core`
Expected: 全 PASS(七字段测试 + 哨兵 debug 值)。

- [ ] **Step 6: 文档两处**

`docs/checksum-mechanism.md` 37 行附近,原句
"**（M0-4 起）CI 守卫**：World 存在后加"**结构体尺寸 / 字段数变更即红**"守卫作二线防护（防"加了 skip 却漏了理由""字段悄悄变纯表现后漏网"之类）。"
改为:

```markdown
- **（2026-07-23 落地）哨兵守卫**：`step.rs` 的 `world_size_sentinel_guards_copy_into_field_list`
  测试钉死 `WorldBody`/`World` 尺寸（debug/release 双值）——**字段集变更即红**，红了先核对
  `copy_into` 清单/checksum skip 理由/D10 预算再更新数字（防"加字段漏拷快照""加了 skip 却漏理由"）。
```

`CLAUDE.md` 「改动前自检清单」第 2 条,原句末尾追加一问:

```
容量进 D10 预算了吗？`copy_into` 同步了吗（尺寸哨兵测试会红）？
```

(即把现有第 2 条的"容量进 D10 预算了吗？"扩成上面这句。)

- [ ] **Step 7: 全绿 + Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "test(step): 快照防漏——尺寸哨兵 + 七字段判别式拷贝测试(审阅 Critical,刀 1/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: D6 硬面四项封口 + 三读口 + 场界常量放行

**Files:**
- Modify: `crates/stg-core/src/world.rs`(frame/rng/events/events_len 可见性 + 两读口 + 场界)
- Modify: `crates/stg-core/src/step.rs`(tasks 可见性 + 三委派读口 + 测试)
- Modify: `crates/stg-core/src/ecl/task.rs`(`TaskPool::new` 可见性)
- Modify: `crates/stg-harness/src/main.rs:1087`(唯一外读迁移)
- Modify: `docs/follow-ups.md`(D6 条目改写)

**Interfaces:**
- Consumes: Task 1 无依赖(可与其并观但顺序执行);既有 `take_requests` 样式。
- Produces(外部契约,M2 桥依赖):`WorldBody::frame(&self) -> u32`、
  `WorldBody::frame_events(&self) -> &[Event]`、`World::frame()`/`World::frame_events()` 委派、
  `World::tasks(&self) -> &TaskPool`;pub 常量 `FIELD_HALF_W`/`FIELD_HEIGHT`/`OOB_MARGIN`/
  `ENEMY_OOB_MARGIN`。

- [ ] **Step 1: 写失败测试(读口未存在 → 编译红)**

`crates/stg-core/src/step.rs` tests mod 追加:

```rust
    /// D6 封口后的三只读口(2026-07-23 审阅 §2):帧号推进可见 / events 切片界=len /
    /// tasks 只读借用。
    #[test]
    fn frame_and_frame_events_and_tasks_read_accessors() {
        let mut w = World::new(1);
        assert_eq!(w.frame(), 0);
        crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.frame(), 1, "advance 后帧号经读口可见");
        assert_eq!(w.tasks().iter_alive().count(), 0, "&TaskPool 只读口");
        assert!(w.frame_events().is_empty());
        w.body.push_event(crate::events::Event {
            kind: crate::events::EVT_PLAYER_DIED,
            ..Default::default()
        });
        assert_eq!(w.frame_events().len(), 1, "切片界 = events_len(不吐陈旧尾槽)");
    }
```

Run: `cargo test -p stg-core frame_and_frame_events 2>&1 | tail -5` → Expected: 编译失败(方法不存在)。

- [ ] **Step 2: 实现**

**(a)** `crates/stg-core/src/world.rs`:

- 144-145 行:`pub frame: u32,` → `pub(crate) frame: u32,`;`pub rng: Pcg32,` →
  `pub(crate) rng: Pcg32,`(各自 doc 注释追加一句"写口唯相位/API;读走 `frame()`"/"I3:状态
  随快照;外部不可触")。
- 168/170 行:`pub events` → `pub(crate) events`;`pub events_len` → `pub(crate) events_len`
  (原 doc"表现层只读"从此与可见性一致)。
- `take_requests` 方法后追加:

```rust
    /// 当前帧号只读口(I6;M2 表现层水位协议消费)。写帧号唯 `advance`(相位 10)。
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// 世界大事记出口(A9 契约名 `frame_events`;代码字段名 `events` 的漂移见 follow-ups D2)。
    /// 幂等只读,按 `events_len` 切片——数组本体从不清零,切片界即真相,消费者永不见陈旧尾槽。
    /// 缓冲下帧 `begin` 清 len,与 `take_requests`/`hits` 同生命周期(A5)。
    pub fn frame_events(&self) -> &[Event] {
        &self.events[..self.events_len as usize]
    }
```

- 99-103 行四场界常量 `pub(crate) const` → `pub const`,doc 各补一句(消费者视角,单位 px,
  D7 中轴坐标系 x∈[-192,192]、y∈[0,448];`OOB_MARGIN`/`ENEMY_OOB_MARGIN` 注明"回收边距,
  表现层通常不需要,py 观测器可用来解释实体消失")。

**(b)** `crates/stg-core/src/step.rs`:

- 20 行:`pub tasks: TaskPool,` → `pub(crate) tasks: TaskPool,`(doc 追加"读走 `tasks()`")。
- `take_requests` 委派后追加:

```rust
    /// 帧号读口委派(见 `WorldBody::frame`)。
    pub fn frame(&self) -> u32 {
        self.body.frame()
    }

    /// 世界大事记读口委派(见 `WorldBody::frame_events`)。
    pub fn frame_events(&self) -> &[crate::events::Event] {
        self.body.frame_events()
    }

    /// 任务池只读口:mutator 全 `pub(crate)`,`&TaskPool` 交出去只能读——同 `&Pool`
    /// 之于通道 A 的论证(刀 A/通道 A);整池重赋值(D6 事故面)从此路封死。
    pub fn tasks(&self) -> &crate::ecl::task::TaskPool {
        &self.tasks
    }
```

**(c)** `crates/stg-core/src/ecl/task.rs:85`:`pub fn new()` → `pub(crate) fn new()`。

**(d)** `crates/stg-harness/src/main.rs:1087`:
`let task_count = w.tasks.iter_alive().count();` → `let task_count = w.tasks().iter_alive().count();`

- [ ] **Step 3: 跑测确认绿**

Run: `cargo test --workspace`
Expected: 全 PASS(in-crate 既有测试不受封口影响;harness 迁移后编译过)。

- [ ] **Step 4: 金向量逐位对拍**

```bash
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t2.txt
diff .superpowers/golden-pre-sealing.txt .superpowers/golden-post-t2.txt && echo BYTE-IDENTICAL
```

Expected: `BYTE-IDENTICAL`(纯可见性刀;不等即回归,停下排查)。

- [ ] **Step 5: follow-ups D6 改写**

`docs/follow-ups.md` D6 条目("WorldBody/World 剩余外部写口")改写:已收
`tasks`(+`TaskPool::new`)/`rng`/`frame`/`events`/`events_len` 五口(2026-07-23 外接前收口刀,
读口 `tasks()`/`frame()`/`frame_events()` 配齐);**剩** `globals`/`boss_ui`/`diag`/`last_status`
四字段(前两者已有 API 全覆盖、越权写不破确定性只破 P1 纪律;`diag` 要先补 `diag()` 读口并迁
harness:1073 直读;`last_status` 纯诊断)。触发点:M2 建桥第一版 PR 顺手。

- [ ] **Step 6: 全绿 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "refactor(world): D6 硬面封口——tasks/rng/frame/events 收 pub(crate) + frame/frame_events/tasks 读口 + 场界常量放行(刀 2/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `spawn_entry*` coherence 守卫 + `ENGINE_VER` + 文档三处

**Files:**
- Modify: `crates/stg-core/src/ecl/binding.rs`(helper 提取 + 两守卫 + 测试)
- Modify: `crates/stg-core/src/ecl/image.rs`(`ResolvedEntry::image` pub(crate) 访问器)
- Modify: `crates/stg-core/src/lib.rs`(`ENGINE_VER`)
- Modify: `docs/ecl-lang.md`(弹 setter 陷阱段)
- Modify: `crates/stg-core/src/field.rs`(过时注释)
- Modify: `stg-world-design.md`(D12 `move_to` 行)

**Interfaces:**
- Consumes: 既有 `TaskStartError::TableImageMismatch`、`start_main` 守卫块
  (binding.rs:129-141)、`EclImage::content_hash()`(image.rs:430)、binding tests 的
  `try_from_parts` 样板(`start_main_rejects_mismatched_hash` ~683 行)。
- Produces: `pub const stg_core::ENGINE_VER: u32 = 1`;`spawn_entry`/`spawn_entry_named`
  错表 → `Err(TableImageMismatch)`(新契约,M2 桥依赖)。

- [ ] **Step 1: 写失败测试**

`crates/stg-core/src/ecl/binding.rs` tests mod 追加(样板:照抄本文件既有
`start_main_rejects_mismatched_hash` 的 image 构造 + 本文件既有 `spawn_entry` 成功测试的
entry/owner 构造,只改 `content_hash`/`tables_hash` 与断言;确保 image 带一个可解析 async 入口):

```rust
    #[test]
    fn spawn_entry_rejects_mismatched_table_hash() {
        // image 构造:本文件 spawn_entry 既有成功测试同款,仅 content_hash 改 0xAAAA_AAAA
        // (具体 SubInit/EntryInit 参数照抄该测试,此处省略的部分以该测试为准——不是留白,
        //  是"同一构造函数换一个哈希字面量")
        let mut w = World::new(0);
        w.tables_hash = 0xBBBB_BBBB;
        let cv0 = w.body.diag.contract_viol;
        let entry = image.resolve_entry("worker").unwrap();
        assert_eq!(
            w.spawn_entry(entry, &[], EclOwner::Stage),
            Err(TaskStartError::TableImageMismatch {
                image: 0xAAAA_AAAA,
                tables: 0xBBBB_BBBB
            })
        );
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "P4-b 计数");
        assert_eq!(w.tasks().iter_alive().count(), 0, "拒配即不派生");
    }

    #[test]
    fn spawn_entry_named_rejects_mismatched_table_hash() {
        // 同上构造;走 spawn_entry_named(&image, "worker", &[], ...) 断言同款 Err + 计数
    }

    #[test]
    fn spawn_entry_allows_matching_or_unbound_hash() {
        // 哈希相等(双侧 0xAAAA)与任一侧 0(未绑定)两种情况:spawn_entry 照常 Ok
        // (照抄既有成功测试,分别设 tables_hash = content_hash 与 tables_hash = 0)
    }
```

(注:`EclOwner` 变体名/`resolve_entry` 入口名以本文件既有测试为准——三测的**断言值**与
**计数语义**是规格,构造样板忠实照抄邻测。)

Run: `cargo test -p stg-core spawn_entry_reject 2>&1 | tail -5`
Expected: FAIL——现无守卫,错表照常派生(断言 `Err` 处红)。

- [ ] **Step 2: 实现**

**(a)** `crates/stg-core/src/ecl/image.rs` `impl<'a> ResolvedEntry<'a>` 内加:

```rust
    /// 背书镜像(binding 层 coherence 守卫用;字段模块私有,crate 内经此读)。
    pub(crate) fn image(&self) -> &'a EclImage {
        self.image
    }
```

**(b)** `crates/stg-core/src/ecl/binding.rs`:

- `impl World`(或既有 binding impl 块)加私有 helper,内容 = `start_main` 129-141 行守卫块
  原样搬移:

```rust
    /// C11 一致性守卫,三站共用(start_main / spawn_entry / spawn_entry_named):
    /// image 所绑表哈希与本 World 建世表不配 → P4-b(计数 + BAD_ARGS + Err),不 panic。
    /// 任一侧 0(空脚本/无真表)= 未绑定,跳过。
    fn check_table_coherence(&mut self, image: &EclImage) -> Result<(), TaskStartError> {
        let image_hash = image.content_hash();
        if image_hash != 0 && self.tables_hash != 0 && image_hash != self.tables_hash {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::TableImageMismatch {
                image: image_hash,
                tables: self.tables_hash,
            });
        }
        Ok(())
    }
```

- `start_main`:原守卫块替换为 `self.check_table_coherence(image)?;`(纯等价改写,注释保留
  "Once, at startup"句挪进 helper doc 或删除)。
- `spawn_entry` 入口(`is_valid_entry_id` 检查后)加:`self.check_table_coherence(entry.image())?;`
- `spawn_entry_named` 入口(resolve 前即可)加:`self.check_table_coherence(image)?;`

**(c)** `crates/stg-core/src/lib.rs`(module docs 后、`pub use` 前):

```rust
/// 引擎确定性契约版本。**bump 纪律**:凡改 op 表/syscall 号语义、校验和算法、烘焙表内容、
/// 池 SoA 布局/字段序、step 相位序——任何使旧回放/旧对端不可对拍的变更——必须 +1 并过评审。
/// 回放头(M3)/联机握手(M4)身份三元组之一(另两个:表 `content_hash`、镜像 `content_hash`)。
pub const ENGINE_VER: u32 = 1;
```

锚定测试(checksum.rs tests mod 或 step.rs tests 末尾):

```rust
    #[test]
    fn engine_ver_anchored() {
        assert_eq!(crate::ENGINE_VER, 1, "bump 必须是有意识决定(评审 + 改本测试)");
    }
```

- [ ] **Step 3: 跑测确认绿**

Run: `cargo test -p stg-core`
Expected: 全 PASS(三新测 + `start_main` 既有守卫测试照旧绿——helper 纯等价)。

- [ ] **Step 4: 文档三处**

**(a)** `docs/ecl-lang.md` 内建函数清单块(「渲染请求（通道 B）」节之前)追加:

```markdown
> **弹 setter 的 handle 参数是陷阱位**:首参 `handle:int` **求值后即丢弃**,setter 恒作用于
> **当前任务的 owner 弹**(`self` 语义)——不能借句柄定向操纵别的弹;owner 不是弹的任务调它
> → 任务 Fault。想操纵 `fire(...)` 出来的那颗弹,用 xformdef 或 `fire` 的 `task` 参数挂子任务。
```

**(b)** `crates/stg-core/src/field.rs` `FIELD_MAX_RADIUS` doc 中过时括注
"（`WorldBody.players`/`PlayerState` 目前是 `pub`，故这是**前提**而非强制）"
改为:"（`WorldBody.players` 已收 `pub(crate)`——刀 A 2026-07-21,crate 外无绕行路径;
crate 内绕过 `spawn` 直写仍属纪律约束）"。

**(c)** `stg-world-design.md` D12 表 `move_to` 行:
`| move_to | 句柄悬垂 / dur=0 | no-op | BAD_HANDLE / BAD_ARGS | diag.contract_viol |`
改为:
`| move_to | 句柄悬垂（dur=0 = 瞬移，合法退化——D5 拍板，此行原文陈旧已勘误 2026-07-23） | no-op | BAD_HANDLE | diag.contract_viol |`

- [ ] **Step 5: 金向量对拍 + 全绿 + Commit**

```bash
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t3.txt
diff .superpowers/golden-pre-sealing.txt .superpowers/golden-post-t3.txt && echo BYTE-IDENTICAL
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "feat(core): spawn_entry* coherence 守卫三站共用 + ENGINE_VER=1 + setter 陷阱文档(刀 3/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: `BYTE-IDENTICAL`(守卫是错误路径,golden 表匹配不触发)。

---

## 合入前收尾(控制器步骤,非 subagent 任务)

终审通过后、合入前,控制器补 docs 收口 commit:

1. `docs/follow-ups.md` 记档批(spec §7 全部条目:generation ABA / hits debug panic /
   ItemPool sprite / Fault-P4b 口径 / bench 过期 / EclImage 路径拍板 / 数学核小项 / 池账目)
   + 销 spawn_entry 无守卫条目。
2. `docs/architecture.md` M2 接缝行焊点补"外接前收口 ✅"。
3. `PROGRESS.md` 史加一行 + 重写「现在」段(下一步:开 `stg-godot` crate)。
