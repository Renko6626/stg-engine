# 敌人运动动词族 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给敌人补齐运动动词——4 条动词（`move_vel`/`move_vel_xy`/`move_angle`/`move_speed`）+ 4 个 `$self_*` 引擎变量，并把 `integrate` 里那条从来没人写过的 `x += vx` 死分支通电。

**Architecture:** 敌池照弹补 `speed`/`angle` 双表示（`vx/vy` 仍是积分真相）；一组速度插值器 + `vel_space` 判别位承载极坐标/笛卡尔两种插值空间；`integrate` 相敌人段改成「速度插值恒跑 → 位置插值决定位置归谁」的分层结构。

**Tech Stack:** Rust 1.94.0（edition 2024），`stg-core` 断层线以下——无浮点/时钟/宿主 RNG/无序容器。

**权威设计：** [`docs/superpowers/specs/2026-07-31-enemy-motion-verbs-design.md`](../specs/2026-07-31-enemy-motion-verbs-design.md)。计划与 spec 冲突时**以 spec 为准**，并把冲突报上来。

## Global Constraints

- **I1 数值**：唯一标量是 `Fx` = Q16.16(i32)。`f32`/`f64` 不得出现在 `stg-core`。
- **I2 角度**：`Angle` = BAM u16，三角函数一律查表。
- **I4 顺序**：一切遍历按池索引升序；禁无序容器。
- **I6 时间**：一切计时用整数帧。
- **I7 布局**：`World` 内无指针/堆容器；快照 = 整块字节复制。
- **P4 错误三铁律**：(a) 资源耗尽 → 确定性降级不 panic；(b) **调用方违约 → 确定性安全结果**（坏参数 no-op + `contract_viol` 计数）；(c) 引擎自身 bug → debug 帧内断言 panic。
- **P6 全量校验**：住在 `World` 里的字段无例外参与校验和（`#[derive(Checksum)]` 自动）。
- **复用槽写满**：`EnemyInit` 是 exhaustive 的（`define_pool!` 编译期强制）。新字段必须进 `EnemyInit` **和每一个**构造 `EnemyInit` 的地方。
- **定点乘法**：`Fx::mul` 仅当至少一个操作数 ≤ ~1.0 时安全。插值里的 `delta * e` 用 **i64 中间量 + 算术右移 16**，照抄 `transform.rs::tick_one_step`。
- **`ENGINE_VER` 必须 bump**（池布局变 ⇒ 快照字节数变、`SaveBytes` 编码变）。当前值 `9`（`lib.rs:82`）。
- **金向量预期会变**——`rainbow.ecl` 场上有敌人，敌池布局一变逐帧校验和流全变。**本刀不能拿"逐字节不变"当验收判据**（前几刀可以，这刀不行）。
- ⚠️ **禁用 `git checkout <file>` 与 `git stash`** 清理临时/变异代码——本仓已被咬过两次，一律用编辑撤回。金向量取 base 用 `git worktree add`。
- 每个 commit 结尾附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- 收口全绿：`cargo fmt --all` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace`。

## 冻结的取值与命名（全计划统一，勿自行改名）

**syscall 号**（`syscall.rs` 当前最大是 `SYS_ENEMY_ALIVE = 82`）：

| 号 | 常量 | 动词/变量 |
|---|---|---|
| 83 | `SYS_MOVE_VEL` | `move_vel(dur, angle, speed, easing)` |
| 84 | `SYS_MOVE_VEL_XY` | `move_vel_xy(dur, vx, vy, easing)` |
| 85 | `SYS_MOVE_ANGLE` | `move_angle(dur, angle, easing)` |
| 86 | `SYS_MOVE_SPEED` | `move_speed(dur, speed, easing)` |
| 87 | `SYS_SELF_VX` | `$self_vx` |
| 88 | `SYS_SELF_VY` | `$self_vy` |
| 89 | `SYS_SELF_SPEED` | `$self_speed` |
| 90 | `SYS_SELF_ANGLE` | `$self_angle` |

**敌池新字段**（`enemy.rs` 的 `define_pool!`）：

```rust
speed: Fx, angle: Angle,
vel_from_0: i32, vel_from_1: i32,
vel_to_0:   i32, vel_to_1:   i32,
vel_t: u16, vel_dur: u16,
vel_easing: u8, vel_active: u8, vel_space: u8, vel_touched: u8,
```

**载体槽编码**（`vel_space` 决定四个 `i32` 怎么读）：

| `vel_space` | 常量 | slot 0 | slot 1 |
|---|---|---|---|
| 0 | `VEL_SPACE_POLAR` | `speed.raw()`（Fx） | `angle.raw() as i32`（u16 零扩展） |
| 1 | `VEL_SPACE_CART` | `vx.raw()`（Fx） | `vy.raw()`（Fx） |

**世界层 API 名**（`world/motion.rs`，`impl WorldBody`）：

```rust
pub(crate) fn refresh_enemy_vel_from_polar(&mut self, i: usize)
pub(crate) fn backfill_enemy_polar(&mut self, i: usize)
pub fn set_enemy_vel_polar(&mut self, h: EnemyHandle, angle: Angle, speed: Fx, dur: u16, easing: u8)
pub fn set_enemy_vel_cart (&mut self, h: EnemyHandle, vx: Fx, vy: Fx,      dur: u16, easing: u8)
pub fn set_enemy_angle    (&mut self, h: EnemyHandle, angle: Angle,        dur: u16, easing: u8)
pub fn set_enemy_speed    (&mut self, h: EnemyHandle, speed: Fx,           dur: u16, easing: u8)
```

---

## Task 1：敌池双表示字段 + 同步核

**Files:**
- Modify: `crates/stg-core/src/enemy.rs`（`define_pool!` 字段表 + 测试助手 `enemy_at`）
- Modify: `crates/stg-core/src/world/motion.rs`（两个同步核）
- Modify: `crates/stg-core/src/step.rs:2192`（世界尺寸哨兵 `EXPECTED`）
- Modify: 所有构造 `EnemyInit` 的地方（用 `rg 'EnemyInit'` 找全，**编译器会逐个报错**）
- Modify: `stg-world-design.md`（D5 敌池字段表 + D10 预算）

**Interfaces:**
- Produces：敌池字段 `speed`/`angle`/`vel_*` 十二个（取值见上表）；
  `WorldBody::refresh_enemy_vel_from_polar(i)`、`WorldBody::backfill_enemy_polar(i)`；
  常量 `VEL_SPACE_POLAR: u8 = 0`、`VEL_SPACE_CART: u8 = 1`（定义在 `enemy.rs`，`pub`）。

- [ ] **Step 1：写失败测试（双表示双向同步）**

放 `crates/stg-core/src/world/motion.rs` 的 `mod tests`（若无则新建，`use` 照文件内既有测试惯例）：

```rust
/// 正向：写 speed/angle → refresh 刷出 vx/vy。取 angle=QUARTER(90°，屏幕坐标朝下)、
/// speed=5.0：cos=0/sin=1 ⇒ (0, 5)。**判别性**：若两条派发臂写反（vx 拿 sin），
/// 这里会得到 (5, 0)，一眼可辨；取 45° 则两分量相等、写反不可辨。
#[test]
fn enemy_polar_to_cart_refresh_is_exact_at_quarter() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = w.body.enemies.get(h).unwrap();
    w.body.enemies.speed[i] = Fx::from_int(5);
    w.body.enemies.angle[i] = Angle::QUARTER;
    w.body.refresh_enemy_vel_from_polar(i);
    assert_eq!(w.body.enemies.vx[i], Fx::ZERO, "90° 的 cos 分量应为 0");
    assert_eq!(w.body.enemies.vy[i], Fx::from_int(5), "90° 的 sin 分量应为满速");
}

/// 反向：写 vx/vy → backfill 反算 speed/angle。取 (3, 4) 这个 x≠y 且勾股整齐的点：
/// speed 应精确为 5.0，angle 应是 atan2(4, 3)。**判别性**：(3,4) 而非 (3,3)——
/// 后者 speed=4.24 不整、且 atan2 参数写反不可辨。
#[test]
fn enemy_cart_to_polar_backfill_is_pythagorean() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = w.body.enemies.get(h).unwrap();
    w.body.enemies.vx[i] = Fx::from_int(3);
    w.body.enemies.vy[i] = Fx::from_int(4);
    w.body.backfill_enemy_polar(i);
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(5), "3-4-5 直角三角形");
    assert_eq!(
        w.body.enemies.angle[i],
        crate::math::cordic::atan2(Fx::from_int(4), Fx::from_int(3)),
        "atan2(vy, vx) 的参数序：y 在前"
    );
}

/// 低速冻结朝向（BACKFILL_MIN_SPEED = 1/16 px/帧）：速度归零时 speed 归 0 但
/// **angle 保持不变**——防 CORDIC 在零向量上吐垃圾角。逐条同弹的既有规则。
#[test]
fn enemy_backfill_freezes_angle_below_min_speed() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = w.body.enemies.get(h).unwrap();
    w.body.enemies.angle[i] = Angle::QUARTER;
    w.body.enemies.vx[i] = Fx::ZERO;
    w.body.enemies.vy[i] = Fx::ZERO;
    w.body.backfill_enemy_polar(i);
    assert_eq!(w.body.enemies.speed[i], Fx::ZERO);
    assert_eq!(w.body.enemies.angle[i], Angle::QUARTER, "零向量不得改写朝向");
}
```

> **注（已核对仓库现状）**：造世界的既有写法是 **`crate::step::World::new(1)`**（不是
> `new_for_test()`）；`spawn_enemy` / `bullet_at` / `step_t` 都住在
> `crate::world::test_support`（定义在 `world.rs:1060` 的 `pub(crate) mod test_support`），
> `integrate.rs` 的测试模块顶部已有
> `use crate::world::test_support::{bullet_at, spawn_enemy};`。现成模板见
> `integrate.rs::move_to_linear_waypoints_exact`（`World::new(1)` → `spawn_enemy` →
> `move_enemy_to` → 循环 `step_t`）。**照它抄，不要另造助手。**

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p stg-core enemy_polar_to_cart_refresh_is_exact_at_quarter
```
Expected：**编译失败** —— `no field 'speed' on type 'EnemyPool'` / `no method named 'refresh_enemy_vel_from_polar'`。

- [ ] **Step 3：加池字段**

`crates/stg-core/src/enemy.rs` 的 `define_pool!` 字段表里，**紧跟 `vx: Fx, vy: Fx,` 之后**插入：

```rust
        // ── 双表示（敌人运动动词族刀 2026-07-31）：vx/vy 是积分真相，speed/angle 是
        //    作者视图。改任一侧后必须同步另一侧（正向 refresh_enemy_vel_from_polar /
        //    反向 backfill_enemy_polar）——"忘了回填"是弹那边被称作火药桶的同一个坑。
        speed: Fx, angle: Angle,
        // ── 速度插值器（一组，极坐标/笛卡尔共用；vel_space 决定四个载体槽怎么读）。
        //    裸 i32 是**载体**不是标量：polar 空间要装 (Fx, Angle)，cart 空间要装 (Fx, Fx)，
        //    把 Angle 塞进 Fx 字段是 newtype 破坏。同 xform 槽 args:[i32;2] 按 op 重解释。
        vel_from_0: i32, vel_from_1: i32,
        vel_to_0: i32, vel_to_1: i32,
        vel_t: u16, vel_dur: u16,
        vel_easing: u8, vel_active: u8, vel_space: u8,
        // ── 黏滞位：脚本**是否表达过**速度意图（四条速度动词任一置 1，move_to 武装归 0）。
        //    位置插值到点只在它为 0 时清速。**不能拿 vel_active 当判据**——速度插值若先于
        //    位置插值到期（常见写法），到点时 vel_active 已是 0，速度会被误清。
        vel_touched: u8,
```

在文件顶部（`ENEMY_DYING` 常量附近）加：

```rust
/// `vel_space`：极坐标插值空间（载体槽 = `(speed.raw(), angle.raw() as i32)`）。
pub const VEL_SPACE_POLAR: u8 = 0;
/// `vel_space`：笛卡尔插值空间（载体槽 = `(vx.raw(), vy.raw())`）。
pub const VEL_SPACE_CART: u8 = 1;
```

- [ ] **Step 4：补全所有 `EnemyInit` 构造点**

```bash
cargo build -p stg-core 2>&1 | grep -c "missing field"
```

编译器会把每一处缺字段的地方点名（exhaustive `Init` 就是干这个的）。**逐个补上，全部初值为零**：

```rust
    speed: Fx::ZERO,
    angle: Angle::ZERO,
    vel_from_0: 0,
    vel_from_1: 0,
    vel_to_0: 0,
    vel_to_1: 0,
    vel_t: 0,
    vel_dur: 0,
    vel_easing: 0,
    vel_active: 0,
    vel_space: 0,
    vel_touched: 0,
```

⚠️ **`spawn_enemy` 的参数面不变**——不给它加初速参数（spec §5）。

- [ ] **Step 5：写两个同步核**

`crates/stg-core/src/world/motion.rs`，放在 `backfill_polar`（弹版）之后，`impl WorldBody` 内：

```rust
    /// 敌人：极坐标 → 积分真相。**改动 `speed`/`angle` 的每条路径改完必须调它。**
    /// 与弹的 `refresh_vel_from_polar` 是同一件事，只是池不同（敌无 POLAR_FX 连续效果，
    /// 故不涉及模式位）。
    #[inline]
    pub(crate) fn refresh_enemy_vel_from_polar(&mut self, i: usize) {
        let (vx, vy) = polar_to_vec(self.enemies.speed[i], self.enemies.angle[i]);
        self.enemies.vx[i] = vx;
        self.enemies.vy[i] = vy;
    }

    /// 敌人：笛卡尔 → 作者视图回填。阈值规则同弹（[`BACKFILL_MIN_SPEED`]）：`speed` 恒回填，
    /// `angle` 仅在 `speed >= 阈值` 时回填——近停冻结朝向，防 CORDIC 低幅垃圾角。
    /// sqrt(Q32.32) = Q16.16，故 `isqrt(len_sq)` 的 raw 直接是 `Fx` raw。
    pub(crate) fn backfill_enemy_polar(&mut self, i: usize) {
        let vx = self.enemies.vx[i];
        let vy = self.enemies.vy[i];
        let sp = Fx::from_raw(isqrt(len_sq(vx, vy) as u64) as i32);
        self.enemies.speed[i] = sp;
        if sp.raw() >= BACKFILL_MIN_SPEED.raw() {
            self.enemies.angle[i] = atan2(vy, vx);
        }
    }
```

`use` 已在文件头（`polar_to_vec`/`len_sq`/`isqrt`/`atan2`/`Angle`/`Fx`）——若 `Angle` 未导入则补。

- [ ] **Step 6：跑测试确认通过 + 修世界尺寸哨兵**

```bash
cargo test -p stg-core
```

`step.rs:2192` 的 `EXPECTED` 会红。**按失败信息里的实际数字更新**（两处 `#[cfg]` 分支都要，取值相同），并在该测试的注释里加一行说明本刀的增量：

```rust
// 敌人运动动词族刀 2026-07-31：敌池 +12 字段（speed/angle 双表示 + 速度插值器十件），
// 30 B/敌 × 256 ≈ 7.5 KB。
```

⚠️ **不要凭计算填数字**——跑出来的实际值才算（对齐填充不好手算）。

- [ ] **Step 7：更新 D10 / D5 文档**

`stg-world-design.md`：D5 敌池字段表补十二行；D10 容量预算表的敌池那行按 Step 6 的实测尺寸更新。

- [ ] **Step 8：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
feat(core): 敌池补 speed/angle 双表示 + 速度插值器字段（T1）

vx/vy 仍是积分真相，speed/angle 是作者视图，双向同步照抄弹的既有机器
（refresh_enemy_vel_from_polar / backfill_enemy_polar，含 BACKFILL_MIN_SPEED
近停冻结朝向）。速度插值器一组、vel_space 判别极坐标/笛卡尔两空间，载体用裸
i32（polar 要装 (Fx, Angle)，塞进 Fx 字段是 newtype 破坏；同 xform 槽按 op
重解释的先例）。vel_touched 是黏滞位，T3 的到点清速判据用它。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 2：速度插值器的武装（世界层写 API）

**Files:**
- Modify: `crates/stg-core/src/world/motion.rs`（四个 `set_enemy_*` + 一个私有武装助手）
- Modify: `crates/stg-core/src/world.rs:495-510`（`move_enemy_to` 两条路径各加 `vel_touched = 0`）

**Interfaces:**
- Consumes：T1 的池字段、`refresh_enemy_vel_from_polar`、`backfill_enemy_polar`、`VEL_SPACE_*`。
- Produces：四个 `pub fn set_enemy_*`（签名见「冻结的取值与命名」）。**本 Task 只负责武装，
  不负责逐帧推进**——`dur > 0` 时只写插值器字段，位置/速度当帧不动；推进归 T3。

- [ ] **Step 1：写失败测试**

放 `crates/stg-core/src/world/motion.rs` 的 `mod tests`：

```rust
/// dur == 0 是瞬时 set：立刻落 speed/angle 并刷 vx/vy，且**不**武装插值器。
#[test]
fn set_enemy_vel_polar_dur_zero_is_instant_and_arms_nothing() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 0, 0);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(5));
    assert_eq!(w.body.enemies.angle[i], Angle::QUARTER);
    assert_eq!(w.body.enemies.vy[i], Fx::from_int(5), "瞬时版也要刷积分真相");
    assert_eq!(w.body.enemies.vel_active[i], 0, "dur=0 不武装插值器");
    assert_eq!(w.body.enemies.vel_touched[i], 1, "瞬时版同样算表达过速度意图");
}

/// dur > 0 只武装、当帧不动值。from 取**当前**值，to 取目标值，space 落 POLAR。
#[test]
fn set_enemy_vel_polar_dur_positive_arms_only() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(2), 0, 0); // 先摆一个起点
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 10, 3);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(2), "武装当帧不动值");
    assert_eq!(w.body.enemies.vel_active[i], 1);
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_POLAR);
    assert_eq!(w.body.enemies.vel_from_0[i], Fx::from_int(2).raw());
    assert_eq!(w.body.enemies.vel_from_1[i], Angle::ZERO.raw() as i32);
    assert_eq!(w.body.enemies.vel_to_0[i], Fx::from_int(5).raw());
    assert_eq!(w.body.enemies.vel_to_1[i], Angle::QUARTER.raw() as i32);
    assert_eq!(w.body.enemies.vel_t[i], 0);
    assert_eq!(w.body.enemies.vel_dur[i], 10);
    assert_eq!(w.body.enemies.vel_easing[i], 3);
}

/// 笛卡尔武装：载体槽装 (vx, vy)，space 落 CART。dur=0 时刷 vx/vy 并**回填** speed/angle。
#[test]
fn set_enemy_vel_cart_dur_zero_backfills_author_view() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_cart(h, Fx::from_int(3), Fx::from_int(4), 0, 0);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.vx[i], Fx::from_int(3));
    assert_eq!(w.body.enemies.vy[i], Fx::from_int(4));
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(5), "回填 3-4-5");
}

/// 单轴保持另一轴：move_angle 只改方向、速率一字不动。取 speed=7.0（≠1.0，
/// 否则"另一分量被填成 ONE"这个错法不可辨）、angle 从 0 到 QUARTER。
#[test]
fn set_enemy_angle_preserves_speed() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(7), 0, 0);
    w.body.set_enemy_angle(h, Angle::QUARTER, 0, 0);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(7), "只转向不改速率");
    assert_eq!(w.body.enemies.angle[i], Angle::QUARTER);
}

/// 单轴保持另一轴（对偶）：move_speed 只改速率、方向一字不动。
#[test]
fn set_enemy_speed_preserves_angle() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(7), 0, 0);
    w.body.set_enemy_speed(h, Fx::from_int(2), 0, 0);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.angle[i], Angle::QUARTER, "只调速不改方向");
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(2));
}

/// 重新武装：插值途中再调 → 无条件重初始化，from 取**当前**值、space 可切换。
#[test]
fn rearming_switches_space_and_restarts_from_current() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_cart(h, Fx::from_int(9), Fx::ZERO, 10, 0); // 笛卡尔在飞
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 4, 0); // 切极坐标
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_POLAR);
    assert_eq!(w.body.enemies.vel_t[i], 0, "重新武装即清计时");
    assert_eq!(w.body.enemies.vel_dur[i], 4);
}

/// P4-b：easing 越界 → no-op + contract_viol 计数，不 panic、不落任何字段。
#[test]
fn bad_easing_is_noop_and_counted() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let before = w.body.diag.contract_viol;
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 10, 8);
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.vel_active[i], 0, "坏参数不得武装");
    assert_eq!(w.body.enemies.vel_touched[i], 0, "坏参数不算表达过意图");
    assert_eq!(w.body.diag.contract_viol, before + 1);
}

/// P4-b：悬垂句柄 → no-op + 计数（同 move_enemy_to 既有做法）。
#[test]
fn stale_handle_is_noop_and_counted() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    assert!(w.body.enemies.free(h));
    let before = w.body.diag.contract_viol;
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 0, 0);
    assert_eq!(w.body.diag.contract_viol, before + 1);
}
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p stg-core set_enemy_vel_polar_dur_zero_is_instant_and_arms_nothing
```
Expected：编译失败 —— `no method named 'set_enemy_vel_polar'`。

- [ ] **Step 3：实现四个 setter + 私有武装助手**

`crates/stg-core/src/world/motion.rs`，`impl WorldBody` 内：

```rust
    /// 四条速度动词共用的前置校验（P4-b）：悬垂 → None + 计数；`easing >= 8` → None + 计数。
    /// **两条都必须在任何字段落地之前**——坏参数是 no-op，不能留半个武装好的插值器。
    fn enemy_vel_precheck(&mut self, h: EnemyHandle, easing: u8) -> Option<usize> {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return None;
        };
        if easing >= 8 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return None;
        }
        Some(i)
    }

    /// 武装速度插值器（`dur > 0` 专用；`dur == 0` 的瞬时路径由各 setter 自己走完）。
    /// `from_*` 一律取**当前**值 —— 重新武装 = 从此刻重新起算（同 `STEP_*` 的
    /// "scratch 无条件重初始化"）。
    fn arm_enemy_vel(
        &mut self,
        i: usize,
        space: u8,
        from_0: i32,
        from_1: i32,
        to_0: i32,
        to_1: i32,
        dur: u16,
        easing: u8,
    ) {
        self.enemies.vel_space[i] = space;
        self.enemies.vel_from_0[i] = from_0;
        self.enemies.vel_from_1[i] = from_1;
        self.enemies.vel_to_0[i] = to_0;
        self.enemies.vel_to_1[i] = to_1;
        self.enemies.vel_t[i] = 0;
        self.enemies.vel_dur[i] = dur;
        self.enemies.vel_easing[i] = easing;
        self.enemies.vel_active[i] = 1;
    }

    /// 极坐标速度（ZUN `404 moveVel` / `405 moveVelTime`）。`dur == 0` = 立即设。
    pub fn set_enemy_vel_polar(
        &mut self,
        h: EnemyHandle,
        angle: Angle,
        speed: Fx,
        dur: u16,
        easing: u8,
    ) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        self.enemies.vel_touched[i] = 1;
        if dur == 0 {
            self.enemies.speed[i] = speed;
            self.enemies.angle[i] = angle;
            self.refresh_enemy_vel_from_polar(i);
            self.enemies.vel_active[i] = 0;
            return;
        }
        self.arm_enemy_vel(
            i,
            crate::enemy::VEL_SPACE_POLAR,
            self.enemies.speed[i].raw(),
            self.enemies.angle[i].raw() as i32,
            speed.raw(),
            angle.raw() as i32,
            dur,
            easing,
        );
    }

    /// 笛卡尔速度。`dur > 0` 时**在笛卡尔空间插值**（spec §3.3：不转极坐标，否则它就
    /// 退化成 `set_enemy_vel_polar` 的语法糖）。
    pub fn set_enemy_vel_cart(&mut self, h: EnemyHandle, vx: Fx, vy: Fx, dur: u16, easing: u8) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        self.enemies.vel_touched[i] = 1;
        if dur == 0 {
            self.enemies.vx[i] = vx;
            self.enemies.vy[i] = vy;
            self.backfill_enemy_polar(i);
            self.enemies.vel_active[i] = 0;
            return;
        }
        self.arm_enemy_vel(
            i,
            crate::enemy::VEL_SPACE_CART,
            self.enemies.vx[i].raw(),
            self.enemies.vy[i].raw(),
            vx.raw(),
            vy.raw(),
            dur,
            easing,
        );
    }

    /// 只转向、保持速率（ZUN `440 moveAngle` / `441`）。走极坐标空间，速率分量填当前值。
    pub fn set_enemy_angle(&mut self, h: EnemyHandle, angle: Angle, dur: u16, easing: u8) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        let keep = self.enemies.speed[i];
        self.set_enemy_vel_polar(h, angle, keep, dur, easing);
    }

    /// 只调速、保持方向（ZUN `444 moveSpeed` / `445`）。走极坐标空间，角度分量填当前值。
    pub fn set_enemy_speed(&mut self, h: EnemyHandle, speed: Fx, dur: u16, easing: u8) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        let keep = self.enemies.angle[i];
        self.set_enemy_vel_polar(h, keep, speed, dur, easing);
    }
```

> **注**：`set_enemy_angle`/`set_enemy_speed` 走两次 precheck（自己一次 + 委托一次），
> 于是**坏参数会计两次 `contract_viol`**。这不可接受——计数参与校验和。**改法**：这两条
> 不委托公开 setter，而是自己读当前值后直接调 `arm_enemy_vel` / 走瞬时路径。实现时按这个
> 口径写，并让 `bad_easing_is_noop_and_counted` 对这两条也各来一次（`before + 1` 而非 `+2`）。

- [ ] **Step 4：`move_enemy_to` 两条路径清 `vel_touched`**

`crates/stg-core/src/world.rs`，`move_enemy_to` 内：

`dur == 0` 分支（现有 `self.enemies.mv_active[i] = 0;` 那行前后）与函数末尾的武装块，**各加一行**：

```rust
        // move_to 是一条全新的位置命令——它之前的速度意图是陈的，由它接管。
        // 到点清速的判据（integrate 相）读的就是这一位。
        self.enemies.vel_touched[i] = 0;
```

- [ ] **Step 5：跑测试确认通过**

```bash
cargo test -p stg-core --lib world::motion
```
Expected：全部 PASS。

- [ ] **Step 6：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
feat(core): 速度插值器的武装——四个 set_enemy_* 写 API（T2）

dur==0 走瞬时 set（并同步另一半表示），dur>0 只武装、当帧不动值。from 一律取
当前值 ⇒ 重新武装 = 从此刻重起算，空间可切换（同 STEP_* 的 scratch 无条件重初始化）。
P4-b 两条前置校验（悬垂/easing 越界）在任何字段落地之前，坏参数不留半个武装好的
插值器、也不置 vel_touched。move_enemy_to 两条路径各加 vel_touched=0。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 3：`integrate` 相的分层仲裁 + 双空间推进

**这是整刀的核心。** 招牌判别式在此。

**Files:**
- Modify: `crates/stg-core/src/world/integrate.rs:67-114`（敌人段）

**Interfaces:**
- Consumes：T1 的字段与同步核、T2 的武装语义。
- Produces：无新公开 API，只有行为。

- [ ] **Step 1：写失败测试（招牌判别式 + 最短弧 + 三条仲裁腿）**

放 `crates/stg-core/src/world/integrate.rs` 的 `mod tests`：

```rust
/// 【招牌判别式】极坐标插值与笛卡尔插值**走的不是同一条路**。两者都从「朝右 5.0」
/// 插到「朝下 5.0」（屏幕坐标 y 向下，故 QUARTER=90° 是朝下），取 t=0.5 那帧看速率：
///   极坐标 → 匀速扫弧，速率恒为 5.0
///   笛卡尔 → 直线穿过 (2.5, 2.5)，速率掉到 2.5·√2 ≈ 3.54
/// 这一条同时逮住两个错法：笛卡尔实现写成了极坐标；move_vel_xy 被做成 move_vel 的糖。
#[test]
fn polar_and_cartesian_velocity_interpolation_take_different_paths() {
    // —— 极坐标腿：速率全程恒定 ——
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(5), 0, 0);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 4, 0); // Linear
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // t = 2/4 = 0.5
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(
        w.body.enemies.speed[i],
        Fx::from_int(5),
        "极坐标插值：速率是被直接插的量，5.0→5.0 全程恒定"
    );

    // —— 笛卡尔腿：同样两端，中途速率必须掉下来 ——
    let mut w2 = crate::step::World::new(1);
    let h2 = crate::world::test_support::spawn_enemy(&mut w2, 0, 0, 5);
    w2.body
        .set_enemy_vel_cart(h2, Fx::from_int(5), Fx::ZERO, 0, 0);
    w2.body
        .set_enemy_vel_cart(h2, Fx::ZERO, Fx::from_int(5), 4, 0); // Linear
    crate::world::test_support::step_t(&mut w2, &InputFrame::empty(0));
    crate::world::test_support::step_t(&mut w2, &InputFrame::empty(1));
    let i2 = w2.body.enemies.get(h2).unwrap();
    // 分量是精确的线性中点
    assert_eq!(w2.body.enemies.vx[i2], Fx::from_raw(163840), "5.0 的一半 = 2.5");
    assert_eq!(w2.body.enemies.vy[i2], Fx::from_raw(163840));
    // 回填出来的速率落在 3.5~3.6（2.5·√2 = 3.5355；isqrt 舍入留余量）
    let sp = w2.body.enemies.speed[i2];
    assert!(
        sp > Fx::from_raw(229376) && sp < Fx::from_raw(235930),
        "笛卡尔插值中点速率应约 3.54，实得 {sp:?}——若这里是 5.0 说明走了极坐标空间"
    );
}

/// 最短弧：350° → 10° 应走 **+20°**（顺时针跨 0° 缝），而不是 −340°。
/// 取 dur=2、Linear，中点应落在 0°（即 360°）附近而非 180° 那边。
#[test]
fn angle_interpolation_takes_shortest_arc_across_the_seam() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let a350 = Angle::from_raw(63715); // 350° ≈ 65536*350/360
    let a10 = Angle::from_raw(1820); //  10° ≈ 65536*10/360
    w.body.set_enemy_vel_polar(h, a350, Fx::from_int(3), 0, 0);
    w.body.set_enemy_vel_polar(h, a10, Fx::from_int(3), 2, 0);
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // t = 1/2
    let i = w.body.enemies.get(h).unwrap();
    let mid = w.body.enemies.angle[i].raw();
    // 中点应在缝上（接近 0 或接近 65536），绝不在 180° 附近
    assert!(
        mid > 64000 || mid < 1500,
        "中点应落在 0° 缝附近，实得 {mid}——若在 32768 附近说明走了长弧"
    );
}

/// 仲裁腿 (a)：move_to 单独 → 到点**仍清速**（守住原契约，一字不变）。
#[test]
fn arrival_still_clears_velocity_when_script_never_touched_it() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    let i = w.body.enemies.get(h).unwrap();
    w.body.enemies.vx[i] = Fx::from_int(7); // 残留速度
    w.body
        .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 2, 0);
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // 到点
    assert_eq!(w.body.enemies.mv_active[i], 0, "已到点");
    assert_eq!(w.body.enemies.vx[i], Fx::ZERO, "未表达速度意图 ⇒ 到点清速");
}

/// 仲裁腿 (b)：move_to 途中调速度动词，且**速度插值先于位置插值到期**
/// → 到点**不清速**、速度立刻接管。
/// 这条正是「拿 vel_active 当判据」会漏掉的那格——速度那条 dur=2 在第 2 帧就结束、
/// vel_active 归 0，而位置那条 dur=4 到第 4 帧才到点。必须让 dur 严格不等。
#[test]
fn arrival_preserves_velocity_when_script_expressed_intent_earlier() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    w.body
        .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 4, 0);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(3), 2, 0); // 先到期
    for k in 0..4 {
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
    }
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.mv_active[i], 0, "位置插值已到点");
    assert_eq!(w.body.enemies.vel_active[i], 0, "速度插值早已到期");
    assert_eq!(
        w.body.enemies.vy[i],
        Fx::from_int(3),
        "到点不得清速——速度意图表达在先，落地即接管"
    );
}

/// 仲裁腿 (c)：速度动词在前、move_to 在后 → 到点**清速**（武装时 vel_touched 归零）。
/// 缺了这条的话，「move_enemy_to 忘了清 vel_touched」这个错法照样绿。
#[test]
fn move_to_rearm_resets_touched_so_arrival_clears_again() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(3), 0, 0); // 先设速度
    w.body
        .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 2, 0); // 再 move_to
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(
        w.body.enemies.vy[i],
        Fx::ZERO,
        "move_to 武装即归零 vel_touched ⇒ 到点照清"
    );
}

/// 死代码通电的正面证据：move_vel 之后，位置纯靠 `x += vx` 推进。
/// 这条分支此前永远在加零（没有任何 syscall 能写 vx/vy），世界层测试绿的是够不着的代码。
#[test]
fn uniform_velocity_actually_moves_the_enemy_now() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(2), 0, 0);
    let i = w.body.enemies.get(h).unwrap();
    let y0 = w.body.enemies.y[i];
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
    assert_eq!(
        w.body.enemies.y[i],
        y0 + Fx::from_int(6),
        "3 帧 × 2.0/帧 = 6.0"
    );
}

/// 速度插值到期写**精确终值**（不吃插值舍入）并清 vel_active。
#[test]
fn velocity_interpolation_lands_on_exact_target_and_disarms() {
    let mut w = crate::step::World::new(1);
    let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body
        .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(1), 0, 0);
    w.body
        .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 3, 4); // CubicIn
    for k in 0..3 {
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
    }
    let i = w.body.enemies.get(h).unwrap();
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(5), "终帧精确终值");
    assert_eq!(w.body.enemies.angle[i], Angle::QUARTER);
    assert_eq!(w.body.enemies.vel_active[i], 0, "到期即解除武装");
}
```

> **注**：`Angle::from_raw` / `InputFrame::empty` / `test_support::step_t` 的确切名字照
> `integrate.rs` 与 `transform.rs` 既有测试原样照抄（`transform.rs:690` 有
> `slot(0, OP_STEP_ANGLE, 1820, 2)` 这样的现成 350°/10° 用法参考）。

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p stg-core polar_and_cartesian_velocity_interpolation_take_different_paths
```
Expected：FAIL —— 速度插值器无人推进，`speed` 停在起点 5.0（极坐标腿意外地绿），
笛卡尔腿的 `vx` 停在 5.0 而非 2.5 ⇒ 断言失败。

- [ ] **Step 3：实现推进 + 仲裁**

`crates/stg-core/src/world/integrate.rs`，敌人段的循环体内，**在 `if self.enemies.mv_active[i] != 0` 之前**插入速度插值推进；并把到点清速那两行改成条件的。

先加一个私有方法（放本文件 `impl WorldBody` 内，敌人段循环之外）：

```rust
    /// 推进一只敌人的速度插值一帧（敌人运动动词族刀）。绝对插值：每帧从 `from` 重算，
    /// 不累积误差；终帧写精确终值。两条空间路径的差别是**这刀的全部要点**——
    /// 极坐标插 `speed`/`angle` 再刷 `vx/vy`（匀速扫弧），笛卡尔插 `vx/vy` 再回填
    /// `speed`/`angle`（直线穿过、中途掉速）。把笛卡尔那条改成转极坐标去插，
    /// `move_vel_xy` 就退化成 `move_vel` 的语法糖了（spec §3.3）。
    fn tick_enemy_vel(&mut self, i: usize) {
        self.enemies.vel_t[i] += 1;
        let done = self.enemies.vel_t[i] >= self.enemies.vel_dur[i];
        // e ∈ [0,1]：done 时不参与（直接写终值），故只在未完成时求
        let e = if done {
            Fx::ONE
        } else {
            let t = Fx::from_raw(
                (((self.enemies.vel_t[i] as i64) << 16) / self.enemies.vel_dur[i] as i64) as i32,
            );
            crate::math::easing::ease(
                crate::math::easing::from_id(self.enemies.vel_easing[i]),
                t,
            )
        };
        let (f0, f1) = (self.enemies.vel_from_0[i], self.enemies.vel_from_1[i]);
        let (t0, t1) = (self.enemies.vel_to_0[i], self.enemies.vel_to_1[i]);
        if self.enemies.vel_space[i] == crate::enemy::VEL_SPACE_CART {
            // 笛卡尔：两个分量各自线性插（负 delta 的 `>>16` 是算术右移，两平台一致）
            let vx = if done {
                t0
            } else {
                (f0 as i64 + (((t0 as i64 - f0 as i64) * e.raw() as i64) >> 16)) as i32
            };
            let vy = if done {
                t1
            } else {
                (f1 as i64 + (((t1 as i64 - f1 as i64) * e.raw() as i64) >> 16)) as i32
            };
            self.enemies.vx[i] = Fx::from_raw(vx);
            self.enemies.vy[i] = Fx::from_raw(vy);
            self.backfill_enemy_polar(i);
        } else {
            // 极坐标：速率线性插；角度走**最短弧**（同 transform.rs 的 STEP_ANGLE）
            let sp = if done {
                t0
            } else {
                (f0 as i64 + (((t0 as i64 - f0 as i64) * e.raw() as i64) >> 16)) as i32
            };
            let start = crate::math::Angle::from_raw(f1 as u16);
            let delta = (t1 as u16).wrapping_sub(start.raw()) as i16;
            let scaled = if done {
                delta
            } else {
                ((delta as i64 * e.raw() as i64) >> 16) as i16
            };
            self.enemies.speed[i] = Fx::from_raw(sp);
            self.enemies.angle[i] = start.add_delta(scaled);
            self.refresh_enemy_vel_from_polar(i);
        }
        if done {
            self.enemies.vel_active[i] = 0;
        }
    }
```

然后敌人段循环体改成：

```rust
                // ① 速度插值恒跑（分层：它只改速度，不决定位置归谁）
                if self.enemies.vel_active[i] != 0 {
                    self.tick_enemy_vel(i);
                }
                // ② 位置插值接管位置，否则匀速积分
                if self.enemies.mv_active[i] != 0 {
                    // ……既有插值代码原样不动，只改到点那三行……
```

到点那段（现有 `self.enemies.vx[i] = Fx::ZERO;` / `vy` 两行）改为：

```rust
                        self.enemies.x[i] = self.enemies.mv_to_x[i];
                        self.enemies.y[i] = self.enemies.mv_to_y[i];
                        // 到点清速**条件化**（敌人运动动词族刀）：只在脚本从未表达过速度
                        // 意图时清。判据是黏滞位 vel_touched 而**不是** vel_active——
                        // 速度插值常常先于位置插值到期，那时 vel_active 已归 0，
                        // 拿它当判据会把刚缓好的速度误清（spec §6.3 的修订记录）。
                        if self.enemies.vel_touched[i] == 0 {
                            self.enemies.vx[i] = Fx::ZERO;
                            self.enemies.vy[i] = Fx::ZERO;
                        }
                        self.enemies.mv_active[i] = 0;
```

⚠️ **`Fx::ONE` 在 `done` 分支只是占位**（走的是 `t0`/`delta` 直写路径），别让它参与运算。
若 clippy 嫌它无用，改成把 `e` 的求值挪进 `else` 分支里。

- [ ] **Step 4：跑测试确认通过**

```bash
cargo test -p stg-core --lib world::integrate
```
Expected：全部 PASS。

- [ ] **Step 5：改既有的「到点清速」测试**

`integrate.rs:692` 附近那条 `assert_eq!(w.body.enemies.vx[i], Fx::ZERO, "到点清速")` 所在的测试——它现在测的是仲裁腿 (a)。**保留它**（原契约仍成立），只在断言消息里补一句
`（未表达速度意图的路径；表达过的走 arrival_preserves_velocity_when_script_expressed_intent_earlier）`。

- [ ] **Step 6：变异验证（两次，各自实证"只有它转红"）**

**变异 1 —— 笛卡尔走成极坐标**：把 `tick_enemy_vel` 里 `VEL_SPACE_CART` 那条分支删掉
（让两个空间都走 else 的极坐标路径）。

```bash
cargo test -p stg-core --lib world 2>&1 | grep -E "^test .* FAILED|test result"
```
Expected：**只有** `polar_and_cartesian_velocity_interpolation_take_different_paths` 转红。
⚠️ 用**编辑撤回**恢复，**不要** `git checkout`。

**变异 2 —— 到点清速判据换成 `vel_active`**：把 `if self.enemies.vel_touched[i] == 0`
改成 `if self.enemies.vel_active[i] == 0`。

Expected：**只有** `arrival_preserves_velocity_when_script_expressed_intent_earlier` 转红
（腿 (a)、(c) 仍绿——这正是那条规则的隐蔽处）。同样编辑撤回。

把两次变异的实际输出记进 Task 报告。

- [ ] **Step 7：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
feat(core): integrate 相分层仲裁 + 速度插值双空间推进（T3）

① 速度插值恒跑（只改速度），② 位置插值决定位置归谁——这就是分层。两条空间路径的
差别是本刀全部要点：极坐标插 speed/angle 再刷 vx/vy（匀速扫弧），笛卡尔插 vx/vy 再
回填（直线穿过、中途掉速）。招牌判别式取 t=0.5 的速率 5.0 vs 3.54，一条同时逮住
"笛卡尔写成极坐标"与"move_vel_xy 做成 move_vel 的糖"两个错法。

到点清速条件化，判据是黏滞位 vel_touched 而非 vel_active——速度插值常先于位置插值
到期，拿 vel_active 当判据会把刚缓好的速度误清。三条仲裁腿各守一格。

死代码通电：`x += vx` 这条分支此前永远在加零（无任何 syscall 能写 vx/vy）。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 4：四条动词 syscall + 编译器接线

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（4 个号常量 + 4 个 `sys_*` + 派发表）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（4 个 `Builtin` 条目 + 两处 exhaustive 名单）

**Interfaces:**
- Consumes：T2 的四个 `set_enemy_*`。
- Produces：syscall 83–86（号见「冻结的取值与命名」）；ECL 表层四条动词。

- [ ] **Step 1：写失败测试**

放 `crates/stg-core/src/ecl/syscall.rs` 的 `mod tests`（照 `sys_move_enemy_to_arms_interpolator` 的体例）：

```rust
/// move_vel（83）：4 参正序 dur,angle,speed,easing；owner 取自 self。
#[test]
fn sys_move_vel_arms_polar_interpolator() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let args = [
        20,
        crate::math::Angle::QUARTER.raw() as i32,
        Fx::from_int(4).raw(),
        3,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.vel_active[i], 1);
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_POLAR);
    assert_eq!(w.body.enemies.vel_to_0[i], Fx::from_int(4).raw());
    assert_eq!(
        w.body.enemies.vel_to_1[i],
        crate::math::Angle::QUARTER.raw() as i32
    );
    assert_eq!(w.body.enemies.vel_dur[i], 20);
    assert_eq!(w.body.enemies.vel_easing[i], 3);
}

/// move_vel_xy（84）：落 CART 空间——**参数序 dur,vx,vy,easing**（vx 在前）。
#[test]
fn sys_move_vel_xy_arms_cartesian_interpolator() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    // vx=3, vy=-7：**x≠y 且异号**，两条派发臂写反立刻可辨
    let args = [12, Fx::from_int(3).raw(), Fx::from_int(-7).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL_XY, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_CART);
    assert_eq!(w.body.enemies.vel_to_0[i], Fx::from_int(3).raw(), "slot0 = vx");
    assert_eq!(w.body.enemies.vel_to_1[i], Fx::from_int(-7).raw(), "slot1 = vy");
}

/// move_angle（85）/ move_speed（86）：3 参，各自保持另一分量。
#[test]
fn sys_move_angle_and_speed_are_single_axis() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let i = eh.index as usize;
    // 先摆一个 speed=7、angle=0 的起点（dur=0 瞬时）
    let seed = [0, 0, Fx::from_int(7).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &seed).is_ok());
    // move_angle(0, QUARTER, 0) → 只转向
    let a = [0, crate::math::Angle::QUARTER.raw() as i32, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ANGLE, &a).is_ok());
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(7), "转向不改速率");
    assert_eq!(w.body.enemies.angle[i], crate::math::Angle::QUARTER);
    // move_speed(0, 2.0, 0) → 只调速
    let s = [0, Fx::from_int(2).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_SPEED, &s).is_ok());
    assert_eq!(
        w.body.enemies.angle[i],
        crate::math::Angle::QUARTER,
        "调速不改方向"
    );
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(2));
}

/// 四条动词 self owner != ENEMY → Fault（同 move_to 现状）。
#[test]
fn motion_verbs_fault_on_non_enemy_owner() {
    let (mut w, ecl) = fresh();
    for (sys, argc) in [
        (SYS_MOVE_VEL, 4),
        (SYS_MOVE_VEL_XY, 4),
        (SYS_MOVE_ANGLE, 3),
        (SYS_MOVE_SPEED, 3),
    ] {
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        let args = vec![0i32; argc];
        let r = call(&mut w, &ecl, &mut task, sys, &args);
        assert_eq!(r, Err(FAULT_BAD_OP), "syscall {sys} 非敌 owner 应 Fault");
    }
}
```

再在 `crates/stg-ecl-compiler/src/lang/mod.rs` 的 `mod tests`（照该文件既有 `.ecl` 编译测试体例）加一条源码级编译测试：

```rust
/// 四条动词在表层语言里可用、类型正确（angle 参数收 angle 型、speed 收 fx 型）。
#[test]
fn motion_verbs_compile_from_source() {
    let src = r#"
        enemy zako {
            move_vel(20, 90deg, 4.0fx, 3);
            move_vel_xy(12, 3.0fx, -7.0fx, 0);
            move_angle(30, 45deg, 2);
            move_speed(15, 2.5fx, 1);
        }
    "#;
    assert!(compile_ok(src), "四条动词应能编译");
}
```

> **注**：`enemy zako { .. }` 的确切表层语法、`compile_ok` 的确切助手名，照
> `crates/stg-ecl-compiler/src/lang/mod.rs` 与 `docs/ecl-lang.md` 里的**现成例子**照抄。
> 不要凭印象造语法。

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p stg-core sys_move_vel_arms_polar_interpolator
```
Expected：编译失败 —— `cannot find value 'SYS_MOVE_VEL'`。

- [ ] **Step 3：加号常量 + 四个 `sys_*` + 派发**

`crates/stg-core/src/ecl/syscall.rs`，在 `SYS_ENEMY_ALIVE` 之后：

```rust
/// 极坐标速度（83；ZUN `404 moveVel` / `405 moveVelTime`）：4 参正序
/// `dur, angle, speed, easing`。self owner 非 ENEMY → Fault（同 `SYS_MOVE_ENEMY_TO`）。
/// `dur == 0` 是合法退化 = 立即设。
pub const SYS_MOVE_VEL: u16 = 83;
/// 笛卡尔速度（84）：4 参正序 `dur, vx, vy, easing`。**`dur > 0` 时在笛卡尔空间插值**
/// ——不转极坐标，否则它就退化成 [`SYS_MOVE_VEL`] 的语法糖（spec §3.3）。
pub const SYS_MOVE_VEL_XY: u16 = 84;
/// 只转向、保持速率（85；ZUN `440 moveAngle`）：3 参正序 `dur, angle, easing`。
pub const SYS_MOVE_ANGLE: u16 = 85;
/// 只调速、保持方向（86；ZUN `444 moveSpeed`）：3 参正序 `dur, speed, easing`。
pub const SYS_MOVE_SPEED: u16 = 86;
```

四个实现（放 `sys_move_enemy_to` 附近，**参数逆序弹出**）：

```rust
/// `SYS_MOVE_VEL`（83）：逆序弹出 `easing, speed, angle, dur`。
fn sys_move_vel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    ctx.body.set_enemy_vel_polar(
        h,
        crate::math::Angle::from_raw(angle as u16),
        Fx::from_raw(speed),
        dur as u16,
        easing as u8,
    );
    Ok(())
}

/// `SYS_MOVE_VEL_XY`（84）：逆序弹出 `easing, vy, vx, dur`。
fn sys_move_vel_xy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let vy = pop(task)?;
    let vx = pop(task)?;
    let dur = pop(task)?;
    ctx.body.set_enemy_vel_cart(
        h,
        Fx::from_raw(vx),
        Fx::from_raw(vy),
        dur as u16,
        easing as u8,
    );
    Ok(())
}

/// `SYS_MOVE_ANGLE`（85）：逆序弹出 `easing, angle, dur`。
fn sys_move_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    ctx.body.set_enemy_angle(
        h,
        crate::math::Angle::from_raw(angle as u16),
        dur as u16,
        easing as u8,
    );
    Ok(())
}

/// `SYS_MOVE_SPEED`（86）：逆序弹出 `easing, speed, dur`。
fn sys_move_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let dur = pop(task)?;
    ctx.body
        .set_enemy_speed(h, Fx::from_raw(speed), dur as u16, easing as u8);
    Ok(())
}
```

派发表（`SYS_ENEMY_ALIVE => ...` 之后）：

```rust
        SYS_MOVE_VEL => sys_move_vel(task, ctx),
        SYS_MOVE_VEL_XY => sys_move_vel_xy(task, ctx),
        SYS_MOVE_ANGLE => sys_move_angle(task, ctx),
        SYS_MOVE_SPEED => sys_move_speed(task, ctx),
```

⚠️ **`ARITY` 表**（若 syscall 也有 arity 表）与白名单沙箱绑定同步——照 `SYS_MOVE_ENEMY_TO`
在这些表里出现的**每一处**都加上对应四条。用 `rg 'SYS_MOVE_ENEMY_TO' crates/` 找全。

- [ ] **Step 4：编译器接线**

`crates/stg-ecl-compiler/src/lang/builtins.rs`，照 `move_to` 那条的体例加四个 `Builtin`：

```rust
    Builtin {
        name: "move_vel",
        syscall: syscall::SYS_MOVE_VEL,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Fx), Val(Int)],
        ret: None,
        doc: "敌自身(self owner 非 ENEMY → Fault)按 easing 在 dur 帧内把速度缓动到「angle 方向、speed 速率」;dur=0 = 立即设。**极坐标空间插值**(匀速扫弧,速率按曲线走)——要笛卡尔直线插值用 move_vel_xy",
        param_names: &["dur", "angle", "speed", "easing"],
    },
    Builtin {
        name: "move_vel_xy",
        syscall: syscall::SYS_MOVE_VEL_XY,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx), Val(Int)],
        ret: None,
        doc: "同 move_vel 但收笛卡尔分量,且 dur>0 时**在笛卡尔空间插值**(两分量各自线性插,中途速率会掉——线性缓动即恒定加速度);要匀速转向用 move_vel。保住一轴的写法:move_vel_xy(30, $self_vx, 4.0fx, 2)",
        param_names: &["dur", "vx", "vy", "easing"],
    },
    Builtin {
        name: "move_angle",
        syscall: syscall::SYS_MOVE_ANGLE,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Int)],
        ret: None,
        doc: "只转向、速率一字不动;dur>0 走**最短弧**(350deg→10deg 走 +20deg 不走 -340deg)。相对转向:move_angle(60, $self_angle + 15deg, 3)",
        param_names: &["dur", "angle", "easing"],
    },
    Builtin {
        name: "move_speed",
        syscall: syscall::SYS_MOVE_SPEED,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Int)],
        ret: None,
        doc: "只调速、方向一字不动。相对加速:move_speed(30, $self_speed * 2.0fx, 2)",
        param_names: &["dur", "speed", "easing"],
    },
```

⚠️ **`builtins.rs` 里有两处 exhaustive 的名字列表**（约 819 / 1030 行，`"move_to"` 各出现
一次）。四个新名字必须**两处都加**，否则测试会红。

- [ ] **Step 5：跑测试确认通过 + 同步元数据**

```bash
cargo test -p stg-core --lib ecl::syscall
cargo test -p stg-ecl-compiler
cargo run -p stg-harness -- gen-ecl-meta      # 改了 builtins.rs ⇒ 必跑
```

- [ ] **Step 6：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
feat(ecl): 四条敌人运动动词 syscall 83-86 + 编译器接线（T4）

move_vel(dur,angle,speed,easing) / move_vel_xy(dur,vx,vy,easing) /
move_angle(dur,angle,easing) / move_speed(dur,speed,easing)。全部 self-only,
owner 非 ENEMY → Fault(同 move_to 现状);dur=0 是合法退化 = 立即设。
参数逆序弹出,正序即文档序。gen-ecl-meta 已同步。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 5：四个 `$self_*` 引擎变量

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（4 个号 + 一个 `self_vel` 助手 + 派发）
- Modify: `crates/stg-ecl-compiler/src/lang/ast.rs:191`（`EngVar` 枚举 +4）
- Modify: `crates/stg-ecl-compiler/src/lang/parse.rs:50`（白名单 +4）、`parse.rs:732`（错误提示串）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs:779`（`engine_var_info` 映射 +4）

**Interfaces:**
- Consumes：T1 的敌池 `speed`/`angle` 字段。
- Produces：`$self_vx` / `$self_vy` / `$self_speed` / `$self_angle`（syscall 87–90）。

- [ ] **Step 1：写失败测试**

`crates/stg-core/src/ecl/syscall.rs` 的 `mod tests`：

```rust
/// 四个 $self_* 在敌 owner 下读到池字段。取 (3, -7) 这个 x≠y 且异号的速度——
/// 派发臂写反立刻可辨。
#[test]
fn self_velocity_vars_read_enemy_pool() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = eh.index as usize;
    w.body.enemies.vx[i] = Fx::from_int(3);
    w.body.enemies.vy[i] = Fx::from_int(-7);
    w.body.enemies.speed[i] = Fx::from_int(9);
    w.body.enemies.angle[i] = crate::math::Angle::QUARTER;
    let mk = || Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    for (sys, want) in [
        (SYS_SELF_VX, Fx::from_int(3).raw()),
        (SYS_SELF_VY, Fx::from_int(-7).raw()),
        (SYS_SELF_SPEED, Fx::from_int(9).raw()),
        (SYS_SELF_ANGLE, crate::math::Angle::QUARTER.raw() as i32),
    ] {
        let mut task = mk();
        assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
        assert_eq!(pop(&mut task).unwrap(), want, "syscall {sys}");
    }
}

/// 弹 owner 下同样有效（弹池本就有这四个字段——双表示是从弹抄来的）。
#[test]
fn self_velocity_vars_dispatch_to_bullet_pool() {
    let (mut w, ecl) = fresh();
    let bh = crate::world::test_support::bullet_at(&mut w, 0, 0);
    let i = bh.index as usize;
    w.body.bullets.vx[i] = Fx::from_int(2);
    let mut task = Task {
        owner_kind: OWNER_BULLET,
        owner_index: bh.index,
        owner_gen: bh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_VX, &[]).is_ok());
    assert_eq!(pop(&mut task).unwrap(), Fx::from_int(2).raw());
}

/// 非敌非弹 owner → 0（同 $self_x 的既有降级）。
#[test]
fn self_velocity_vars_degrade_to_zero_for_stage_owner() {
    let (mut w, ecl) = fresh();
    for sys in [SYS_SELF_VX, SYS_SELF_VY, SYS_SELF_SPEED, SYS_SELF_ANGLE] {
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
        assert_eq!(pop(&mut task).unwrap(), 0, "syscall {sys} 非敌非弹应降级 0");
    }
}
```

`crates/stg-ecl-compiler/src/lang/mod.rs` 的 `mod tests` 加**闭环测试**：

```rust
/// 【白名单完整性的判别式】"保住一轴"用例：只插 vy、vx 一字不动。
/// 这是 §4.2 那个用例的正面证据,也是**白名单只加 $self_speed/$self_angle
/// 而漏掉两个笛卡尔变量时唯一会红的测试**。
#[test]
fn keeping_one_cartesian_axis_compiles() {
    let src = r#"
        enemy zako {
            move_vel_xy(30, $self_vx, 4.0fx, 2);
            move_angle(60, $self_angle + 15deg, 3);
            move_speed(30, $self_speed * 2.0fx, 2);
            move_vel_xy(0, 1.0fx, $self_vy, 0);
        }
    "#;
    assert!(compile_ok(src), "四个 $self_* 都要在白名单里");
}
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p stg-core self_velocity_vars_read_enemy_pool
```
Expected：编译失败 —— `cannot find value 'SYS_SELF_VX'`。

- [ ] **Step 3：core 侧实现**

`syscall.rs`，号常量：

```rust
/// owner 的笛卡尔速度 x（87）：ENEMY → 敌池、BULLET → 弹池、其余 → 0（同 [`SYS_SELF_X`]）。
/// 存在的理由：笛卡尔没有单轴动词，"只插 vy 保住 vx"唯一的写法是把当前 vx 读出来填回去。
pub const SYS_SELF_VX: u16 = 87;
/// owner 的笛卡尔速度 y（88）——镜像 [`SYS_SELF_VX`]。
pub const SYS_SELF_VY: u16 = 88;
/// owner 的速率（89，作者视图）：与 [`SYS_SELF_VX`]/[`SYS_SELF_VY`] 恒同步（双表示）。
pub const SYS_SELF_SPEED: u16 = 89;
/// owner 的朝向（90，作者视图，BAM）。近停时冻结（`BACKFILL_MIN_SPEED`），故零速下
/// 读到的是**最后一次有效朝向**而非垃圾角。
pub const SYS_SELF_ANGLE: u16 = 90;
```

助手（放 `self_pos` 旁边，**结构逐条照抄它**）：

```rust
/// owner 的速度四件（`$self_vx`/`$self_vy`/`$self_speed`/`$self_angle` 共用）。
/// 派发规则逐条同 [`self_pos`]：ENEMY → 敌池、BULLET → 弹池、其余 → 全零。
/// 返回 `(vx, vy, speed, angle_raw)`，四个都已是可直接押栈的 raw。
fn self_vel(task: &Task, ctx: &VmCtx) -> (i32, i32, i32, i32) {
    match task.owner_kind {
        OWNER_ENEMY => {
            let i = task.owner_index as usize;
            (
                ctx.body.enemies.vx[i].raw(),
                ctx.body.enemies.vy[i].raw(),
                ctx.body.enemies.speed[i].raw(),
                ctx.body.enemies.angle[i].raw() as i32,
            )
        }
        OWNER_BULLET => {
            let i = task.owner_index as usize;
            (
                ctx.body.bullets.vx[i].raw(),
                ctx.body.bullets.vy[i].raw(),
                ctx.body.bullets.speed[i].raw(),
                ctx.body.bullets.angle[i].raw() as i32,
            )
        }
        _ => (0, 0, 0, 0),
    }
}
```

派发：

```rust
        SYS_SELF_VX => {
            let (vx, _, _, _) = self_vel(task, ctx);
            push(task, vx)
        }
        SYS_SELF_VY => {
            let (_, vy, _, _) = self_vel(task, ctx);
            push(task, vy)
        }
        SYS_SELF_SPEED => {
            let (_, _, sp, _) = self_vel(task, ctx);
            push(task, sp)
        }
        SYS_SELF_ANGLE => {
            let (_, _, _, a) = self_vel(task, ctx);
            push(task, a)
        }
```

- [ ] **Step 4：编译器侧接线（四处，缺一不可）**

**(1) `ast.rs:191`** —— `EngVar` 枚举尾部加：

```rust
    SelfVx,
    SelfVy,
    SelfSpeed,
    SelfAngle,
```

**(2) `parse.rs:50`** —— `resolve_engine_var` 白名单加：

```rust
        "self_vx" => Some(EngVar::SelfVx),
        "self_vy" => Some(EngVar::SelfVy),
        "self_speed" => Some(EngVar::SelfSpeed),
        "self_angle" => Some(EngVar::SelfAngle),
```

同时把该函数的文档注释 `（拍板 6，v1 固定 8 个）` 改成 `（v1 固定 8 个；敌人运动动词族刀
2026-07-31 扩到 12 个）`。

**(3) `parse.rs:732`** —— 那句错误提示把合法名字逐个列了出来，**必须同步**，否则报错信息
会漏报新名字：

```rust
                                 self_x/self_y/self_hp/self_hp_max/self_age/
                                 self_vx/self_vy/self_speed/self_angle）"
```

（确切拼法以该行现状为准，只在末尾追加四个名字。）

**(4) `builtins.rs:779`** —— `engine_var_info` 映射加：

```rust
        EngVar::SelfVx => (syscall::SYS_SELF_VX, Fx),
        EngVar::SelfVy => (syscall::SYS_SELF_VY, Fx),
        EngVar::SelfSpeed => (syscall::SYS_SELF_SPEED, Fx),
        EngVar::SelfAngle => (syscall::SYS_SELF_ANGLE, Angle),
```

⚠️ **`$self_angle` 的类型是 `Angle` 不是 `Fx`** ——它要能直接参与 `$self_angle + 15deg`
这样的角度运算。其余三个是 `Fx`。

- [ ] **Step 5：跑测试确认通过**

```bash
cargo test -p stg-core --lib ecl::syscall
cargo test -p stg-ecl-compiler
cargo run -p stg-harness -- gen-ecl-meta
```

- [ ] **Step 6：变异验证（白名单完整性）**

把 `parse.rs` 白名单里的 `"self_vx"` 那行**注释掉**，跑：

```bash
cargo test -p stg-ecl-compiler 2>&1 | grep -E "^test .* FAILED|test result"
```
Expected：`keeping_one_cartesian_axis_compiles` 转红。⚠️ 编辑撤回恢复。

- [ ] **Step 7：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
feat(ecl): 四个 $self_* 速度引擎变量（syscall 87-90，白名单 8→12）（T5）

$self_vx/$self_vy/$self_speed/$self_angle,派发逐条同 $self_x(ENEMY→敌池/
BULLET→弹池/其余→0)——弹池本就有这四个字段,故任务弹里天然有效。$self_angle
的类型是 Angle 不是 Fx,好让 `$self_angle + 15deg` 直接成立。

四个全要而非只 speed/angle:笛卡尔没有单轴动词,"只插 vy 保住 vx"唯一的写法是
move_vel_xy(30, $self_vx, 4.0fx, 2)。相对移动由此 composed,不做 Rel 一列。

parse.rs 的错误提示串同步(它把合法名字逐个列了出来,漏改会漏报新名字)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 6：e2e + 文档 + `ENGINE_VER` + 金向量基线

**Files:**
- Modify: `docs/ecl-lang.md`、`docs/ecl-ops.md`、`crates/stg-core/src/lib.rs:82`
- Modify: `PROGRESS.md`、`docs/follow-ups.md`
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（e2e 测试）

**Interfaces:**
- Consumes：前五个 Task 的全部产出。

- [ ] **Step 1：写 `.ecl` 源码级 e2e**

`crates/stg-ecl-compiler/src/lang/mod.rs` 的 `mod tests`，**真编译 + 真 VM 跑帧**
（照该文件里既有的 e2e 测试体例——上一刀的 `nearest_enemy → enemy_x/y → atan2 → fire`
那条是现成模板）：

```rust
/// e2e：spec §3.2 那个编排——杂兵被拉到点位、同时把速度缓动到朝下,落地继续飘走。
/// 位置那条 dur=4、速度那条 dur=2(**先到期**),故到点时 vel_active 已归 0——
/// 这正是黏滞位存在的理由,拿 vel_active 当判据这条 e2e 就红。
#[test]
fn e2e_enemy_lands_and_keeps_drifting() {
    // 脚本:move_to(4, ...) 后立刻 move_vel(2, 90deg, 3.0fx, 0),然后 wait 足够久
    // 跑 6 帧后断言:mv_active==0(已到点)、vy==3.0(速度没被清)、
    // y 比到点位置又多走了 2 帧 × 3.0
    // ——具体的世界构造/取帧方式照本文件既有 e2e 测试照抄
}
```

⚠️ 上面是**结构说明不是可提交代码**——实现者必须照本文件既有 e2e 的真实写法把它写成完整
可跑的测试（含脚本源码、编译、`start_main`、逐帧 `step`、最终断言）。**不得留空壳**。

- [ ] **Step 2：bump `ENGINE_VER`**

`crates/stg-core/src/lib.rs:82`：`9` → `10`。在其文档注释里按既有体例加一行理由：

```
/// 10（2026-07-31，敌人运动动词族刀）：敌池 +12 字段（speed/angle 双表示 + 速度插值器）
///    ⇒ **快照字节数与 SaveBytes 编码变**，旧存档/旧回放按新布局解读会走出另一条世界线，
///    必须拒载。这比号表新增（83-90 八个号）硬得多——后者单独不足以 bump。
```

- [ ] **Step 3：文档**

**`docs/ecl-lang.md`**：
- 运动一节补四条动词，**重点写清 `move_vel` 与 `move_vel_xy` 的插值空间不同**（这是最容易
  踩的一格）：极坐标匀速扫弧 / 笛卡尔直线穿过（线性缓动即恒定加速度）。
- 引擎变量表 8 → 12 行。
- 相对移动的 composed 写法给例（spec §4.3 的三行）。
- ⚠️ 围栏示例是**真编译**的（`every_ecl_fenced_example_in_doc_compiles` 押运），
  写完跑 `cargo test -p stg-harness`。
- ⚠️ 手写散文段**不受漂移测试保护**（只有 `<!-- gen -->` 块受保护）——改完通读一遍运动
  相关的手写段落，确认没有与新动词矛盾的旧话（上一刀在这里踩过：手写段说"别当长期句柄
  存着"而新段说"可以跨帧带着用"）。

**`docs/ecl-ops.md`**：八个新 syscall 号（4 动词 + 4 引擎变量——引擎变量在号表层同样是
syscall，见 `builtins::engine_var_info` 的映射）。

- [ ] **Step 4：金向量基线重新生成**

⚠️ **本刀金向量预期会变**（敌池布局变 ⇒ `rainbow.ecl` 的逐帧校验和流全变）。

```bash
cargo run -p stg-harness -- golden --out /tmp/golden-new.txt
```

用 `git worktree` 拉一份 base 副本跑出旧流做**对照**（不是判据）：

```bash
git worktree add /tmp/wt-base <本刀第一个 commit 的父>
cd /tmp/wt-base && cargo run -p stg-harness -- golden --out /tmp/golden-old.txt
diff -u /tmp/golden-old.txt /tmp/golden-new.txt | head -40
```

**验收判据不是"无差异"，而是**：
1. 差异**从某一帧开始**且此后持续（布局变化的表现），不是零星几帧跳变；
2. 若仓库有 committed 的金向量基线文件，按新值更新并在 commit 里说明；
3. 跨平台闸（`determinism-gate`）不受影响——它比的是三平台之间，不是与历史基线。

跑完 `git worktree remove /tmp/wt-base`。

- [ ] **Step 5：`PROGRESS.md` + `follow-ups.md`**

`PROGRESS.md`：史加一行（`date +%F`）+「现在」段重写（≤10 行，**保留 B26 余量那条**）。

`docs/follow-ups.md`：spec §4.4 的四条非目标逐条记为 follow-up——
高阶轨迹（`moveCircle`/`moveEllipse`/`moveBezier`/`moveCurve`）、敌人的连续效果
（`ang_vel`/`accel`，即弹的 `POLAR_FX`/`CART_FX`）、`moveEnm`/`moveRand`/`moveLimit`、
句柄版 `enemy_speed(e)`/`enemy_angle(e)`。

- [ ] **Step 6：全量收口**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- gen-ecl-meta
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
cargo test -p stg-harness
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

⚠️ 两个冒烟都要跑（`ENGINE_VER` 变了，桥面与真工程都要确认还能开机）。

- [ ] **Step 7：commit**

```bash
git add -A && git commit -F- <<'MSG'
feat(core): 敌人运动动词族收口——e2e + 文档 + ENGINE_VER 9→10（T6）

ENGINE_VER bump 的理由是**池布局**(快照字节数与 SaveBytes 编码变),不是号表新增
——后者单独不足以 bump。金向量基线**预期变化**并已重新生成:这是本刀与前几刀的
不同,"逐字节不变"这条验收判据这次用不了。

e2e 复刻 spec §3.2 的编排(拉到点位 + 速度缓到朝下 + 落地继续飘),位置 dur=4、
速度 dur=2 先到期——拿 vel_active 当判据这条 e2e 就红。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## 计划自查

**1. Spec 覆盖**

| spec 节 | 落在 |
|---|---|
| §3.1 双表示 | T1（字段 + 两个同步核） |
| §3.2 分层仲裁 | T3（① 恒跑 / ② 决定位置归谁） |
| §3.3 笛卡尔插值空间 | T3（`tick_enemy_vel` 的两条分支）+ 招牌判别式 |
| §4.1 四条动词 | T2（世界层）+ T4（syscall + 编译器） |
| §4.2 四个 `$self_*` | T5 |
| §4.3 不做 Rel | T5 的闭环测试 + T6 文档给 composed 例 |
| §4.4 非目标 | T6 记 follow-up |
| §5 数据模型 | T1 |
| §6.1 相位结构 | T3 |
| §6.2 数值纪律 | T3（绝对插值 / 终帧精确终值 / `dur==0` 退化） |
| §6.3 到点清速条件化 | T2（`vel_touched` 归零）+ T3（判据）+ 三条仲裁腿 |
| §7 测试 | T1/T2/T3/T4/T5 各自的测试步 + T6 e2e |
| §8 P4 降级 | T2（precheck 两条）+ T4（Fault 那条） |
| §9 兼容性 | T6（`ENGINE_VER` + 金向量） |
| §10 文档 | T6 |

无缺口。

**2. 占位符扫描**：T6 Step 1 的 e2e 是**唯一**的结构说明而非完整代码，已显式标注"不得留
空壳"并指到本仓现成模板（上一刀的 `nearest_enemy → atan2 → fire` e2e）。理由：该测试要
照抄的既有 e2e 写法跨三个文件，凭空写会造出与仓库不符的助手名。其余步骤均为可直接落地
的完整代码。

**3. 类型一致性**：`set_enemy_vel_polar(h, angle: Angle, speed: Fx, dur: u16, easing: u8)`
在 T2 定义、T4 调用，参数序一致；`VEL_SPACE_POLAR/CART` 在 T1 定义（`enemy.rs`，`pub`），
T2/T3 引用路径统一为 `crate::enemy::VEL_SPACE_*`；`refresh_enemy_vel_from_polar` /
`backfill_enemy_polar` 在 T1 定义（`pub(crate)`），T2/T3 引用。`$self_angle` 的 ECL 类型是
`Angle`、其余三个是 `Fx`（T5 Step 4 已显式标注）。

**4. 已知的实现期陷阱**（写进相应 Task）：
- T2 Step 3 的 `set_enemy_angle`/`set_enemy_speed` 若委托公开 setter 会**双重计数
  `contract_viol`**（计数参与校验和）——已在该步注明改法。
- T3 的 `Fx::ONE` 占位若被 clippy 嫌弃，把 `e` 的求值挪进 `else`。
- T4 的 `builtins.rs` 有**两处** exhaustive 名单。
- T5 的编译器侧接线有**四处**（枚举/白名单/错误提示串/映射），漏任一处都编译不过或行为不全。
