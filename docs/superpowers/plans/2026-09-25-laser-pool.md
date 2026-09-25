# 激光池 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引擎新增直线激光池（omega、挂靠、三态时序、旋转盒判定、清弹 field 取消），ECL 能发射和操作激光，Tier 0 填充 `lasers` 表，Godot 能画出来。

**Architecture:** 照弹池的写法：`define_pool!` 加 WorldBody 字段，相位 5 推进，相位 6 收集碰撞行 9/10，相位 7 结算，写 API 集中在 `world/laser.rs`。ECL 新开 8xx syscall 族，句柄沿用敌号的打包格式。Tier 0 和渲染只读 `view().lasers()`。

**Tech Stack:** Rust 1.94（workspace），PyO3 / maturin（wheel `stg_rl` 0.3.0），Godot 4（gdext）。

**Spec:** `docs/superpowers/specs/2026-09-25-laser-pool-design.md`。实施前先读，里面有拍板理由和原作依据（§2 的三组真实参数是测试数据来源）。

## Global Constraints

- 仓库 `/data/sunyunbo/www/stg-engine`，**直接在 `main` 上提交，不开分支，不 push**。硬规则见仓内 `CLAUDE.md`（I1–I7、P1–P6）：`stg-core` 里不得出现浮点，遍历按池索引升序，新字段自动进校验和。
- `ENGINE_VER` 23 → **24**，只在 Task 1 改一次，changelog 条目写进 `crates/stg-core/src/lib.rs` 的文档注释。
- 池：`Laser, cap = 256`；诊断下标 `POOL_LASER = 7`（`pool_full` 数组的最后一格）。
- syscall **800–809**（新族 8xx）。句柄打包格式同敌号：`((gen & 0x7FFF) << 16) | index`，无效为 -1。
- 判定半高 = `width / 2`，只在 state 1 判定。取消来源：带 `FIELD_CLEAR_BULLETS` 的 field 碰到激光线段。
- 时序口径（全计划统一）：相位 5 里**先按 `timer >= 时长` 判切换（切换时 `timer = 0`），再 `timer += 1`**。效果：预警恰好 `warn` 帧不判定，生效恰好 `active` 帧判定，`warn == 0` 出生帧就判定。
- 激光只在相位 5 回收（fade 结束、`start >= 640`、`active` 结束且 `fade == 0`）。相位 7 的取消只切状态，`fade == 0` 的激光在下一帧相位 5 回收。
- wheel `stg_rl` 0.2.0 → **0.3.0**（`crates/stg-py/Cargo.toml`、`crates/stg-py/pyproject.toml`）。proto 不改。
- 测试：`cd /data/sunyunbo/www/stg-engine && cargo test --workspace --exclude stg-godot`；`stg-godot` 只跑 `cargo check -p stg-godot`。提交前跑 `cargo fmt --all` 和 `cargo clippy --workspace --exclude stg-godot -- -D warnings`。
- 提交信息沿用仓内风格（中文，`type(scope): 摘要`），末尾加：
  `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`

**相对 spec 的两处简化**（实施时照这里做，Task 6 回写 spec）：
1. `laser()` 只取 `color`，不取 `sprite`。激光只有一种截面贴图，颜色就是全部外观；池字段 `sprite` 存颜色号 0..15。
2. 截面渐变先由 shader 按 UV 程序化生成，不往图集里补一行。美术补图作为后续待办。

## Review Focus

1. **挂靠的敌人死了、槽位被同帧新生的敌人复用**：代际不符，激光必须脱钩留在原地，不能跳到新敌人身上（Task 2）。
2. **同一帧里清弹 field 碰到激光、激光也碰到自机**：趟一先取消，趟二跳过，自机不死（Task 3）。
3. **激光已回收、槽位又被新激光复用后，拿旧句柄调 `lz_*`**：一律 no-op 加计数，不能改到新激光（Task 4）。
4. **原点在屏外很远、长度 640、自机靠近远端**：`seg_box_dist_sq` 不能溢出（Task 1）。
5. **时停期间**：激光不推进、不判定，`dx/dy/dang` 报 0（Task 2、Task 3）。

---

### Task 1: 判定原语 + 激光池骨架 + ENGINE_VER 24

**Files:**
- Modify: `crates/stg-core/src/math/geom.rs`（新函数 + 测试）
- Create: `crates/stg-core/src/lasers.rs`（池定义与常量）
- Modify: `crates/stg-core/src/lib.rs`（`pub mod lasers;`，`ENGINE_VER = 24` 与 changelog）
- Modify: `crates/stg-core/src/world.rs`（`POOL_LASER`、WorldBody 字段 `lasers`）
- Create: `crates/stg-core/src/world/laser.rs`（`create_laser`；Task 2/4 的写 API 也放这里）
- Modify: `crates/stg-core/src/world/view.rs`（`lasers()` 读口）
- Modify: `crates/stg-core/src/step.rs`（`copy_into` 加一行；`world_size_sentinel_guards_copy_into_field_list` 按实测更新）
- Modify: `stg-world-design.md`（D10 预算表加一行）

**Interfaces:**
- Produces: `pub fn seg_box_dist_sq(px: Fx, py: Fx, ox: Fx, oy: Fx, angle: Angle, start: Fx, end: Fx, half: Fx) -> i64`（`stg_core::math::geom`）
- Produces: `LaserPool / LaserHandle / LaserInit`（`stg_core::lasers`），常量 `LASER_WARN = 0, LASER_ACTIVE = 1, LASER_FADE = 2`、`LASER_CULL: Fx = Fx::from_int(640)`、`LASER_FLAG_FADE_ALPHA: u8 = 1`、`ANCHOR_NONE: u16 = 0xFFFF`
- Produces: `WorldBody::create_laser(&mut self, init: LaserInit) -> LaserHandle`；`WorldView::lasers(self) -> &'w LaserPool`；`world::POOL_LASER: usize = 7`

- [ ] **Step 1: 原语与判别式测试**

在 `geom.rs` 追加：

```rust
/// 点到「旋转线段盒」的平方距离（Q32.32，i64，不开根）。盒 = 从 (ox,oy) 沿 angle 的射线上
/// `[start, end]` 一段，横向半高 `half`。把点转进盒的局部系后钳位，求到钳位点的距离。
/// 激光判定（碰撞行 9/10）用：`seg_box_dist_sq(..) <= r.raw()²` 即相交。
/// 溢出：dx/dy 是屏幕坐标差（|·| < 32768 px 即不溢出 Fx），乘 cos/sin（|·| ≤ 1）走 Fx::mul 安全。
pub fn seg_box_dist_sq(
    px: Fx, py: Fx, ox: Fx, oy: Fx, angle: Angle, start: Fx, end: Fx, half: Fx,
) -> i64 {
    let (s, c) = crate::math::sincos(angle);
    let (dx, dy) = (px - ox, py - oy);
    let along = dx * c + dy * s;
    let perp = dy * c - dx * s;
    let qa = if along < start { start } else if along > end { end } else { along };
    let qp = if perp < -half { -half } else if perp > half { half } else { perp };
    len_sq(along - qa, perp - qp)
}
```

（`sincos` 的返回顺序、`Fx` 的比较与取负以 `math/trig.rs`、`math/fx.rs` 为准。）

测试用能区分错误实现的取值，每条注明它抓的是哪种错：

```rust
#[cfg(test)]
mod seg_box_tests {
    use super::*;
    const R: i64 = 65536; // 1 px 的 raw
    fn fx(v: i32) -> Fx { Fx::from_int(v) }
    fn d(px: i32, py: i32, a: u16, st: i32, en: i32, half: i32) -> i64 {
        seg_box_dist_sq(fx(px), fx(py), fx(0), fx(0), Angle(a), fx(st), fx(en), fx(half))
    }
    #[test] fn inside_box_is_zero() { assert_eq!(d(50, 3, 0, 0, 100, 4), 0); }
    // 半高 4：点在 y=6 → 距 2 px。若误用 width/4 或 half*2，结果不是 4 px²。
    #[test] fn perp_distance_uses_half() { assert_eq!(d(50, 6, 0, 0, 100, 4), 4 * R * R); }
    // 近端留空 start=64：点在 x=10 → 到 x=64 距 54。若钳位下界误写成 0，结果是 0。
    #[test] fn clamps_to_start_not_zero() { assert_eq!(d(10, 0, 0, 64, 500, 4), 54 * 54 * R * R); }
    #[test] fn beyond_end() { assert_eq!(d(110, 0, 0, 0, 100, 4), 100 * R * R); }
    // angle = 90°（BAM 16384，指向 +y，屏幕向下）：点 (0,50) 在盒内，点 (50,0) 在侧面 46 px 外。
    // along/perp 写反会让前两条互换。点 (0,−50) 在原点**后方** 50 px：sin 符号写反时它会被算进盒内（得 0）。
    #[test] fn rotated_quarter_turn() {
        assert_eq!(d(0, 50, 16384, 0, 100, 4), 0);
        assert_eq!(d(50, 0, 16384, 0, 100, 4), 46 * 46 * R * R);
        assert_eq!(d(0, -50, 16384, 0, 100, 4), 50 * 50 * R * R);
    }
    // Review Focus 4：原点在屏外很远、长 640，点在远端附近，结果精确且不溢出。
    #[test] fn far_origin_no_overflow() {
        let v = seg_box_dist_sq(fx(0), fx(440), fx(0), fx(-200), Angle(16384), fx(0), fx(640), fx(3));
        assert_eq!(v, 0);
        let v = seg_box_dist_sq(fx(700), fx(440), fx(0), fx(-200), Angle(16384), fx(0), fx(640), fx(3));
        assert_eq!(v, 697 * 697 * R * R);
    }
}
```

先跑 `cargo test -p stg-core seg_box` 确认红（函数不存在），实现后转绿。**变异检验**：临时把 `perp` 的钳位改成 `±half*2`、把 `start` 的钳位改成 0、把 `perp` 的符号取反，逐个确认至少一条测试会红，然后还原。

- [ ] **Step 2: 池定义**

`crates/stg-core/src/lasers.rs`：

```rust
//! 激光池（spec 2026-09-25-laser-pool-design）——直线激光：射线原点 + 方向 + 射线上 [start, end] 一段。
//! 每帧 `end += speed; start = max(start, end − start_len, 0)`，`angle += omega`；三态 0 预警 / 1 生效 / 2 收缩。
//! 判定半高 = width/2（画多宽判多宽），只在 state 1 判。原作口径与转写换算见 spec §2、§9.1。

use crate::define_pool;
use crate::math::{Angle, Fx};

pub const LASER_WARN: u8 = 0;
pub const LASER_ACTIVE: u8 = 1;
pub const LASER_FADE: u8 = 2;
/// `start` 越过它即回收（原作 640.0，整条已出屏）。
pub const LASER_CULL: Fx = Fx::from_int(640);
/// `flags` 位 0：收缩态改为 alpha 淡出（纯表现；0 = 变窄）。
pub const LASER_FLAG_FADE_ALPHA: u8 = 1 << 0;
/// `anchor_idx` 取这个值表示没有挂靠。
pub const ANCHOR_NONE: u16 = 0xFFFF;

define_pool! {
    Laser, cap = 256,
    fields {
        ox: Fx, oy: Fx, angle: Angle, omega: i16,
        start: Fx, end: Fx, start_len: Fx, speed: Fx,
        width: Fx, sprite: u16,
        warn: u16, active: u16, fade: u16, timer: u16, state: u8,
        anchor_idx: u16, anchor_gen: u16, ax: Fx, ay: Fx,
        // 观测：本帧相位 5 结束时相对上一帧同一时刻的变化（含 ECL rotate/aim/origin、omega、挂靠）。
        dx: Fx, dy: Fx, dang: i16,
        px: Fx, py: Fx, pang: Angle,
        flags: u8, born_frame: u32
    }
}
```

宏生成的 `LaserInit` 是 exhaustive 的：照 `bullets.rs` 的测试，加一条全覆写单测（写满非零值再 alloc，逐字段读回）。

- [ ] **Step 3: 接进 WorldBody**

- `world.rs`：`pub const POOL_LASER: usize = 7;`；WorldBody 在 `items` 后加 `pub(crate) lasers: crate::lasers::LaserPool,`。
- `step.rs::copy_into`：`s.lasers.copy_into(&mut d.lasers);`。运行 `world_size_sentinel_guards_copy_into_field_list`，按实测值更新哨兵（注释写明「激光池 +N B」）。
- `view.rs`：`pub fn lasers(self) -> &'w LaserPool { &self.body.lasers }`。
- `world/laser.rs`（在 `world.rs` 里声明 `mod laser;`）：

```rust
//! 激光写 API（P1：调用方只走这里，不直接碰池内存）。
use super::{WorldBody, POOL_LASER, STATUS_POOL_FULL};
use crate::lasers::*;
use crate::math::Fx;

impl WorldBody {
    /// 建一条激光。P4-a：池满 → NULL + 计数。P4-b：width/start/end/start_len/speed 为负 → 钳到 0 并计一次 contract_viol。
    /// 调用方给几何、外观、时长和 omega，其余字段由本函数定：state 按 warn 是否为 0、timer 0、不挂靠、
    /// 观测字段清零、px/py/pang = 初值、born_frame = 当前帧。
    pub fn create_laser(&mut self, mut init: LaserInit) -> LaserHandle {
        let mut bad = false;
        for v in [&mut init.width, &mut init.start, &mut init.end, &mut init.start_len, &mut init.speed] {
            if *v < Fx::ZERO { *v = Fx::ZERO; bad = true; }
        }
        if bad { self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1); }
        init.state = if init.warn == 0 { LASER_ACTIVE } else { LASER_WARN };
        init.timer = 0;
        init.anchor_idx = ANCHOR_NONE;
        init.anchor_gen = 0;
        init.dx = Fx::ZERO; init.dy = Fx::ZERO; init.dang = 0;
        init.px = init.ox; init.py = init.oy; init.pang = init.angle;
        init.born_frame = self.frame;
        match self.lasers.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_LASER] = self.diag.pool_full[POOL_LASER].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                LaserHandle::NULL
            }
        }
    }
}
```

测试（放 `world/laser.rs` 的 `mod tests`）：`warn == 0` 出生即 state 1；池满第 257 条返回 NULL 且 `pool_full[POOL_LASER] == 1`；负宽度被钳到 0 且 `contract_viol` 加 1；快照往返后校验和相等（照 `step.rs` 里已有的快照测试写法）。

- [ ] **Step 4: 版本与文档，提交**

`lib.rs`：`ENGINE_VER = 24`，changelog 写「**23 → 24**（激光池刀，2026-09-25，spec `2026-09-25-laser-pool-design.md`）：新增 `LaserPool`（cap 256）、相位 5 推进、碰撞行 9/10、syscall 族 8xx；校验和与存档载荷均变化，旧回放失效」。
`stg-world-design.md` D10 表加一行「激光池 | 256 | 实测 B | 实测小计」（按哨兵测出的增量填）。

全量测试绿后提交：`feat(core): 激光池骨架 + seg_box_dist_sq 判定原语；ENGINE_VER 24`。

---

### Task 2: 相位 5 推进（几何、时序、omega、挂靠、观测）

**Files:**
- Modify: `crates/stg-core/src/world/integrate.rs`（新趟 `integrate_lasers`，时停分支清零观测）
- Modify: `crates/stg-core/src/world/laser.rs`（写 API：speed / start / omega / rotate / aim / anchor / origin / cancel / alive）
- Test: `crates/stg-core/src/world/laser.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 1 的池、常量与 `create_laser`
- Produces（全部在 `impl WorldBody`，返回 `bool` = 句柄是否有效；无效时 no-op 并计一次 `diag.contract_viol`）：
  `laser_set_speed(h, speed: Fx, start_len: Fx)`（同时 `end = start`，从近端长出去）、`laser_set_start(h, s: Fx)`、
  `laser_set_omega(h, omega: i16)`、`laser_rotate(h, a: Angle)`、`laser_aim(h, off: Angle)`（指向 `players[0]`）、
  `laser_anchor(h, e: EnemyHandle, ax: Fx, ay: Fx)`（`e` 为 NULL 表示解除）、`laser_origin(h, x: Fx, y: Fx)`（同时解除挂靠）、
  `laser_cancel(h)`（state < 2 → 2、timer 0）；`laser_alive(&self, h) -> bool`（不计数）
- Produces: `WorldBody::cancel_laser_index(&mut self, i: usize)`（`pub(crate)`，Task 3 的行 10 用）

- [ ] **Step 1: 逐帧对拍测试（先红）**

用 spec §2 的三组原作参数（宽度按裁定 ④ 减半），直接调 `create_laser` 加写 API，再逐帧 `step_t`。每帧断言 `(state, start, end)`；「是否判定」等价于 state == 1。

```rust
use crate::input::InputFrame;
use crate::lasers::*;
use crate::math::{Angle, Fx};

fn step(w: &mut crate::step::World, f: u32) {
    crate::world::test_support::step_t(w, &InputFrame::empty(f));
}
/// 原点 (0,100)，朝下（BAM 16384），形态一：start 0、end = start_len = len、speed 0。
fn laser_init(warn: u16, active: u16, fade: u16, len: i32, width: i32) -> LaserInit {
    LaserInit {
        ox: Fx::ZERO, oy: Fx::from_int(100), angle: Angle(16384), omega: 0,
        start: Fx::ZERO, end: Fx::from_int(len), start_len: Fx::from_int(len), speed: Fx::ZERO,
        width: Fx::from_int(width), sprite: 0, warn, active, fade, timer: 0, state: 0,
        anchor_idx: ANCHOR_NONE, anchor_gen: 0, ax: Fx::ZERO, ay: Fx::ZERO,
        dx: Fx::ZERO, dy: Fx::ZERO, dang: 0, px: Fx::ZERO, py: Fx::ZERO, pang: Angle(0),
        flags: 0, born_frame: 0,
    }
}
/// 逐帧跑 `frames` 步，断言每步之后的 state，返回下一个帧号。
fn run_state(w: &mut crate::step::World, i: usize, f0: u32, frames: u32, want: u8) -> u32 {
    for f in f0..f0 + frames {
        step(w, f);
        assert_eq!(w.body.lasers.state[i], want, "帧 {f}");
    }
    f0 + frames
}

/// s1 Sub12（宽 32 → 16）：预警 30、生效 120、收缩 16，speed 0，end 恒为 500。
#[test]
fn sub12_warn_then_active_then_fade_frame_exact() {
    let mut w = crate::step::World::new(1);
    let h = w.body.create_laser(laser_init(30, 120, 16, 500, 16));
    let i = h.index as usize;
    let f = run_state(&mut w, i, 0, 30, LASER_WARN);
    let f = run_state(&mut w, i, f, 120, LASER_ACTIVE);
    let f = run_state(&mut w, i, f, 16, LASER_FADE);
    assert_eq!((w.body.lasers.start[i], w.body.lasers.end[i]), (Fx::ZERO, Fx::from_int(500)));
    step(&mut w, f);
    assert!(!w.body.laser_alive(h), "收缩 16 帧后回收");
}

/// s1 Sub22（宽 16 → 8）：预警 120、生效 60、收缩 16。
#[test]
fn sub22_long_warning() {
    let mut w = crate::step::World::new(1);
    let h = w.body.create_laser(laser_init(120, 60, 16, 500, 8));
    let i = h.index as usize;
    let f = run_state(&mut w, i, 0, 120, LASER_WARN);
    let f = run_state(&mut w, i, f, 60, LASER_ACTIVE);
    run_state(&mut w, i, f, 16, LASER_FADE);
}

/// s2 Sub27（宽 6 → 3）：warn 0 出生即生效；speed 4、start_len 192。
/// 第 f 步后 end = 4(f+1)，start = max(0, end − 192)；start 到 640（end 832）的那一步回收，即 f = 207。
#[test]
fn sub27_sliding_bar_until_cull() {
    let mut w = crate::step::World::new(1);
    let h = w.body.create_laser(laser_init(0, 9999, 30, 0, 3));
    assert!(w.body.laser_set_speed(h, Fx::from_int(4), Fx::from_int(192)));
    let i = h.index as usize;
    assert_eq!(w.body.lasers.state[i], LASER_ACTIVE, "warn 0 出生即生效");
    for f in 0..207u32 {
        step(&mut w, f);
        let end = 4 * (f as i32 + 1);
        assert_eq!(w.body.lasers.end[i], Fx::from_int(end), "帧 {f}");
        assert_eq!(w.body.lasers.start[i], Fx::from_int((end - 192).max(0)), "帧 {f}");
        assert_eq!(w.body.lasers.state[i], LASER_ACTIVE, "帧 {f}");
    }
    step(&mut w, 207);
    assert!(!w.body.laser_alive(h), "start 到 640 回收");
}
```

`InputFrame::empty`、`World::new` 的确切签名以现有测试（如 `world/transform.rs` 的 `mod tests`）为准。

另外几条（同一个 `mod tests`）：
- `omega_and_rotate_both_land_in_dang`：`omega = 100`，某帧在相位 2 之前调一次 `laser_rotate(h, Angle(1000))`，那帧 `dang == 1100`，其余帧 `dang == 100`。
- `anchor_follows_enemy_then_detaches_on_death`：挂到一个 `vx = 2` 的敌人上，偏移 (0, 8)；每帧 `ox == 敌 x`、`oy == 敌 y + 8`、`dx == 2`。`kill_enemy_by_handle` 后的下一帧 `anchor_idx == ANCHOR_NONE`、原点不动、`dx == 0`。
- **Review Focus 1** `anchor_does_not_jump_to_recycled_slot`：敌人死亡回收后，同帧新建一个敌人占用同一个下标（`create_enemy` 取最低空位，必然复用），激光必须脱钩，原点不能跳到新敌人位置。
- **Review Focus 5** `frozen_scene_does_not_advance`：照 `integrate.rs` 里敌人 dx 时停测试的写法冻结场景；期间 `end/timer/state` 不变，`dx/dy/dang == 0`。
- `stale_handle_setters_are_noop_and_counted`：回收后调每个 setter 都返回 false，`contract_viol` 每次加 1。

- [ ] **Step 2: 实现 `integrate_lasers`**

在 `integrate()` 的 `if !scene { self.integrate_enemies(); ... }` 里，**紧跟 `integrate_enemies()` 之后**调 `self.integrate_lasers();`。时停的 `else` 分支里，照敌人的写法把存活激光的 `dx/dy/dang` 清零（不动 `px/py/pang`）。

```rust
/// 相位 5 · 激光（在敌人之后：挂靠读敌人本帧新位置）。唯一的回收点。
fn integrate_lasers(&mut self) {
    use crate::lasers::*;
    let nw = self.lasers.alive.len();
    for w in 0..nw {
        let mut bits = self.lasers.alive[w];
        while bits != 0 {
            let i = w * 64 + bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let l = &mut self.lasers;
            // 挂靠：代际相符才跟，否则脱钩（原点留在原地）
            if l.anchor_idx[i] != ANCHOR_NONE {
                let e = l.anchor_idx[i] as usize;
                if self.enemies.is_alive(e) && self.enemies.generation[e] == l.anchor_gen[i] {
                    l.ox[i] = self.enemies.x[e] + l.ax[i];
                    l.oy[i] = self.enemies.y[e] + l.ay[i];
                } else {
                    l.anchor_idx[i] = ANCHOR_NONE;
                }
            }
            l.angle[i] = l.angle[i].add_delta(l.omega[i]);
            l.end[i] = l.end[i] + l.speed[i];
            if l.end[i] - l.start[i] > l.start_len[i] { l.start[i] = l.end[i] - l.start_len[i]; }
            if l.start[i] < Fx::ZERO { l.start[i] = Fx::ZERO; }
            // 时序：先判切换再 +1（Global Constraints 的口径）
            let mut dead = false;
            match l.state[i] {
                LASER_WARN if l.timer[i] >= l.warn[i] => { l.state[i] = LASER_ACTIVE; l.timer[i] = 0; }
                LASER_ACTIVE if l.timer[i] >= l.active[i] => {
                    if l.fade[i] == 0 { dead = true; } else { l.state[i] = LASER_FADE; l.timer[i] = 0; }
                }
                LASER_FADE if l.timer[i] >= l.fade[i] => dead = true,
                _ => {}
            }
            if l.start[i] >= LASER_CULL { dead = true; }
            if dead { l.free_index(i); continue; }
            l.timer[i] = l.timer[i].saturating_add(1);
            l.dx[i] = l.ox[i] - l.px[i];
            l.dy[i] = l.oy[i] - l.py[i];
            l.dang[i] = l.angle[i].raw().wrapping_sub(l.pang[i].raw()) as i16;
            l.px[i] = l.ox[i]; l.py[i] = l.oy[i]; l.pang[i] = l.angle[i];
        }
    }
}
```

（借用：`self.enemies` 和 `self.lasers` 是 WorldBody 的两个字段，按字段分别借用即可；`add_delta` 和 `free_index` 的确切签名以 `angle.rs`、宏生成的代码为准。）

写 API 放在 `world/laser.rs`，统一写法是先 `get(h)` 取下标，失败就计数返回 false：

```rust
fn laser_slot(&mut self, h: LaserHandle) -> Option<usize> {
    if self.lasers.get(h).is_some() { Some(h.index as usize) } else {
        self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        None
    }
}
pub fn laser_rotate(&mut self, h: LaserHandle, a: Angle) -> bool {
    let Some(i) = self.laser_slot(h) else { return false };
    self.lasers.angle[i] = self.lasers.angle[i].add(a);
    true
}
pub fn laser_aim(&mut self, h: LaserHandle, off: Angle) -> bool {
    let Some(i) = self.laser_slot(h) else { return false };
    let p = &self.players[0];
    let to = crate::math::cordic::atan2(p.y - self.lasers.oy[i], p.x - self.lasers.ox[i]);
    self.lasers.angle[i] = to.add(off);
    true
}
pub(crate) fn cancel_laser_index(&mut self, i: usize) {
    if self.lasers.state[i] < crate::lasers::LASER_FADE {
        self.lasers.state[i] = crate::lasers::LASER_FADE;
        self.lasers.timer[i] = 0;
    }
}
```

其余 setter 照这个样子写（`laser_set_speed` 同时写 `speed`、`start_len`，并令 `end = start`；负值钳 0 并计数）。`atan2` 的确切路径以 `math/cordic.rs` 为准。`laser_alive` 只读，不计数。

- [ ] **Step 3: 全绿，提交** `feat(core): 激光相位 5 推进——三态时序、omega、挂靠跟随/脱钩、dx/dy/dang 观测`

---

### Task 3: 碰撞行 9/10 与结算

**Files:**
- Modify: `crates/stg-core/src/events.rs`（`ROW_LASER_PLAYER_HIT = 9`、`ROW_FIELD_LASER = 10`）
- Modify: `crates/stg-core/src/world/collide.rs`（两个收集函数，接在行 7 之后）
- Modify: `crates/stg-core/src/world/settle.rs`（行 10 在趟一，行 9 在趟二）
- Modify: `stg-world-design.md`（D8 矩阵加两行）

**Interfaces:**
- Consumes: `seg_box_dist_sq`（Task 1）、`cancel_laser_index`（Task 2）
- Produces: 两个 `ROW_*` 常量；行 9 命中调用现有的 `trigger_player_hit`

- [ ] **Step 1: 测试（先红）**，放 `settle.rs` 的 `mod tests`：
- `laser_kills_only_when_active`：自机放在一条 warn 30 的激光正中。前 30 帧不死，第 30 帧那一步进入死亡窗（断言沿用该模块判断自机中弹的现有写法）。
- `laser_hit_respects_invuln`：`invuln > 0` 时不判。
- `laser_width_is_hit_width`：自机半径 r，激光 width 16。自机中心放在离轴 8 + r − 1 px 处会死，放在 8 + r + 1 px 处不死。若误用 width/4，第一条就会失败。
- `fullscreen_field_cancels_all_lasers`：用 `fullscreen_clear_field()` 的等价 field（照 `world.rs:1782` 的 `spawn_field` 写法）。当帧所有 state < 2 的激光都变成 state 2；`fade == 0` 的激光在下一帧回收。
- `local_field_cancels_only_touched`：两条平行激光，小圆 field 只碰到其中一条。
- **Review Focus 2** `field_cancel_same_frame_saves_player`：同一帧里 field 碰到激光、激光也罩住自机，自机不死。
- **Review Focus 5** `frozen_scene_laser_does_not_kill`：时停期间自机站在生效的激光上不死。

- [ ] **Step 2: 实现**

`collide()` 里接在 `collide_field_enemy()` 之后（时停分支已经提前 return，不需要额外处理）：

```rust
self.collide_lasers_player(); // 行 9：激光 × 自机（仅 state 1）
self.collide_field_laser();   // 行 10：清弹 field × 激光（state 0/1）
```

```rust
/// 激光 i 到点 (x,y) 的判定平方距离（半高 width/2）。行 9/10 共用。
fn laser_dist_sq(&self, i: usize, x: Fx, y: Fx) -> i64 {
    let l = &self.lasers;
    let half = Fx::from_raw(l.width[i].raw() / 2);
    seg_box_dist_sq(x, y, l.ox[i], l.oy[i], l.angle[i], l.start[i], l.end[i], half)
}

/// 行 9：激光（仅 state 1）× 自机 hit_radius。门禁同行 1。
/// 借用：每轮只拷出局部值，不持有对池的长借用，这样 push_hit(&mut self) 能编过（同行 1 写法）。
fn collide_lasers_player(&mut self) {
    for p in 0..crate::MAX_PLAYERS {
        if self.players[p].life_state != crate::player::LIFE_ALIVE || self.players[p].invuln != 0 {
            continue;
        }
        let (px, py) = (self.players[p].x, self.players[p].y);
        let r = self.players[p].hit_radius.raw() as i64;
        let nw = self.lasers.alive.len();
        for w in 0..nw {
            let mut bits = self.lasers.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.lasers.state[i] != crate::lasers::LASER_ACTIVE {
                    continue;
                }
                if self.laser_dist_sq(i, px, py) <= r * r {
                    self.push_hit(ROW_LASER_PLAYER_HIT, i as u16, p as u16);
                }
            }
        }
    }
}

/// 行 10：清弹 field（`FIELD_CLEAR_BULLETS`）× 激光（state 0/1）——field 圆碰到线段即命中。
fn collide_field_laser(&mut self) {
    let nf = self.fields.alive.len();
    for fw in 0..nf {
        let mut fbits = self.fields.alive[fw];
        while fbits != 0 {
            let f = fw * 64 + fbits.trailing_zeros() as usize;
            fbits &= fbits - 1;
            if self.fields.flags[f] & FIELD_CLEAR_BULLETS == 0 {
                continue;
            }
            let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
            let r = self.fields.radius[f].raw() as i64;
            let nw = self.lasers.alive.len();
            for w in 0..nw {
                let mut bits = self.lasers.alive[w];
                while bits != 0 {
                    let i = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.lasers.state[i] >= crate::lasers::LASER_FADE {
                        continue;
                    }
                    if self.laser_dist_sq(i, fx, fy) <= r * r {
                        self.push_hit(ROW_FIELD_LASER, f as u16, i as u16);
                    }
                }
            }
        }
    }
}
```

（FieldPool 的 `alive` 位图字数和遍历写法以行 6 `collide_field_bullet` 为准，照抄它。）

`settle.rs`：
- 趟一（行 6 循环之后）：`ROW_FIELD_LASER` 命中调 `self.cancel_laser_index(h.passive as usize)`，本身幂等，不发事件。
- 趟二的 `match` 加一支：

```rust
crate::events::ROW_LASER_PLAYER_HIT => {
    let i = h.active as usize;
    if !self.lasers.is_alive(i) || self.lasers.state[i] != crate::lasers::LASER_ACTIVE {
        continue; // 趟一被 field 取消的激光不杀人
    }
    self.trigger_player_hit(h.passive as usize);
}
```

`stg-world-design.md` D8 表加行 9、10（列同 spec §4.2），模块文档注释里的行号清单也同步更新。

- [ ] **Step 3: 全绿，提交** `feat(core): 碰撞行 9（激光 × 自机，仅生效态）与行 10（清弹 field 取消激光）`

---

### Task 4: ECL 8xx 族 + 手册

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（常量 800–809、派发、handler、`pack_laser_handle` / `resolve_laser_handle`）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（10 个 Builtin）
- Modify: `docs/ecl-ops.md`（8xx 族表）
- Create: `docs/ecl-lang/9-lasers.md`（教学篇）；Modify: `docs/ecl-lang.md`（索引加一行）
- 重跑：`cargo run -p stg-harness -- gen-ecl-meta`（刷新 `7-reference.md` 的生成段与 `ecl-meta.json`）

**Interfaces:**
- Consumes: Task 1/2 的 `create_laser` 和全部写 API
- Produces（表层，参数正序）：
  `laser(color: int, x: fx, y: fx, angle: angle, len: fx, width: fx, warn: int, active: int, fade: int) -> int`（800）、
  `lz_speed(lz, speed: fx, start_len: fx)`（801）、`lz_start(lz, s: fx)`（802）、`lz_omega(lz, a: angle)`（803）、
  `lz_rotate(lz, a: angle)`（804）、`lz_aim(lz, off: angle)`（805）、`lz_anchor(lz, enemy: int, ox: fx, oy: fx)`（806）、
  `lz_origin(lz, x: fx, y: fx)`（807）、`lz_cancel(lz)`（808）、`lz_alive(lz) -> int`（809）

- [ ] **Step 1: 测试（先红）**：照 `ecl/syscall.rs` 现有 syscall 测试的写法，用小段 `.ecl` 编译运行：
- `laser()` 返回的句柄解码后指向一条字段正确的激光：`start = 0`、`end = len`、`start_len = len`、`speed = 0`、`sprite = color`、`omega = 0`。
- `color` 不在 0..15 → Fault（`FAULT_BAD_OP`）；池满 → 返回 -1，不 Fault。
- `warn/active/fade` 超出 0..=65535 → 钳位并计 `contract_viol`。
- `lz_omega` 的角度超出 i16 → 钳位并计数。
- **Review Focus 3** `stale_laser_handle_does_not_touch_reused_slot`：激光 A 回收，新激光 B 复用同一槽；拿 A 的句柄调 `lz_rotate`，B 的 `angle` 不变，`contract_viol` 加 1，`lz_alive(A) == 0`。
- `lz_anchor($self_enemy, 0, 8)` 之后激光跟着敌人走；`lz_anchor(lz, -1, 0, 0)` 解除挂靠。

- [ ] **Step 2: 实现**

句柄编解码放在 `syscall.rs`，和 `resolve_enemy_handle` 相邻：

```rust
fn pack_laser_handle(h: LaserHandle) -> i32 {
    if h == LaserHandle::NULL { -1 } else { (((h.generation & 0x7FFF) as i32) << 16) | h.index as i32 }
}
fn resolve_laser_handle(packed: i32, ctx: &VmCtx) -> Option<LaserHandle> {
    if packed < 0 { return None; }
    let idx = (packed & 0xFFFF) as usize;
    let g = ((packed >> 16) & 0x7FFF) as u16;
    (idx < LaserPool::CAP && ctx.body.lasers.is_alive(idx) && (ctx.body.lasers.generation[idx] & 0x7FFF) == g)
        .then(|| LaserHandle { index: idx as u16, generation: ctx.body.lasers.generation[idx] })
}
```

`resolve` 失败时，setter 仍要调用写 API 让它计数：传 `LaserHandle::NULL`，由写 API 负责计数。这样计数只发生在一处。

handler 按参数逆序 `pop`，样例：

```rust
fn sys_laser_rotate(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_rotate(h, bam(a));
    Ok(())
}
```

`sys_laser_create` 依次弹出 `fade, active, warn, width, len, angle, y, x, color`，校验 color，组装 `LaserInit`（`start = 0, end = len, start_len = len, speed = 0, omega = 0, flags = 0`，其余交给 `create_laser`），然后压入 `pack_laser_handle(h)`。`lz_alive` 压入 0 或 1。

builtins 照 `fire` 的条目写法加 10 条，每条都要有 `doc` 和 `param_names`（单测 `all_builtins_have_doc_and_matching_param_names` 会检查）。有返回值的是 `laser` 和 `lz_alive`（`Some(Int)`）。

- [ ] **Step 3: 手册**：`docs/ecl-lang/9-lasers.md`，按 spec §2 的三种形态各给一个完整可编译的例子，分别是「预警线 → 扫射（omega）」「自机狙（`aim_player() + a`）」「飞出去的棒子（`lz_speed`）」，再给一个挂靠例子。另写一节「和原作对照」：宽度是判定宽度（转写时减半）；`laser_index` / `laser_clear_all` 不需要；每 N 帧转一次改写成 omega。`docs/ecl-ops.md` 8xx 族表逐条登记号、参数序和降级口径。跑 `gen-ecl-meta`，再跑 `cargo test -p stg-harness`（手册里的 ```ecl 代码块会被真编译）。

- [ ] **Step 4: 全绿，提交** `feat(ecl): 激光 syscall 族 8xx（laser / lz_*）+ 手册第 9 篇`

---

### Task 5: Tier 0 `lasers` 表 + wheel 0.3.0 + 金向量场景

**Files:**
- Modify: `crates/stg-rl/src/encode.rs`（`write_lasers`）
- Modify: `crates/stg-rl/src/vec_env.rs`（写行处调用；去掉 `compact` 里的 `lasers_count.fill(0)`，改为照 enemies 的方式按 env 写入计数；暂存区照 enemies 的做法扩展）
- Modify: `crates/stg-rl/tests/vec_env.rs`（`lasers_count == 0` 的断言改为有激光的场景断言）
- Modify: `crates/stg-py/Cargo.toml`、`crates/stg-py/pyproject.toml`（0.3.0）
- Modify: `crates/stg-harness/src/main.rs`（`cmd_golden` 加场景 3：激光）

**Interfaces:**
- Consumes: `view().lasers()`、`seg_box_dist_sq`
- Produces: `pub fn write_lasers(w: &World, rows: &mut [u8]) -> usize`（`stg_rl::encode`）

- [ ] **Step 1: 测试（先红）**：`encode.rs` 的测试模块里建一个带 3 条激光的世界（每种形态一条，其中一条挂 omega），`step` 几帧后调用 `write_lasers`，逐列断言：
  - `x, y` = `ox, oy`；`angle` 为 BAM 原值；`start, end, start_len, speed` 原样；`half_h == width/2`。
  - `omega == (dang as i64 * 411775) >> 16`（BAM/帧换算成弧度/帧的 Q16.16；411775 ≈ 2π·65536）。
  - `vx, vy` = `dx, dy`；state 0 时 `t_active == warn − timer`，否则为 0；`state`；`type == 0`。
  - 条数上限：建 70 条，只写出 64 条，且都是离自机最近的（平局按下标）；重复调用输出逐字节相同。

- [ ] **Step 2: 实现**

```rust
pub fn write_lasers(w: &World, rows: &mut [u8]) -> usize {
    let v = w.view();
    let l = v.lasers();
    let p = &v.players()[0];
    let st = crate::layout::LASERS.stride;
    // ≤ 256 条：定长暂存 + 排序，不做堆分配。键 (平方距离, 下标) 保证确定性。
    let mut keys = [(0i64, 0u16); stg_core::lasers::LaserPool::CAP];
    let mut n = 0;
    for i in l.iter_alive() {
        let half = Fx::from_raw(l.width()[i].raw() / 2);
        keys[n] = (seg_box_dist_sq(p.x, p.y, l.ox()[i], l.oy()[i], l.angle()[i], l.start()[i], l.end()[i], half), i as u16);
        n += 1;
    }
    let keys = &mut keys[..n];
    keys.sort_unstable();
    let k = n.min(LASERS_CAP);
    for (row, &(_, i)) in keys[..k].iter().enumerate() {
        let i = i as usize;
        let r = &mut rows[row * st..(row + 1) * st];
        r.fill(0);
        put_i32(r, off::laser::X, l.ox()[i].raw());
        put_i32(r, off::laser::Y, l.oy()[i].raw());
        put_u16(r, off::laser::ANGLE, l.angle()[i].raw());
        put_i32(r, off::laser::START, l.start()[i].raw());
        put_i32(r, off::laser::END, l.end()[i].raw());
        put_i32(r, off::laser::START_LEN, l.start_len()[i].raw());
        put_i32(r, off::laser::SPEED, l.speed()[i].raw());
        put_i32(r, off::laser::HALF_H, l.width()[i].raw() / 2);
        put_i32(r, off::laser::OMEGA, ((l.dang()[i] as i64 * 411_775) >> 16) as i32);
        put_i32(r, off::laser::VX, l.dx()[i].raw());
        put_i32(r, off::laser::VY, l.dy()[i].raw());
        let t_active = if l.state()[i] == stg_core::lasers::LASER_WARN {
            l.warn()[i] as i32 - l.timer()[i] as i32
        } else { 0 };
        put_i32(r, off::laser::T_ACTIVE, t_active);
        r[off::laser::STATE] = l.state()[i];
    }
    k
}
```

（池访问器是宏生成的切片读口，写法同 `write_enemies`。`seg_box_dist_sq` 在 stg-rl 里通过 `stg_core::math::geom` 引用。若 proto 规定 `angle` 列不是 BAM，按 `layout.rs` 里该列的类型换算。）

`vec_env.rs`：lasers 和 enemies 一样，每个 env 占定长的 64 行，直接写进该 env 的行区，不走 bullets/items 那套 CSR 压实。
在写行处（`write_enemies` 旁边）加 `*lasers_count = encode::write_lasers(w, lasers_rows) as i32;`，
`lasers_rows` / `lasers_count` 的切片按 enemies 在 `Slot` 与 `BufferSet` 之间传递的方式照抄。去掉 `compact` 里的 `buf.lasers_count.fill(0)`。

- [ ] **Step 3: 金向量场景 3**：在 `cmd_golden` 末尾加一段激光场景，只用 `World` 的公开 API，参数写成惰性字面量（和前两个场景同样的纪律，不引用引擎内部构造口）：三种形态各一条，外加一条挂在一个直线移动的敌人上、带 omega 的激光；自机放在会被扫到的位置；跑 600 帧，逐帧输出校验和。在 x86_64 本机跑两次，确认输出一致；三平台一致性交给 CI。

- [ ] **Step 4: wheel**：版本改为 0.3.0，本地打包 `uvx --from "maturin>=1.15,<2" maturin build --release -m crates/stg-py/Cargo.toml --out dist`，确认 `import stg_rl` 后从 `lasers` 缓冲能读到非零行（在训练仓 venv 里用一段一次性脚本验证，不入库）。

- [ ] **Step 5: 全绿，提交** `feat(rl): Tier 0 填充 lasers 表（按最近 64 条）；stg_rl 0.3.0；金向量加激光场景`

---

### Task 6: Godot 激光层 + 文档收口

**Files:**
- Modify: `crates/stg-godot/src/frame.rs`（`LAYER_LASERS = 3`、`LAYER_COUNT = 4`、编码分支、`layer_cap`）
- Modify: `crates/stg-godot/src/bridge.rs`（导出常量同步）
- Modify: `godot/scripts/playfield.gd`（`CAPS` 加 `3: 256`，新建一个 MultiMesh 实例）
- Create: `godot/shaders/laser.gdshader`（程序化截面渐变）
- Modify: `docs/render-contract.md`、`docs/follow-ups.md`、`docs/rl-card-pool.md`、`PROGRESS.md`、`stg-world-design.md`（Part IV §1）、spec（回写两处简化）

**Interfaces:**
- Consumes: `view().lasers()`
- Produces: 激光层实例布局（stride 12 不变）：基 = 旋转(angle + 90°) × 缩放(显示宽度, end − start)，原点 = 线段中点；`custom = [color, alpha, 0, 0]`

- [ ] **Step 1: 编码**：显示宽度和 alpha 放在一个纯函数里算（表现层，允许 f32）：

```rust
/// 激光显示宽度与 alpha（spec §7；原作画法见 spec §2「贴图」）。预警：1.2 px，最后 min(warn,30) 帧线性长到全宽；
/// 生效：全宽；收缩：flags 位 0 为 1 时 alpha 线性到 0，否则宽度线性到 0。
fn laser_display(state: u8, timer: u16, warn: u16, fade: u16, width: f32, fade_alpha: bool) -> (f32, f32) {
    match state {
        0 => {
            let ramp = warn.min(30);
            let t0 = warn - ramp;
            if timer >= t0 && ramp > 0 { (1.2 + (width - 1.2) * (timer - t0) as f32 / ramp as f32, 1.0) } else { (1.2, 1.0) }
        }
        1 => (width, 1.0),
        _ => {
            let k = if fade == 0 { 0.0 } else { 1.0 - timer as f32 / fade as f32 };
            if fade_alpha { (width, k) } else { (width * k, 1.0) }
        }
    }
}
```

编码分支：中点 = 原点 + dir × (start + end)/2；基向量 = (cos, sin) 旋转后分别乘显示宽度和长度，写进 `write_instance` 的四个基分量（这个函数目前接收的是单位 cos/sin，要么扩成接收缩放后的基，要么为激光另写一个 `write_instance_basis`，二选一，保持弹层的调用不变）。`laser_display` 给 3 条单测（预警早期 1.2、预警最后一帧接近全宽、收缩一半时宽度减半）。如果 `cargo test -p stg-godot` 在本机因为 Godot 链接问题跑不起来，就把 `laser_display` 挪到 `frame.rs` 里一个不依赖 godot 的子模块，用 `cargo test -p stg-godot --lib frame::laser` 跑；再不行就只 `cargo check`，并在报告里写明原因。

- [ ] **Step 2: shader 与场景**：`laser.gdshader` 用 `UV.x` 画截面：中心白芯，向两侧按实例颜色渐暗，加色混合，alpha 乘 `custom.y`。颜色从 16 色表里按 `custom.x` 取，16 色表照 `bullets` 图集的色列顺序写成常量数组。`playfield.gd` 仿照现有三层建第四个 MultiMesh。原点闪光先不做，记进 follow-ups。
  验证：在 VNC 桌面（`~/.claude/CLAUDE.md`「VNC 虚拟桌面」）打开 Godot 工程，放一张带激光的卡，目视三种形态和预警/收缩动画。截一张图，附在报告里。

- [ ] **Step 3: 文档收口**
- `render-contract.md`：激光层的布局、`custom` 语义，以及「显示宽度 = 判定宽度」。
- `follow-ups.md`：关闭 D23#10；新增条目：激光擦弹、取消时沿线掉星、曲线激光、时停 stop-touch 对激光、原点闪光、图集补截面贴图、th06nc 转写的宽度减半与 omega 改写（指向 spec §9）。
- `rl-card-pool.md` 第 7 条改写：引擎已有激光池；转写口径见 spec §9.1。
- `stg-world-design.md` Part IV §1 标注「已落地（2026-09-25，spec …）」。
- spec 回写 Global Constraints 里的两处简化（`laser()` 只取 color；截面渐变由 shader 程序化生成）。
- `PROGRESS.md`：里程碑一行，下一步指向训练仓（转写和模型，spec §9）。

- [ ] **Step 4: 全绿，提交** `feat(godot): 激光渲染层；docs: 激光池收口（render-contract / follow-ups / PROGRESS）`
