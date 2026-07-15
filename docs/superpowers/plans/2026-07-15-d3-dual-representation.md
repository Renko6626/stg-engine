# D3 双表示运动模型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让弹会拐弯——实现 D3 双表示运动模型（`vx/vy` 积分真相 + `speed/angle` 作者视图 + `POLAR_FX`/`CART_FX` 模式位 + 封闭 setter 集），弹池六个休眠字段全部通电。

**Architecture:** 新建 `world/motion.rs` 承载 setter 双层（公开 handle 写 API + `pub(crate)` 索引核）与回填核；`world/integrate.rs` 弹循环在 delay 门后加两个互斥模式分支。无新 World 字段、无布局变化。上游 spec：`docs/superpowers/specs/2026-07-15-d3-dual-representation-design.md`；上游设计：`stg-world-design.md` D3。

**Tech Stack:** Rust 2024 / stg-core（断层线以下：纯定点、查表三角、CORDIC atan2、整数 isqrt）。

## Global Constraints

- **I1/I2**：唯一标量 `Fx = Q16.16(i32)`；角度 `Angle = BAM u16`，加减一律 `wrapping`（用 `Angle::add/add_delta`，禁裸 `+`）。
- **P1**：调用方不碰池内存——外部只走公开 setter；`polar_to_vec` 乘法安全性依赖"speed × 单位三角值"白名单模式。
- **P4-b**：悬垂句柄 → no-op + `diag.contract_viol` **一次** + `last_status = STATUS_STALE_HANDLE`（本计划新增该码 = 2）。
- **契约常量**：`BACKFILL_MIN_SPEED = Fx::from_raw(4096)`（1/16 px/帧）。回填规则：`speed` 恒回填；`angle` 仅 `speed >= BACKFILL_MIN_SPEED` 时回填。
- **互斥律**：开 `POLAR_FX` 清 `CART_FX`，反之亦然；只动这两位，不碰 `BULLET_CLEARED`/预留位。
- **TDD**：每个任务先写测试、看它红、再实现。合入前跑变异检验（Task 6）。
- 常用命令：`cargo test -p stg-core`、`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`。

---

### Task 1: 模式位常量 + `world/motion.rs`（refresh 核 + 模式互斥核）

**Files:**
- Modify: `crates/stg-core/src/bullets.rs`（`BULLET_CLEARED` 旁加两常量）
- Modify: `crates/stg-core/src/world.rs:30-34`（子模块声明区加 `mod motion;`）
- Modify: `crates/stg-core/src/world.rs:268`（`test_support::bullet_at` 改为返回句柄）
- Create: `crates/stg-core/src/world/motion.rs`（实现 + 测试同文件，仓库惯例）

**Interfaces:**
- Produces: `BULLET_POLAR_FX: u8 = 1<<1`、`BULLET_CART_FX: u8 = 1<<2`（bullets.rs，pub）；
  `WorldBody::refresh_vel_from_polar(&mut self, i: usize)`、`set_ang_vel_at(&mut self, i: usize, w: i16)`、
  `set_accel_at(&mut self, i: usize, a: Fx)`、`set_gravity_at(&mut self, i: usize, ax: Fx, ay: Fx)`、
  `stop_fx_at(&mut self, i: usize)`（全部 `pub(crate)`）；
  `test_support::bullet_at(w, x, y) -> crate::bullets::BulletHandle`。

- [ ] **Step 0: 开特性分支**

```bash
git checkout -b d3-motion
```

- [ ] **Step 1: 写失败测试**

先做两处零风险准备（不属实现，属测试基建）：

`bullets.rs` 的 `BULLET_CLEARED` 常量下方加：

```rust
/// `flags` 位：POLAR_FX 连续效果（integrate 每帧 `angle += ang_vel; speed += accel;` 后刷 v）。
pub const BULLET_POLAR_FX: u8 = 1 << 1;
/// `flags` 位：CART_FX 连续效果（integrate 每帧 `vx += ax; vy += ay;` 后按阈值回填极坐标）。
/// 与 `BULLET_POLAR_FX` 互斥：置一清另一（TH16 `c68 &= ~0x9` 语义）。位 3-4 预留反弹计数（D4）。
pub const BULLET_CART_FX: u8 = 1 << 2;
```

`world.rs` 的 `test_support::bullet_at` 签名改为返回句柄（现有调用点忽略返回值，零波及）：

```rust
pub(crate) fn bullet_at(w: &mut crate::step::World, x: i32, y: i32) -> crate::bullets::BulletHandle {
    w.body.create_bullet(crate::bullets::BulletInit {
        // ……原字面量全部不动……
    })
}
```

新建 `crates/stg-core/src/world/motion.rs`，先只放测试：

```rust
//! D3 运动写 API：setter 双层（公开 handle 层 + pub(crate) 索引核）+ 极坐标回填核。
//! 模型见 stg-world-design.md D3；本刀拍板见 specs/2026-07-15-d3-dual-representation-design.md。

#[cfg(test)]
mod tests {
    use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
    use crate::math::geom::polar_to_vec;
    use crate::math::{Angle, Fx};
    use crate::world::test_support::bullet_at;

    /// 互斥律：开 POLAR 清 CART、开 CART 清 POLAR、stop 清两位；不碰其他位。
    #[test]
    fn mode_bits_mutually_exclusive() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        w.body.set_ang_vel_at(0, 256);
        assert_ne!(w.body.bullets.flags[0] & BULLET_POLAR_FX, 0);
        w.body.set_gravity_at(0, Fx::ZERO, Fx::from_raw(6554));
        assert_eq!(w.body.bullets.flags[0] & BULLET_POLAR_FX, 0, "开 CART 应清 POLAR");
        assert_ne!(w.body.bullets.flags[0] & BULLET_CART_FX, 0);
        w.body.set_accel_at(0, Fx::from_raw(100));
        assert_ne!(w.body.bullets.flags[0] & BULLET_POLAR_FX, 0, "开 POLAR 应置位");
        assert_eq!(w.body.bullets.flags[0] & BULLET_CART_FX, 0, "开 POLAR 应清 CART");
        w.body.stop_fx_at(0);
        assert_eq!(w.body.bullets.flags[0] & (BULLET_POLAR_FX | BULLET_CART_FX), 0);
    }

    /// refresh 核 = polar_to_vec 查表参考值逐位相等（判别式：换 sin/cos 即红）。
    #[test]
    fn refresh_matches_table_reference() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        w.body.bullets.speed[0] = Fx::from_int(3);
        w.body.bullets.angle[0] = Angle::QUARTER;
        w.body.refresh_vel_from_polar(0);
        let (rvx, rvy) = polar_to_vec(Fx::from_int(3), Angle::QUARTER);
        assert_eq!(w.body.bullets.vx[0], rvx);
        assert_eq!(w.body.bullets.vy[0], rvy);
    }
}
```

`world.rs` 子模块声明区（字母序）：

```rust
mod cleanup;
mod collide;
mod integrate;
mod motion;
mod player;
mod settle;
```

- [ ] **Step 2: 跑测试确认按预期失败**

Run: `cargo test -p stg-core motion:: 2>&1 | grep -E "error|cannot find"`
Expected: `E0599: no method named set_ang_vel_at`（缺实现，编译失败 = Rust 的 RED）

- [ ] **Step 3: 最小实现**

`motion.rs` 测试模块上方加：

```rust
use super::WorldBody;
use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
use crate::math::Fx;
use crate::math::geom::polar_to_vec;

impl WorldBody {
    /// 极坐标 → 积分真相：`(vx,vy) = polar_to_vec(speed, angle)`。
    /// 一切改动 speed/angle 的路径改完必须调它（"忘了回填"火药桶的唯一出口）。
    #[inline]
    pub(crate) fn refresh_vel_from_polar(&mut self, i: usize) {
        let (vx, vy) = polar_to_vec(self.bullets.speed[i], self.bullets.angle[i]);
        self.bullets.vx[i] = vx;
        self.bullets.vy[i] = vy;
    }

    /// 开 POLAR_FX（清 CART_FX，互斥律）；只动两模式位。
    pub(crate) fn set_ang_vel_at(&mut self, i: usize, w: i16) {
        self.bullets.ang_vel[i] = w;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_POLAR_FX) & !BULLET_CART_FX;
    }

    /// 沿向加速，开 POLAR_FX（清 CART_FX）。
    pub(crate) fn set_accel_at(&mut self, i: usize, a: Fx) {
        self.bullets.accel[i] = a;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_POLAR_FX) & !BULLET_CART_FX;
    }

    /// 笛卡尔加速（重力/漂移），开 CART_FX（清 POLAR_FX）。
    pub(crate) fn set_gravity_at(&mut self, i: usize, ax: Fx, ay: Fx) {
        self.bullets.ax[i] = ax;
        self.bullets.ay[i] = ay;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_CART_FX) & !BULLET_POLAR_FX;
    }

    /// 清两模式位（字段留陈值，确定性无损——ZUN 语义只关开关）。
    pub(crate) fn stop_fx_at(&mut self, i: usize) {
        self.bullets.flags[i] &= !(BULLET_POLAR_FX | BULLET_CART_FX);
    }
}
```

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core motion::`
Expected: `2 passed`。再跑 `cargo test --workspace` 确认无波及（`bullet_at` 签名变更零调用点破坏）。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/bullets.rs crates/stg-core/src/world.rs crates/stg-core/src/world/motion.rs
git commit -m "feat(world): D3 模式位常量 + motion.rs 骨架（refresh 核 + 互斥模式核）"
```

---

### Task 2: 积分相位 POLAR_FX 分支

**Files:**
- Modify: `crates/stg-core/src/world/integrate.rs`（弹循环，delay 门之后、位移之前）

**Interfaces:**
- Consumes: Task 1 的 `refresh_vel_from_polar` / `set_ang_vel_at` / `set_accel_at`、`Angle::add_delta(i16)`。
- Produces: integrate 内 POLAR_FX 每帧语义（后续 Task 3 在同处加 `else if` CART 分支）。

- [ ] **Step 1: 写失败测试**

加进 `integrate.rs` 的 `mod tests`（已有 `bullet_at` 导入）：

```rust
use crate::bullets::BULLET_POLAR_FX;
use crate::math::Angle;
use crate::math::geom::polar_to_vec;

/// 螺旋判别式：ω=1024 BAM/帧 × 16 帧 = 1/4 圈，vx/vy 与查表参考逐位相等。
#[test]
fn polar_fx_spiral_matches_table_after_quarter_turn() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    let i = w.body.bullets.get(h).unwrap();
    w.body.bullets.speed[i] = Fx::from_int(2);
    w.body.bullets.angle[i] = Angle::ZERO;
    w.body.refresh_vel_from_polar(i);
    w.body.set_ang_vel_at(i, 1024);
    for f in 0..16u32 {
        crate::step::step(&mut w, &InputFrame::empty(f));
    }
    assert_eq!(w.body.bullets.angle[i], Angle::QUARTER);
    let (rvx, rvy) = polar_to_vec(Fx::from_int(2), Angle::QUARTER);
    assert_eq!(w.body.bullets.vx[i], rvx);
    assert_eq!(w.body.bullets.vy[i], rvy);
}

/// 沿向加速判别式：speed 线性累加，v 与查表参考一致。
#[test]
fn polar_fx_accel_grows_speed() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    let i = w.body.bullets.get(h).unwrap();
    w.body.bullets.speed[i] = Fx::from_int(1);
    w.body.bullets.angle[i] = Angle::ZERO;
    w.body.refresh_vel_from_polar(i);
    w.body.set_accel_at(i, Fx::from_raw(3277)); // ~0.05 px/帧²
    for f in 0..10u32 {
        crate::step::step(&mut w, &InputFrame::empty(f));
    }
    assert_eq!(w.body.bullets.speed[i].raw(), 65536 + 10 * 3277);
    let (rvx, _) = polar_to_vec(Fx::from_raw(65536 + 10 * 3277), Angle::ZERO);
    assert_eq!(w.body.bullets.vx[i], rvx);
}

/// delay 门冻结 POLAR：delay 期 angle/speed/位置全不动（D3：变换不走）。
#[test]
fn delay_gate_freezes_polar_fx() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    let i = w.body.bullets.get(h).unwrap();
    w.body.bullets.speed[i] = Fx::from_int(2);
    w.body.bullets.angle[i] = Angle::ZERO;
    w.body.refresh_vel_from_polar(i);
    w.body.set_ang_vel_at(i, 1024);
    w.body.bullets.delay[i] = 2;
    for f in 0..2u32 {
        crate::step::step(&mut w, &InputFrame::empty(f));
    }
    assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "delay 期角度不得推进");
    assert_eq!(w.body.bullets.x[i], Fx::ZERO, "delay 期不得移动");
    crate::step::step(&mut w, &InputFrame::empty(2));
    assert_eq!(w.body.bullets.angle[i], Angle(1024), "delay 尽后首帧推进一步");
}
```

注意：弹放在 (0,100)，自机在 (0,384)，16 帧 × 2px 走不拢，绝不误触碰撞。
`Angle(1024)` 需要 `Angle` 的 tuple 构造（pub 字段，已可用）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core integrate::`
Expected: 三个新测试 FAIL（angle 仍为 ZERO / vx 不等参考值）——POLAR 分支不存在，弹仍直飞。

- [ ] **Step 3: 最小实现**

`integrate.rs` 弹循环，delay 门与位移之间插入：

```rust
if self.bullets.delay[i] > 0 {
    self.bullets.delay[i] -= 1; // delay 期不动
    continue;
}
let fl = self.bullets.flags[i];
debug_assert_ne!(
    fl & (crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX),
    crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX,
    "模式位互斥被破坏（P4-c 帧内断言）"
);
if fl & crate::bullets::BULLET_POLAR_FX != 0 {
    self.bullets.angle[i] = self.bullets.angle[i].add_delta(self.bullets.ang_vel[i]);
    self.bullets.speed[i] = self.bullets.speed[i] + self.bullets.accel[i];
    self.refresh_vel_from_polar(i);
}
self.bullets.x[i] = self.bullets.x[i] + self.bullets.vx[i];
// ……以下原样……
```

模块头注释同步：把"敌人的 move_to 插值器待后续切片"一句上方的弹行为描述改为
"弹：delay 门 → 模式效果（POLAR/CART 互斥）→ `pos += vel` → life 倒数"。

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core`
Expected: 全绿（含既有 delay 门测试——语义不变，模式分支在门后）。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world/integrate.rs
git commit -m "feat(world): integrate 相位 POLAR_FX 分支——螺旋/加速判别式 + delay 门冻结"
```

---

### Task 3: 回填核 + CART_FX 分支（含阈值契约）

**Files:**
- Modify: `crates/stg-core/src/world/motion.rs`（`BACKFILL_MIN_SPEED` + `backfill_polar`）
- Modify: `crates/stg-core/src/world/integrate.rs`（POLAR 分支后加 `else if` CART）

**Interfaces:**
- Consumes: `crate::math::cordic::atan2(y: Fx, x: Fx) -> Angle`、`crate::math::isqrt::isqrt(u64) -> u32`、`crate::math::geom::len_sq(dx: Fx, dy: Fx) -> i64`。
- Produces: `pub const BACKFILL_MIN_SPEED: Fx`（motion.rs）、`WorldBody::backfill_polar(&mut self, i: usize)`（`pub(crate)`）。

- [ ] **Step 1: 写失败测试**

`motion.rs` tests 加：

```rust
/// 阈值判别式：阈值下 speed 照回填、angle 冻结；阈值上 angle == atan2 参考。
#[test]
fn backfill_freezes_angle_below_threshold() {
    let mut w = crate::step::World::new(1);
    bullet_at(&mut w, 0, 100);
    w.body.bullets.angle[0] = Angle::QUARTER; // 旧朝向
    w.body.bullets.vx[0] = Fx::from_raw(2048); // < 4096 = 阈值
    w.body.bullets.vy[0] = Fx::ZERO;
    w.body.backfill_polar(0);
    assert_eq!(w.body.bullets.speed[0].raw(), 2048, "speed 恒回填");
    assert_eq!(w.body.bullets.angle[0], Angle::QUARTER, "低速角度冻结");
    w.body.bullets.vx[0] = Fx::from_raw(8192); // ≥ 阈值
    w.body.backfill_polar(0);
    assert_eq!(w.body.bullets.angle[0], crate::math::cordic::atan2(Fx::ZERO, Fx::from_raw(8192)));
}
```

`integrate.rs` tests 加：

```rust
use crate::bullets::BULLET_CART_FX;

/// 重力弹判别式：上抛过顶点 vy 翻号、angle 每帧跟随 atan2 参考（几何可判对错）。
#[test]
fn cart_fx_gravity_parabola_flips_vy_and_tracks_angle() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 200);
    let i = w.body.bullets.get(h).unwrap();
    w.body.bullets.vx[i] = Fx::from_int(1);
    w.body.bullets.vy[i] = Fx::from_int(-3); // 上抛（y 向下为正）
    w.body.set_gravity_at(i, Fx::ZERO, Fx::from_raw(16384)); // ay = 0.25 px/帧²
    for f in 0..20u32 {
        crate::step::step(&mut w, &InputFrame::empty(f));
    }
    // vy = -3 + 20×0.25 = +2：过了顶点
    assert_eq!(w.body.bullets.vy[i].raw(), -3 * 65536 + 20 * 16384);
    assert!(w.body.bullets.vy[i].raw() > 0);
    // angle/speed 每帧回填：与参考逐位相等
    let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
    assert_eq!(w.body.bullets.angle[i], crate::math::cordic::atan2(vy, vx));
    let sp = crate::math::isqrt::isqrt(crate::math::geom::len_sq(vx, vy) as u64) as i32;
    assert_eq!(w.body.bullets.speed[i].raw(), sp);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core 2>&1 | grep -E "FAILED|no method|cannot find"`
Expected: `backfill_polar` 缺失编译失败（motion 测试）；实现 stub 后 CART 测试 assert 失败。

- [ ] **Step 3: 最小实现**

`motion.rs`（use 区补 `use crate::math::cordic::atan2; use crate::math::geom::len_sq; use crate::math::isqrt::isqrt;`）：

```rust
/// 低速回填阈值 = 1/16 px/帧。契约常量：`speed` 恒回填、`angle` 仅 `speed >= 此值` 时回填
/// （近停冻结朝向：防 CORDIC 低幅垃圾角污染作者视图与 sprite 朝向）。
/// 改值 = 确定性契约变更，须过评审（spec 2026-07-15）。
pub const BACKFILL_MIN_SPEED: Fx = Fx::from_raw(4096);

impl WorldBody {
    /// 笛卡尔 → 作者视图回填（阈值规则见 `BACKFILL_MIN_SPEED`）。
    /// sqrt(Q32.32) = Q16.16，故 isqrt(len_sq) 的 raw 直接是 Fx raw。
    pub(crate) fn backfill_polar(&mut self, i: usize) {
        let vx = self.bullets.vx[i];
        let vy = self.bullets.vy[i];
        let sp = Fx::from_raw(isqrt(len_sq(vx, vy) as u64) as i32);
        self.bullets.speed[i] = sp;
        if sp.raw() >= BACKFILL_MIN_SPEED.raw() {
            self.bullets.angle[i] = atan2(vy, vx);
        }
    }
}
```

`integrate.rs` POLAR 分支后：

```rust
} else if fl & crate::bullets::BULLET_CART_FX != 0 {
    self.bullets.vx[i] = self.bullets.vx[i] + self.bullets.ax[i];
    self.bullets.vy[i] = self.bullets.vy[i] + self.bullets.ay[i];
    self.backfill_polar(i);
}
```

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core`
Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world/motion.rs crates/stg-core/src/world/integrate.rs
git commit -m "feat(world): CART_FX 重力分支 + 极坐标回填核（BACKFILL_MIN_SPEED=1/16px/帧 契约）"
```

---

### Task 4: 公开 handle setter 层（8 个）+ `STATUS_STALE_HANDLE`

**Files:**
- Modify: `crates/stg-core/src/world.rs:41-42`（STATUS 码区加一行）
- Modify: `crates/stg-core/src/world/motion.rs`

**Interfaces:**
- Consumes: `BulletPool::get(h) -> Option<usize>`、Task 1/3 的索引核。
- Produces: `pub const STATUS_STALE_HANDLE: u16 = 2`（world.rs）；`WorldBody` 公开方法：
  `set_bullet_speed(h, Fx)` / `set_bullet_angle(h, Angle)` / `turn_bullet(h, Angle)` /
  `set_bullet_vel(h, Fx, Fx)` / `set_bullet_ang_vel(h, i16)` / `set_bullet_accel(h, Fx)` /
  `set_bullet_gravity(h, Fx, Fx)` / `stop_bullet_fx(h)`；内部 `fn bullet_index_checked(&mut self, h) -> Option<usize>`。

- [ ] **Step 1: 写失败测试**（`motion.rs` tests）

```rust
use crate::world::STATUS_STALE_HANDLE;

/// P4-b：悬垂句柄 → 8 个 setter 全部 no-op + contract_viol 各计一次 + last_status。
#[test]
fn stale_handle_setters_noop_and_count() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    w.body.bullets.free(h);
    let cv0 = w.body.diag.contract_viol;
    let ck0 = {
        // 释放后世界指纹基线（setter 若偷改任何字段，指纹会变）
        crate::checksum::Checksum::checksum(&w.body)
    };
    w.body.set_bullet_speed(h, Fx::from_int(5));
    w.body.set_bullet_angle(h, Angle::QUARTER);
    w.body.turn_bullet(h, Angle::QUARTER);
    w.body.set_bullet_vel(h, Fx::from_int(1), Fx::from_int(1));
    w.body.set_bullet_ang_vel(h, 100);
    w.body.set_bullet_accel(h, Fx::from_raw(50));
    w.body.set_bullet_gravity(h, Fx::ZERO, Fx::from_raw(50));
    w.body.stop_bullet_fx(h);
    assert_eq!(w.body.diag.contract_viol, cv0 + 8);
    assert_eq!(w.body.last_status, STATUS_STALE_HANDLE);
    // 除 diag/last_status 外世界零变化：把两者还原后指纹应等于基线
    w.body.diag.contract_viol = cv0;
    w.body.last_status = crate::world::STATUS_OK;
    assert_eq!(crate::checksum::Checksum::checksum(&w.body), ck0);
}

/// 快乐路径抽查：set_bullet_speed 触发 refresh；set_bullet_vel 触发回填。
#[test]
fn handle_setters_happy_path() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    w.body.set_bullet_angle(h, Angle::ZERO);
    w.body.set_bullet_speed(h, Fx::from_int(3));
    assert_eq!(w.body.bullets.vx[0], Fx::from_int(3)); // cos(0)=1 → vx=speed
    w.body.set_bullet_vel(h, Fx::ZERO, Fx::from_int(2));
    assert_eq!(w.body.bullets.speed[0], Fx::from_int(2));
    assert_eq!(w.body.bullets.angle[0], crate::math::cordic::atan2(Fx::from_int(2), Fx::ZERO));
    w.body.turn_bullet(h, Angle::QUARTER);
    let (rvx, rvy) = polar_to_vec(w.body.bullets.speed[0], w.body.bullets.angle[0]);
    assert_eq!((w.body.bullets.vx[0], w.body.bullets.vy[0]), (rvx, rvy));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core motion::`
Expected: 编译失败（`STATUS_STALE_HANDLE`/8 个方法缺失）。

- [ ] **Step 3: 最小实现**

`world.rs` STATUS 区：

```rust
pub const STATUS_STALE_HANDLE: u16 = 2;
```

`motion.rs`（use 区补 `use crate::bullets::BulletHandle; use crate::math::Angle; use crate::world::STATUS_STALE_HANDLE;`）：

```rust
impl WorldBody {
    /// 句柄查验（P4-b）：悬垂 → None + contract_viol 一次 + last_status。
    fn bullet_index_checked(&mut self, h: BulletHandle) -> Option<usize> {
        match self.bullets.get(h) {
            Some(i) => Some(i),
            None => {
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                self.last_status = STATUS_STALE_HANDLE;
                None
            }
        }
    }

    /// 改速率并回填 v（D3 极坐标 setter）。悬垂句柄 no-op。
    pub fn set_bullet_speed(&mut self, h: BulletHandle, speed: Fx) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.bullets.speed[i] = speed;
            self.refresh_vel_from_polar(i);
        }
    }

    /// 改朝向并回填 v。
    pub fn set_bullet_angle(&mut self, h: BulletHandle, angle: Angle) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.bullets.angle[i] = angle;
            self.refresh_vel_from_polar(i);
        }
    }

    /// 相对转向（回绕加）并回填 v。
    pub fn turn_bullet(&mut self, h: BulletHandle, delta: Angle) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.bullets.angle[i] = self.bullets.angle[i].add(delta);
            self.refresh_vel_from_polar(i);
        }
    }

    /// 直写积分真相并按阈值规则回填作者视图（D3 笛卡尔 setter）。
    pub fn set_bullet_vel(&mut self, h: BulletHandle, vx: Fx, vy: Fx) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.bullets.vx[i] = vx;
            self.bullets.vy[i] = vy;
            self.backfill_polar(i);
        }
    }

    /// 开角速度（POLAR_FX，清 CART_FX）。
    pub fn set_bullet_ang_vel(&mut self, h: BulletHandle, ang_vel: i16) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.set_ang_vel_at(i, ang_vel);
        }
    }

    /// 开沿向加速（POLAR_FX，清 CART_FX）。
    pub fn set_bullet_accel(&mut self, h: BulletHandle, accel: Fx) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.set_accel_at(i, accel);
        }
    }

    /// 开笛卡尔加速（CART_FX，清 POLAR_FX）。
    pub fn set_bullet_gravity(&mut self, h: BulletHandle, ax: Fx, ay: Fx) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.set_gravity_at(i, ax, ay);
        }
    }

    /// 关全部连续效果。
    pub fn stop_bullet_fx(&mut self, h: BulletHandle) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.stop_fx_at(i);
        }
    }
}
```

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core`
Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/world/motion.rs
git commit -m "feat(world): D3 公开 handle setter 层（8 个）+ STATUS_STALE_HANDLE（P4-b）"
```

---

### Task 5: `aim_bullet_at_player`（第 9 个 setter）

**Files:**
- Modify: `crates/stg-core/src/world/motion.rs`

**Interfaces:**
- Consumes: `crate::player::{LIFE_ABSENT, LIFE_GAMEOVER}`、`len_sq`、`atan2`、Task 4 的 `bullet_index_checked`。
- Produces: `WorldBody::aim_bullet_at_player(h, delta: Angle)`（pub）、
  `WorldBody::nearest_aimable_player(&self, x: Fx, y: Fx) -> Option<usize>`（`pub(crate)`，将来 D4 `AIM_PLAYER` op 复用）。

- [ ] **Step 1: 写失败测试**（`motion.rs` tests）

```rust
/// 瞄准判别式：angle == atan2(dy,dx)+delta 参考；可瞄状态 = 非 ABSENT 非 GAMEOVER。
#[test]
fn aim_targets_nearest_alive_player() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 100, 100); // 自机在 (0,384)
    w.body.aim_bullet_at_player(h, Angle::ZERO);
    let expect = crate::math::cordic::atan2(
        Fx::from_int(384 - 100),
        Fx::from_int(0 - 100),
    );
    assert_eq!(w.body.bullets.angle[0], expect);
    // delta 偏移生效
    w.body.aim_bullet_at_player(h, Angle::QUARTER);
    assert_eq!(w.body.bullets.angle[0], expect.add(Angle::QUARTER));
}

/// 无可瞄自机 → 纯 no-op（不计数，非违约——世界状态使然）。
#[test]
fn aim_with_no_alive_player_is_silent_noop() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 100, 100);
    w.body.bullets.angle[0] = Angle::QUARTER;
    w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
    let cv0 = w.body.diag.contract_viol;
    w.body.aim_bullet_at_player(h, Angle::ZERO);
    assert_eq!(w.body.bullets.angle[0], Angle::QUARTER, "角度不得变");
    assert_eq!(w.body.diag.contract_viol, cv0, "不得计违约");
}

/// 悬垂句柄照常计数（与其余 setter 同律）。
#[test]
fn aim_stale_handle_counts() {
    let mut w = crate::step::World::new(1);
    let h = bullet_at(&mut w, 0, 100);
    w.body.bullets.free(h);
    let cv0 = w.body.diag.contract_viol;
    w.body.aim_bullet_at_player(h, Angle::ZERO);
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core motion::`
Expected: 编译失败（方法缺失）。

- [ ] **Step 3: 最小实现**（use 区补 `use crate::math::geom::len_sq; use crate::player::{LIFE_ABSENT, LIFE_GAMEOVER};`）

```rust
impl WorldBody {
    /// 最近可瞄自机：平方距离最小、并列取低索引（I4：升序遍历 + 严格小于才替换）。
    /// 可瞄 = 非 ABSENT 且非 GAMEOVER（决死窗口/重生无敌期仍在场上，照瞄——ZUN 语义）。
    pub(crate) fn nearest_aimable_player(&self, x: Fx, y: Fx) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for p in 0..crate::MAX_PLAYERS {
            let st = self.players[p].life_state;
            if st == LIFE_ABSENT || st == LIFE_GAMEOVER {
                continue;
            }
            let d2 = len_sq(self.players[p].x - x, self.players[p].y - y);
            if best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((p, d2));
            }
        }
        best.map(|(p, _)| p)
    }

    /// 瞄最近可瞄自机 + delta 偏移，回填 v。无可瞄自机 → 纯 no-op（不计数）。
    pub fn aim_bullet_at_player(&mut self, h: BulletHandle, delta: Angle) {
        let Some(i) = self.bullet_index_checked(h) else {
            return;
        };
        let Some(p) = self.nearest_aimable_player(self.bullets.x[i], self.bullets.y[i]) else {
            return;
        };
        let dx = self.players[p].x - self.bullets.x[i];
        let dy = self.players[p].y - self.bullets.y[i];
        self.bullets.angle[i] = crate::math::cordic::atan2(dy, dx).add(delta);
        self.refresh_vel_from_polar(i);
    }
}
```

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core`
Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world/motion.rs
git commit -m "feat(world): aim_bullet_at_player——最近可瞄自机（I4 低索引并列）+ 无自机静默 no-op"
```

---

### Task 6: 变异检验（合入前义务，不产生提交物）

**Files:** 临时改动 `crates/stg-core/src/world/motion.rs`，验完全部还原。

- [ ] **Step 1: 变异 A——refresh 对调 vx/vy**

`refresh_vel_from_polar` 里 `self.bullets.vx[i] = vx;` 与 `vy` 赋值对调。
Run: `cargo test -p stg-core 2>&1 | grep -c FAILED`
Expected: ≥1（`refresh_matches_table_reference`、螺旋、happy path 必须红）。还原。

- [ ] **Step 2: 变异 B——互斥律阉割**

`set_ang_vel_at` 删掉 `& !BULLET_CART_FX`。
Run: `cargo test -p stg-core motion::mode_bits_mutually_exclusive`
Expected: FAIL。还原。

- [ ] **Step 3: 变异 C——阈值比较反转**

`backfill_polar` 的 `>=` 改 `<`。
Run: `cargo test -p stg-core motion::backfill_freezes_angle_below_threshold`
Expected: FAIL。还原。

- [ ] **Step 4: 确认工作区干净**

Run: `git diff --exit-code`
Expected: 退出码 0（全部还原）。变异结论记入最终 PR/合入说明一行即可。

---

### Task 7: 金向量就地加戏 + 双跑确定性

**Files:**
- Modify: `crates/stg-harness/src/main.rs`（golden 场景导演闭包）

**Interfaces:**
- Consumes: Task 4/5 的全部公开 setter；`stg_core::math::{Angle, Fx}`；`stg_core::bullets::BulletHandle`。

- [ ] **Step 1: 实现三类压力源**

golden 函数内、帧循环之前加状态：

```rust
let mut spiral_h = stg_core::bullets::BulletHandle::NULL;
```

导演闭包内（既有补敌/敌弹逻辑之后）加——若 main.rs 已有敌弹 `BulletInit` 字面量助手则复用，
否则内联同款字面量（radius 3、life 0xFFFF、其余零/哨兵，同 `test_support::bullet_at` 形状）：

```rust
// ② D3 压力源一：螺旋圈（POLAR_FX——每帧 sincos 查表路径）
if frame % 40 == 0 {
    for k in 0..8u16 {
        let h = b.create_bullet(/* (0, 60) 处哑弹字面量 */);
        b.set_bullet_speed(h, Fx::from_raw(98_304)); // 1.5 px/帧
        b.set_bullet_angle(h, Angle(k * 8192));      // 八方位
        b.set_bullet_ang_vel(h, if k % 2 == 0 { 512 } else { -512 });
        spiral_h = h;
    }
}
// ③ D3 压力源二：上抛重力弹（CART_FX——每帧 CORDIC+isqrt 回填，顶点扫过阈值两侧）
if frame % 90 == 0 {
    for k in 0..3i32 {
        let h = b.create_bullet(/* (-60 + 60k, 200) 处哑弹字面量 */);
        b.set_bullet_vel(h, Fx::ZERO, Fx::from_int(-3));
        b.set_bullet_gravity(h, Fx::ZERO, Fx::from_raw(16_384)); // 0.25 px/帧²
    }
}
// ④ D3 压力源三：setter 骚扰（句柄可能已死——P4-b 路径顺带入金向量，确定性无损）
if frame % 75 == 0 {
    match (frame / 75) % 3 {
        0 => b.turn_bullet(spiral_h, Angle::QUARTER),
        1 => b.aim_bullet_at_player(spiral_h, Angle::ZERO),
        _ => b.set_bullet_vel(spiral_h, Fx::from_int(2), Fx::from_int(1)),
    }
}
```

- [ ] **Step 2: 全量测试 + 金向量双跑对拍**

```bash
cargo test --workspace
cargo run -p stg-harness -- golden --out /tmp/g1.txt
cargo run -p stg-harness -- golden --out /tmp/g2.txt
diff /tmp/g1.txt /tmp/g2.txt && echo DETERMINISTIC
```

Expected: 测试全绿；`DETERMINISTIC`（600 帧逐帧校验和双跑全等）。

- [ ] **Step 3: Commit**

```bash
git add crates/stg-harness/src/main.rs
git commit -m "feat(harness): 金向量 D3 加戏——螺旋圈/重力弹/setter 骚扰（查表+CORDIC+isqrt 入对拍）"
```

---

### Task 8: 收尾——设计回写 + 文档 + 全绿门

**Files:**
- Modify: `stg-world-design.md`（D3 节笛卡尔 setter 一句旁补阈值定值）
- Modify: `CLAUDE.md`（仓库结构 `src/world/` 行加 motion）
- Modify: `PROGRESS.md`（史加一行 M0-10 + 重写「现在」段）

- [ ] **Step 1: 设计回写**

`stg-world-design.md` D3 的"笛卡尔 setter……带速度阈值（阈值规则钉死进契约，低速不回填防抖）"
一句后追加：

```
    （已定值：`BACKFILL_MIN_SPEED = Fx::from_raw(4096)` = 1/16 px/帧，speed 恒回填、
    angle 仅达阈回填——见 specs/2026-07-15-d3-dual-representation-design.md）
```

`CLAUDE.md` 仓库结构中 `src/world/` 行改为：

```
    src/world/       【模块结构镜像相位骨架】player(相1+3) / integrate(相5)
                     / collide(相6) / settle(相7) / cleanup(相9) / motion(D3 运动写 API)
```

`PROGRESS.md`：里程碑史顶部加一行
`| 2026-07-15 | M0-10 | D3 双表示运动模型：POLAR/CART 模式位 + 九 setter 写 API + 1/16 阈值回填契约 |`；
「现在」段重写（位置推进至 M0-10、下一步候选改为"D4 变换系统（需求讨论中）· bomb · 道具池 · move_to"）。

- [ ] **Step 2: 全绿门**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p stg-harness -- verify-tables
```

Expected: 全部通过、零告警。

- [ ] **Step 3: Commit + 收枝**

```bash
git add stg-world-design.md CLAUDE.md PROGRESS.md
git commit -m "docs: D3 收尾——阈值定值回写设计 + CLAUDE 结构 + PROGRESS 里程碑 M0-10"
```

然后走 `superpowers:finishing-a-development-branch`（合回 main、跑 CI）。

---

## Self-Review 记录

- **Spec 覆盖**：flags 位分配→T1；互斥律→T1；integrate 五步序→T2/T3；回填三条契约→T3；
  九个 setter→T4(8)+T5(1)；悬垂 no-op→T4/T5；无自机 aim no-op→T5；delay 门→T2；
  变异检验→T6；金向量三压力源→T7；收尾回写三件→T8。无缺口。
- **占位扫描**：T7 弹字面量以"同 `test_support::bullet_at` 形状"给出（16 字段字面量在 T1 引用的
  现有代码中完整可抄），其余步骤全部实码。
- **类型一致性**：索引核 `*_at(i: usize, ...)` / 公开层 `*_bullet*(h: BulletHandle, ...)` 贯穿一致；
  `atan2(y, x)` 参数序在 T3/T4/T5 测试与实现中一致（y 前 x 后）。
