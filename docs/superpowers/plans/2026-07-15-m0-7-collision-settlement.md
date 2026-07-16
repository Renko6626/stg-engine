# M0-7 碰撞 + 结算 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给"能跑的世界"装上后果——自机弹打死敌人、敌弹打死自机、敌体撞死自机、擦弹计数，全部确定性、跨平台逐字节一致。

**Architecture:** 新增 `EnemyPool`（D5 全字段）与两条输出缓冲 `hits`/`events`（A5）。碰撞相位 6 **纯读只收集** `hits`（圆-圆半径和 + i64 平方距离不开根，暴力 O(N×M)）；结算相位 7 是**唯一改状态者**，按 D9 三趟消费 `hits`（趟一清弹空/趟二伤害/趟三计分）。自机生死状态机的**中弹触发**在 settle、**计时推进**在 update_players（相位 3）。金向量扩成一条碰撞病态诊断场景。

**Tech Stack:** Rust 2024 / stg-core（断层线以下，纯定点、无浮点/时钟/宿主 RNG）/ `define_pool!` proc-macro / FNV-1a 校验和。

## Global Constraints

- **I1 数值**：唯一标量 `Fx = Q16.16(i32)`；断层线以下禁 `f32/f64`。碰撞比较用平方距离（i64），不开根。
- **I4 顺序**：一切遍历按池索引升序；碰撞收集循环嵌套固定 → 收集序确定；禁无序容器参与模拟。
- **I7 布局**：`World` 内无指针/堆容器；新缓冲 `[Hit; 8192]` / `[Event; 512]` **内联**在 WorldBody（快照=整块字节）。
- **P4 错误三铁律**：(a) 资源耗尽（hits/events 满、池满）→ 确定性降级不 panic，计数入校验和；(b) 调用方违约（悬垂句柄）→ 安全 no-op；(c) 引擎 bug → debug 帧内断言。
- **P6 全量校验**：住 World 的字段一律入校验和；唯一豁免是三条纯输出缓冲（`hits`/`events` + 各自 len），`#[checksum(skip="理由")]` 带理由。
- **只收集不改状态硬规则**：collide 纯读、只 append `hits`；改状态只准在 settle。
- **复用槽写满**：`EnemyInit` exhaustive（漏字段编译不过）。
- **确定性契约**：`define_pool!` 生成 `EnemyPool`/`EnemyHandle{index:u16,generation:u16}`/`EnemyInit`；字段数组 + `generation`/`alive` 均 `pub(crate)`；方法 `new/alloc/get/free/is_alive/iter_alive/copy_into/free_index`、常量 `CAP`。

---

## File Structure

- **Create** `crates/stg-core/src/enemy.rs` — `EnemyPool`（D5 全字段 `define_pool!`）+ `ENEMY_DYING` 位常量。
- **Create** `crates/stg-core/src/events.rs` — `Hit`、`Event` 类型 + 容量/矩阵行号/事件种类常量。
- **Modify** `crates/stg-core/src/lib.rs` — `pub mod enemy; pub mod events;`。
- **Modify** `crates/stg-core/src/player.rs` — 生死状态常量（DEATHWINDOW/RESPAWNING/GAMEOVER）+ 决死窗口/重生无敌/出生坐标常量。
- **Modify** `crates/stg-core/src/world.rs` — WorldBody 接入 `enemies`/`hits`/`events`；`DiagCounters` 加溢出计数；`POOL_ENEMY`/`create_enemy`/`push_hit`/`push_event`；`begin` 清缓冲；`integrate`/`cleanup` 加敌人；`collide`/`settle` 实现；`update_players` 重构 + `commit_death`。
- **Modify** `crates/stg-core/src/step.rs` — `copy_into` 拷贝 `enemies`（缓冲跳过）。
- **Modify** `crates/stg-harness/src/main.rs` — 金向量扩成碰撞诊断场景。

## 关键类型与常量（贯穿全计划，先在此定稿）

**矩阵行号 / 事件种类 / 容量**（`events.rs`）:
```rust
pub(crate) const HITS_CAP: usize = 8192;
pub(crate) const EVENTS_CAP: usize = 512;

pub(crate) const ROW_BULLET_PLAYER_HIT: u8 = 1;   // 敌弹 × 自机 hit_radius → 中弹
pub(crate) const ROW_BULLET_PLAYER_GRAZE: u8 = 2; // 敌弹 × 自机 graze_radius → 擦弹
pub(crate) const ROW_BODY_PLAYER_HIT: u8 = 3;     // 敌体 × 自机 hit_radius → 中弹
pub(crate) const ROW_SHOT_ENEMY: u8 = 4;          // 自机弹 × 敌人 hurtbox → 扣血

pub const EVT_ENEMY_DIED: u8 = 1;
pub const EVT_PLAYER_DIED: u8 = 2;
```

**生死状态**（`player.rs`，已有 `LIFE_ABSENT=0`/`LIFE_ALIVE=1`）:
```rust
pub const LIFE_DEATHWINDOW: u8 = 2;
pub const LIFE_RESPAWNING: u8 = 3;
pub const LIFE_GAMEOVER: u8 = 4;
pub const DEATHBOMB_WINDOW: u16 = 8;  // 决死窗口帧
pub const RESPAWN_INVULN: u16 = 120;  // 重生无敌帧（2 秒 @60Hz）
```

---

## Task 1: EnemyPool 模块 + create_enemy + integrate/cleanup 敌人

**Files:**
- Create: `crates/stg-core/src/enemy.rs`
- Modify: `crates/stg-core/src/lib.rs`
- Modify: `crates/stg-core/src/world.rs`
- Modify: `crates/stg-core/src/step.rs`

**Interfaces:**
- Produces: `enemy::EnemyPool` / `EnemyHandle` / `EnemyInit`（`define_pool!`）；`enemy::ENEMY_DYING: u8`；`WorldBody.enemies: EnemyPool`；`WorldBody::create_enemy(&mut self, EnemyInit) -> EnemyHandle`；`world::POOL_ENEMY: usize = 2`。
- Consumes: `define_pool!`、`math::Fx`、既有 integrate/cleanup 掩码字遍历模式（world.rs 现有 bullets 段）。

- [ ] **Step 1: 写 enemy.rs（EnemyPool + dying 位）**

Create `crates/stg-core/src/enemy.rs`:
```rust
//! 敌人池（D5）——`define_pool!` 第 3 个实例。SoA ~70 B/敌，cap 256。
//! 本切片：全字段落池 + 直线积分（move_to 插值器/主控 AI 延后，字段惰性）。
//! `flags` 的 dying 位由 settle 置、cleanup 回收（敌人槽活到相位 8 供死亡脚本/表现层读）。

use crate::define_pool;
use crate::math::Fx;

/// `flags` 的 dying 标记位（D5 预留位）：settle 命中致死置位，cleanup 回收。
pub const ENEMY_DYING: u8 = 1 << 0;

define_pool! {
    Enemy, cap = 256,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        mv_from_x: Fx, mv_from_y: Fx, mv_to_x: Fx, mv_to_y: Fx,
        mv_t: u16, mv_dur: u16, mv_easing: u8, mv_active: u8,
        hp: i32, hp_max: i32,
        radius: Fx, hurtbox: Fx,
        invuln: u16, hit_flash: u8, flags: u8,
        sprite: u16, anm_state: u16,
        main_task: u32, death_script: u16, drop_table: u16,
        score: u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全字段 Init 助手（exhaustive；move_to/挂钩字段本切片惰性置零）。
    fn enemy_at(x: i32, y: i32, hp: i32) -> EnemyInit {
        EnemyInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp,
            hp_max: hp,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 100,
        }
    }

    #[test]
    fn enemy_pool_alloc_get_free() {
        let mut p = EnemyPool::new();
        let h = p.alloc(enemy_at(10, 20, 5)).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.hp[i], 5);
        assert_eq!(p.x[i], Fx::from_int(10));
        assert!(p.free(h));
        assert_eq!(p.get(h), None);
    }

    #[test]
    fn enemy_pool_new_deterministic() {
        assert_eq!(EnemyPool::new().checksum(), EnemyPool::new().checksum());
    }
}
```

- [ ] **Step 2: 注册模块**

Modify `crates/stg-core/src/lib.rs`, 在 `pub mod bullets;` 之后一行加 `pub mod enemy;`（保持字母序附近；紧跟 bullets 即可）:
```rust
pub mod bullets;
pub mod checksum;
pub mod enemy;
pub mod input;
```

- [ ] **Step 3: 跑池单测（应通过）**

Run: `cargo test -p stg-core enemy::tests`
Expected: PASS（`enemy_pool_alloc_get_free`、`enemy_pool_new_deterministic`）。

- [ ] **Step 4: 写 WorldBody 接入敌人的失败测试**

Modify `crates/stg-core/src/world.rs`，在 `mod tests` 内追加（借用 step.rs 的 World 构造）:
```rust
    #[test]
    fn create_enemy_and_integrate_moves() {
        use crate::enemy::EnemyInit;
        let mut w = crate::step::World::new(1);
        let init = EnemyInit {
            x: Fx::ZERO, y: Fx::from_int(50), vx: Fx::from_int(1), vy: Fx::from_int(2),
            mv_from_x: Fx::ZERO, mv_from_y: Fx::ZERO, mv_to_x: Fx::ZERO, mv_to_y: Fx::ZERO,
            mv_t: 0, mv_dur: 0, mv_easing: 0, mv_active: 0,
            hp: 5, hp_max: 5, radius: Fx::from_int(12), hurtbox: Fx::from_int(16),
            invuln: 0, hit_flash: 0, flags: 0, sprite: 0, anm_state: 0,
            main_task: 0, death_script: 0, drop_table: 0, score: 100,
        };
        let h = w.body.create_enemy(init);
        assert_ne!(h, crate::enemy::EnemyHandle::NULL);
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.x[i], Fx::from_int(1)); // 0+1
        assert_eq!(w.body.enemies.y[i], Fx::from_int(52)); // 50+2
    }
```

- [ ] **Step 5: 跑测试确认失败**

Run: `cargo test -p stg-core create_enemy_and_integrate_moves`
Expected: FAIL（编译错误：`enemies` 字段不存在、`create_enemy` 未定义）。

- [ ] **Step 6: WorldBody 加 enemies 字段 + POOL_ENEMY + create_enemy**

Modify `crates/stg-core/src/world.rs`:

(a) 顶部 use 加：
```rust
use crate::enemy::{EnemyHandle, EnemyInit, EnemyPool, ENEMY_DYING};
```

(b) 池 id 常量区加：
```rust
pub const POOL_ENEMY: usize = 2;
```

(c) `WorldBody` 结构体加字段（放在 `shots` 之后、`diag` 之前）:
```rust
    pub enemies: EnemyPool,
```

(d) 写 API 区加 `create_enemy`（紧跟 `create_player_shot`）:
```rust
    /// 创建一个敌人（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_enemy(&mut self, init: EnemyInit) -> EnemyHandle {
        match self.enemies.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_ENEMY] = self.diag.pool_full[POOL_ENEMY].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                EnemyHandle::NULL
            }
        }
    }
```

- [ ] **Step 7: integrate 加敌人段（直线积分 + 计时器 tick）**

Modify `crates/stg-core/src/world.rs` 的 `integrate`，在自机弹段之后追加:
```rust
        // 敌人：pos += vel（move_to 插值器延后，mv_* 惰性）+ 计时器 tick
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.enemies.x[i] = self.enemies.x[i] + self.enemies.vx[i];
                self.enemies.y[i] = self.enemies.y[i] + self.enemies.vy[i];
                if self.enemies.invuln[i] > 0 {
                    self.enemies.invuln[i] -= 1;
                }
                if self.enemies.hit_flash[i] > 0 {
                    self.enemies.hit_flash[i] -= 1;
                }
            }
        }
```

- [ ] **Step 8: cleanup 加敌人段（dying 或越界回收）**

Modify `crates/stg-core/src/world.rs` 的 `cleanup`，在自机弹段之后追加:
```rust
        // 敌人：dying 标记或越界 → 回收
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.enemies.flags[i] & ENEMY_DYING != 0)
                    || Self::out_of_bounds(self.enemies.x[i], self.enemies.y[i]);
                if dead {
                    self.enemies.free_index(i);
                }
            }
        }
```

- [ ] **Step 9: copy_into 拷贝敌人**

Modify `crates/stg-core/src/step.rs` 的 `copy_into`，在 `s.shots.copy_into(&mut d.shots);` 之后加:
```rust
        s.enemies.copy_into(&mut d.enemies);
```

- [ ] **Step 10: 跑测试确认通过**

Run: `cargo test -p stg-core create_enemy_and_integrate_moves`
Expected: PASS。

- [ ] **Step 11: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS，clippy 无警告。

- [ ] **Step 12: Commit**

```bash
git add crates/stg-core/src/enemy.rs crates/stg-core/src/lib.rs crates/stg-core/src/world.rs crates/stg-core/src/step.rs
git commit -m "feat(world): EnemyPool(D5 全字段) + create_enemy + 敌人 integrate/cleanup

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: hits/events 输出缓冲 + push 助手 + begin 清空

**Files:**
- Create: `crates/stg-core/src/events.rs`
- Modify: `crates/stg-core/src/lib.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `events::{Hit, Event, HITS_CAP, EVENTS_CAP, ROW_*, EVT_*}`；`WorldBody.hits/hits_len/events/events_len`（checksum-skip）；`WorldBody::push_hit(&mut self, row:u8, active:u16, passive:u16)`、`push_event(&mut self, Event)`；`DiagCounters.hits_overflow/events_overflow`（入校验和）。
- Consumes: Task 1 的 WorldBody；`math::Fx`。

- [ ] **Step 1: 写 events.rs（Hit/Event/常量）**

Create `crates/stg-core/src/events.rs`:
```rust
//! 碰撞命中缓冲 `Hit` 与世界大事记 `Event`（A5）。两者皆**纯输出**：帧内私有、
//! checksum-skip、begin 清空、重演确定性再生。Hit 由相位 6 收集、相位 7 三趟消费；
//! Event 由相位 7/相位 3 死亡结算产出，相位 8 ECL 挂钩 + 表现层只读消费。

use crate::math::Fx;

pub(crate) const HITS_CAP: usize = 8192;
pub(crate) const EVENTS_CAP: usize = 512;

// ── 碰撞矩阵行号（D8）─────────────────────────────────────────────
pub(crate) const ROW_BULLET_PLAYER_HIT: u8 = 1;
pub(crate) const ROW_BULLET_PLAYER_GRAZE: u8 = 2;
pub(crate) const ROW_BODY_PLAYER_HIT: u8 = 3;
pub(crate) const ROW_SHOT_ENEMY: u8 = 4;

// ── 事件种类 ──────────────────────────────────────────────────────
pub const EVT_ENEMY_DIED: u8 = 1;
pub const EVT_PLAYER_DIED: u8 = 2;

/// 一条碰撞命中（6 B）：矩阵行 + 主动/被动池索引。收集序天然按收集循环嵌套，无需排序。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Hit {
    pub row: u8,
    pub active: u16,
    pub passive: u16,
}

/// 一条世界大事记（A5）：`a_index/a_gen` = 相关实体句柄（玩家死亡时 a_index=自机号、a_gen=0）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Event {
    pub kind: u8,
    pub a_index: u16,
    pub a_gen: u16,
    pub x: Fx,
    pub y: Fx,
    pub data: [i32; 2],
}
```

- [ ] **Step 2: 注册模块**

Modify `crates/stg-core/src/lib.rs`，在 `pub mod enemy;` 之后加 `pub mod events;`:
```rust
pub mod enemy;
pub mod events;
pub mod input;
```

- [ ] **Step 3: 写 push/清空的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加:
```rust
    #[test]
    fn hits_push_clear_and_overflow() {
        use crate::events::HITS_CAP;
        let mut w = crate::step::World::new(1);
        w.body.push_hit(1, 3, 0);
        w.body.push_hit(2, 4, 0);
        assert_eq!(w.body.hits_len, 2);
        // 溢出：填满后再推 → 停收 + 计数，不 panic
        w.body.hits_len = HITS_CAP as u16;
        w.body.push_hit(1, 0, 0);
        assert_eq!(w.body.hits_len, HITS_CAP as u16); // 未增
        assert_eq!(w.body.diag.hits_overflow, 1);
        // begin 清空
        w.body.begin();
        assert_eq!(w.body.hits_len, 0);
        assert_eq!(w.body.events_len, 0);
    }

    #[test]
    fn events_push_records_fact() {
        use crate::events::{Event, EVT_PLAYER_DIED};
        let mut w = crate::step::World::new(1);
        w.body.push_event(Event {
            kind: EVT_PLAYER_DIED,
            a_index: 0,
            a_gen: 0,
            x: Fx::ZERO,
            y: Fx::from_int(384),
            data: [2, 0],
        });
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_PLAYER_DIED);
    }
```

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test -p stg-core hits_push_clear_and_overflow`
Expected: FAIL（`hits`/`push_hit`/`hits_overflow` 未定义）。

- [ ] **Step 5: WorldBody 加缓冲字段 + diag 溢出计数 + push 助手 + begin 清空**

Modify `crates/stg-core/src/world.rs`:

(a) 顶部 use 加：
```rust
use crate::events::{Event, Hit, EVENTS_CAP, HITS_CAP};
```

(b) `DiagCounters` 加两个溢出计数（入校验和，P4-a）:
```rust
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum)]
pub struct DiagCounters {
    pub pool_full: [u32; 8], // 按池 id
    pub contract_viol: u32,
    pub hits_overflow: u32,   // hits 满丢弃计数（P4-a）
    pub events_overflow: u32, // events 满丢弃计数（P4-a）
}
```

(c) `WorldBody` 加缓冲字段（放在 `enemies` 之后、`diag` 之前）:
```rust
    #[checksum(skip = "纯输出缓冲，帧内私有，重演确定性再生（A5）")]
    pub(crate) hits: [Hit; HITS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 hits 一并 skip（A5）")]
    pub(crate) hits_len: u16,
    #[checksum(skip = "纯输出缓冲，相位 8/表现层只读，重演确定性再生（A5）")]
    pub events: [Event; EVENTS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 events 一并 skip（A5）")]
    pub events_len: u16,
```

(d) `begin` 清空两条缓冲:
```rust
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN);
        self.hits_len = 0;
        self.events_len = 0;
    }
```

(e) 写 API 区加 push 助手（紧跟 `create_enemy`）:
```rust
    /// 收集一条碰撞命中（P4-a：满则停收 + 计数，不 panic）。
    pub(crate) fn push_hit(&mut self, row: u8, active: u16, passive: u16) {
        if (self.hits_len as usize) < HITS_CAP {
            self.hits[self.hits_len as usize] = Hit { row, active, passive };
            self.hits_len += 1;
        } else {
            self.diag.hits_overflow = self.diag.hits_overflow.wrapping_add(1);
        }
    }

    /// 产出一条世界大事记（P4-a：满则丢弃 + 计数，不 panic）。
    pub(crate) fn push_event(&mut self, ev: Event) {
        if (self.events_len as usize) < EVENTS_CAP {
            self.events[self.events_len as usize] = ev;
            self.events_len += 1;
        } else {
            self.diag.events_overflow = self.diag.events_overflow.wrapping_add(1);
        }
    }
```

> **注**：`copy_into` **不**拷贝 `hits`/`events`（帧内私有、checksum-skip、下次 begin 清空重生）——省 ~60KB/快照且不影响校验和。此为有意为之，无需改 step.rs。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p stg-core hits_push_clear_and_overflow events_push_records_fact`
Expected: PASS。

- [ ] **Step 7: 快照往返不受缓冲影响（回归既有测试）**

Run: `cargo test -p stg-core snapshot_restore_roundtrip`
Expected: PASS（缓冲 checksum-skip，restore 后指纹仍等）。

- [ ] **Step 8: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。

- [ ] **Step 9: Commit**

```bash
git add crates/stg-core/src/events.rs crates/stg-core/src/lib.rs crates/stg-core/src/world.rs
git commit -m "feat(world): hits/events 输出缓冲 + push 助手 + begin 清空 + P4-a 溢出计数

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## ⏸ 检查点 ①（Task 1–2）：地基就绪，停下等 CI 绿 + 用户确认

推分支、盯 CI 三平台绿。**闸门 = determinism-gate 三平台逐字节一致**。注意：金向量校验和**绝对值会变**（`enemies` 空池 + `diag` 新字段都入校验和被哈希，即便全零也改变 FNV 累加）——这是预期，不与合并前比对，只看三平台是否一致。缓冲 `hits`/`events` 是 checksum-skip、且敌人未生成，故无额外行为。

---

## Task 3: collide 行 1/2（敌弹 × 自机：中弹 + 擦弹）

**Files:**
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `WorldBody::collide` 不再是 stub；私有 `collide_bullets_player(&mut self)`。
- Consumes: `math::geom::len_sq`；`events::{ROW_BULLET_PLAYER_HIT, ROW_BULLET_PLAYER_GRAZE}`；`player::LIFE_ALIVE`；Task 2 的 `push_hit`。

**算法**：对每个 `Alive && invuln==0` 的自机，扫所有 `delay==0` 的活弹，一次 `len_sq` 复用两半径——`d2 <= (弹.radius + graze_radius)²` 收行 2；再 `<= (弹.radius + hit_radius)²` 收行 1。半径和相加安全（皆小），平方停 i64（Q32.32）直接比、不开根。

- [ ] **Step 1: 写行 1/2 的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加:
```rust
    // 造一颗停在 (x,y) 的哑弹（半径 2）。
    fn bullet_at(w: &mut crate::step::World, x: i32, y: i32) {
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::from_int(x), y: Fx::from_int(y),
            vx: Fx::ZERO, vy: Fx::ZERO, speed: Fx::ZERO, angle: crate::math::Angle::ZERO,
            ang_vel: 0, accel: Fx::ZERO, ax: Fx::ZERO, ay: Fx::ZERO,
            sprite: 0, radius: Fx::from_int(2), delay: 0, life: 0xFFFF,
            flags: 0, grazed_by: 0, transform_head: 0xFFFF, xform_wait: 0, xform_next: 0,
        });
    }

    #[test]
    fn collide_bullet_on_player_collects_hit_and_graze() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        // 自机在 (0,384)，hit_radius=2.5、graze_radius=16。弹压在自机身上 → 中弹+擦弹都收。
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        w.body.collide();
        let hit = (0..w.body.hits_len as usize).filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT).count();
        let graze = (0..w.body.hits_len as usize).filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE).count();
        assert_eq!(hit, 1);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_near_bullet_grazes_only() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 10, 384); // 距 10px：在 graze 圈(≈18)内、hit 圈(≈4.5)外
        w.body.collide();
        let hit = (0..w.body.hits_len as usize).filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT).count();
        let graze = (0..w.body.hits_len as usize).filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE).count();
        assert_eq!(hit, 0);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_skips_invuln_player() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        w.body.players[0].invuln = 60; // 无敌 → 不参与
        bullet_at(&mut w, 0, 384);
        w.body.collide();
        assert_eq!(w.body.hits_len, 0);
    }

    #[test]
    fn collide_skips_delay_bullet() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        // 把刚造的弹设 delay>0（索引 0）
        w.body.bullets.delay[0] = 5;
        w.body.collide();
        assert_eq!(w.body.hits_len, 0);
    }
```

> **注**：测试直接调 `w.body.collide()`。因 PhaseGuard 要求保序，需先让 guard 处于 collide 相位——见下步在测试里用 `phase_guard` 直接置位；或简化：collide 测试**在 debug 下**需绕过 guard。采用**直接置 guard**方案（仅测试内）。将下列改动并入 Step 1 的每个测试：在 `w.body.collide()` 前加一行 `#[cfg(debug_assertions)] { w.body.phase_guard = crate::world::PH_COLLIDE; }`。

修正：把上面四个测试里的 `w.body.collide();` 均改为先置 guard：
```rust
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
```
（`PH_COLLIDE` 已是 `pub(crate)`，测试在同 crate 内可见。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core collide_bullet_on_player`
Expected: FAIL（collide 是 stub，hits_len 恒 0）。

- [ ] **Step 3: 实现 collide 行 1/2**

Modify `crates/stg-core/src/world.rs`，替换 `collide` stub:
```rust
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
    }

    /// 行 1（hit）+ 行 2（graze）：敌弹 × 自机。一次 len_sq 复用两半径。
    fn collide_bullets_player(&mut self) {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        use crate::math::geom::len_sq;
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE || self.players[p].invuln != 0
            {
                continue; // 门禁：只 Alive 且非无敌参与
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let graze_r = self.players[p].graze_radius;
            let nw = self.bullets.alive.len();
            for w in 0..nw {
                let mut bits = self.bullets.alive[w];
                while bits != 0 {
                    let b = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.bullets.delay[b] > 0 {
                        continue; // delay 弹不参与
                    }
                    let dx = self.bullets.x[b] - px;
                    let dy = self.bullets.y[b] - py;
                    let d2 = len_sq(dx, dy);
                    let br = self.bullets.radius[b];
                    let graze_sum = (br + graze_r).raw() as i64;
                    if d2 <= graze_sum * graze_sum {
                        self.push_hit(ROW_BULLET_PLAYER_GRAZE, b as u16, p as u16);
                        let hit_sum = (br + hit_r).raw() as i64;
                        if d2 <= hit_sum * hit_sum {
                            self.push_hit(ROW_BULLET_PLAYER_HIT, b as u16, p as u16);
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core collide_`
Expected: PASS（四个 collide 测试全绿）。

- [ ] **Step 5: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-core/src/world.rs
git commit -m "feat(world): collide 行1/2 敌弹×自机（半径和平方距离，一次 len_sq 复用两半径）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: collide 行 3（敌体 × 自机）+ 行 4（自机弹 × 敌人）

**Files:**
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: 私有 `collide_body_player(&mut self)`、`collide_shot_enemy(&mut self)`；`collide` 调这两者。
- Consumes: `events::{ROW_BODY_PLAYER_HIT, ROW_SHOT_ENEMY}`；`enemy::ENEMY_DYING`。

**算法**：行 3 = 敌.radius（体碰）+ 自机.hit_radius；行 4 = 自机弹.radius + 敌.hurtbox（受击）。行 4 嵌套固定为 **shot 外层、enemy 内层**（升序），使 settle 扣血序确定。**行 4 不查敌无敌帧**（事件照收，无敌帧过滤留给 settle）。

- [ ] **Step 1: 写行 3/4 的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加:
```rust
    fn spawn_enemy(w: &mut crate::step::World, x: i32, y: i32, hp: i32) -> crate::enemy::EnemyHandle {
        w.body.create_enemy(crate::enemy::EnemyInit {
            x: Fx::from_int(x), y: Fx::from_int(y), vx: Fx::ZERO, vy: Fx::ZERO,
            mv_from_x: Fx::ZERO, mv_from_y: Fx::ZERO, mv_to_x: Fx::ZERO, mv_to_y: Fx::ZERO,
            mv_t: 0, mv_dur: 0, mv_easing: 0, mv_active: 0,
            hp, hp_max: hp, radius: Fx::from_int(12), hurtbox: Fx::from_int(16),
            invuln: 0, hit_flash: 0, flags: 0, sprite: 0, anm_state: 0,
            main_task: 0, death_script: 0, drop_table: 0, score: 100,
        })
    }

    #[test]
    fn collide_enemy_body_on_player() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        spawn_enemy(&mut w, 0, 100, 5); // 敌体 radius 12 + 自机 hit 2.5 → 圆心重合必撞
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
        let n = (0..w.body.hits_len as usize).filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn collide_shot_on_enemy() {
        use crate::events::ROW_SHOT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 5);
        let ei = w.body.enemies.get(e).unwrap();
        // 造一发压在敌人身上的自机弹
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei], y: w.body.enemies.y[ei],
            vx: Fx::ZERO, vy: Fx::ZERO, damage: 1, radius: Fx::from_int(4),
            sprite: 0, owner: 0, flags: 0,
        });
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
        let hits: Vec<_> = (0..w.body.hits_len as usize).map(|k| w.body.hits[k]).filter(|h| h.row == ROW_SHOT_ENEMY).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // shot 索引
        assert_eq!(hits[0].passive as usize, ei); // enemy 索引
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core collide_enemy_body_on_player collide_shot_on_enemy`
Expected: FAIL（行 3/4 未实现）。

- [ ] **Step 3: 实现行 3/4**

Modify `crates/stg-core/src/world.rs` 的 `collide`，扩为:
```rust
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
        self.collide_body_player(); // 行 3：敌体 × 自机
        self.collide_shot_enemy(); // 行 4：自机弹 × 敌人
    }
```

在 `collide_bullets_player` 之后加两个私有方法:
```rust
    /// 行 3：敌体（enemy.radius）× 自机 hit_radius。
    fn collide_body_player(&mut self) {
        use crate::events::ROW_BODY_PLAYER_HIT;
        use crate::math::geom::len_sq;
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE || self.players[p].invuln != 0
            {
                continue;
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let nw = self.enemies.alive.len();
            for w in 0..nw {
                let mut bits = self.enemies.alive[w];
                while bits != 0 {
                    let e = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let dx = self.enemies.x[e] - px;
                    let dy = self.enemies.y[e] - py;
                    let d2 = len_sq(dx, dy);
                    let sum = (self.enemies.radius[e] + hit_r).raw() as i64;
                    if d2 <= sum * sum {
                        self.push_hit(ROW_BODY_PLAYER_HIT, e as u16, p as u16);
                    }
                }
            }
        }
    }

    /// 行 4：自机弹（shot.radius）× 敌人 hurtbox（受击圈）。
    /// 嵌套固定：shot 外层、enemy 内层（升序）→ settle 扣血序确定。无敌帧过滤留给 settle。
    fn collide_shot_enemy(&mut self) {
        use crate::events::ROW_SHOT_ENEMY;
        use crate::math::geom::len_sq;
        let ne = self.enemies.alive.len();
        let nws = self.shots.alive.len();
        for sw in 0..nws {
            let mut sbits = self.shots.alive[sw];
            while sbits != 0 {
                let s = sw * 64 + sbits.trailing_zeros() as usize;
                sbits &= sbits - 1;
                let (sx, sy) = (self.shots.x[s], self.shots.y[s]);
                let sr = self.shots.radius[s];
                for ew in 0..ne {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - sx;
                        let dy = self.enemies.y[e] - sy;
                        let d2 = len_sq(dx, dy);
                        let sum = (sr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_SHOT_ENEMY, s as u16, e as u16);
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core collide_enemy_body_on_player collide_shot_on_enemy`
Expected: PASS。

- [ ] **Step 5: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-core/src/world.rs
git commit -m "feat(world): collide 行3 敌体×自机 + 行4 自机弹×敌人（shot 外层升序，无敌帧留待 settle）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## ⏸ 检查点 ②（Task 3–4）：碰撞收集完成，停下等 CI 绿 + 用户确认

collide 现产 hits，但 settle 仍是 stub → hits 无消费者、**世界状态不变**（hits checksum-skip，collide 本身不改校验和；绝对值仍是检查点 ① 后那一版）。推分支盯 CI 三平台一致。

---

## Task 5: settle 趟二（敌人扣血/dying/EnemyDied + 中弹→DeathWindow）+ 生死计时

**Files:**
- Modify: `crates/stg-core/src/player.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `player::{LIFE_DEATHWINDOW, LIFE_RESPAWNING, LIFE_GAMEOVER, DEATHBOMB_WINDOW, RESPAWN_INVULN}`；`settle` 趟二逻辑；`WorldBody::commit_death(&mut self, usize)`；`update_players` 重构为生死状态机。
- Consumes: `events::{ROW_SHOT_ENEMY, ROW_BULLET_PLAYER_HIT, ROW_BODY_PLAYER_HIT, EVT_ENEMY_DIED, EVT_PLAYER_DIED, Event}`；`enemy::ENEMY_DYING`；Task 2 的 `push_event`。

- [ ] **Step 1: player.rs 加生死常量**

Modify `crates/stg-core/src/player.rs`，在 `LIFE_ALIVE` 之后加:
```rust
pub const LIFE_DEATHWINDOW: u8 = 2; // 决死窗口（中弹后可 bomb 救）
pub const LIFE_RESPAWNING: u8 = 3; // 场底重生、无敌
pub const LIFE_GAMEOVER: u8 = 4; // 命尽、不再重生
pub const DEATHBOMB_WINDOW: u16 = 8; // 决死窗口帧
pub const RESPAWN_INVULN: u16 = 120; // 重生无敌帧（2 秒 @60Hz）
```

- [ ] **Step 2: 写 settle 趟二的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加（复用 Task 3/4 的 `bullet_at`/`spawn_enemy`）:
```rust
    #[test]
    fn settle_shot_kills_enemy_marks_dying_and_event() {
        use crate::enemy::ENEMY_DYING;
        use crate::events::EVT_ENEMY_DIED;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei], y: w.body.enemies.y[ei], vx: Fx::ZERO, vy: Fx::ZERO,
            damage: 1, radius: Fx::from_int(4), sprite: 0, owner: 0, flags: 0,
        });
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
        w.body.settle();
        assert!(w.body.enemies.hp[ei] <= 0);
        assert_ne!(w.body.enemies.flags[ei] & ENEMY_DYING, 0);
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_ENEMY_DIED);
    }

    #[test]
    fn settle_overkill_two_shots_one_death_event() {
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1，两发都打中
        let ei = w.body.enemies.get(e).unwrap();
        for _ in 0..2 {
            w.body.create_player_shot(crate::shots::ShotInit {
                x: w.body.enemies.x[ei], y: w.body.enemies.y[ei], vx: Fx::ZERO, vy: Fx::ZERO,
                damage: 1, radius: Fx::from_int(4), sprite: 0, owner: 0, flags: 0,
            });
        }
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.events_len, 1); // 只死一次
    }

    #[test]
    fn settle_bullet_hit_triggers_deathwindow() {
        use crate::player::{DEATHBOMB_WINDOW, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        { w.body.phase_guard = PH_COLLIDE; }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(w.body.players[0].state_timer, DEATHBOMB_WINDOW);
    }

    #[test]
    fn deathwindow_expires_to_respawn_after_window() {
        use crate::input::InputFrame;
        use crate::player::{LIFE_ALIVE, LIFE_RESPAWNING};
        let mut w = crate::step::World::new(1);
        // 手动置决死窗口（模拟已中弹）
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        let lives0 = w.body.players[0].lives;
        // 跑够窗口帧数 → Dead → Respawning
        for _ in 0..crate::player::DEATHBOMB_WINDOW {
            crate::step::step(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_RESPAWNING);
        assert_eq!(w.body.players[0].lives, lives0 - 1);
        assert!(w.body.players[0].invuln > 0);
        // 再跑够无敌帧 → Alive
        for _ in 0..crate::player::RESPAWN_INVULN {
            crate::step::step(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE);
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core settle_shot_kills_enemy deathwindow_expires`
Expected: FAIL（settle 是 stub、update_players 无状态机）。

- [ ] **Step 4: 实现 settle 趟二**

Modify `crates/stg-core/src/world.rs`，替换 `settle` stub:
```rust
    pub(crate) fn settle(&mut self) {
        self.phase_enter(PH_SETTLE);
        // 趟一 · 清除/防护：bomb 清弹（行 6）—— 本切片无 bomb，空。
        // 趟二 · 伤害
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            match h.row {
                crate::events::ROW_SHOT_ENEMY => {
                    let s = h.active as usize;
                    let e = h.passive as usize;
                    if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 {
                        continue; // 悬垂 / overkill：已死跳过，弹照消耗
                    }
                    if self.enemies.invuln[e] != 0 || !self.shots.is_alive(s) {
                        continue; // 无敌帧跳伤害；悬垂弹跳过
                    }
                    self.enemies.hp[e] -= self.shots.damage[s] as i32;
                    self.enemies.hit_flash[e] = 4;
                    if self.enemies.hp[e] <= 0 {
                        self.enemies.flags[e] |= ENEMY_DYING;
                        let ev = Event {
                            kind: crate::events::EVT_ENEMY_DIED,
                            a_index: e as u16,
                            a_gen: self.enemies.generation[e],
                            x: self.enemies.x[e],
                            y: self.enemies.y[e],
                            data: [
                                self.enemies.score[e] as i32,
                                self.enemies.death_script[e] as i32,
                            ],
                        };
                        self.push_event(ev);
                    }
                }
                crate::events::ROW_BULLET_PLAYER_HIT | crate::events::ROW_BODY_PLAYER_HIT => {
                    let p = h.passive as usize;
                    if self.players[p].life_state != crate::player::LIFE_ALIVE {
                        continue; // 已在窗口/无敌/重生 → 一次中弹只触发一次
                    }
                    self.players[p].life_state = crate::player::LIFE_DEATHWINDOW;
                    self.players[p].state_timer = crate::player::DEATHBOMB_WINDOW;
                }
                _ => {}
            }
        }
        // 趟三 · 计分/拾取 —— Task 6 填 graze。
    }
```

- [ ] **Step 5: 重构 update_players 为生死状态机 + commit_death**

Modify `crates/stg-core/src/world.rs`，替换 `update_players`:
```rust
    pub(crate) fn update_players(&mut self) {
        self.phase_enter(PH_PLAYERS);
        use crate::player::{
            LIFE_ABSENT, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_GAMEOVER, LIFE_RESPAWNING,
        };
        for i in 0..crate::MAX_PLAYERS {
            // 生死状态机计时（A4 相位 3 职责）
            match self.players[i].life_state {
                LIFE_ABSENT | LIFE_GAMEOVER => continue,
                LIFE_DEATHWINDOW => {
                    // bomb 救人 stub：本切片无 bomb 输入 → 窗口必耗尽。
                    if self.players[i].state_timer > 0 {
                        self.players[i].state_timer -= 1;
                    }
                    if self.players[i].state_timer == 0 {
                        self.commit_death(i);
                    }
                }
                LIFE_RESPAWNING => {
                    if self.players[i].invuln > 0 {
                        self.players[i].invuln -= 1;
                    }
                    if self.players[i].invuln == 0 {
                        self.players[i].life_state = LIFE_ALIVE;
                    }
                }
                LIFE_ALIVE => {
                    if self.players[i].invuln > 0 {
                        self.players[i].invuln -= 1; // bomb 无敌（本切片恒 0）
                    }
                }
                _ => {}
            }
            // commit_death 可能刚把 lives 耗尽置 GAMEOVER → 再判一次跳过移动/发弹
            if self.players[i].life_state == LIFE_GAMEOVER {
                continue;
            }
            self.move_player(i);
            #[allow(clippy::single_match)]
            match self.players[i].character_id {
                0 => self.char0_update_shot(i),
                _ => {}
            }
        }
    }

    /// 决死窗口耗尽的死亡连带结算（世界侧固定，D6）：lives−1、PlayerDied、重生或 game over。
    fn commit_death(&mut self, i: usize) {
        use crate::player::{LIFE_GAMEOVER, LIFE_RESPAWNING, RESPAWN_INVULN};
        self.players[i].lives = self.players[i].lives.saturating_sub(1);
        let ev = Event {
            kind: crate::events::EVT_PLAYER_DIED,
            a_index: i as u16,
            a_gen: 0,
            x: self.players[i].x,
            y: self.players[i].y,
            data: [self.players[i].lives as i32, 0],
        };
        self.push_event(ev);
        // 掉 power / power 道具回撒 → 道具池切片（此处暂不动 power）。
        if self.players[i].lives == 0 {
            self.players[i].life_state = LIFE_GAMEOVER;
        } else {
            self.players[i].life_state = LIFE_RESPAWNING;
            self.players[i].x = Fx::ZERO; // 场底中心（与 spawn 一致）
            self.players[i].y = Fx::from_int(384);
            self.players[i].invuln = RESPAWN_INVULN;
            self.players[i].state_timer = 0;
        }
    }
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p stg-core settle_ deathwindow_expires`
Expected: PASS。

- [ ] **Step 7: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS（既有 `player_moves_right_with_input` 等仍绿：Alive 路径不变）。

- [ ] **Step 8: Commit**

```bash
git add crates/stg-core/src/player.rs crates/stg-core/src/world.rs
git commit -m "feat(world): settle 趟二（敌人扣血/dying/EnemyDied + 中弹→DeathWindow）+ 生死状态机计时

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: settle 趟三（graze 逐弹一次）

**Files:**
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `settle` 趟三 graze 逻辑（`grazed_by` check-and-set）。
- Consumes: `events::ROW_BULLET_PLAYER_GRAZE`；弹池 `grazed_by: u8` 字段。

- [ ] **Step 1: 写 graze 的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加:
```rust
    #[test]
    fn settle_graze_counts_once_per_bullet() {
        use crate::input::InputFrame;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        // 一颗停在 graze 圈内、hit 圈外的弹（距 10px）
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::from_int(10), y: Fx::from_int(384),
            vx: Fx::ZERO, vy: Fx::ZERO, speed: Fx::ZERO, angle: crate::math::Angle::ZERO,
            ang_vel: 0, accel: Fx::ZERO, ax: Fx::ZERO, ay: Fx::ZERO,
            sprite: 0, radius: Fx::from_int(2), delay: 0, life: 0xFFFF,
            flags: 0, grazed_by: 0, transform_head: 0xFFFF, xform_wait: 0, xform_next: 0,
        });
        // 弹静止、贴着自机 → 连跑 3 帧，graze 只 +1（grazed_by 逐弹一次）
        for _ in 0..3 {
            step(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].graze, 1);
    }
```

> **注**：此测试用真实 `step`（不手置 guard），弹静止不越界、自机无输入不动，三帧持续处于 graze 圈。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core settle_graze_counts_once`
Expected: FAIL（graze 恒 0，趟三未实现）。

- [ ] **Step 3: 实现 settle 趟三**

Modify `crates/stg-core/src/world.rs` 的 `settle`，把末尾注释 `// 趟三 ...` 替换为:
```rust
        // 趟三 · 计分/拾取
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row == crate::events::ROW_BULLET_PLAYER_GRAZE {
                let b = h.active as usize;
                let p = h.passive as usize;
                let bit = 1u8 << p; // MAX_PLAYERS=2 → bit 0/1
                if self.bullets.grazed_by[b] & bit == 0 {
                    self.bullets.grazed_by[b] |= bit;
                    self.players[p].graze = self.players[p].graze.wrapping_add(1);
                }
            }
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core settle_graze_counts_once`
Expected: PASS。

- [ ] **Step 5: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-core/src/world.rs
git commit -m "feat(world): settle 趟三 graze（grazed_by 位掩码逐弹一次）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## ⏸ 检查点 ③（Task 5–6）：结算三趟 + 生死全通，停下等 CI 绿 + 用户确认

碰撞后果全接通。推分支盯 CI。注意：金向量此时仍未铺敌人（导演未改）→ 校验和可能仍与合并前一致或仅因新 diag 字段位移而变；以三平台一致为准。

---

## Task 7: 金向量扩成碰撞病态诊断场景 + 设计回写

**Files:**
- Modify: `crates/stg-harness/src/main.rs`
- Modify: `crates/stg-core/src/world.rs`（相位注释回写）
- Modify: `stg-world-design.md`（A4/D8/D9 落地状态）

**Interfaces:**
- Consumes: `WorldBody::create_enemy`（pub）、`enemy::EnemyInit`（pub）、`enemies.iter_alive().count()`（pub 方法 + pub 字段）、既有 `create_bullet`、`step_with_director`。

- [ ] **Step 1: 扩展金向量（导演铺敌人 + 敌弹；自机脚本上射杀敌 + 走位吃弹）**

Modify `crates/stg-harness/src/main.rs` 的 `cmd_golden`。替换 use 段 + 导演闭包:

(a) use 段（在既有 use 基础上**仅新增** `use stg_core::enemy::EnemyInit;` 一行；其余保持原样、原顺序，勿改动否则 `cargo fmt --check` 报序错）:
```rust
    use stg_core::bullets::BulletInit;
    use stg_core::enemy::EnemyInit;
    use stg_core::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame};
    use stg_core::math::{Angle, Fx, polar_to_vec};
    use stg_core::step::{World, step_with_director};
```

(b) 在 `cmd_golden` 里、`for frame` 循环之前加敌人 Init 助手:
```rust
    // 全字段 EnemyInit 助手（顶部三敌人固定位；move_to/挂钩惰性）。
    let enemy_at = |x: i32, y: i32| EnemyInit {
        x: Fx::from_int(x), y: Fx::from_int(y), vx: Fx::ZERO, vy: Fx::ZERO,
        mv_from_x: Fx::ZERO, mv_from_y: Fx::ZERO, mv_to_x: Fx::ZERO, mv_to_y: Fx::ZERO,
        mv_t: 0, mv_dur: 0, mv_easing: 0, mv_active: 0,
        hp: 5, hp_max: 5, radius: Fx::from_int(12), hurtbox: Fx::from_int(16),
        invuln: 0, hit_flash: 0, flags: 0, sprite: 0, anm_state: 0,
        main_task: 0, death_script: 0, drop_table: 0, score: 100,
    };
```

(c) 替换 `step_with_director(&mut world, &input, |b| { ... })` 闭包体:
```rust
        step_with_director(&mut world, &input, |b| {
            // ① 每 60 帧把敌人补到 3 个（顶部固定三点；被自机弹打死→cleanup 回收→补位 churn）
            if frame % 60 == 0 {
                let alive = b.enemies.iter_alive().count();
                let slots = [(-80, 80), (0, 80), (80, 80)];
                for &(ex, ey) in slots.iter().skip(alive) {
                    b.create_enemy(enemy_at(ex, ey));
                }
            }
            // ② 每 8 帧从顶部中心铺一圈 10 发敌弹（rng 抖动；部分下行抵达自机 → 中弹/擦弹）
            if frame % 8 == 0 {
                let base = (frame.wrapping_mul(797) & 0xFFFF) as u16;
                let n: u16 = 10;
                let astep = (65536u32 / n as u32) as u16;
                for k in 0..n {
                    let spread = b.rng.rand_range(384) as u16;
                    let a = Angle(
                        base.wrapping_add(k.wrapping_mul(astep)).wrapping_add(spread),
                    );
                    let (vx, vy) = polar_to_vec(Fx::from_int(2), a);
                    b.create_bullet(BulletInit {
                        x: Fx::ZERO, y: Fx::from_int(100), vx, vy,
                        speed: Fx::from_int(2), angle: a, ang_vel: 0, accel: Fx::ZERO,
                        ax: Fx::ZERO, ay: Fx::ZERO, sprite: 0, radius: Fx::from_int(3),
                        delay: 0, life: 300, flags: 0, grazed_by: 0,
                        transform_head: 0xFFFF, xform_wait: 0, xform_next: 0,
                    });
                }
            }
        });
```

(d) 替换自机脚本输入段（把原"走方框"改成"多数时间上冲吃弹 + 全程射击"，以确定性触发中弹/擦弹/杀敌）:
```rust
        let mut input = InputFrame::empty(frame);
        let mut btn = BTN_SHOT; // 全程射击 → 自机弹上飞杀顶部敌人（行 4）
        // 每 90 帧一个周期：前 50 帧上冲（吃弹/擦弹/逼近敌体），后 40 帧下退（喘息）
        let phase = frame % 90;
        if phase < 50 {
            btn |= BTN_UP;
        } else {
            btn |= BTN_DOWN;
        }
        // 左右缓移增加位形多样性
        btn |= if (frame / 45) % 2 == 0 { BTN_RIGHT } else { BTN_LEFT };
        if (frame / 120) % 2 == 0 {
            btn |= BTN_SLOW;
        }
        input.actions[0].buttons = btn;
```

> 删掉原来的 `let dir = (frame / 30) % 4;` 及其 `match dir` 块（被上面取代）。

- [ ] **Step 2: 本地确定性自检（跑两遍逐帧校验和须完全一致）**

Run:
```bash
cargo run -p stg-harness -- golden --out /tmp/g1.txt && cargo run -p stg-harness -- golden --out /tmp/g2.txt && diff /tmp/g1.txt /tmp/g2.txt && echo IDENTICAL
```
Expected: 打印 `IDENTICAL`（同机两遍逐字节相同 = 单机确定性）。

- [ ] **Step 3: 目测场景真的触发了碰撞（校验和逐帧变化、非全零 churn）**

Run: `cargo run -p stg-harness -- golden | head -20`
Expected: 前 20 帧校验和逐帧变化（敌人生成、弹流、自机移动+射击都在改状态）。

- [ ] **Step 4: 相位注释回写 world.rs**

Modify `crates/stg-core/src/world.rs` 顶部模块注释第 2 行，更新落地状态:
```rust
//! M0-7：collide（D8 行1/2/3/4，圆-圆平方距离，只收集）+ settle（D9 三趟）+ 生死状态机 + EnemyPool 已落。
```

- [ ] **Step 5: 设计文档回写**

Modify `stg-world-design.md`，在 M0 系列落地记录段（搜索 `M0-6 续`）之后追加一句 M0-7 记录:
```markdown
**M0-7**：`EnemyPool`（D5 全字段，move_to/主控 AI 惰性延后）+ `hits`/`events` 缓冲（A5，checksum-skip）+ collide（D8 行1/2/3/4，半径和 i64 平方距离不开根、暴力 O(N×M)、只收集不改状态）+ settle（D9 三趟：趟一空/趟二敌人扣血 dying+EnemyDied、自机中弹→DeathWindow/趟三 graze grazed_by 逐弹一次）+ 生死状态机（settle 触发、update_players 计时：DeathWindow→Dead→Respawning→Alive/GAMEOVER）。金向量扩为碰撞诊断场景（导演铺 3 敌 + 敌弹，自机上冲吃弹 + 全程射击杀敌）。bomb/道具/掉 power/move_to 插值器待后续。
```

- [ ] **Step 6: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo run -p stg-harness -- verify-tables`
Expected: 全 PASS，verify-tables 一致。

- [ ] **Step 7: Commit**

```bash
git add crates/stg-harness/src/main.rs crates/stg-core/src/world.rs stg-world-design.md
git commit -m "feat(harness): 金向量碰撞诊断场景（导演铺敌+敌弹，自机上冲吃弹全程射击）+ 设计回写

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: 收口 + CI + 合并

**Files:** 无代码改动（流程任务）。

- [ ] **Step 1: 最终全绿自检**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo run -p stg-harness -- verify-tables`
Expected: 全 PASS。

- [ ] **Step 2: 推分支 + 盯 CI 三平台绿**

推 `feat/m0-7-collision-settlement`，`gh run watch <run> --exit-status`。确认 `lint`/`vector`×3/`determinism-gate` 全绿——**金向量含碰撞/死亡/敌人 churn 的逐帧校验和三平台逐字节一致**。

- [ ] **Step 3: 用户确认后 ff 合并 main + 清理分支**

```bash
git checkout main && git merge --ff-only feat/m0-7-collision-settlement && cargo test --workspace && git push origin main
git branch -d feat/m0-7-collision-settlement && git push origin --delete feat/m0-7-collision-settlement
```

- [ ] **Step 4: 报告 M0-7 完成**

报告：世界从"能动"变"有后果"——自机弹杀敌、敌弹/敌体杀自机、擦弹计数、生死状态机全通，跨平台逐帧对拍绿。

---

## Self-Review

**1. Spec coverage（对照 grill 十项决策）：**
- Q1 范围（敌人+行1/2/3/4）→ Task 1（EnemyPool）+ Task 3（行1/2）+ Task 4（行3/4）✓
- Q2 暴力 O(N×M) → Task 3/4 嵌套直扫 ✓
- Q3 半径和 i64 平方距离、一次 len_sq 复用多半径 → Task 3 collide_bullets_player（复用 d2）✓
- Q4 只收集不改状态 + hits 6B/cap8192/P4-a → Task 2（Hit/push_hit/hits_overflow）+ collide 纯读 ✓
- Q5 敌死 settle 置 dying + EnemyDied、cleanup free → Task 5（settle）+ Task 1（cleanup dying）✓
- Q6 生死拆分（settle 触发、update_players 计时）+ 门禁 Alive&&invuln==0 → Task 5 + Task 3/4 门禁 ✓
- Q7 grazed_by 位掩码逐弹一次 → Task 6 ✓
- Q8 敌人全字段、move_to/AI 延后、导演傀儡 → Task 1（惰性 mv_*）+ Task 7（导演铺敌）✓
- Q9 建最小 frame_events（EnemyDied/PlayerDied）→ Task 2（Event）+ Task 5（产事件）✓
- Q10 扩现有为一条碰撞诊断场景 → Task 7 ✓

**2. Placeholder scan：** 无 TBD/TODO；每步含完整代码 + 精确命令。掉 power 明确注释延后（非占位，是范围决策）。

**3. Type consistency：**
- `push_hit(row:u8, active:u16, passive:u16)` — Task 2 定义、Task 3/4 调用签名一致 ✓
- `Event{kind,a_index,a_gen,x,y,data}` — Task 2 定义、Task 5 构造字段一致 ✓
- `ROW_*`/`EVT_*` 常量 — Task 2 定义、Task 3/4/5/6 引用一致 ✓
- `EnemyInit` 字段序 — Task 1/4/7 三处 exhaustive 列举一致（26 字段）✓
- `LIFE_DEATHWINDOW/RESPAWNING/GAMEOVER`、`DEATHBOMB_WINDOW`、`RESPAWN_INVULN` — Task 5 player.rs 定义、world.rs 引用一致 ✓
- collide 测试手置 `phase_guard = PH_COLLIDE`（debug）绕 PhaseGuard —— Task 3/4 一致 ✓

**已知取舍（非缺陷）：**
- hits 收集序非严格行主序（行1/2 就同一弹×自机对交错），但 settle 三趟按行过滤 + 各行内索引升序 → 确定性成立（见 Q4 论证）。
- `copy_into` 不拷 hits/events（帧内私有 + checksum-skip + 次帧 begin 清空重生）——省内存且不破快照校验。
- EnemyPool 实测 ~70B/敌（含双 u16 句柄 main_task + i32 hp_max），略超 D10 的 64B 估算；容量预算金向量后调参。
