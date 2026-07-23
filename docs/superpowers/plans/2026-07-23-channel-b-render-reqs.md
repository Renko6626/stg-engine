# 通道 B / anm call（渲染请求队列）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地断层线第二条向上出口——`RenderReq` + WorldBody `reqs` 缓冲 + `emit_req`
（世界 API / `SYS_EMIT_REQ` syscall / `.ecl` 内建）+ `take_requests()` 出口 + settle 敌死
特效请求（蓝图 §207 机械产出者）。

**Architecture:** 权威 spec = `docs/superpowers/specs/2026-07-23-channel-b-render-reqs-design.md`
（含全部预决定契约与评审议决，动手前先读）。三刀：① stg-core 世界侧（类型/缓冲/写读口/settle）
→ ② VM syscall 层 → ③ `.ecl` 表层（`ParamKind::RawVal` + 内建）+ 语言文档。

**Tech Stack:** Rust 1.92（workspace 钉死），无新依赖。

## Global Constraints

- **断层线（I1-I7）**：`stg-core` 内禁 `f32/f64`/时钟/宿主 RNG/无序容器；**零新增依赖**
  （CI `cargo tree -p stg-core` 防火墙）。
- **P4 惯用法内联重复**，不提前抽象：计数一律 `wrapping_add(1)`；调用方违约 → no-op +
  `diag.contract_viol` + `last_status`；资源耗尽 → 确定性丢弃 + 计数，不 panic。
- **checksum skip 必须给理由字符串**（derive 强制）；`diag.reqs_dropped` **必须入校验和**
  （P4-a，不得 skip）。
- **金向量预期（勿误判）**：`DiagCounters` 增列使每帧校验和**取值平移**——与基线 byte-diff
  不同是**预期效应**，不是回归；回归看守 = 判别式单测 + golden 行数/退出码不变 + 三平台 CI。
  金向量脚本与场景（`scenes/`、harness 导演）**一字不动**。
- 每任务收尾全绿：`cargo test --workspace` + `cargo fmt --all -- --check` +
  `cargo clippy --workspace --all-targets -- -D warnings`。
- commit 结尾附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 常量取值即契约：`REQS_CAP = 256`、`REQ_ENEMY_DEATH = 1`、`REQ_SCRIPT_BASE = 64`、
  `STATUS_TRUNCATED = 4`、`SYS_EMIT_REQ = 27`、计数器名 `reqs_dropped`（D12 名，**不是**
  `reqs_overflow`——hits/events 的 `*_overflow` 是历史命名，本刀跟 D12）。

---

### Task 1: stg-core 世界侧——`reqs.rs` + 缓冲 + `emit_req`/`take_requests` + settle 死亡请求

**Files:**
- Create: `crates/stg-core/src/reqs.rs`
- Modify: `crates/stg-core/src/lib.rs`（模块声明，`player` 与 `rng` 之间）
- Modify: `crates/stg-core/src/consts.rs`（`structural` 段 +2 行）
- Modify: `crates/stg-core/src/world.rs`（STATUS 常量、DiagCounters、WorldBody 字段、
  `emit_req`/`take_requests`、`begin`、tests）
- Modify: `crates/stg-core/src/step.rs`（`copy_into` 清 len + `World::take_requests` 委派）
- Modify: `crates/stg-core/src/world/settle.rs`（趟二死亡请求 + test）

**Interfaces:**
- Consumes: 既有 `WorldBody`（`push_event` 同栏写法）、`crate::step::World`、
  `enemies.{x,y,sprite,score}`（`sprite: u16` / `score: u16`）。
- Produces（后续任务依赖，签名精确）：
  - `crate::reqs::RenderReq { pub id: u16, pub seq: u16, pub args: [i32; 6] }`（repr(C)）
  - `pub(crate) crate::reqs::REQS_CAP: usize = 256`
  - `crate::consts::{REQ_ENEMY_DEATH: u16 = 1, REQ_SCRIPT_BASE: u16 = 64}`
  - `WorldBody::emit_req(&mut self, id: u16, args: [i32; 6])`（Task 2 的 syscall 调它）
  - `WorldBody::take_requests(&self) -> &[RenderReq]` / `World::take_requests(&self) -> &[RenderReq]`
  - `crate::world::STATUS_TRUNCATED: u16 = 4`、`DiagCounters.reqs_dropped: u32`

- [ ] **Step 1: 金向量基线（行数参照）**

```bash
mkdir -p .superpowers && cargo run -q -p stg-harness -- golden --out .superpowers/golden-pre-channel-b.txt && wc -l .superpowers/golden-pre-channel-b.txt
```

记下行数（预期 1201）。后续只比**行数与退出码**，不比字节（Global Constraints 说明了为什么）。

- [ ] **Step 2: 写失败测试（world.rs + settle.rs）**

`crates/stg-core/src/world.rs` 既有 `mod tests` 末尾追加（沿用邻测风格，`use super::*` 已在）：

```rust
#[test]
fn emit_req_records_id_seq_args_in_push_order() {
    let mut w = crate::step::World::new(1);
    w.body.emit_req(7, [1, 2, 3, 4, 5, 6]);
    w.body.emit_req(8, [-1, -2, -3, -4, -5, -6]);
    let reqs = w.body.take_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!((reqs[0].id, reqs[0].seq, reqs[0].args), (7, 0, [1, 2, 3, 4, 5, 6]));
    assert_eq!(
        (reqs[1].id, reqs[1].seq, reqs[1].args),
        (8, 1, [-1, -2, -3, -4, -5, -6])
    );
}

#[test]
fn emit_req_overflow_drops_counts_and_sets_truncated() {
    use crate::reqs::REQS_CAP;
    let mut w = crate::step::World::new(1);
    for i in 0..REQS_CAP {
        w.body.emit_req(1, [i as i32, 0, 0, 0, 0, 0]);
    }
    assert_eq!(w.body.diag.reqs_dropped, 0);
    w.body.emit_req(2, [999, 0, 0, 0, 0, 0]);
    let reqs = w.body.take_requests();
    assert_eq!(reqs.len(), REQS_CAP, "溢出后 len 停在 cap");
    assert_eq!(w.body.diag.reqs_dropped, 1);
    assert_eq!(w.body.last_status, STATUS_TRUNCATED);
    assert_eq!(reqs[REQS_CAP - 1].args[0], (REQS_CAP - 1) as i32, "已有内容不受扰");
    assert_eq!(reqs[REQS_CAP - 1].seq, (REQS_CAP - 1) as u16);
}

#[test]
fn begin_clears_reqs_and_take_requests_is_idempotent() {
    let mut w = crate::step::World::new(1);
    w.body.emit_req(7, [0; 6]);
    let (p1, l1) = {
        let r = w.body.take_requests();
        (r.as_ptr(), r.len())
    };
    let r2 = w.body.take_requests();
    assert_eq!((p1, l1), (r2.as_ptr(), r2.len()), "帧内幂等：同一切片（蓝图 §256）");
    w.body.begin();
    assert!(w.body.take_requests().is_empty(), "begin 清空通道 B");
}

#[test]
fn emit_req_is_invisible_to_checksum() {
    let mut w = crate::step::World::new(1);
    let c0 = w.checksum();
    w.body.emit_req(9, [1, 2, 3, 4, 5, 6]);
    assert_eq!(w.checksum(), c0, "reqs/reqs_len 是 checksum-skip 纯输出（P6）");
}

#[test]
fn copy_into_restores_world_with_no_stale_reqs() {
    let mut w = crate::step::World::new(1);
    let mut dst = crate::step::World::new(1);
    w.body.emit_req(7, [0; 6]);
    w.copy_into(&mut dst);
    assert!(
        dst.body.take_requests().is_empty(),
        "恢复出的 World 必须无陈旧通道 B 输出（同 hits/events 契约，见 step.rs copy_into 注释）"
    );
}
```

`crates/stg-core/src/world/settle.rs` 既有 `mod tests` 末尾追加（脚手架照抄邻测
`settle_shot_kills_enemy_marks_dying_and_event`，仅覆写判别字段）：

```rust
#[test]
fn settle_enemy_death_emits_render_req_with_pos_sprite_score() {
    use crate::consts::REQ_ENEMY_DEATH;
    let mut w = crate::step::World::new(1);
    let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1
    let ei = w.body.enemies.get(e).unwrap();
    w.body.enemies.sprite[ei] = 7;
    w.body.enemies.score[ei] = 450;
    w.body.create_player_shot(crate::shots::ShotInit {
        x: w.body.enemies.x[ei],
        y: w.body.enemies.y[ei],
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        damage: 1,
        radius: Fx::from_int(4),
        sprite: 0,
        owner: 0,
        flags: 0,
    });
    #[cfg(debug_assertions)]
    {
        w.body.phase_guard = PH_COLLIDE;
    }
    w.body.collide(&crate::tables::TABLES_V0);
    w.body.settle(&crate::tables::TABLES_V0);
    let reqs = w.body.take_requests();
    assert_eq!(reqs.len(), 1, "一敌一死一请求");
    assert_eq!(reqs[0].id, REQ_ENEMY_DEATH);
    assert_eq!(
        reqs[0].args,
        [
            w.body.enemies.x[ei].raw(),
            w.body.enemies.y[ei].raw(),
            7,
            450,
            0,
            0
        ],
        "位序 x/y/sprite/score——判别值防对调假绿"
    );
}
```

- [ ] **Step 3: 跑测确认红**

Run: `cargo test -p stg-core 2>&1 | tail -20`
Expected: **编译失败**（`emit_req`/`take_requests`/`reqs` 模块不存在）——新 API 的红即编译错。

- [ ] **Step 4: 实现**

**(a)** 新建 `crates/stg-core/src/reqs.rs`：

```rust
//! 通道 B 渲染请求（design_doc §6.2/§6.3，"核出请求，壳做演出"）。**半冻结跨语言契约**：
//! repr(C) 布局（2+2+24 = 28 B）+ id 分区 + 引擎 id args 约定表——M2 Godot 分发器按此
//! 路由/解码，改动过评审 + 视情 bump `engine_ver`。
//!
//! ## id 命名空间分区
//! - `0`：保留无效值（零结构体防呆；分发器忽略）。
//! - `1..=63`：引擎保留（`crate::consts` structural 段注册，Rust/脚本双侧单源）。
//! - `64..`：脚本 / mod 自由段（作者自配 `.ecl` `const`，与自家分发器 handler 自成契约）。
//!
//! ## 引擎 id args 约定表（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）
//! | id | args[0] | args[1] | args[2] | args[3] | args[4..] |
//! |---|---|---|---|---|---|
//! | `REQ_ENEMY_DEATH` | x (fx raw) | y (fx raw) | sprite (int) | score (int) | 0 |

/// 一条渲染请求（§6.2，28 B）。id 语义世界不解释；`(frame, seq)` 全局唯一。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RenderReq {
    /// 请求名（编译期驻留；`0` = 保留无效值）。
    pub id: u16,
    /// 帧内自增序号（= 入缓冲索引）。
    pub seq: u16,
    /// 按 id 约定解释的裸载荷（模块文档约定表）。
    pub args: [i32; 6],
}

/// 每帧请求上限（D10 预算既有行：256 × 28 B = 7 KB）。
pub(crate) const REQS_CAP: usize = 256;
```

**(b)** `crates/stg-core/src/lib.rs`：`pub mod player;` 与 `pub mod rng;` 之间插
`pub mod reqs;`。

**(c)** `crates/stg-core/src/consts.rs` `structural` 块（`GLOBALS_SYS_SEGMENT` 行后）：

```rust
        //  通道 B 引擎保留请求 id（分区与 args 约定见 `crate::reqs` 模块文档）
        REQ_ENEMY_DEATH:     u16 as int = 1;
        REQ_SCRIPT_BASE:     u16 as int = 64;
```

**(d)** `crates/stg-core/src/world.rs`：

- 顶部 use 区（`use crate::events::{...}` 旁）加：`use crate::reqs::{REQS_CAP, RenderReq};`
- `STATUS_BAD_ARGS` 后加：

```rust
/// 缓冲满截断（D12：`emit_req` 满 → 丢弃 + 本状态 + `diag.reqs_dropped`）。
pub const STATUS_TRUNCATED: u16 = 4;
```

- `DiagCounters` 的 `events_overflow` 行后加：

```rust
    /// reqs 满丢弃计数（P4-a/D12 名 `reqs_dropped`——表现可以掉，确定性不能破，
    /// 两机必须丢得一样多，故**必须入校验和**、不得 skip）。
    pub reqs_dropped: u32,
```

- `WorldBody` 的 `events_len` 字段后加（**生而 `pub(crate)`**，不同于 events 的 pub 残留，
  不长 follow-up D6）：

```rust
    #[checksum(skip = "纯输出缓冲，回滚重演确定性再生（P6/§6.2 通道 B）")]
    pub(crate) reqs: [RenderReq; REQS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 reqs 一并 skip（通道 B）")]
    pub(crate) reqs_len: u16,
```

- `push_event` 后加两个方法：

```rust
    /// 通道 B 推送（§6.2）。id 语义世界不解释（含 0——保留无效值，分发器忽略）；
    /// 满 → 确定性丢弃 + `TRUNCATED` + 计数（P4-a/D12），不 panic。成功不动 `last_status`。
    pub fn emit_req(&mut self, id: u16, args: [i32; 6]) {
        if (self.reqs_len as usize) < REQS_CAP {
            self.reqs[self.reqs_len as usize] = RenderReq {
                id,
                seq: self.reqs_len,
                args,
            };
            self.reqs_len += 1;
        } else {
            self.diag.reqs_dropped = self.diag.reqs_dropped.wrapping_add(1);
            self.last_status = STATUS_TRUNCATED;
        }
    }

    /// 通道 B 出口（蓝图 §256）：本帧请求切片。**幂等非消费**——名字沿契约叫 take，
    /// 帧内多次调用返回同一切片；缓冲下帧 `begin` 清空，headless 无人消费 = 零成本。
    pub fn take_requests(&self) -> &[RenderReq] {
        &self.reqs[..self.reqs_len as usize]
    }
```

- `begin` 里 `self.events_len = 0;` 后加 `self.reqs_len = 0;`。

**(e)** `crates/stg-core/src/step.rs`：

- `copy_into` 的 `d.events_len = 0;` 后加 `d.reqs_len = 0;`（上方那段"恢复出的 World 必须
  无陈旧输出"注释把 `hits/events` 措辞扩为 `hits/events/reqs`）。
- `World::view()` 方法后加：

```rust
    /// 通道 B 出口委派（镜像 `view()`；幂等语义见 `WorldBody::take_requests`）。
    pub fn take_requests(&self) -> &[crate::reqs::RenderReq] {
        self.body.take_requests()
    }
```

**(f)** `crates/stg-core/src/world/settle.rs` 趟二 hp≤0 块，`self.push_event(ev);`
（`EVT_ENEMY_DIED` 那条）之后加：

```rust
            // 死亡特效请求（蓝图 §207 机械产出者；args 约定见 `crate::reqs` 模块文档）。
            self.emit_req(
                crate::consts::REQ_ENEMY_DEATH,
                [
                    self.enemies.x[e].raw(),
                    self.enemies.y[e].raw(),
                    self.enemies.sprite[e] as i32,
                    self.enemies.score[e] as i32,
                    0,
                    0,
                ],
            );
```

- [ ] **Step 5: 跑测确认绿**

Run: `cargo test -p stg-core`
Expected: 全 PASS（含新 6 测）。

- [ ] **Step 6: 金向量行数 + 全绿**

```bash
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t1.txt && wc -l .superpowers/golden-post-t1.txt
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 行数与 Step 1 相同（1201）、退出码 0；workspace 全绿。
（byte-diff 与基线不同 = 预期，见 Global Constraints，**不要**据此回退。）

- [ ] **Step 7: Commit**

```bash
git add -A ':!.superpowers'
git commit -m "feat(world): 通道 B 世界侧——RenderReq + reqs 缓冲 + emit_req/take_requests + settle 死亡请求（刀 1/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `SYS_EMIT_REQ` syscall（号 27）+ ecl-ops.md

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（常量 + dispatch 臂 + `sys_emit_req` + tests）
- Modify: `docs/ecl-ops.md`（syscall 号表 +1 行 + 处置注记）

**Interfaces:**
- Consumes: Task 1 的 `WorldBody::emit_req(id: u16, args: [i32; 6])`、
  `WorldBody::take_requests()`、`crate::world::STATUS_BAD_ARGS`；既有 `pop`/`push` 助手、
  tests 的 `call`/`fresh` 助手。
- Produces: `pub const SYS_EMIT_REQ: u16 = 27;`（Task 3 内建表引用）。

- [ ] **Step 1: 写失败测试**

`crates/stg-core/src/ecl/syscall.rs` 既有 `mod tests` 末尾追加（`call` 助手把 args
**声明正序**压栈后直连 `dispatch`，见其文档）：

```rust
    #[test]
    fn sys_emit_req_pushes_request_and_drains_stack() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[64, 1, 2, 3, 4, 5, 6]).is_ok());
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!((reqs[0].id, reqs[0].seq), (64, 0));
        assert_eq!(reqs[0].args, [1, 2, 3, 4, 5, 6], "声明序 id,a0..a5 ↔ 弹栈逆序还原");
        assert_eq!(task.sp, 0, "七值全弹栈");
    }

    #[test]
    fn sys_emit_req_bad_id_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let cv0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[-1, 0, 0, 0, 0, 0, 0]).is_ok());
        assert_eq!(w.body.take_requests().len(), 0, "坏 id no-op 不入缓冲");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert!(call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[65536, 0, 0, 0, 0, 0, 0]).is_ok());
        assert_eq!(w.body.take_requests().len(), 0, "越上界同款");
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
    }

    #[test]
    fn sys_emit_req_stack_underflow_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[1, 2]),
            Err(FAULT_STACK),
            "参数不足 → 栈下溢 Fault（同全族处置）"
        );
    }
```

- [ ] **Step 2: 跑测确认红**

Run: `cargo test -p stg-core ecl::syscall 2>&1 | tail -8`
Expected: 编译失败（`SYS_EMIT_REQ` 未定义）。

- [ ] **Step 3: 实现**

`crates/stg-core/src/ecl/syscall.rs`：

- 号表 2x 族 `SYS_PULSE_SIGNAL` 行后加：

```rust
/// 通道 B 渲染请求推送（M2 前置刀；D12/spec §2.5）。无 owner 类别限制——宣言/音效/震屏
/// 常由 STAGE 任务发。
pub const SYS_EMIT_REQ: u16 = 27;
```

- `dispatch` 的 `SYS_PULSE_SIGNAL` 臂后加：`SYS_EMIT_REQ => sys_emit_req(task, ctx),`
- sys_* 函数区加（弹栈逆序 = `sys_boss_set` 同款）：

```rust
/// `SYS_EMIT_REQ`（27）：通道 B 推送。id 收窄 P4-b——栈值超出 `0..=65535` →
/// no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（作者违约 → 确定性安全结果）；
/// 值域内转交 `WorldBody::emit_req`（满缓冲处置 TRUNCATED + 计数在那边）。
fn sys_emit_req(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a5 = pop(task)?;
    let a4 = pop(task)?;
    let a3 = pop(task)?;
    let a2 = pop(task)?;
    let a1 = pop(task)?;
    let a0 = pop(task)?;
    let id = pop(task)?;
    if !(0..=u16::MAX as i32).contains(&id) {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    ctx.body.emit_req(id as u16, [a0, a1, a2, a3, a4, a5]);
    Ok(())
}
```

- [ ] **Step 4: 跑测确认绿**

Run: `cargo test -p stg-core`
Expected: 全 PASS。

- [ ] **Step 5: `docs/ecl-ops.md` 号表补行**

`| 26 | pulse_signal | ch | — |` 行后加：

```markdown
| 27 | `emit_req` | id a0 a1 a2 a3 a4 a5 | — |
```

表后（或该节既有注记列表里）加一条：

```markdown
- **`emit_req`（27）**：通道 B 渲染请求（`docs/ecl-lang.md`"渲染请求"节）。id 收窄 P4-b：
  栈值超出 `0..=65535` → no-op + `contract_viol` + `BAD_ARGS`，不 Fault；缓冲满走 D12
  （丢弃 + `TRUNCATED` + `diag.reqs_dropped`）。无 owner 类别限制（STAGE 任务可发）。
```

- [ ] **Step 6: 全绿 + Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "feat(ecl): SYS_EMIT_REQ=27 通道 B syscall——id 收窄 P4-b + 满缓冲 TRUNCATED（刀 2/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `.ecl` 表层——`ParamKind::RawVal` + `emit_req` 内建 + ecl-lang.md

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（RawVal 变体 + 内建条目 + 两处测试名单）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`（RawVal 判型臂）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（RawVal 发射配对 + e2e test）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/tests.rs`（正/负例）
- Modify: `docs/ecl-lang.md`（内建清单行 + 新"渲染请求（通道 B）"节）

**Interfaces:**
- Consumes: Task 2 的 `syscall::SYS_EMIT_REQ`；Task 1 的 `w.body.take_requests()`；
  既有 `ParamKind::{Val,XformRef,SubRef}`、typeck `check_call` 参数循环、codegen
  `gen_builtin_call` 的 `(CallArg, ParamKind)` 配对 match、codegen tests 的
  `run(src, frames)` 助手、typeck tests 的 `ok`/`err` 助手。
- Produces: 表层 `emit_req(id:int, a0..a5:raw)`（固定 7 参、`ret: None`）。

- [ ] **Step 1: 写失败测试**

`crates/stg-ecl-compiler/src/lang/typeck/tests.rs` 末尾追加：

```rust
#[test]
fn emit_req_rawval_accepts_all_three_types_id_stays_int() {
    // RawVal 六位三型任意（fx/int/angle/表达式混填皆良型）
    ok("sub main() { emit_req(64, 1.5fx, -3, 90deg, 2 + 3, 0fx, 0); loop { wait(1); } }");
    // id 位仍是 Val(Int)：传 fx 必须报参数类型错（本断言在 emit_req 未注册前会因
    // "未定义函数"类错误而**不含此文案**——红），实现后转为精确命中
    let errs = err("sub main() { emit_req(1.0fx, 0, 0, 0, 0, 0, 0); loop { wait(1); } }");
    assert!(
        errs.iter().any(|e| e.msg.contains("第 1 个参数期待")),
        "id 位传 fx 应报参数类型错：{errs:?}"
    );
}
```

`crates/stg-ecl-compiler/src/lang/codegen.rs` 既有 tests 末尾追加：

```rust
    /// 通道 B 表层端到端：字面量折叠（1.5fx→98304 / 90deg→16384）+ RawVal 三型 raw 直通
    /// + RawVal 位表达式求值（2+3→5），各位判别值防错位假绿。帧序：0=main 出生跳过、
    /// 1=main 首跑（emit 落在 step(f=1) 内）——begin 每帧清缓冲，故恰步 2 帧后读。
    #[test]
    fn emit_req_rawval_literal_folding_reaches_channel_b() {
        let src = "sub main() {\n\
                     emit_req(64, 1.5fx, -3, 90deg, 2 + 3, 0, 0);\n\
                     wait(10);\n\
                   }";
        let w = run(src, 2);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!((reqs[0].id, reqs[0].seq), (64, 0));
        assert_eq!(reqs[0].args, [98304, -3, 16384, 5, 0, 0]);
        assert_eq!(w.body.diag.task_faults, 0);
    }
```

- [ ] **Step 2: 跑测确认红**

Run: `cargo test -p stg-ecl-compiler emit_req 2>&1 | tail -8`
Expected: 两测 FAIL（`emit_req` 未注册：typeck 报未定义函数、e2e 编译 panic）。

- [ ] **Step 3: 实现**

**(a)** `builtins.rs`：

- `ParamKind` 加变体（`SubRef` 后）：

```rust
    /// 裸载荷参数（通道 B `emit_req` 六个 args 位）：接受 int/fx/angle 任意**良型**表达式，
    /// codegen 与 `Val` 同路径求值入栈、原样发射（VM 栈本就是裸 i32，零转换指令）——语义
    /// 镜像 `RenderReq.args` 的不透明本质：fx 过 Q16.16 raw、angle 过 BAM raw、int 原样。
    RawVal,
```

- use 行改为：`use ParamKind::{RawVal, SubRef as Sub, Val, XformRef as Xf};`
- 2x 段 `pulse_signal` 条目后加：

```rust
    Builtin {
        name: "emit_req",
        syscall: syscall::SYS_EMIT_REQ,
        is_op: false,
        // 固定 7 参（不足位作者手补 0）；id 位钉 Int，六载荷位 RawVal（spec §2.6）
        params: &[Val(Int), RawVal, RawVal, RawVal, RawVal, RawVal, RawVal],
        ret: None,
    },
```

- tests：`lookup_finds_every_documented_builtin_by_name` 名单加 `"emit_req"`；
  `void_builtins_have_none_return_type` 名单加 `"emit_req"`。

**(b)** `typeck/exprs.rs` `check_call` 参数循环，`ParamKind::Val(pty)` 臂后加：

```rust
                ParamKind::RawVal => match self.type_expr(a, locals) {
                    // 三型任意，良型即过；raw 直通（无转换、无收窄）
                    Some(t) => out.push(CallArg::Val(t)),
                    None => ok = false,
                },
```

**(c)** `codegen.rs` `gen_builtin_call` 的配对 match，首臂改为：

```rust
                (CallArg::Val(e), ParamKind::Val(_) | ParamKind::RawVal) => {
```

- [ ] **Step 4: 跑测确认绿 + 全绿**

Run: `cargo test -p stg-ecl-compiler && cargo test --workspace`
Expected: 全 PASS（含 Step 1 两测转绿、builtins 两名单测试仍绿）。

- [ ] **Step 5: `docs/ecl-lang.md`**

内建函数清单（`pulse_signal(ch)` 之后）加：
`emit_req(id:int, a0..a5:raw)`（通道 B 渲染请求，见下节）·

「xformdef」节**之前**插入新节：

```markdown
## 渲染请求（通道 B）

`emit_req(id:int, a0,a1,a2,a3,a4,a5: raw)` —— 向表现层推送一次性演出请求（爆炸/音效/宣言/
震屏）。固定 7 参，不足位补 `0`；六个载荷位是 **raw 参数**：接受 `int`/`fx`/`angle` 任意型
表达式**按位原样**传出（`1.5fx` → Q16.16 raw = 98304、`90deg` → BAM raw = 16384、int 原样），
表现层按 id 约定解码。无返回值（只能做语句）；缓冲满确定性丢弃（不 Fault）；无 owner 类别
限制——关卡任务也能发。

id 命名空间：`0` 保留无效 · `1..=63` 引擎保留（如 `REQ_ENEMY_DEATH`）· `64+` 脚本自由——
建议 `const MY_REQ: int = REQ_SCRIPT_BASE + n;` 起名。引擎 id 的逐位 args 约定表见
`stg-core/src/reqs.rs` 模块文档（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）。
```

- [ ] **Step 6: 金向量行数 + fmt/clippy + Commit**

```bash
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t3.txt && wc -l .superpowers/golden-post-t3.txt
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "feat(ecl-lang): emit_req 内建 + ParamKind::RawVal 裸载荷参数（刀 3/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: 行数仍与基线一致（金向量脚本一字未动）。

---

## 合入前收尾（控制器步骤，非 subagent 任务）

终审通过后、合入前，控制器补一个 docs 收口 commit：

1. `CLAUDE.md`：P6 名字漂移警告改口（"`reqs` 尚未实现，属 M2"→ 已落地，指
   `take_requests`）；仓库结构图 `src/{input,events}.rs` 行补 `reqs.rs`（通道 B 请求类型）。
2. `docs/follow-ups.md` 名字漂移记："`reqs` 尚未实现（M2）"→ 已落地（2026-07-23 通道 B 刀），
   `frame_events`↔`events` 漂移本身仍在。
3. `docs/architecture.md` M2 接缝行：焊点补"通道 B `emit_req`/`take_requests` ✅"，
   还缺只剩"新建 crate（WorldBridge + MultiMesh + 请求分发器）"。
4. `PROGRESS.md`：史加一行 + 重写「现在」段（下一候选：M2 建 crate / M3 / 玩法小刀）。
