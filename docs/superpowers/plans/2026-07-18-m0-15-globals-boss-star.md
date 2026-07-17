# M0-15 globals + boss_ui + 消弹转星星 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** M1 ECL 硬前置（`globals[1024]` 脚本状态地基 + `boss_ui` 公告板 + 三个 syscall 级写 API）+ 东方消弹经济闭环（消弹一律转星星，出生即磁吸，30 分/颗）。

**Architecture:** `WorldBody` 加两字段（derive 自动入校验和 + **`copy_into` 手工加拷贝行**）；新数据模块 `boss.rs`；`settle` 趟一消弹处逐弹原位生星。上游 spec：`docs/superpowers/specs/2026-07-18-m0-15-globals-boss-star.md`（短，先整读）。

**Tech Stack:** Rust 2024 / stg-core。复用 define_pool Item 池、磁吸机器、Checksum derive。

## Global Constraints

- **新字段必入校验和**（derive 自动）**且 `step.rs::copy_into` 必须加拷贝行**——漏拷 = 回滚丢状态，快照往返测试钉死。
- **P4-b 照 D12 表**：`set_var`/`get_var` 坏槽（≥1024）、`boss_set` 坏槽（≥MAX_BOSSES=2）→ no-op（get 回 0）+ `contract_viol` + `last_status = STATUS_BAD_ARGS`。`get_var` 取 `&mut self`（坏槽计数入校验和）。
- **消弹转化一律**、星星**无散布无 RNG**、磁吸目标 = 升序首个 ALIVE 自机（I4）、无则 `MAGNET_NONE`；道具池满逐颗计 `pool_full[POOL_ITEM]`（P4-a，不短路）。
- **世界自身不读** `globals`/`boss_ui`（P2 纯数据；boss_set 不校验 enemy 句柄有效性）。
- **TDD** 每任务红绿；合入前变异检验（T5）。commit 中文 conventional + 尾签 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 名字适配纪律：计划代码按假设名书写，实况不同以 codebase 为准（`Fx`/句柄/`ItemInit` 字段名等），所有适配记入报告。

---

### Task 1: globals + set_var/get_var

**Files:**
- Modify: `crates/stg-core/src/world.rs`（字段 + 两 API）、`crates/stg-core/src/step.rs`（copy_into 加行 + tests）

**Interfaces:**
- Produces: `WorldBody.globals: [i32; 1024]`（pub）；`pub fn set_var(&mut self, slot: u16, val: i32)`；`pub fn get_var(&mut self, slot: u16) -> i32`。`pub const GLOBALS_CAP: usize = 1024;`

- [ ] **Step 0: 开分支** `git checkout -b m0-15-globals-boss-star`
- [ ] **Step 1: 失败测试**（step.rs tests）

```rust
    /// globals 往返 + 坏槽 P4-b + 入校验和 + 快照往返（copy_into 漏拷即红）。
    #[test]
    fn globals_set_get_badslot_checksum_snapshot() {
        let mut w = World::new(1);
        let c0 = w.checksum();
        w.body.set_var(0, 42);
        w.body.set_var(1023, -7);
        assert_eq!(w.body.get_var(0), 42);
        assert_eq!(w.body.get_var(1023), -7);
        assert_ne!(w.checksum(), c0, "globals 必须入校验和");
        let cv0 = w.body.diag.contract_viol;
        w.body.set_var(1024, 1); // 坏槽
        assert_eq!(w.body.get_var(1024), 0); // 坏槽读 0
        assert_eq!(w.body.diag.contract_viol, cv0 + 2, "set/get 各计一次");
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.globals[1023], -7, "坏槽写不得触碰任何真槽");
        // 快照往返：copy_into 漏拷 globals 即红
        let mut snap = World::new(2);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.get_var(0), 42);
        w.body.set_var(0, 99);
        snap.copy_into(&mut w);
        assert_eq!(w.body.get_var(0), 42, "restore 必须还原 globals");
    }
```

- [ ] **Step 2: RED**（方法/字段不存在）
- [ ] **Step 3: 实现**——`WorldBody` 在 `rng` 后加 `pub globals: [i32; 1024]`（doc：全局变量竞技场，语义归脚本，世界自身不读不写——A2）；`copy_into` 加 `d.globals = s.globals;`；API 放写 API 区：

```rust
    /// ECL 全局变量槽写（D12）。slot ≥ 1024 → no-op + 计数（P4-b）。
    pub fn set_var(&mut self, slot: u16, val: i32) {
        if let Some(g) = self.globals.get_mut(slot as usize) {
            *g = val;
        } else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
        }
    }
    /// ECL 全局变量槽读（D12）。slot ≥ 1024 → 0 + 计数——取 &mut self 正是为了
    /// 坏槽计数入校验和（两机必须一样错）。
    pub fn get_var(&mut self, slot: u16) -> i32 {
        match self.globals.get(slot as usize) {
            Some(&v) => v,
            None => {
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                self.last_status = STATUS_BAD_ARGS;
                0
            }
        }
    }
```

  注意：`[i32; 1024]` 的 Checksum impl 若 derive/trait 只覆盖小数组（查 `checksum.rs` 的数组 impl——`signals: [u32; 8]` 走的哪条），不够就照现有模式补 impl（小端逐元素，与既有数组语义一致）。
- [ ] **Step 4: GREEN + 全套门 + Commit** `feat(world): globals[1024] + set_var/get_var——ECL 状态地基（校验和/快照往返判别测试）`

### Task 2: boss.rs + boss_ui + boss_set

**Files:**
- Create: `crates/stg-core/src/boss.rs`；Modify: `lib.rs`（mod）、`world.rs`（字段+API）、`step.rs`（copy_into + tests）

**Interfaces:**
- Produces: `boss::{MAX_BOSSES=2, BossUiSlot}`（字段照 spec B 节，`#[repr(C)] Clone Copy Checksum` + 手工 `Default`（`enemy: EnemyHandle::NULL`））；`WorldBody.boss_ui: [BossUiSlot; MAX_BOSSES]`（pub）；`pub fn boss_set(&mut self, slot: u8, ui: BossUiSlot)`。

- [ ] **Step 1: 失败测试**

```rust
    /// boss_ui 整槽写读 + 坏槽 P4-b + 入校验和 + 快照往返。
    #[test]
    fn boss_set_roundtrip_badslot_checksum_snapshot() {
        use crate::boss::{BossUiSlot, MAX_BOSSES};
        let mut w = World::new(1);
        let c0 = w.checksum();
        let ui = BossUiSlot {
            enemy: crate::enemy::EnemyHandle::NULL,
            hp_ratio: Fx::from_raw(32768),
            spell_id: 7,
            timer_frames: 3600,
            phase_left: 2,
            active: 1,
        };
        w.body.boss_set(0, ui);
        assert_eq!(w.body.boss_ui[0].spell_id, 7);
        assert_eq!(w.body.boss_ui[0].active, 1);
        assert_ne!(w.checksum(), c0, "boss_ui 必须入校验和");
        let cv0 = w.body.diag.contract_viol;
        w.body.boss_set(MAX_BOSSES as u8, ui); // 坏槽
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.boss_ui[1].active, 0, "坏槽写不得溢到别槽");
        let mut snap = World::new(2);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.boss_ui[0].spell_id, 7, "快照必须带 boss_ui");
    }
```

- [ ] **Step 2-4: RED → 实现 → GREEN + Commit**。boss.rs 模块 doc 写明"脚本写、UI 读、世界自身不读；超时/换卡判断归 boss 主控任务（A2）"。`boss_set` 坏槽处置同 set_var 模式；`copy_into` 加 `d.boss_ui = s.boss_ui;`。若 `EnemyHandle::NULL` 名不符（查 enemy.rs），以实况为准。Commit：`feat(world): boss_ui 公告板 + boss_set 整槽写——M1 boss 血条/符卡 UI 地基`

### Task 3: ITEM_STAR + 消弹转星星

**Files:**
- Modify: `crates/stg-core/src/items.rs`（四步清单 1/2/4）、`crates/stg-core/src/world/settle.rs`（趟一挂点 + credit_item STAR 臂 + tests）

**Interfaces:**
- Produces: `items::ITEM_STAR = 4`、`ITEM_CFG` STAR 行（score=30）；settle 内部星星生成路径。

- [ ] **Step 1: 先读现状**——settle.rs 趟一消弹循环（`FIELD_CLEAR_BULLETS` 处置 + `FieldCleared` count）、`credit_item` 的臂结构、items.rs `ITEM_CFG`/`ItemInit` 全字段、`LIFE_ALIVE` 判定惯用法、模块 doc 的扩展四步清单。**测试写在既有道具测试同址**（settle.rs 或 step.rs，从众）。
- [ ] **Step 2: 失败测试**（按实况适配字段名；核心判别断言如下）

```rust
    /// 消弹转星星：3 弹异位被全屏 field 消 → 恰 3 星各在原弹位、磁吸指向 ALIVE 自机。
    #[test]
    fn clear_converts_bullets_to_stars_at_positions() {
        // 造 3 颗异位弹 → 铺全屏消弹 field → step 过 settle →
        // 断言 items 池恰 3 颗、item_type 全 ITEM_STAR、(x,y) 与三弹原位一一对应（升序）、
        // magnet_to == 0（P0 ALIVE）、FieldCleared.count == 3。
    }
    /// 无 ALIVE 自机 → MAGNET_NONE 正常下落。
    #[test]
    fn star_without_alive_player_falls() { /* 自机置死亡窗口态后消弹，断言 magnet_to == MAGNET_NONE */ }
    /// 拾取入账 +30。
    #[test]
    fn star_credit_adds_30_score() { /* 星星磁吸到位拾取 → score +30、EVT_ITEM_PICKED */ }
    /// 道具池满 → 少生成 + pool_full[POOL_ITEM] 逐颗计数。
    #[test]
    fn star_pool_full_degrades_counted() { /* 预填道具池到剩 1，消 3 弹 → 1 星 + 计数 +2 */ }
```

- [ ] **Step 3: RED → 实现**——items.rs：`ITEM_STAR` + CFG 行（score 30，磁吸参数抄 STD 基线）+ 有效性断言随 CFG 长度自动覆盖；settle 趟一：消弹标记处逐弹调内部 `fn spawn_star_at(&mut self, x: Fx, y: Fx)`（升序首个 ALIVE 自机 → `magnet_to`；ItemInit 全字段写满 `vx/vy=0, timer=0`；池满逐颗计数）；`credit_item` 加 STAR 臂（score += 30，读 CFG）。掉落表不动（回归腿：既有 drop 测试全绿即证）。
- [ ] **Step 4: GREEN + Commit** `feat(world): 消弹一律转星星——原位生成/出生即磁吸/30 分入账/池满降级（D9 趟一）`

### Task 4: 金向量验证（导演零改动）

- [ ] director 函数 doc 补一句（现有每 150 帧全屏消弹自动转星入流：星星雨/磁吸/入账/可能的道具池满全链）；金向量**双跑 diff → IDENTICAL** + 引一帧消弹后的校验和对比改前基线（worktree at HEAD~ 法，禁 stash）证明入流；`cargo test --workspace` 全绿。
- [ ] Commit：`feat(harness): 金向量消弹转星入流验证——导演零改动 + 双跑对拍`

### Task 5: 变异检验 + 收尾 + 终审 + 收枝

- [ ] 变异三杀（反向 Edit 还原，禁 checkout/restore/stash）：①星星生成位置改 `Fx::ZERO` 常量 → 原位测试红；②credit STAR 臂 30→0 → 入账测试红；③`copy_into` 删 `d.globals` 行 → 快照往返测试红。树净 + 全绿。
- [ ] 回写：`stg-world-design.md` D12 成员表括注（set_var/get_var/boss_set 已落地 M0-15）+ D9 趟一补消弹转星句 + D7 道具类型表 STAR 行；`PROGRESS.md` 史行 + 现在段（下一步候选：M1 ECL · bomb · power→火力 · 激光池）。
- [ ] 全绿门 → docs commit → 终审（最强模型全分支）→ `superpowers:finishing-a-development-branch`。

---

## Self-Review 记录

- **Spec 覆盖**：A→T1、B→T2、C→T3、金向量→T4、变异/回写→T5。无缺口。
- **占位说明**：T3 测试体是断言大纲非完整代码——settle 造场惯用法（field 铺设/step 推进/自机置态）必须从既有 M0-12 测试抄，先读后写是该任务 Step 1 的硬要求；T1/T2 为完整代码。
- **类型一致**：`GLOBALS_CAP=1024`/`MAX_BOSSES=2`/`ITEM_STAR=4`/score 30 全计划一致；`get_var` 取 `&mut self` 两处一致。
