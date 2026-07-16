# M0-11b 信号 / 反弹 / STEP 插值 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 弹会听号令（`WAIT_SIGNAL` + 世界级信号黑板）、会弹墙（`BOUNCE_ARM` + 场界折返镜像）、会缓动（`STEP_SPEED/STEP_ANGLE` 插值 tick）——D4 的 16 个 op 全部齐装，顺带还清 follow-ups **A1 的四条先决缝**。

**Architecture:** `WorldBody.signals: [u32; 8]` 黑板（存 `frame+1`，边沿消费）；STEP 插值状态住扩展槽 scratch（发射时初始化，`run_transforms` 游标前 tick）；反弹物理住 integrate 位移后（walls 从弹自有段读，剩余次数住 flags 位 3-4）；create 验证升级为 **arity 走格**（跳过 scratch 槽）+ LOOP target 边界校验。上游 spec：`docs/superpowers/specs/2026-07-16-d4-transform-design.md`（信号/反弹/STEP 三节）。

**Tech Stack:** Rust 2024 / stg-core。easing 复用 `math::easing::ease(curve: Easing, t: Fx) -> Fx`（8 曲线，repr(u8)）。

## Global Constraints

- **op 编号（族号制 v2，已冻结）**：`OP_STEP_SPEED=12`、`OP_STEP_ANGLE=23`、`OP_WAIT_SIGNAL=51`、`OP_BOUNCE_ARM=52`。每任务把自己的 op 加进 `xform.rs::op_implemented`，不提前放行。
- **信号**：`signals[ch]` 存 `frame.wrapping_add(1)`，0 = 从未脉冲；消费判据 `signals[ch] == frame + 1`（**边沿**：只有当帧停驻弹响应，次帧不重复放行）。`pulse_signal(ch)`：`ch >= 8` → P4-b（`contract_viol` + `STATUS_BAD_ARGS`）；**debug 帧内断言 `phase_guard <= PH_XFORM`**（相位 4 后脉冲=当帧蒸发，断言把"结算期广播"的正路钉死——上层读事件、次帧经导演转发）。
- **STEP**：主槽 `args[0]`=target、`args[1]`=frames(低16)|easing id(高8)；`frames==0` 视同瞬时 SET（合法退化不计数）；easing id ≥ 8 → create 期 BAD_ARGS。扩展槽 scratch：`args[0]`=起点值、`args[1]`=active(bit31)|elapsed(低16)。**发射时无条件重初始化 scratch**（LOOP 重访自动重新武装）。tick 在 `run_transforms` 每弹的**游标推进之前**、delay 门之后；t = `(elapsed<<16)/frames`，`value = start + ease(t)×(target−start)`（ease ≤ 1.0 白名单乘法）；`STEP_ANGLE` 最短弧 `Δ = (target−start) as i16`；`elapsed == frames` → 写精确终值、清 active。序列终止不停 tick（发射即排程）。
- **反弹**：剩余次数住 `flags` 位 3-4（`BULLET_BOUNCE_MASK = 0b0001_1000`）；walls **从弹自有段读**（升序首个 `BOUNCE_ARM` 槽 `args[0]` 低 4 位：bit0 左 / bit1 右 / bit2 上 / bit3 下）；位点 = integrate 位移后同帧折返（`x' = 2·墙 − x`）；每帧每轴至多一次；速度全域一致——POLAR 镜 angle（垂直墙 `HALF−θ` / 水平墙 `ZERO−θ`）后 `refresh_vel_from_polar`，CART/哑弹翻 v 分量（`Fx::ZERO - v`）后 `backfill_polar`；`speed` 不变；归零墙失效。
- **A1 四缝**（本刀必还）：(1) ARITY 步进判别测试；(2) create 坏参扫描**按 arity 走格**（scratch 槽字节不判 op）；(3) LOOP target 边界校验（不得指进扩展槽中间；zero-tail 槽 = 合法 END 边界）；(4) 已由族号制还清（ARITY [u8;256]），无动作。
- **TDD** 每任务红绿；合入前变异检验（T6）。commit 中文 conventional + 尾签 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 既有测试助手：`transform.rs` tests 有 `slot(wait, op, a0, a1)` 与 `xf_bullet(w, seq) -> usize`。

---

### Task 1: 信号黑板 + `pulse_signal` + `WAIT_SIGNAL`

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`SIGNAL_CHANNELS` 常量 + `signals` 字段 + `pulse_signal`）
- Modify: `crates/stg-core/src/step.rs`（`copy_into` 加一行）
- Modify: `crates/stg-core/src/xform.rs`（`op_implemented` 加 `OP_WAIT_SIGNAL`）
- Modify: `crates/stg-core/src/world/transform.rs`（停驻分支 + 测试）

**Interfaces:**
- Produces: `pub const SIGNAL_CHANNELS: usize = 8;`（world.rs）；`WorldBody.signals: [u32; SIGNAL_CHANNELS]`（`pub(crate)`，derive 自动入校验和）；`pub fn pulse_signal(&mut self, ch: usize)`。

- [ ] **Step 0: 开分支**

```bash
git checkout -b d4-xform-b
```

- [ ] **Step 1: 写失败测试**（`transform.rs` tests；`world.rs`/`step.rs` 的接线由测试倒逼）

```rust
    /// 边沿语义三连：停驻不动 → 当帧脉冲放行（同帧转向）→ 次帧不重复放行。
    #[test]
    fn wait_signal_edge_release() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
            slot(0, OP_WAIT_SIGNAL, 3, 0),
            slot(0, OP_TURN, 16384, 0),
        ]);
        // 帧 0-1：无脉冲，停驻
        crate::step::step(&mut w, &InputFrame::empty(0));
        crate::step::step(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "无脉冲不得放行");
        // 帧 2：导演槽脉冲（step_with_director 在相位 2 调闭包）→ 相位 4 同帧放行
        crate::step::step_with_director(&mut w, &InputFrame::empty(2), |b| b.pulse_signal(3));
        assert_eq!(w.body.bullets.angle[i], Angle::QUARTER, "当帧脉冲当帧放行");
        // 帧 3：无新脉冲——已放行的弹不受影响，且新停驻弹听不到旧脉冲
        let j = xf_bullet(&mut w, &[slot(0, OP_WAIT_SIGNAL, 3, 0), slot(0, OP_SET_SPRITE, 9, 0)]);
        crate::step::step(&mut w, &InputFrame::empty(3));
        assert_eq!(w.body.bullets.sprite[j], 0, "旧脉冲是边沿不是电平：次帧不得放行");
    }

    /// 坏通道号：create 期放行（op 合法），运行期 P4-b——计数 + 序列终止。
    #[test]
    fn wait_signal_bad_channel_terminates() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_WAIT_SIGNAL, 8, 0)]);
        let cv0 = w.body.diag.contract_viol;
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.bullets.xform_next[i], 16, "坏通道终止序列");
    }

    /// pulse_signal 本体契约：写 frame+1；坏通道 no-op + 计数。
    #[test]
    fn pulse_signal_contract() {
        let mut w = crate::step::World::new(1);
        w.body.pulse_signal(2);
        assert_eq!(w.body.signals[2], w.body.frame.wrapping_add(1));
        let cv0 = w.body.diag.contract_viol;
        w.body.pulse_signal(8); // 越界
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }
```

`step.rs` tests 加快照覆盖：

```rust
    #[test]
    fn snapshot_covers_signals() {
        let mut w = World::new(3);
        w.body.pulse_signal(5);
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "signals 随快照且入校验和");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core 2>&1 | grep -E "error|cannot find" | head -5`
Expected: 编译失败（`signals`/`pulse_signal` 不存在）。

- [ ] **Step 3: 最小实现**

`world.rs`：常量区加 `pub const SIGNAL_CHANNELS: usize = 8;`；`WorldBody` 字段（`xforms` 之后）：

```rust
    /// 信号黑板（D4 11b）：每通道存"最后脉冲帧号 + 1"，0 = 从未脉冲（零初始化合法）。
    /// 边沿消费：相位 4 只放行 `signals[ch] == frame + 1` 的停驻弹。
    pub(crate) signals: [u32; SIGNAL_CHANNELS],
```

写 API 区：

```rust
    /// 脉冲一条信号通道（相位 4 前有效——导演槽/ECL；边沿语义见 `signals` 字段文档）。
    /// P4-b：坏通道 no-op + 计数。debug 断言相位窗口：相位 4 之后的脉冲当帧蒸发，
    /// 正路是上层读事件、次帧经导演/ECL 转发。
    pub fn pulse_signal(&mut self, ch: usize) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.phase_guard <= PH_XFORM,
            "pulse_signal 晚于相位 4：本帧无人能听见（请次帧经导演/ECL 转发）"
        );
        if ch >= SIGNAL_CHANNELS {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        self.signals[ch] = self.frame.wrapping_add(1);
    }
```

`step.rs::copy_into`（`d.diag = s.diag;` 之前）加 `d.signals = s.signals;`。
`xform.rs::op_implemented` 的 matches! 清单加 `| OP_WAIT_SIGNAL`。
`transform.rs::advance_cursor` 的 END 检查之后、`fire_op` 之前加停驻分支：

```rust
            if slot.op == OP_WAIT_SIGNAL {
                let ch = slot.args[0];
                if !(0..crate::world::SIGNAL_CHANNELS as i32).contains(&ch) {
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                    self.bullets.xform_next[i] = SLOTS_PER_SEG as u8;
                    return;
                }
                if self.signals[ch as usize] != self.frame.wrapping_add(1) {
                    return; // 停驻：不步进、不设 wait，次帧再看
                }
                // 边沿命中：视同已发射，落到下方公共步进（本 op 的 wait 生效）
            } else {
                match self.fire_op(i, slot) { /* ……原有三路出口不动…… */ }
            }
```

（保持原有 `fire_op` match 与公共步进代码不动——只是把 fire 调用包进 else。）

- [ ] **Step 4: 跑测试确认过**

Run: `cargo test -p stg-core` → 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/step.rs crates/stg-core/src/xform.rs crates/stg-core/src/world/transform.rs
git commit -m "feat(world): 信号黑板 + pulse_signal（相位窗口断言）+ WAIT_SIGNAL 边沿停驻"
```

---

### Task 2: STEP 插值（`STEP_SPEED`/`STEP_ANGLE` + tick + A1-(1) 判别）

**Files:**
- Modify: `crates/stg-core/src/xform.rs`（`op_implemented` 加 `OP_STEP_SPEED | OP_STEP_ANGLE`）
- Modify: `crates/stg-core/src/world/transform.rs`

**Interfaces:**
- Consumes: `crate::math::easing::{ease, Easing}`、D3 索引核 `set_speed_at/set_angle_at`。
- Produces: scratch 位常量 `pub(crate) const STEP_ACTIVE: i32 = 1 << 31;`（transform.rs）。

- [ ] **Step 1: 写失败测试**（`transform.rs` tests）

```rust
    /// STEP_SPEED 判别式：Linear 缓动 4 帧从 1.0 到 3.0——逐帧值与手算参考逐位相等，
    /// 第 4 帧后恰为精确终值且 active 清零。
    #[test]
    fn step_speed_linear_hits_exact_waypoints() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
            slot(0, OP_STEP_SPEED, Fx::from_int(3).raw(), 4), // frames=4, easing=Linear(0)
            slot(0, OP_SET_SPRITE, 0, 0), // scratch 扩展槽由步进跳过——这里是槽 3
        ]);
        // 帧 0：SET_SPEED 发射 + STEP 发射（scratch 初始化，elapsed=0，本帧不 tick）
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(1), "发射帧不 tick");
        // 帧 1..4：每帧 +0.5（linear：1 + t×2，t = k/4）
        for k in 1..=4u32 {
            crate::step::step(&mut w, &InputFrame::empty(k));
            let expect = Fx::from_raw(65536 + (k as i32 * 2 * 65536) / 4);
            assert_eq!(w.body.bullets.speed[i], expect, "第 {k} tick");
        }
        // 完成：active 清零，速度停在精确终值
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(w.body.xforms.seg_slots(seg)[2].args[1] & (1 << 31), 0, "active 应清");
        crate::step::step(&mut w, &InputFrame::empty(5));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3), "完成后值冻结");
    }

    /// A1-(1) ARITY 步进判别：STEP 双槽——游标必须跳过扩展槽，直接发射其后的 op。
    /// 若步进错成 1，游标会把 scratch 当 op 读（垃圾/END）→ SET_SPRITE 永不发射。
    #[test]
    fn arity_stepping_skips_extension_slot() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_STEP_SPEED, Fx::from_int(2).raw(), 8),
            slot(0, OP_END, 0, 0),          // 槽 1 = 扩展槽（发射时被 scratch 覆写）
            slot(0, OP_SET_SPRITE, 7, 0),   // 槽 2：STEP 之后的下一个真 op
        ]);
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.sprite[i], 7, "游标须按 1+ARITY 跳过扩展槽");
    }

    /// STEP_ANGLE 最短弧：从 350°(BAM 63715) 缓动到 10°(BAM 1820)——走 +20° 短弧而非 −340°。
    #[test]
    fn step_angle_shortest_arc_wraps() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_SET_ANGLE, 63715, 0),
            slot(0, OP_STEP_ANGLE, 1820, 2), // 2 帧, Linear
        ]);
        crate::step::step(&mut w, &InputFrame::empty(0));
        crate::step::step(&mut w, &InputFrame::empty(1)); // t=0.5：中点应在回绕缝上
        let mid = w.body.bullets.angle[i].raw();
        assert!(mid > 63715 || mid < 1820, "中点须在短弧上（跨 0），实际 {mid}");
        crate::step::step(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.bullets.angle[i], Angle(1820), "终值精确");
    }

    /// frames=0 合法退化：视同瞬时 SET，不激活 scratch、不计数。
    #[test]
    fn step_zero_frames_is_instant_set() {
        let mut w = crate::step::World::new(1);
        let cv0;
        let i = {
            let i = xf_bullet(&mut w, &[slot(0, OP_STEP_SPEED, Fx::from_int(5).raw(), 0)]);
            cv0 = w.body.diag.contract_viol;
            i
        };
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(5));
        assert_eq!(w.body.diag.contract_viol, cv0, "合法退化不计数");
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(w.body.xforms.seg_slots(seg)[1].args[1] & (1 << 31), 0, "不激活");
    }

    /// LOOP 重访 STEP = 自动重新武装（scratch 重初始化纪律）：第二轮从新起点插值。
    #[test]
    fn loop_rearms_step_from_new_start() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(2, OP_STEP_SPEED, Fx::from_int(2).raw(), 2), // 槽0（扩展槽=1）；wait 2 让插值先跑完
            slot(0, OP_ADD_SPEED, Fx::from_int(3).raw(), 0),  // 槽2：完成后猛加 3（起点被改变）
            slot(1, OP_LOOP, 0, 2),                            // 槽3：跳回槽0，共 2 轮
        ]);
        for f in 0..12u32 {
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        // 第一轮：0→2（2帧）→ +3 = 5；第二轮重武装：5→2（2帧）→ +3 = 5；终态 5
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(5), "第二轮须从 5 重新插到 2 再 +3");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core transform:: 2>&1 | grep -cE "FAILED"`
Expected: 新 5 测试全 FAIL（STEP op 落未知臂 → 序列终止 / create 拒收——注意 `op_implemented`
未加前 create 直接 BAD_ARGS，`xf_bullet` 的 `unwrap` panic 也算合法 RED 形态）。

- [ ] **Step 3: 最小实现**

`xform.rs::op_implemented` 加 `| OP_STEP_SPEED | OP_STEP_ANGLE`。
`transform.rs`：

```rust
/// STEP scratch 活跃位（扩展槽 args[1] bit31；低 16 位 = elapsed）。
pub(crate) const STEP_ACTIVE: i32 = 1 << 31;

fn easing_from_id(id: u8) -> crate::math::easing::Easing {
    use crate::math::easing::Easing::*;
    match id {
        1 => QuadIn,
        2 => QuadOut,
        3 => QuadInOut,
        4 => CubicIn,
        5 => CubicOut,
        6 => CubicInOut,
        7 => Smoothstep,
        _ => Linear, // 0 与一切越界值（create 期已拒 ≥8，此处兜底确定性）
    }
}
```

`fire_op` 加两臂（需要槽索引：`let idx = self.bullets.xform_next[i] as usize;` 与 `fire_loop` 同法）：

```rust
            OP_STEP_SPEED | OP_STEP_ANGLE => {
                let frames = (slot.args[1] & 0xFFFF) as u32;
                if frames == 0 {
                    // 合法退化：瞬时 SET
                    if slot.op == OP_STEP_SPEED {
                        self.set_speed_at(i, Fx::from_raw(slot.args[0]));
                    } else {
                        self.set_angle_at(i, Angle(slot.args[0] as u16));
                    }
                } else {
                    // scratch 无条件重初始化（LOOP 重访 = 自动重新武装）
                    let idx = self.bullets.xform_next[i] as usize;
                    let seg = self.bullets.transform_head[i];
                    let start = if slot.op == OP_STEP_SPEED {
                        self.bullets.speed[i].raw()
                    } else {
                        self.bullets.angle[i].raw() as i32
                    };
                    let ext = &mut self.xforms.seg_slots_mut(seg)[idx + 1];
                    ext.args[0] = start;
                    ext.args[1] = STEP_ACTIVE; // elapsed = 0
                }
            }
```

`run_transforms` 每弹处理：delay 门之后、wait 门之前插入 `self.tick_steps(i);`。新方法：

```rust
    /// 推进本弹全部活跃 STEP 插值（升序，I4）。与游标并发：序列终止后插值照走完。
    fn tick_steps(&mut self, i: usize) {
        let seg = self.bullets.transform_head[i];
        let fired_end = (self.bullets.xform_next[i] as usize).min(SLOTS_PER_SEG);
        let mut s = 0usize;
        while s < fired_end {
            let main = self.xforms.seg_slots(seg)[s];
            let is_step = main.op == OP_STEP_SPEED || main.op == OP_STEP_ANGLE;
            if is_step && s + 1 < SLOTS_PER_SEG {
                let ext = self.xforms.seg_slots(seg)[s + 1];
                if ext.args[1] & STEP_ACTIVE != 0 {
                    self.tick_one_step(i, seg, s, main, ext);
                }
            }
            s += 1 + ARITY[main.op as usize] as usize;
        }
    }

    /// 单个活跃 STEP 的一帧推进。绝对插值（每帧从 start 重算，不累积误差）。
    fn tick_one_step(&mut self, i: usize, seg: u16, s: usize, main: XformSlot, ext: XformSlot) {
        let frames = (main.args[1] & 0xFFFF) as i64; // 发射时已保证 > 0
        let elapsed = ((ext.args[1] & 0xFFFF) as i64 + 1).min(frames);
        let done = elapsed == frames;
        let t = crate::math::Fx::from_raw(((elapsed << 16) / frames) as i32);
        let e = crate::math::easing::ease(easing_from_id((main.args[1] >> 16) as u8 & 0xFF), t);
        if main.op == OP_STEP_SPEED {
            let (start, target) = (Fx::from_raw(ext.args[0]), Fx::from_raw(main.args[0]));
            let v = if done {
                target // 终帧写精确终值（不吃插值舍入）
            } else {
                let d = (target.raw() as i64 - start.raw() as i64) * e.raw() as i64 >> 16;
                Fx::from_raw((start.raw() as i64 + d) as i32)
            };
            self.set_speed_at(i, v);
        } else {
            let start = Angle(ext.args[0] as u16);
            let delta = (main.args[0] as u16).wrapping_sub(start.raw()) as i16; // 最短弧带方向
            let scaled = if done {
                delta
            } else {
                ((delta as i64 * e.raw() as i64) >> 16) as i16
            };
            self.set_angle_at(i, start.add_delta(scaled));
        }
        let ext_mut = &mut self.xforms.seg_slots_mut(seg)[s + 1];
        ext_mut.args[1] = if done {
            elapsed as i32 // 清 active
        } else {
            STEP_ACTIVE | elapsed as i32
        };
    }
```

（`>>` 对负 i64 是算术右移——确定性，向负无穷取整，钉进注释。）

- [ ] **Step 4: 跑测试确认过 + Commit**

Run: `cargo test -p stg-core` → 全绿。

```bash
git add crates/stg-core/src/xform.rs crates/stg-core/src/world/transform.rs
git commit -m "feat(world): STEP_SPEED/STEP_ANGLE 插值——scratch 重武装/最短弧/精确终值 + ARITY 步进判别（A1-1）"
```

---

### Task 3: create 验证升级（A1-(2)(3) 还债）

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`create_bullet_with_xform` 的坏参检查重写）

**Interfaces:**
- Consumes: `ARITY`、`op_implemented`、`OP_LOOP/OP_STEP_*`、`SLOTS_PER_SEG`。

- [ ] **Step 1: 写失败测试**（`step.rs` tests，与既有 create 测试同址）

```rust
    /// A1-(2)：扩展槽的字节不判 op——scratch 位置放任意垃圾值也必须过 create。
    #[test]
    fn create_validation_skips_extension_slots() {
        let mut w = World::new(1);
        let seq = [
            slot(0, crate::xform::OP_STEP_SPEED, 65536, 4),
            slot(0, 99, -1, -1), // 扩展槽：垃圾字节合法（会被 fire 时的 scratch 覆写）
            slot(0, crate::xform::OP_SET_SPRITE, 1, 0),
        ];
        assert_ne!(
            w.body.create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL,
            "scratch 槽不得被当 op 判"
        );
    }

    /// STEP 在末槽（槽 15）没有扩展槽空间 → BAD_ARGS 整体失败。
    #[test]
    fn create_rejects_step_without_extension_room() {
        let mut w = World::new(1);
        let mut seq = [slot(0, crate::xform::OP_SET_SPRITE, 0, 0); 16];
        seq[15] = slot(0, crate::xform::OP_STEP_SPEED, 65536, 4);
        assert_eq!(
            w.body.create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }

    /// A1-(3)：LOOP target 指进扩展槽中间 → BAD_ARGS；指向 zero-tail（END）→ 合法。
    #[test]
    fn create_validates_loop_target_boundaries() {
        let mut w = World::new(1);
        let bad = [
            slot(0, crate::xform::OP_STEP_SPEED, 65536, 4), // 槽0（扩展槽=1）
            slot(0, 0, 0, 0),                                // 扩展槽
            slot(0, crate::xform::OP_LOOP, 1, 0),            // target=1 = 扩展槽中间 → 拒
        ];
        assert_eq!(
            w.body.create_bullet_with_xform(straight(0, 0, 0, 0, 1), &bad),
            BulletHandle::NULL
        );
        let ok = [
            slot(0, crate::xform::OP_SET_SPRITE, 1, 0),
            slot(0, crate::xform::OP_LOOP, 10, 3), // target=10 在 zero-tail：落地即 END，合法
        ];
        assert_ne!(
            w.body.create_bullet_with_xform(straight(0, 0, 0, 0, 1), &ok),
            BulletHandle::NULL
        );
    }

    /// easing id ≥ 8 → BAD_ARGS（作者错误 create 期就拒）。
    #[test]
    fn create_rejects_bad_easing_id() {
        let mut w = World::new(1);
        let seq = [slot(0, crate::xform::OP_STEP_SPEED, 65536, 4 | (8 << 16))];
        assert_eq!(
            w.body.create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
    }
```

（`slot` 助手若 step.rs tests 没有，就地补一个与 transform.rs 同款。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core create_ 2>&1 | grep -cE "FAILED"`
Expected: 第 1/3(ok 分支)/… 失败——现行逐槽 `any` 扫描把 scratch 垃圾字节判成未知 op、
不查 LOOP target 边界、不查 easing id。

- [ ] **Step 3: 最小实现**——`create_bullet_with_xform` 的 `let bad = …` 替换为两遍走格：

```rust
        // 坏参检查（A1-(2)(3)）：按 arity 走格——扩展槽是 scratch，字节不判 op。
        // 第一遍：验 op/扩展槽空间/easing id，收集合法边界位图（zero-tail 全为 END = 合法边界）。
        let mut bad = xform.len() > crate::xform::SLOTS_PER_SEG;
        let mut boundaries: u16 = 0;
        let mut k = 0usize;
        while !bad && k < xform.len() {
            let s = &xform[k];
            if !crate::xform::op_implemented(s.op) {
                bad = true;
                break;
            }
            boundaries |= 1 << k;
            let ar = crate::xform::ARITY[s.op as usize] as usize;
            if ar > 0 {
                if k + ar >= crate::xform::SLOTS_PER_SEG {
                    bad = true; // 扩展槽越出段（如 STEP 在槽 15）
                    break;
                }
                if (s.args[1] >> 16) as u8 & 0xFF >= 8 {
                    bad = true; // easing id 越界
                    break;
                }
            }
            k += 1 + ar;
        }
        // zero-tail（含恰好越出提供长度的走格终点）：全零 = END，合法边界
        for t in xform.len()..crate::xform::SLOTS_PER_SEG {
            boundaries |= 1 << t;
        }
        // 第二遍：LOOP target 必须落在边界上
        if !bad {
            let mut k = 0usize;
            while k < xform.len() {
                let s = &xform[k];
                if s.op == crate::xform::OP_LOOP {
                    let t = s.args[0];
                    if !(0..crate::xform::SLOTS_PER_SEG as i32).contains(&t)
                        || boundaries & (1 << t) == 0
                    {
                        bad = true;
                        break;
                    }
                }
                k += 1 + crate::xform::ARITY[s.op as usize] as usize;
            }
        }
```

注意：走格终点 `k` 可能等于 `xform.len()`（正常）或跳过 len 尾部（STEP 主槽在 len−1、
扩展槽由 zero-tail 提供——合法，扩展槽内容本就是 scratch）。easing id 检查只对 `ar > 0`
的 op（当前恰为 STEP 族）。

- [ ] **Step 4: 跑测试确认过 + Commit**

Run: `cargo test -p stg-core` → 全绿（含 11a 既有 create 测试——`slot(0, 99, …)` 垃圾 op
仍被拒、`>16` 仍被拒）。

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/step.rs
git commit -m "feat(world): create 验证按 arity 走格——scratch 不判 op + LOOP 边界 + easing id（A1-2/3 还债）"
```

---

### Task 4: `BOUNCE_ARM` + 反弹物理

**Files:**
- Modify: `crates/stg-core/src/bullets.rs`（位 3-4 常量）
- Modify: `crates/stg-core/src/xform.rs`（`op_implemented` 加 `OP_BOUNCE_ARM`）
- Modify: `crates/stg-core/src/world/transform.rs`（fire 臂 + walls 查询助手）
- Modify: `crates/stg-core/src/world/integrate.rs`（位移后反弹）

**Interfaces:**
- Produces: `pub const BULLET_BOUNCE_SHIFT: u32 = 3; pub const BULLET_BOUNCE_MASK: u8 = 0b0001_1000;`（bullets.rs）；
  `WorldBody::bounce_walls_of(&self, i: usize) -> u8`（`pub(crate)`，transform.rs——段知识住相位 4 的家）。

- [ ] **Step 1: 写失败测试**（`integrate.rs` tests；几何全用判别值）

```rust
    /// 右墙折返判别式（哑弹）：x 越界量镜像 + vx 翻号 + 计数递减。
    /// 场界 x=+192；弹从 190 以 vx=+5 一帧到 195 → 折返到 189、vx=-5。
    #[test]
    fn bounce_right_wall_folds_position_and_flips_vx() {
        use crate::bullets::BULLET_BOUNCE_MASK;
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_BOUNCE_ARM, 0b0010, 2), // walls=右；n=2
        ]);
        w.body.bullets.x[i] = Fx::from_int(190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(5);
        crate::step::step(&mut w, &InputFrame::empty(0)); // BOUNCE_ARM 发射 + 位移 195 → 折返
        assert_eq!(w.body.bullets.x[i], Fx::from_int(189), "2·192−195 = 189");
        assert_eq!(w.body.bullets.vx[i], Fx::from_int(-5));
        assert_eq!((w.body.bullets.flags[i] & BULLET_BOUNCE_MASK) >> 3, 1, "计数 2→1");
    }

    /// POLAR 弹镜像走角度域：右墙后 angle = HALF − θ，且 vx/vy 与查表参考一致。
    #[test]
    fn bounce_polar_bullet_mirrors_angle() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[
            slot(0, OP_SET_SPEED, Fx::from_int(6).raw(), 0),
            slot(0, OP_SET_ANGLE, 8192, 0),        // 45°（右下）
            slot(0, OP_SET_ANG_VEL, 0, 0),         // 开 POLAR（ω=0：只为进角度域）
            slot(0, OP_BOUNCE_ARM, 0b0010, 1),
        ]);
        w.body.bullets.x[i] = Fx::from_int(189);
        w.body.bullets.y[i] = Fx::from_int(100);
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.angle[i], Angle::HALF.sub(Angle(8192)), "垂直墙：HALF−θ");
        let (rvx, rvy) = polar_to_vec(Fx::from_int(6), Angle::HALF.sub(Angle(8192)));
        assert_eq!((w.body.bullets.vx[i], w.body.bullets.vy[i]), (rvx, rvy));
    }

    /// 未武装的墙不反弹：只武装右墙的弹撞上墙照常越界、被 OOB 回收。
    #[test]
    fn unarmed_wall_does_not_bounce() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_BOUNCE_ARM, 0b0010, 3)]); // 只右墙
        w.body.bullets.x[i] = Fx::from_int(-190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(-8); // 左飞
        for f in 0..12u32 {
            crate::step::step(&mut w, &InputFrame::empty(f)); // −190−8k，越 OOB(−256) 即回收
        }
        assert!(!w.body.bullets.is_alive(i), "未武装左墙：照常越界回收");
    }

    /// 计数耗尽墙失效：n=1 弹第一次反弹后第二次撞墙直接穿出。
    #[test]
    fn bounce_count_exhausts() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_BOUNCE_ARM, 0b0011, 1)]); // 左右墙 n=1
        w.body.bullets.x[i] = Fx::from_int(190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(60); // 大步伐来回撞
        let mut bounced_once = false;
        for f in 0..20u32 {
            crate::step::step(&mut w, &InputFrame::empty(f));
            if w.body.bullets.is_alive(i) && w.body.bullets.vx[i].raw() < 0 {
                bounced_once = true;
            }
        }
        assert!(bounced_once, "第一次必须反弹");
        assert!(!w.body.bullets.is_alive(i), "耗尽后必须穿出被回收");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core 2>&1 | grep -cE "FAILED"`
Expected: 新 4 测试 FAIL（BOUNCE_ARM 落未知臂 create 拒收 → `xf_bullet` unwrap panic）。

- [ ] **Step 3: 最小实现**

`bullets.rs`（`BULLET_CART_FX` 下方）：

```rust
/// `flags` 位 3-4：反弹剩余次数（D4 `BOUNCE_ARM`，≤3）。walls 掩码不进弹本体——从弹自有段读。
pub const BULLET_BOUNCE_SHIFT: u32 = 3;
pub const BULLET_BOUNCE_MASK: u8 = 0b0001_1000;
```

`xform.rs::op_implemented` 加 `| OP_BOUNCE_ARM`。
`transform.rs::fire_op` 加臂：

```rust
            OP_BOUNCE_ARM => {
                let n = slot.args[1];
                if !(0..=3).contains(&n) {
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                }
                let n = n.clamp(0, 3) as u8;
                self.bullets.flags[i] = (self.bullets.flags[i]
                    & !crate::bullets::BULLET_BOUNCE_MASK)
                    | (n << crate::bullets::BULLET_BOUNCE_SHIFT);
            }
```

`transform.rs` 新助手（放 `impl WorldBody` 里）：

```rust
    /// 武装墙掩码：扫弹自有段已发射区间的首个 BOUNCE_ARM（升序，I4），读 args[0] 低 4 位。
    /// 只对 flags 位 3-4 非零的弹调用（调用方保证）；武装弹必然有段。
    pub(crate) fn bounce_walls_of(&self, i: usize) -> u8 {
        let seg = self.bullets.transform_head[i];
        if seg as usize >= SEG_CAP {
            return 0; // 伪造段号护栏同款：无段即无墙
        }
        let fired_end = (self.bullets.xform_next[i] as usize).min(SLOTS_PER_SEG);
        let mut s = 0usize;
        while s < fired_end {
            let slot = self.xforms.seg_slots(seg)[s];
            if slot.op == OP_BOUNCE_ARM {
                return (slot.args[0] & 0xF) as u8;
            }
            s += 1 + ARITY[slot.op as usize] as usize;
        }
        0
    }
```

`integrate.rs` 弹循环，`x += vx; y += vy;` 之后、life 倒数之前：

```rust
                // D4 反弹：位移后同帧折返（每帧每轴至多一次；速度按模式全域一致更新）
                if self.bullets.flags[i] & crate::bullets::BULLET_BOUNCE_MASK != 0 {
                    self.bounce_bullet(i);
                }
```

`integrate.rs` 新方法（同文件 `impl WorldBody`）：

```rust
    /// 场界折返镜像（D4 11b 拍板）。walls：bit0 左 / bit1 右 / bit2 上 / bit3 下。
    fn bounce_bullet(&mut self, i: usize) {
        use crate::bullets::{BULLET_BOUNCE_MASK, BULLET_BOUNCE_SHIFT, BULLET_POLAR_FX};
        let walls = self.bounce_walls_of(i);
        if walls == 0 {
            return;
        }
        let left = Fx::from_int(-super::FIELD_HALF_W);
        let right = Fx::from_int(super::FIELD_HALF_W);
        let top = Fx::ZERO;
        let bottom = Fx::from_int(super::FIELD_HEIGHT);
        // x 轴（每帧至多一次）
        let mut count = (self.bullets.flags[i] & BULLET_BOUNCE_MASK) >> BULLET_BOUNCE_SHIFT;
        let x = self.bullets.x[i];
        let hit_x = (walls & 0b0001 != 0 && x.raw() < left.raw())
            .then_some(left)
            .or((walls & 0b0010 != 0 && x.raw() > right.raw()).then_some(right));
        if let (Some(wall), true) = (hit_x, count > 0) {
            self.bullets.x[i] = Fx::from_raw(2 * wall.raw() - x.raw()); // 折返镜像
            if self.bullets.flags[i] & BULLET_POLAR_FX != 0 {
                let a = self.bullets.angle[i];
                self.bullets.angle[i] = crate::math::Angle::HALF.sub(a); // 垂直墙：HALF−θ
                self.refresh_vel_from_polar(i);
            } else {
                self.bullets.vx[i] = Fx::ZERO - self.bullets.vx[i];
                self.backfill_polar(i);
            }
            count -= 1;
        }
        // y 轴（重读计数——角撞允许同帧双轴各一次）
        let y = self.bullets.y[i];
        let hit_y = (walls & 0b0100 != 0 && y.raw() < top.raw())
            .then_some(top)
            .or((walls & 0b1000 != 0 && y.raw() > bottom.raw()).then_some(bottom));
        if let (Some(wall), true) = (hit_y, count > 0) {
            self.bullets.y[i] = Fx::from_raw(2 * wall.raw() - y.raw());
            if self.bullets.flags[i] & BULLET_POLAR_FX != 0 {
                let a = self.bullets.angle[i];
                self.bullets.angle[i] = crate::math::Angle::ZERO.sub(a); // 水平墙：−θ
                self.refresh_vel_from_polar(i);
            } else {
                self.bullets.vy[i] = Fx::ZERO - self.bullets.vy[i];
                self.backfill_polar(i);
            }
            count -= 1;
        }
        self.bullets.flags[i] = (self.bullets.flags[i] & !BULLET_BOUNCE_MASK)
            | (count << BULLET_BOUNCE_SHIFT);
    }
```

（`2 * wall.raw()` 最大 2×448×65536 ≪ i32::MAX，无溢出。CART 弹翻分量后 `ax/ay` 不翻——
重力弹落地反弹是正确语义。）

- [ ] **Step 4: 跑测试确认过 + Commit**

Run: `cargo test -p stg-core` → 全绿。

```bash
git add crates/stg-core/src/bullets.rs crates/stg-core/src/xform.rs crates/stg-core/src/world/transform.rs crates/stg-core/src/world/integrate.rs
git commit -m "feat(world): BOUNCE_ARM + 场界折返镜像——POLAR 镜角/CART 翻 v/每帧每轴一次（16 op 齐装）"
```

---

### Task 5: 金向量三压力源 + 双跑

**Files:**
- Modify: `crates/stg-harness/src/main.rs`（导演闭包，续 ⑦⑧ → ⑨⑩⑪）

- [ ] **Step 1: 实现**（复用既有 `bullet_at` init 助手与 `XformSlot` 全展开风格）

```rust
        // ⑨ 11b 压力源一：信号齐转——停驻弹群每 120 帧一声令下集体转向
        if frame % 40 == 25 {
            let h = b.create_bullet_with_xform(bullet_at(-120, 90), &[
                XformSlot { wait: 0, op: OP_SET_SPEED, _pad: 0, args: [49_152, 0] },
                XformSlot { wait: 0, op: OP_WAIT_SIGNAL, _pad: 0, args: [0, 0] },
                XformSlot { wait: 0, op: OP_TURN, _pad: 0, args: [32_768, 0] },
                XformSlot { wait: 0, op: OP_WAIT_SIGNAL, _pad: 0, args: [0, 0] },
                XformSlot { wait: 0, op: OP_TURN, _pad: 0, args: [32_768, 0] },
            ]);
            let _ = h;
        }
        if frame % 120 == 60 {
            b.pulse_signal(0);
        }
        // ⑩ 11b 压力源二：三墙反弹弹（左右上，n=3）——POLAR 域镜像入对拍
        if frame % 90 == 45 {
            let h = b.create_bullet_with_xform(bullet_at(0, 120), &[
                XformSlot { wait: 0, op: OP_SET_SPEED, _pad: 0, args: [196_608, 0] },
                XformSlot { wait: 0, op: OP_SET_ANGLE, _pad: 0, args: [6_000, 0] },
                XformSlot { wait: 0, op: OP_SET_ANG_VEL, _pad: 0, args: [0, 0] },
                XformSlot { wait: 0, op: OP_BOUNCE_ARM, _pad: 0, args: [0b0111, 3] },
            ]);
            let _ = h;
        }
        // ⑪ 11b 压力源三：STEP 缓动弹——Smoothstep 40 帧从 0.5 缓到 3.0
        if frame % 65 == 20 {
            let h = b.create_bullet_with_xform(bullet_at(60, 70), &[
                XformSlot { wait: 0, op: OP_SET_SPEED, _pad: 0, args: [32_768, 0] },
                XformSlot { wait: 0, op: OP_SET_ANGLE, _pad: 0, args: [16_384, 0] },
                XformSlot { wait: 0, op: OP_STEP_SPEED, _pad: 0, args: [196_608, 40 | (7 << 16)] },
                XformSlot { wait: 0, op: OP_END, _pad: 0, args: [0, 0] }, // 扩展槽占位
            ]);
            let _ = h;
        }
```

导入补 `OP_WAIT_SIGNAL/OP_BOUNCE_ARM/OP_STEP_SPEED`；函数级文档注释同步补三块（含相位偏移）。

- [ ] **Step 2: 全量验证 + 双跑**

```bash
cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out /tmp/g1.txt && cargo run -p stg-harness -- golden --out /tmp/g2.txt
diff /tmp/g1.txt /tmp/g2.txt && echo DETERMINISTIC
```

Expected: 全绿 + `DETERMINISTIC`（debug 溢出断言 + 相位断言 + pulse 窗口断言全程武装）。

- [ ] **Step 3: Commit**

```bash
git add crates/stg-harness/src/main.rs
git commit -m "feat(harness): 金向量 11b 加戏——信号齐转/三墙反弹/STEP 缓动（黑板+镜像+easing 入对拍）"
```

---

### Task 6: 变异检验（无提交物；还原用定向反向 Edit，禁 git checkout/restore/stash）

- [ ] **Step 1: 变异 A——信号边沿退化成电平**（transform.rs 停驻检查 `!= frame+1` 改 `== 0`，即"脉冲过就永远放行"）
Run: `cargo test -p stg-core transform::tests::wait_signal_edge_release`
Expected: FAIL（第三段"次帧不得放行"断言红）。还原。

- [ ] **Step 2: 变异 B——ARITY 步进阉割**（`advance_cursor` 步进 `1 + ARITY[...]` 改 `1`）
Run: `cargo test -p stg-core transform::tests::arity_stepping_skips_extension_slot`
Expected: FAIL（sprite 未发射）。还原。

- [ ] **Step 3: 变异 C——反弹丢折返**（integrate.rs `2 * wall.raw() - x.raw()` 改 `wall.raw()`）
Run: `cargo test -p stg-core integrate::tests::bounce_right_wall_folds_position_and_flips_vx`
Expected: FAIL（189 ≠ 192）。还原。

- [ ] **Step 4: `git diff --exit-code` = 0 + `cargo test -p stg-core` 全绿；三杀结论记入报告。**

---

### Task 7: 收尾——设计回写 + 文档 + A1 销账 + 全绿门

**Files:**
- Modify: `stg-world-design.md`（D4 节三处）、`docs/xform-ops.md`、`PROGRESS.md`、`docs/follow-ups.md`

- [ ] **Step 1: 设计回写 `stg-world-design.md` D4**（各注明"（spec 2026-07-16 / 11b 实现定稿）"）：
  ① 信号段落补：存 `frame+1`（0=从未脉冲，零初始化合法）+ `pulse_signal` 相位窗口 debug 断言
  （相位 4 后脉冲当帧蒸发，正路=次帧经导演/ECL 转发）；② `BOUNCE_ARM` 行补：剩余次数 flags
  位 3-4、walls 从弹自有段读（升序首个）、位移后同帧折返、每帧每轴一次、POLAR 镜角/CART 翻 v；
  ③ `STEP_*` 行补：scratch 布局（ext.args[0]=start、args[1]=active|elapsed）、发射帧不 tick、
  终帧写精确终值、最短弧、easing id ≥8 create 期拒。
- [ ] **Step 2: `docs/xform-ops.md`**：四个 🚧 翻 ✅ M0-11b；关键语义小节补三条（信号边沿+脉冲
  窗口 / 反弹 walls 位图与耗尽语义 / STEP 发射帧不 tick+精确终值）；示例区补一个"信号齐转"
  两槽小例。`docs/follow-ups.md`：**A1 整条删除**（(1)(2)(3) 本刀已还、(4) 已由族号制还——
  维护规矩"解决一条删一条"）；B9 保留不动。
- [ ] **Step 3: `PROGRESS.md`**：史表顶加
  `| 2026-07-16 | M0-11b | 信号黑板/场界反弹/STEP 缓动——D4 十六 op 齐装，A1 四缝清账 |`；
  「现在」段重写（位置 M0-11b；下一步候选：bomb · 道具池 · 敌人 move_to · SPAWN_PATTERN+图样表）。
- [ ] **Step 4: 全绿门 + Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p stg-harness -- verify-tables
git add stg-world-design.md docs/xform-ops.md PROGRESS.md docs/follow-ups.md
git commit -m "docs: M0-11b 收尾——信号/反弹/STEP 语义回写 + 速查表翻绿 + A1 销账 + PROGRESS"
```

然后 `superpowers:finishing-a-development-branch`。

---

## Self-Review 记录

- **Spec 覆盖**：信号（字段/frame+1/pulse API/边沿/坏通道/窗口断言）→T1；STEP（打包/scratch/
  重武装/最短弧/frames=0/精确终值/tick 位点/easing id）→T2+T3；A1-(1)→T2、(2)(3)→T3、(4) 已还
  （无任务，销账在 T7）；反弹（位 3-4/walls 从段读/折返/域一致/每帧每轴/耗尽）→T4；金向量三源→T5；
  变异→T6；回写+销账→T7。无缺口。
- **占位扫描**：全部实码；T5 的 `bullet_at` 为 harness 既有助手（11a T7 建）。
- **类型一致性**：`STEP_ACTIVE: i32 = 1<<31`（i32 位运算域一致）；`bounce_walls_of` 在 transform.rs
  定义、integrate.rs 消费（同 crate `pub(crate)`）；`pulse_signal(ch: usize)` 全计划一致；
  `fire_op` 需槽索引处沿用 `xform_next[i]` 既有惯例（fire 时游标尚未步进）。
- **已知妥协**：T1 的 WAIT_SIGNAL 分支把 fire_op 调用包进 else——重排了 advance_cursor 的
  控制流形状，11a 的既有测试是护网；T4 的 `bounce_walls_of` 每帧扫段 O(16) 只发生在武装弹上
  （spec 拍板的成本预算内）。
