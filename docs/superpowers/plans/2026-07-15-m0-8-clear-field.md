# M0-8 消弹区（Field）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 世界层多一个通用圆形作用区原语 `FieldPool` —— 消弹（行6）+ 可选伤敌（行7），bomb 只是它的首个租户。

**Architecture:** `define_pool!` 第 4 个实例（cap 16，静止哑数据）。collide 相位 6 加行 6/7（按 `flags` 能力位 gate，纯读只收集）；settle 相位 7 **趟一**兑现（消弹打 `BULLET_CLEARED` 标记 + 按 field 索引升序发聚合 `FieldCleared` 事件），**趟二**加行 7 伤敌 + 行 1 跳过已清除弹（= bomb 救命）；cleanup 相位 9 回收已清除弹与寿命尽的 field。跟随由上层每帧重铺 `life=1` 实现，世界层零 follow 逻辑。

**Tech Stack:** Rust 2024 / stg-core（断层线以下，纯定点）/ `define_pool!` proc-macro。

**权威 spec:** `docs/superpowers/specs/2026-07-15-clear-field-design.md` —— 冲突时以 spec 为准。

## Global Constraints

- **I1 数值**：唯一标量 `Fx = Q16.16(i32)`；断层线以下禁 `f32/f64`。碰撞用 i64 平方距离比较，不开根、不归一化回 `Fx`。
- **I4 顺序**：一切遍历按池索引升序；碰撞收集嵌套固定（field 外层↑ × 被动内层↑）；settle 按 `hits` 收集序消费；禁无序容器。
- **I6 时间**：逻辑固定 60Hz，一切计时用整数帧。
- **I7 布局**：`World` 内无指针/堆容器。
- **P1 边界**：调用方只走安全写 API（`create_field`），不直接摸池内存。
- **P4 错误三铁律**：(a) 资源耗尽（池满/缓冲满）→ 确定性降级不 panic，计数入校验和；(b) 调用方违约（`radius` 超限）→ 确定性安全结果（钳制）+ 计数；(c) 引擎 bug → debug 帧内断言。
- **P6 全量校验**：住 World 的字段一律入校验和；唯一豁免是纯输出缓冲（`hits`/`events` + 各自 len）。
- **只收集不改状态硬规则**：collide 纯读、只 append `hits`；改状态只准在 settle（+ `update_players` 对自机自身的计时）。
- **复用槽写满**：`FieldInit` exhaustive（漏字段编译不过）。
- **趟一标记不回收**：settle 趟一只打 `BULLET_CLEARED` 位，**绝不 `free_index`**——趟二/趟三随后要按索引读这颗弹；回收统一在 cleanup（相位 9）。

## 关键常量（先在此定稿，贯穿全计划）

```rust
// field.rs
pub const FIELD_CLEAR_BULLETS: u8 = 1 << 0;
pub const FIELD_DAMAGE:        u8 = 1 << 1;
pub const FIELD_RADIUS_FULLSCREEN: Fx = Fx::from_int(400);
pub const FIELD_MAX_RADIUS:        Fx = Fx::from_int(1024);
// bullets.rs
pub const BULLET_CLEARED: u8 = 1 << 0;
// events.rs
pub(crate) const ROW_FIELD_BULLET: u8 = 6;
pub(crate) const ROW_FIELD_ENEMY:  u8 = 7;
pub const EVT_FIELD_CLEARED: u8 = 3;   // 既有：EVT_ENEMY_DIED=1, EVT_PLAYER_DIED=2
// world.rs
pub const POOL_FIELD: usize = 3;       // 既有：BULLET=0, SHOT=1, ENEMY=2
```

## File Structure

- **Create** `crates/stg-core/src/field.rs` — `FieldPool` + 能力位 + 半径常量。
- **Modify** `crates/stg-core/src/lib.rs` — `pub mod field;`
- **Modify** `crates/stg-core/src/bullets.rs` — `BULLET_CLEARED` 位常量。
- **Modify** `crates/stg-core/src/events.rs` — `ROW_FIELD_BULLET/ROW_FIELD_ENEMY`、`EVT_FIELD_CLEARED`。
- **Modify** `crates/stg-core/src/world.rs` — `fields` 字段、`POOL_FIELD`、`create_field`(钳制)、integrate/cleanup 的 field 段、cleanup 的已清除弹回收、collide 行6/7、settle 趟一 + 趟二改动、两个共用助手。
- **Modify** `crates/stg-core/src/step.rs` — `copy_into` 加 `fields`。
- **Modify** `crates/stg-harness/src/main.rs` — 金向量导演周期性铺全屏 field。
- **Modify** `stg-world-design.md` — D6/D8/D9/A5/D10 五处回写。

---

## Task 1: FieldPool 地基 + create_field(钳制) + integrate/cleanup

**Files:**
- Create: `crates/stg-core/src/field.rs`
- Modify: `crates/stg-core/src/lib.rs`, `crates/stg-core/src/bullets.rs`, `crates/stg-core/src/world.rs`, `crates/stg-core/src/step.rs`

**Interfaces:**
- Produces: `field::{FieldPool, FieldHandle, FieldInit, FIELD_CLEAR_BULLETS, FIELD_DAMAGE, FIELD_RADIUS_FULLSCREEN, FIELD_MAX_RADIUS}`；`bullets::BULLET_CLEARED`；`WorldBody.fields`；`world::POOL_FIELD`；`WorldBody::create_field(&mut self, FieldInit) -> FieldHandle`。
- Consumes: `define_pool!`、`math::Fx`、既有 integrate/cleanup 掩码字遍历模式。

- [ ] **Step 1: 写 field.rs**

Create `crates/stg-core/src/field.rs`:
```rust
//! 作用区池（消弹区 / 伤敌区）——`define_pool!` 第 4 个实例。
//!
//! **通用哑原语**：bomb 只是首个租户；符卡切换清弹、阶段清场、ECL 死亡脚本清弹都是平等租户，
//! 一律走 `WorldBody::create_field` 写 API。
//!
//! **静止**：世界层零 follow 逻辑。跟随 = 上层每帧在目标位重铺 `life=1`（`life=1` 恰好活一帧
//! 且当帧生效：相位5 减到 0、相位6 alive 位仍在照常判定、相位9 才回收）；静止爆炸 = 铺一次 `life=N`。

use crate::define_pool;
use crate::math::Fx;

/// 能力位：启用碰撞矩阵行 6（Field × EnemyBullet → 消弹）。
pub const FIELD_CLEAR_BULLETS: u8 = 1 << 0;
/// 能力位：启用碰撞矩阵行 7（Field × EnemyBody → 按 `dmg_per_frame` 扣血）。
pub const FIELD_DAMAGE: u8 = 1 << 1;

/// 覆盖全场含越界边距的半径。
///
/// 场界 x∈[-192,192]、y∈[0,448]，边距 64 → 弹最远可在 (±256, −64..512)；field 置场心 (0,224)
/// 到最远角 = √(256² + 288²) = √148480 ≈ 385.3 px < 400。给脚本算好的常量，免得各自猜。
pub const FIELD_RADIUS_FULLSCREEN: Fx = Fx::from_int(400);

/// `create_field` 的半径钳制上限（P4-b）。
///
/// `Fx` 上限 32767.99998。若调用方传 32767 表达"无限大"，`field.radius + bullet.radius` 的
/// **Fx 加法会溢出** → debug panic / release 回绕成负数 → 平方后全场无条件判撞（debug/release 分歧）。
/// 钳到 1024 后 `1024 + 16 ≪ 32767`，Fx 加法永不溢出，行 6/7 得以与行 1-4 写法完全一致。
pub const FIELD_MAX_RADIUS: Fx = Fx::from_int(1024);

define_pool! {
    Field, cap = 16,
    fields {
        x: Fx, y: Fx, radius: Fx,
        dmg_per_frame: u16, life: u16,
        owner: u8, flags: u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    fn field_at(x: i32, y: i32, radius: i32, flags: u8) -> FieldInit {
        FieldInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            radius: Fx::from_int(radius),
            dmg_per_frame: 0,
            life: 1,
            owner: 0,
            flags,
        }
    }

    #[test]
    fn field_pool_alloc_get_free() {
        let mut p = FieldPool::new();
        let h = p.alloc(field_at(0, 100, 20, FIELD_CLEAR_BULLETS)).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.radius[i], Fx::from_int(20));
        assert_eq!(p.flags[i], FIELD_CLEAR_BULLETS);
        assert!(p.free(h));
        assert_eq!(p.get(h), None);
    }

    #[test]
    fn field_pool_new_deterministic() {
        assert_eq!(FieldPool::new().checksum(), FieldPool::new().checksum());
    }

    #[test]
    fn fullscreen_radius_covers_farthest_corner() {
        // 场心 (0,224) 到最远角 (256, 512)：dx=256, dy=288 → dist ≈ 385.3 < 400
        let d2 = crate::math::geom::len_sq(Fx::from_int(256), Fx::from_int(288));
        let r = FIELD_RADIUS_FULLSCREEN.raw() as i64;
        assert!(d2 < r * r, "全屏半径必须覆盖最远角");
    }
}
```

- [ ] **Step 2: 注册模块 + 弹的已清除位**

Modify `crates/stg-core/src/lib.rs`，在 `pub mod events;` 之后加：
```rust
pub mod events;
pub mod field;
pub mod input;
```

Modify `crates/stg-core/src/bullets.rs`，在 `use` 之后、`define_pool!` 之前加：
```rust
/// `flags` 位：本帧被作用区清除（settle 趟一置、趟二读【中弹跳过=bomb 救命】、cleanup 回收）。
///
/// **生命只在同一帧的相位 7→9 之间**，从不跨帧——故 collide 无需检查此位（次帧看不到已清除的弹）。
pub const BULLET_CLEARED: u8 = 1 << 0;
```

- [ ] **Step 3: 跑池单测**

Run: `cargo test -p stg-core field::tests`
Expected: PASS（3 个测试）。

- [ ] **Step 4: 写 WorldBody 接入的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加：
```rust
    fn spawn_field(
        w: &mut crate::step::World,
        x: i32,
        y: i32,
        radius: i32,
        flags: u8,
        life: u16,
    ) -> crate::field::FieldHandle {
        w.body.create_field(crate::field::FieldInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            radius: Fx::from_int(radius),
            dmg_per_frame: 0,
            life,
            owner: 0,
            flags,
        })
    }

    #[test]
    fn create_field_clamps_radius() {
        use crate::field::FIELD_MAX_RADIUS;
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 30000, crate::field::FIELD_CLEAR_BULLETS, 1);
        let i = w.body.fields.get(h).unwrap();
        assert_eq!(w.body.fields.radius[i], FIELD_MAX_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn field_life_one_lives_exactly_one_frame() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 20, crate::field::FIELD_CLEAR_BULLETS, 1);
        assert!(w.body.fields.get(h).is_some());
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.fields.get(h), None); // 活一帧后 cleanup 回收
    }

    #[test]
    fn field_life_n_survives_n_frames() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 20, crate::field::FIELD_CLEAR_BULLETS, 3);
        for _ in 0..2 {
            crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
            assert!(w.body.fields.get(h).is_some()); // 前 2 帧仍在
        }
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.fields.get(h), None); // 第 3 帧尽
    }
```

- [ ] **Step 5: 跑测试确认失败**

Run: `cargo test -p stg-core create_field_clamps_radius`
Expected: FAIL（`fields` 字段/`create_field` 未定义）。

- [ ] **Step 6: WorldBody 接入 fields + POOL_FIELD + create_field(钳制)**

Modify `crates/stg-core/src/world.rs`:

(a) 顶部 use 加：
```rust
use crate::field::{FieldHandle, FieldInit, FieldPool, FIELD_MAX_RADIUS};
```

(b) 池 id 常量区加：
```rust
pub const POOL_FIELD: usize = 3;
```

(c) `WorldBody` 加字段（放在 `enemies` 之后、`hits` 之前）：
```rust
    pub fields: FieldPool,
```

(d) 写 API 区加（紧跟 `create_enemy`）：
```rust
    /// 创建一个作用区（P4-a：池满 → NULL + 计数；P4-b：radius 超限 → 钳制 + 计数）。
    pub fn create_field(&mut self, mut init: FieldInit) -> FieldHandle {
        // P4-b：调用方违约 → 确定性安全结果。钳后 Fx 半径和永不溢出（见 field::FIELD_MAX_RADIUS）。
        if init.radius.raw() > FIELD_MAX_RADIUS.raw() {
            init.radius = FIELD_MAX_RADIUS;
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        match self.fields.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_FIELD] = self.diag.pool_full[POOL_FIELD].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                FieldHandle::NULL
            }
        }
    }
```

- [ ] **Step 7: integrate 加 field 段（寿命倒数）**

Modify `crates/stg-core/src/world.rs` 的 `integrate`，在敌人段之后追加：
```rust
        // 作用区：寿命倒数（照抄弹的模式；life=1 → 本帧减到 0，相位6 仍参与判定，相位9 回收）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] > 0 {
                    self.fields.life[i] -= 1;
                }
            }
        }
```

- [ ] **Step 8: cleanup 加 field 段 + 弹的已清除回收**

Modify `crates/stg-core/src/world.rs` 的 `cleanup`：

(a) 弹段的 `dead` 判据加已清除位：
```rust
                let dead = (self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0)
                    || self.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0
                    || Self::out_of_bounds(self.bullets.x[i], self.bullets.y[i]);
```

(b) 敌人段之后追加 field 段：
```rust
        // 作用区：寿命尽回收（不做越界——field 是有意放置的静止圆，非飞行物）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] == 0 {
                    self.fields.free_index(i);
                }
            }
        }
```

- [ ] **Step 9: copy_into 拷贝 fields**

Modify `crates/stg-core/src/step.rs` 的 `copy_into`，在 `s.enemies.copy_into(&mut d.enemies);` 之后加：
```rust
        s.fields.copy_into(&mut d.fields);
```

- [ ] **Step 10: 跑测试确认通过**

Run: `cargo test -p stg-core create_field_clamps_radius field_life_one_lives_exactly_one_frame field_life_n_survives_n_frames`
Expected: PASS。

- [ ] **Step 11: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS，零警告。

- [ ] **Step 12: Commit**

```bash
git add crates/stg-core/src/field.rs crates/stg-core/src/lib.rs crates/stg-core/src/bullets.rs crates/stg-core/src/world.rs crates/stg-core/src/step.rs
git commit -m "feat(world): FieldPool（通用圆形作用区）+ create_field(P4-b 钳制) + 寿命/回收

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: collide 行 6/7（能力位 gate + 判别式测试）

**Files:**
- Modify: `crates/stg-core/src/events.rs`, `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `events::{ROW_FIELD_BULLET, ROW_FIELD_ENEMY}`；私有 `collide_field_bullet`、`collide_field_enemy`；`collide` 调这两者。
- Consumes: Task 1 的 `fields`；`push_hit`；`math::geom::len_sq`；`field::{FIELD_CLEAR_BULLETS, FIELD_DAMAGE}`。

**算法**：与行 1-4 同款——`len_sq(dx,dy) <= (r_active + r_passive)²`，i64 Q32.32 同域比、不开根。
行 6 半径 = `field.radius + bullet.radius`；行 7 = `field.radius + enemy.hurtbox`（受击圈，与行 4 同）。
嵌套固定 **field 外层↑ × 被动内层↑**。**能力位在收集前 gate**（省 O(N×M)）。行 7 **不查敌 invuln**（事件照收、结算时判，与行 4 同规）。

- [ ] **Step 1: events.rs 加行号常量**

Modify `crates/stg-core/src/events.rs`，在 `ROW_SHOT_ENEMY` 之后加：
```rust
pub(crate) const ROW_FIELD_BULLET: u8 = 6; // 作用区 × 敌弹 → 消弹
pub(crate) const ROW_FIELD_ENEMY: u8 = 7; // 作用区 × 敌人 hurtbox → 扣血
```

- [ ] **Step 2: 写行 6/7 的判别式失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加（复用 Task 1 的 `spawn_field`、既有 `bullet_at`/`spawn_enemy`）：
```rust
    #[test]
    fn collide_field_bullet_discriminates_radius_sum() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_CLEAR_BULLETS;
        // field 半径 20 + 弹半径 2 = 和 22 → 21px 撞、23px 不撞（判别式，非圆心重合）
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 21, 100); // 索引 0：内
        bullet_at(&mut w, 23, 100); // 索引 1：外
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let hits: Vec<_> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k])
            .filter(|h| h.row == ROW_FIELD_BULLET)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // field 索引
        assert_eq!(hits[0].passive, 0); // 只有 21px 那颗
    }

    #[test]
    fn collide_field_skips_bullets_without_clear_bit() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_DAMAGE;
        // 只开 DAMAGE 位的 field 压着弹 → 不消弹
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_BULLET)
                .count(),
            0
        );
    }

    #[test]
    fn collide_field_enemy_uses_hurtbox() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_DAMAGE;
        // field 半径 20 + 敌 hurtbox 16 = 和 36；若误用敌 radius 12 → 和 32
        // 敌人放 34px：正确(≤36)撞；误用 radius(≤32) 则不撞 → 判别式
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        spawn_enemy(&mut w, 34, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            1
        );
    }

    #[test]
    fn collide_field_skips_enemy_without_damage_bit() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_CLEAR_BULLETS;
        // 只开 CLEAR 位的 field 压着敌人 → 不伤敌
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        spawn_enemy(&mut w, 0, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            0
        );
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core collide_field_`
Expected: FAIL（行 6/7 未实现，hits 里没有这两行）。

- [ ] **Step 4: 实现行 6/7**

Modify `crates/stg-core/src/world.rs` 的 `collide`，扩为：
```rust
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
        self.collide_body_player(); // 行 3：敌体 × 自机
        self.collide_shot_enemy(); // 行 4：自机弹 × 敌人
        self.collide_field_bullet(); // 行 6：作用区 × 敌弹（消弹）
        self.collide_field_enemy(); // 行 7：作用区 × 敌人（伤敌）
    }
```

在 `collide_shot_enemy` 之后加两个私有方法：
```rust
    /// 行 6：作用区（field.radius）× 敌弹（bullet.radius）→ 消弹。
    /// 能力位在收集前 gate（未开 CLEAR 的 field 整行跳过，省 O(N×M)）。
    fn collide_field_bullet(&mut self) {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::math::geom::len_sq;
        let nwf = self.fields.alive.len();
        let nwb = self.bullets.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_CLEAR_BULLETS == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for bw in 0..nwb {
                    let mut bbits = self.bullets.alive[bw];
                    while bbits != 0 {
                        let b = bw * 64 + bbits.trailing_zeros() as usize;
                        bbits &= bbits - 1;
                        if self.bullets.delay[b] > 0 {
                            continue; // delay 弹不参与
                        }
                        let dx = self.bullets.x[b] - fx;
                        let dy = self.bullets.y[b] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.bullets.radius[b]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_BULLET, f as u16, b as u16);
                        }
                    }
                }
            }
        }
    }

    /// 行 7：作用区（field.radius）× 敌人 hurtbox（受击圈，与行 4 同）→ 扣血。
    /// **不查敌 invuln**（事件照收、结算时判，与行 4 同规）。
    fn collide_field_enemy(&mut self) {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_DAMAGE;
        use crate::math::geom::len_sq;
        let nwf = self.fields.alive.len();
        let nwe = self.enemies.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_DAMAGE == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for ew in 0..nwe {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - fx;
                        let dy = self.enemies.y[e] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_ENEMY, f as u16, e as u16);
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p stg-core collide_field_`
Expected: PASS（4 个）。

- [ ] **Step 6: 变异检验（证明测试能因正确的理由失败）**

临时把 `collide_field_enemy` 的 `self.enemies.hurtbox[e]` 改成 `self.enemies.radius[e]`，跑：
Run: `cargo test -p stg-core collide_field_enemy_uses_hurtbox`
Expected: **FAIL**（34px 落在 32 之外 → 收不到 hit）。确认后**改回** `hurtbox`，重跑确认 PASS。
**不要提交这次改动。** 在报告里记录这次变异检验的结果。

- [ ] **Step 7: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。

- [ ] **Step 8: Commit**

```bash
git add crates/stg-core/src/events.rs crates/stg-core/src/world.rs
git commit -m "feat(world): collide 行6 作用区×敌弹 + 行7 作用区×敌人（能力位 gate，判别式测试）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## ⏸ 检查点 ①（Task 1–2）：地基 + 收集完成，停下等 CI 绿 + 用户确认

collide 现产行 6/7 hits，但 settle 尚未消费 → 世界状态不变（hits checksum-skip）。金向量校验和绝对值会因 `fields` 空池入校验和而变——闸门是三平台一致，不与合并前比。

---

## Task 3: settle 趟一（消弹 + 聚合事件）+ 趟二（行7 伤敌 + bomb 救命）

**Files:**
- Modify: `crates/stg-core/src/events.rs`, `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `events::EVT_FIELD_CLEARED`；settle 趟一实体；趟二加行 7；私有助手 `trigger_player_hit`、`damage_enemy`。
- Consumes: Task 2 的行 6/7 hits；`bullets::BULLET_CLEARED`；`push_event`；`FieldPool::CAP`。

**设计要点**：
- **趟一只标记不回收**（趟二/趟三随后要按索引读这颗弹；回收在 cleanup）。
- **幂等**：多 field 压同一弹 → 第一个打标记+计数，后续见位跳过。
- **聚合事件的确定序**：栈上 `counts: [i32; FieldPool::CAP]` 累计，扫完**按 field 索引升序**发事件——**不**依赖"行6 hits 按 field 连续"（那会把 settle 的聚合与 collide 的循环结构耦死）。
- **bomb 救命**：趟二行 1 跳过已清除的弹 —— 趟一先于趟二，故同帧消弹能救下本会命中的弹。
- **行 3 不查已清除位**（敌人没有"被清除"概念，它们经 hp≤0 → dying）。
- **趟三不动**（graze 照算：擦在相位 6 已发生，清弹是相位 7 的事）。

- [ ] **Step 1: events.rs 加事件种类**

Modify `crates/stg-core/src/events.rs`，在 `EVT_PLAYER_DIED` 之后加：
```rust
/// 作用区本帧消弹的聚合事实（每 field 每帧至多一条，`data[0]` = 本帧本 field 消了几颗）。
///
/// **聚合而非逐弹**：弹池 cap 8192 而 events cap 512，逐弹发在全屏消弹下必爆（溢出 16×）。
pub const EVT_FIELD_CLEARED: u8 = 3;
```

- [ ] **Step 2: 写趟一/趟二的失败测试**

Modify `crates/stg-core/src/world.rs` 的 `mod tests`，追加：
```rust
    #[test]
    fn settle_field_clears_bullet_and_emits_aggregate() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::EVT_FIELD_CLEARED;
        use crate::field::FIELD_CLEAR_BULLETS;
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 0, 100);
        bullet_at(&mut w, 10, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_ne!(w.body.bullets.flags[0] & BULLET_CLEARED, 0);
        assert_ne!(w.body.bullets.flags[1] & BULLET_CLEARED, 0);
        // 聚合：一条事件、count=2
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.events[0].data[0], 2);
    }

    #[test]
    fn settle_two_fields_clear_same_bullet_counts_once() {
        use crate::field::FIELD_CLEAR_BULLETS;
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1); // field 0
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1); // field 1，同位置
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        // 幂等：弹只被计一次 → 只有 field 0 计到 1，field 1 计 0（无事件）
        let total: i32 = (0..w.body.events_len as usize)
            .map(|k| w.body.events[k].data[0])
            .sum();
        assert_eq!(total, 1);
    }

    #[test]
    fn settle_field_clear_saves_player_from_death() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::player::{LIFE_ALIVE, LIFE_DEATHWINDOW};
        // 招牌语义：弹压在自机身上 + 同帧 field 消它 → 自机不进决死窗口（趟一先于趟二）
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        spawn_field(&mut w, 0, 384, 20, FIELD_CLEAR_BULLETS, 1);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE); // 被救
        assert_ne!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(w.body.players[0].graze, 1); // 但 graze 照算（擦在先、清在后）
    }

    #[test]
    fn settle_field_damages_enemy() {
        use crate::field::FIELD_DAMAGE;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_field(crate::field::FieldInit {
            x: Fx::ZERO,
            y: Fx::from_int(100),
            radius: Fx::from_int(20),
            dmg_per_frame: 2,
            life: 1,
            owner: 0,
            flags: FIELD_DAMAGE,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.enemies.hp[ei], 3); // 5 - 2
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core settle_field_`
Expected: FAIL（趟一为空、行 7 未结算）。

- [ ] **Step 4: 抽出两个共用助手（消除重复）**

Modify `crates/stg-core/src/world.rs`，在 `settle` 之前加两个私有方法：
```rust
    /// 中弹触发（行 1/3 共用）：只 Alive 者转入决死窗口 —— 一次中弹只触发一次。
    fn trigger_player_hit(&mut self, p: usize) {
        if self.players[p].life_state != crate::player::LIFE_ALIVE {
            return; // 已在窗口/无敌/重生
        }
        self.players[p].life_state = crate::player::LIFE_DEATHWINDOW;
        self.players[p].state_timer = crate::player::DEATHBOMB_WINDOW;
    }

    /// 敌人扣血 + 致死则标记 dying 并产出 `EnemyDied`（行 4/行 7 共用；只发一次）。
    /// **只标记不回收**——槽要活到相位 8 供死亡脚本/表现层读；相位 9 cleanup 收尸。
    fn damage_enemy(&mut self, e: usize, dmg: u16) {
        self.enemies.hp[e] -= dmg as i32;
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
```

- [ ] **Step 5: 实现趟一 + 改造趟二**

Modify `crates/stg-core/src/world.rs` 的 `settle`。把整个方法体替换为：
```rust
    pub(crate) fn settle(&mut self) {
        self.phase_enter(PH_SETTLE);
        // ── 趟一 · 清除/防护：行 6 消弹 ──────────────────────────────────
        // **只标记不回收**（趟二/趟三随后按索引读这颗弹；回收在相位 9 cleanup）。
        // **先于趟二**——故同帧作用区能救下本会命中自机的弹（bomb 救命）。
        let mut cleared_counts = [0i32; FieldPool::CAP];
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row != crate::events::ROW_FIELD_BULLET {
                continue;
            }
            let b = h.passive as usize;
            if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                continue; // 幂等：已被别的 field 消掉，不重复计
            }
            self.bullets.flags[b] |= crate::bullets::BULLET_CLEARED;
            cleared_counts[h.active as usize] += 1;
        }
        // 聚合事件：按 field 索引升序产出（不依赖 hits 的分组连续性 → 与 collide 循环结构解耦）
        for f in 0..FieldPool::CAP {
            if cleared_counts[f] > 0 {
                let ev = Event {
                    kind: crate::events::EVT_FIELD_CLEARED,
                    a_index: f as u16,
                    a_gen: self.fields.generation[f],
                    x: self.fields.x[f],
                    y: self.fields.y[f],
                    data: [cleared_counts[f], 0],
                };
                self.push_event(ev);
            }
        }
        // ── 趟二 · 伤害 ──────────────────────────────────────────────────
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            match h.row {
                crate::events::ROW_SHOT_ENEMY => {
                    let s = h.active as usize;
                    let e = h.passive as usize;
                    if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 {
                        continue; // 悬垂 / overkill
                    }
                    if self.enemies.invuln[e] != 0 || !self.shots.is_alive(s) {
                        continue; // 无敌帧跳伤害；悬垂弹跳过
                    }
                    self.damage_enemy(e, self.shots.damage[s]);
                }
                crate::events::ROW_FIELD_ENEMY => {
                    let f = h.active as usize;
                    let e = h.passive as usize;
                    if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 {
                        continue; // 悬垂 / overkill
                    }
                    if self.enemies.invuln[e] != 0 || !self.fields.is_alive(f) {
                        continue; // 无敌帧跳伤害（收集时不查、结算时判）
                    }
                    self.damage_enemy(e, self.fields.dmg_per_frame[f]);
                }
                crate::events::ROW_BULLET_PLAYER_HIT => {
                    let b = h.active as usize;
                    if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                        continue; // 趟一清掉的弹不中弹 —— bomb 救命
                    }
                    self.trigger_player_hit(h.passive as usize);
                }
                crate::events::ROW_BODY_PLAYER_HIT => {
                    // 敌体无"被清除"概念（敌人经 hp≤0 → dying），故不查已清除位
                    self.trigger_player_hit(h.passive as usize);
                }
                _ => {}
            }
        }
        // ── 趟三 · 计分/拾取 ─────────────────────────────────────────────
        // graze **不**查已清除位：碰撞检测在相位 6 发生（那时弹活着、确实进了擦圈），
        // 清弹是相位 7 的事 —— 擦在先、清在后；且设计明写 graze 独立于中弹。
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row == crate::events::ROW_BULLET_PLAYER_GRAZE {
                let b = h.active as usize;
                let p = h.passive as usize;
                let bit = 1u8 << p; // MAX_PLAYERS=2 → bit 0/1
                const _: () = assert!(crate::MAX_PLAYERS <= 8, "grazed_by 位掩码只容 8 自机");
                if self.bullets.grazed_by[b] & bit == 0 {
                    self.bullets.grazed_by[b] |= bit;
                    self.players[p].graze = self.players[p].graze.wrapping_add(1);
                }
            }
        }
    }
```

> **注**：顶部 use 需加 `use crate::field::FieldPool;`（为 `FieldPool::CAP`）。若 Task 1 已因 `FieldPool` 字段类型引入，则无需重复。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p stg-core settle_field_`
Expected: PASS（4 个）。

- [ ] **Step 7: 变异检验（证明 bomb 救命测试能失败）**

临时把趟一的 `cleared_counts` 循环整段注释掉（即不打 `BULLET_CLEARED` 标记），跑：
Run: `cargo test -p stg-core settle_field_clear_saves_player_from_death`
Expected: **FAIL**（自机进了 DEATHWINDOW）。确认后**改回**，重跑确认 PASS。**不要提交这次改动。**
在报告里记录结果 —— 这证明"趟一先于趟二"这条招牌语义真的被测试守着。

- [ ] **Step 8: 全量回归 + fmt + clippy**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 PASS。既有的 `settle_shot_kills_enemy_marks_dying_and_event` / `settle_overkill_two_shots_one_death_event` / `settle_bullet_hit_triggers_deathwindow` / `settle_graze_counts_once_per_bullet` 必须仍绿（助手抽取不得改变行为）。

- [ ] **Step 9: Commit**

```bash
git add crates/stg-core/src/events.rs crates/stg-core/src/world.rs
git commit -m "feat(world): settle 趟一消弹（标记+聚合 FieldCleared）+ 趟二行7 伤敌 + 行1 跳过已清除弹（bomb 救命）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: 金向量铺 field + 五处设计回写 + 收口

**Files:**
- Modify: `crates/stg-harness/src/main.rs`, `crates/stg-core/src/world.rs`（模块注释）, `stg-world-design.md`

- [ ] **Step 1: 金向量导演周期性铺全屏消弹区**

Modify `crates/stg-harness/src/main.rs` 的 `cmd_golden`：

(a) use 段**仅新增**一行（其余勿动，否则 fmt 报序错）：
```rust
    use stg_core::field::{FieldInit, FIELD_CLEAR_BULLETS, FIELD_RADIUS_FULLSCREEN};
```

(b) 在导演闭包的敌弹段之后追加：
```rust
            // ③ 每 150 帧全屏消弹一次（压消弹标记/回收 churn + 聚合事件路径）
            if frame % 150 == 0 && frame > 0 {
                b.create_field(FieldInit {
                    x: Fx::ZERO,
                    y: Fx::from_int(224), // 场心
                    radius: FIELD_RADIUS_FULLSCREEN,
                    dmg_per_frame: 0,
                    life: 1, // 只活本帧
                    owner: 0,
                    flags: FIELD_CLEAR_BULLETS,
                });
            }
```

(c) 更新 `cmd_golden` 的 rustdoc，补一句消弹场景（保持现有简洁风格，勿写长文）：在描述敌弹环那句之后加
「每 150 帧全屏消弹一次（`FIELD_RADIUS_FULLSCREEN` 作用区，`life=1`）」。

- [ ] **Step 2: 两遍确定性自检**

Run:
```powershell
cargo run -p stg-harness -- golden --out $env:TEMP\c1.txt; cargo run -p stg-harness -- golden --out $env:TEMP\c2.txt; fc.exe $env:TEMP\c1.txt $env:TEMP\c2.txt
```
Expected: 无差异（`FC: no differences encountered`）。若有差异 → **STOP，报告 BLOCKED**（非确定性泄漏）。

- [ ] **Step 3: 确认消弹真的发生了**

临时在金向量循环里统计 `EVT_FIELD_CLEARED` 事件数与 `data[0]` 合计，跑一次记录数字，然后**移除探针**（不得提交）。报告里给出实测：全屏消弹触发了几次、共消了几颗弹。若为 0 → 场景没压到消弹路径，需调参。

- [ ] **Step 4: world.rs 模块注释回写**

Modify `crates/stg-core/src/world.rs` 顶部模块注释，在 M0-7 那行之后加：
```rust
//! M0-8：FieldPool（通用圆形作用区，静止哑原语）+ 行6 消弹/行7 伤敌 + settle 趟一（标记 + 聚合 FieldCleared）已落。
```

- [ ] **Step 5: stg-world-design.md 五处回写**

按 spec 的「权威设计回写」节逐条改：
1. **D6**（约 line 450）`BombFieldPool` → `FieldPool`：正名 + 能力位（`FIELD_CLEAR_BULLETS`/`FIELD_DAMAGE`）+ 半径常量（`FIELD_RADIUS_FULLSCREEN=400` 含推导、`FIELD_MAX_RADIUS=1024` 含溢出理由）+ 注明"bomb 只是首个租户，符卡清弹/阶段清场/死亡脚本清弹平等"+ "静止；跟随=上层每帧重铺 life=1"。
2. **D8 矩阵行 6/7**（约 line 482-483）：主动方 `BombField` → `Field`，注明按 `flags` 能力位启用。
3. **D8 要点**（约 line 486）：删去"与已清除标记的弹"半句 —— 保留 `delay > 0` 那半条。附理由：`BULLET_CLEARED` 由趟一（相位7）置、cleanup（相位9）同帧回收，次帧 collide 看不到 → 该规则不可达。
4. **D9 趟一**（约 line 494）：逐弹 `BulletCleared` → 每 field 每帧一条聚合 `FieldCleared{count}`，附容量推导（弹 8192 vs events 512 → 逐弹溢出 16×）。
5. **A5**（约 line 182）events 生产者清单补 `FieldCleared{count}`；**D10**（约 line 507 容量表）加 `FieldPool` 16 × ~18 B ≈ 320 B。

在 M0 落地记录段（搜 `M0-7`）追加一句 M0-8 记录。

- [ ] **Step 6: 全绿自检**

Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo run -p stg-harness -- verify-tables`
Expected: 全 PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/stg-harness/src/main.rs crates/stg-core/src/world.rs stg-world-design.md
git commit -m "feat(harness): 金向量全屏消弹场景 + D6/D8/D9/A5/D10 设计回写

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 8: 推分支 + CI 三平台绿 + 用户确认后合并**

推 `feat/m0-8-clear-field`（需开 PR 才触发 CI —— 工作流只在 `push: main` / `pull_request` 上跑）。
`gh run watch <run> --exit-status` 确认 `lint`/`vector`×3/`determinism-gate` 全绿。
**用户点头后**：`git checkout main && git merge --ff-only feat/m0-8-clear-field && cargo test --workspace && git push origin main`，再删分支。

---

## Self-Review

**1. Spec coverage：** 逐条对照 spec —— FieldPool 字段/常量(T1) · 静止+life 语义(T1) · create_field 钳制 P4-b(T1) · BULLET_CLEARED 位(T1) · integrate/cleanup(T1) · 行6/7 + 能力位 gate(T2) · 趟一标记+幂等+聚合事件确定序(T3) · 趟二行7+bomb 救命+行3 不查(T3) · 趟三不动(T3) · 判别式测试(T2/T3) · 金向量(T4) · 五处回写(T4) ✓

**2. Placeholder scan：** 无 TBD/TODO；每步含完整代码 + 精确命令。

**3. Type consistency：**
- `FieldInit` 7 字段（x/y/radius/dmg_per_frame/life/owner/flags）—— T1/T2/T3/T4 四处 exhaustive 列举一致 ✓
- `ROW_FIELD_BULLET=6`/`ROW_FIELD_ENEMY=7`（T2 定义、T3 消费）✓；`EVT_FIELD_CLEARED=3`（T3）不与既有 1/2 冲突 ✓；`POOL_FIELD=3`（T1）不与既有 0/1/2 冲突 ✓
- `hits` 索引约定：行6 `active`=field/`passive`=bullet；行7 `active`=field/`passive`=enemy —— T2 push 与 T3 读一致 ✓
- `damage_enemy(e, dmg)` / `trigger_player_hit(p)`（T3 定义并在同 Task 内消费）✓

**4. 已知取舍：**
- 趟一的 `cleared_counts: [i32; 16]` 是栈上 64 B 临时量 —— 刻意不入 World（它是帧内聚合中间量，非状态）。
- field 不做越界回收（有意放置的静止圆，非飞行物），只按 `life` 回收。
- `owner` 字段本刀无消费者（co-op 记分/道具未实现），spec 已记。
