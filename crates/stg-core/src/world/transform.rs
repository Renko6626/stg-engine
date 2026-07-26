//! 相位 4 · 变换游标执行器（D4）。只排程：瞬时 op 调 D3 索引核，连续效果开模式位；
//! wait 语义 = 散文修正版（wait=W ⇒ 下一 op 恰在 W 帧后；见 spec 2026-07-16）。

use super::WorldBody;
use crate::math::{Angle, Fx};
use crate::xform::*;

pub(crate) enum FireResult {
    Continue,
    Terminate,
    /// LOOP 已跳（xform_next 已改写到 target）：护栏——本帧到此为止，不设 wait。
    Jumped,
}

/// STEP scratch 活跃位（扩展槽 args[1] bit31；低 16 位 = elapsed）。
pub(crate) const STEP_ACTIVE: i32 = 1 << 31;

impl WorldBody {
    pub(crate) fn run_transforms(&mut self) {
        self.phase_enter(super::PH_XFORM);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.bullets.transform_head[i] == XFORM_NONE {
                    continue; // 哑弹
                }
                if self.bullets.delay[i] > 0 {
                    continue; // 激活前变换不走（delay 递减归 integrate）
                }
                self.tick_steps(i); // 与游标并发：wait/WAIT_SIGNAL 期间插值照走
                if self.bullets.xform_wait[i] > 0 {
                    self.bullets.xform_wait[i] -= 1;
                    if self.bullets.xform_wait[i] > 0 {
                        continue;
                    }
                    // 递减归零：当帧放行（wait=W ⇒ 恰 W 帧后发射）
                }
                self.advance_cursor(i);
            }
        }
    }

    /// 游标推进：从 xform_next 起连发直到 wait 门/终止/LOOP 护栏。
    fn advance_cursor(&mut self, i: usize) {
        if self.bullets.xform_next[i] as usize >= SLOTS_PER_SEG {
            return; // 已终止（哨兵 16）——短路，含伪造段号已终止后的重复调用
        }
        let seg = self.bullets.transform_head[i];
        if seg as usize >= SEG_CAP {
            // P4-b：非哨兵越界段号（只能来自绕过写 API 的伪造）→ 计数 + 序列终止，不 panic
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.bullets.xform_next[i] = SLOTS_PER_SEG as u8;
            return;
        }
        loop {
            let next = self.bullets.xform_next[i] as usize;
            if next >= SLOTS_PER_SEG {
                return; // 已终止（哨兵 16）
            }
            let slot = self.xforms.seg_slots(seg)[next];
            if slot.op == OP_END {
                self.bullets.xform_next[i] = SLOTS_PER_SEG as u8;
                return;
            }
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
                match self.fire_op(i, slot) {
                    FireResult::Terminate => {
                        self.bullets.xform_next[i] = SLOTS_PER_SEG as u8;
                        return;
                    }
                    FireResult::Jumped => return, // LOOP 护栏：本帧到此为止
                    FireResult::Continue => {}
                }
            }
            self.bullets.xform_wait[i] = slot.wait;
            self.bullets.xform_next[i] = (next + 1 + ARITY[slot.op as usize] as usize) as u8;
            if slot.wait > 0 {
                return; // wait 门（设置帧不递减）
            }
        }
    }

    /// 发射一个 op。瞬时/连续 = D3 索引核薄封装；未知 op = P4-b 终止。
    fn fire_op(&mut self, i: usize, slot: XformSlot) -> FireResult {
        match slot.op {
            OP_SET_SPEED => self.set_speed_at(i, Fx::from_raw(slot.args[0])),
            OP_ADD_SPEED => {
                let v = self.bullets.speed[i] + Fx::from_raw(slot.args[0]);
                self.set_speed_at(i, v);
            }
            OP_SET_ANGLE => self.set_angle_at(i, Angle(slot.args[0] as u16)),
            OP_TURN => self.turn_at(i, Angle(slot.args[0] as u16)),
            OP_AIM_PLAYER => self.aim_at_player_at(i, Angle(slot.args[0] as u16)),
            OP_SET_SPRITE => self.bullets.sprite[i] = slot.args[0] as u16,
            // 部分设：把 sprite 拆回 (形, 色) 再只改一维。stride 从槽里来（编译器写入），
            // 故世界层不需要表、也不认识"颜色"这回事（spec §4.4）。
            OP_SET_SHAPE => {
                let stride = slot.args[1];
                if stride <= 0 {
                    // P4-b：手工构造的坏槽（编译器不会产出）——确定性 no-op + 计数，不除零
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                } else {
                    let cur = self.bullets.sprite[i] as i32;
                    self.bullets.sprite[i] = (slot.args[0] + cur % stride) as u16;
                }
            }
            OP_SET_COLOR => {
                let stride = slot.args[1];
                if stride <= 0 {
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                } else {
                    let cur = self.bullets.sprite[i] as i32;
                    self.bullets.sprite[i] = (cur - cur % stride + slot.args[0]) as u16;
                }
            }
            OP_SET_LIFE => self.bullets.life[i] = slot.args[0] as u16,
            OP_SET_ANG_VEL => self.set_ang_vel_at(i, slot.args[0] as i16),
            OP_SET_ACCEL => self.set_accel_at(i, Fx::from_raw(slot.args[0])),
            OP_SET_GRAVITY => {
                self.set_gravity_at(i, Fx::from_raw(slot.args[0]), Fx::from_raw(slot.args[1]))
            }
            OP_STOP_FX => self.stop_fx_at(i),
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
                    if idx + 1 >= SLOTS_PER_SEG {
                        // P4-b：末槽 STEP 无扩展槽空间（create 期空间校验属后续任务；此为发射期兜底）
                        self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                        return FireResult::Terminate;
                    }
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
            OP_LOOP => return self.fire_loop(i, slot),
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
            _ => {
                // 未实现 op（11b 预留编号 / 族内空隙 / 一切垃圾值）：P4-b——计数 + 序列终止（两机同样跳过）
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                return FireResult::Terminate;
            }
        }
        FireResult::Continue
    }

    /// LOOP（spec 三细则）：count 0=无限跳；1=耗尽不跳（永停 1，走正常步进）；N=写回 N-1 跳。
    /// 跳转不设 wait、本帧到此为止（护栏）；target 越界（<0 或 ≥16）→ P4-b 终止。
    fn fire_loop(&mut self, i: usize, slot: XformSlot) -> FireResult {
        let target = slot.args[0];
        if !(0..SLOTS_PER_SEG as i32).contains(&target) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            return FireResult::Terminate;
        }
        let next = self.bullets.xform_next[i] as usize;
        let seg = self.bullets.transform_head[i];
        let count = self.xforms.seg_slots(seg)[next].args[1];
        match count {
            0 => { /* 无限 */ }
            1 => return FireResult::Continue, // 耗尽：不跳，落空走正常步进（wait 生效）
            n => self.xforms.seg_slots_mut(seg)[next].args[1] = n - 1,
        }
        self.bullets.xform_next[i] = target as u8;
        FireResult::Jumped
    }

    /// 推进本弹全部活跃 STEP 插值（升序，I4）。与游标并发：序列终止后插值照走完。
    fn tick_steps(&mut self, i: usize) {
        let seg = self.bullets.transform_head[i];
        if seg as usize >= SEG_CAP {
            // P4-b：伪造越界段号——advance_cursor 负责计数+终止，这里只需不 panic。
            return;
        }
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
        let e =
            crate::math::easing::ease(crate::math::easing::from_id((main.args[1] >> 16) as u8), t);
        if main.op == OP_STEP_SPEED {
            let (start, target) = (Fx::from_raw(ext.args[0]), Fx::from_raw(main.args[0]));
            let v = if done {
                target // 终帧写精确终值（不吃插值舍入）
            } else {
                // 负 i64 >> 为算术右移（向负无穷取整）——确定性，两平台一致
                let d = ((target.raw() as i64 - start.raw() as i64) * e.raw() as i64) >> 16;
                Fx::from_raw((start.raw() as i64 + d) as i32)
            };
            self.set_speed_at(i, v);
        } else {
            let start = Angle(ext.args[0] as u16);
            let delta = (main.args[0] as u16).wrapping_sub(start.raw()) as i16; // 最短弧带方向
            let scaled = if done {
                delta
            } else {
                // 算术右移，同上
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
}

#[cfg(test)]
mod tests {
    use crate::input::InputFrame;
    use crate::math::{Angle, Fx};
    use crate::xform::*;

    fn slot(wait: u16, op: u8, a0: i32, a1: i32) -> XformSlot {
        XformSlot {
            wait,
            op,
            _pad: 0,
            args: [a0, a1],
        }
    }

    /// 造一颗静止带段弹（远离自机），返回池索引（单弹场景恒 0）。
    fn xf_bullet(w: &mut crate::step::World, seq: &[XformSlot]) -> usize {
        let h = w.body.create_bullet_with_xform(
            crate::bullets::BulletInit {
                x: Fx::from_int(0),
                y: Fx::from_int(100),
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                speed: Fx::ZERO,
                angle: Angle::ZERO,
                ang_vel: 0,
                accel: Fx::ZERO,
                ax: Fx::ZERO,
                ay: Fx::ZERO,
                sprite: 0,
                radius: Fx::from_int(2),
                delay: 0,
                life: 0xFFFF,
                flags: 0,
                grazed_by: 0,
                transform_head: 0,
                xform_wait: 0,
                xform_next: 0,
            },
            seq,
        );
        w.body.bullets.get(h).unwrap()
    }

    /// wait 语义判别式（散文修正版）：wait=W ⇒ 下一 op 恰在 W 帧后发射。
    /// 帧 0 发 SET_SPEED（当帧生成当帧参与，相位 4 在相位 5 前）→ 帧 2 发 TURN（wait=2）。
    #[test]
    fn wait_semantics_fires_exactly_w_frames_later() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(2, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_TURN, 16384, 0), // +90°
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 帧0：SET_SPEED 发射
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(1));
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "帧0 TURN 不得发射");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // 帧1：wait 中
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "帧1 TURN 不得发射");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2)); // 帧2：TURN 发射
        assert_eq!(
            w.body.bullets.angle[i],
            Angle::QUARTER,
            "wait=2 应恰在 2 帧后发射"
        );
    }

    /// wait=0 同帧连发：三个 op 一帧全发。
    #[test]
    fn wait_zero_chains_same_frame() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(2).raw(), 0),
                slot(0, OP_TURN, 8192, 0),
                slot(0, OP_SET_SPRITE, 9, 0),
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(2));
        assert_eq!(w.body.bullets.angle[i], Angle(8192));
        assert_eq!(w.body.bullets.sprite[i], 9);
    }

    /// END 终止：其后的槽永不发射；终止后弹照常存活飞行。
    #[test]
    fn end_terminates_sequence() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_END, 0, 0),
                slot(0, OP_SET_SPEED, Fx::from_int(9).raw(), 0), // 死代码
            ],
        );
        for f in 0..5u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(1), "END 后不得再发射");
        assert!(w.body.bullets.is_alive(i), "终止 ≠ 弹死");
    }

    /// 伪造越界段号（绕过写 API 直接改 `transform_head`，非 XFORM_NONE 哨兵）：P4-b——
    /// contract_viol + 序列终止，不 panic；终止哨兵短路使第二次 step 不重复计数。
    #[test]
    fn forged_out_of_range_segment_terminates_without_panic() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_SET_SPRITE, 1, 0)]);
        w.body.bullets.transform_head[i] = 3000;
        let cv0 = w.body.diag.contract_viol;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "伪造越界段号恰计一次");
        assert_eq!(w.body.bullets.xform_next[i], 16, "序列终止（哨兵）");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(
            w.body.diag.contract_viol,
            cv0 + 1,
            "终止哨兵短路——第二次 step 不重复计数"
        );
    }

    /// 未知 op（运行期段被涂改出垃圾值）：P4-b——contract_viol + 序列终止，弹活着。
    #[test]
    fn unknown_op_terminates_and_counts() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(1, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_TURN, 100, 0),
            ],
        );
        let seg = w.body.bullets.transform_head[i];
        w.body.xforms.seg_slots_mut(seg)[1].op = 99; // 涂改成未知（族外垃圾值）
        let cv0 = w.body.diag.contract_viol;
        for f in 0..4u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "未知 op 恰计一次");
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "未知 op 后序列终止");
    }

    /// delay 门：delay 期变换不走（D3 已挡移动，这里挡游标）。
    #[test]
    fn delay_gate_blocks_transforms() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_SET_SPEED, Fx::from_int(3).raw(), 0)]);
        w.body.bullets.delay[i] = 2;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.speed[i], Fx::ZERO, "delay 期不发射");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3), "delay 尽后发射");
    }

    /// 连续开关 op：一帧内开 POLAR（SET_ANG_VEL）→ 开 CART（SET_GRAVITY 清 POLAR）→ STOP 清两位。
    #[test]
    fn continuous_ops_toggle_mode_bits() {
        use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_ANG_VEL, 512, 0),
                slot(0, OP_SET_GRAVITY, 0, 16384),
                slot(0, OP_STOP_FX, 0, 0),
                slot(0, OP_SET_ACCEL, 3277, 0),
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        let fl = w.body.bullets.flags[i];
        assert_ne!(fl & BULLET_POLAR_FX, 0, "终态 SET_ACCEL 开 POLAR");
        assert_eq!(fl & BULLET_CART_FX, 0);
        assert_eq!(w.body.bullets.ang_vel[i], 512);
        assert_eq!(w.body.bullets.ay[i].raw(), 16384);
        assert_eq!(w.body.bullets.accel[i].raw(), 3277);
    }

    /// LOOP 地板判别式：count=2 ⇒ 循环体恰执行 2 次（TURN 两次 = 半圈），耗尽停 1、不再跳。
    #[test]
    fn loop_count_two_executes_body_exactly_twice() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(1, OP_TURN, 16384, 0),   // 槽0：+90°，wait 1
                slot(1, OP_LOOP, 0, 2),       // 槽1：跳回槽0，count=2
                slot(0, OP_SET_SPRITE, 5, 0), // 槽2：耗尽落空后发射
            ],
        );
        for f in 0..10u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle::HALF, "TURN 恰两次 = 半圈");
        assert_eq!(w.body.bullets.sprite[i], 5, "耗尽后落空到槽2");
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(w.body.xforms.seg_slots(seg)[1].args[1], 1, "耗尽态永停 1");
    }

    /// LOOP count=0 无限：跑 60 帧角度持续推进、序列不终止。
    #[test]
    fn loop_count_zero_is_infinite() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(1, OP_TURN, 1024, 0), slot(0, OP_LOOP, 0, 0)]);
        for f in 0..60u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert!(w.body.bullets.xform_next[i] < 16, "无限循环不终止");
        assert_ne!(w.body.bullets.angle[i], Angle::ZERO, "持续转向");
    }

    /// LOOP 护栏：wait=0 的循环体每帧至多推进一轮（帧内不自旋）。
    #[test]
    fn loop_guard_bounds_one_round_per_frame() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_TURN, 1024, 0), // wait=0
                slot(0, OP_LOOP, 0, 0),    // 无限
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(
            w.body.bullets.angle[i],
            Angle(1024),
            "第一帧恰转一步——护栏生效"
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.angle[i], Angle(2048), "次帧恢复再转一步");
    }

    /// LOOP target 越界：P4-b——contract_viol + 序列终止。
    /// create 期已挡 target 越界（A1-3 还债，见 `create_bullet_with_xform` 的边界位图校验）——
    /// 这里改走"运行期段被涂改"绕过前门（create 期 target=0 合法：自环，同款自环已由
    /// `loop_guard_bounds_one_round_per_frame` 钉住护栏行为），钉住 fire 侧兜底仍在。
    #[test]
    fn loop_bad_target_terminates() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_LOOP, 0, 0)]); // create 期合法：target=0 自环
        let seg = w.body.bullets.transform_head[i];
        w.body.xforms.seg_slots_mut(seg)[0].args[0] = 16; // 涂改成越界 target
        let cv0 = w.body.diag.contract_viol;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.bullets.xform_next[i], 16, "序列终止");
    }

    /// 瞬时 op 全家判别：ADD_SPEED 累加、SET_ANGLE 置角、AIM 指向自机、SET_LIFE 重设寿命。
    #[test]
    fn instant_ops_discriminating_sweep() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_ADD_SPEED, Fx::from_int(2).raw(), 0), // → 3
                slot(0, OP_SET_ANGLE, 4096, 0),
                slot(0, OP_AIM_PLAYER, 0, 0), // 弹(0,100)→自机(0,384)：正下
                slot(0, OP_SET_LIFE, 7, 0),
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));
        let aim = crate::math::cordic::atan2(Fx::from_int(284), Fx::ZERO);
        assert_eq!(
            w.body.bullets.angle[i], aim,
            "AIM 覆盖 SET_ANGLE 且指向自机"
        );
        assert_eq!(
            w.body.bullets.life[i],
            7 - 1,
            "SET_LIFE=7 且本帧 integrate 已倒数 1"
        );
    }

    /// 边沿语义三连：停驻不动 → 当帧脉冲放行（同帧转向）→ 次帧不重复放行。
    #[test]
    fn wait_signal_edge_release() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_WAIT_SIGNAL, 3, 0),
                slot(0, OP_TURN, 16384, 0),
            ],
        );
        // 帧 0-1：无脉冲，停驻
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "无脉冲不得放行");
        // 帧 2：导演槽脉冲（step_with_director 在相位 2 调闭包）→ 相位 4 同帧放行
        crate::step::step_with_director(
            &mut w,
            &crate::tables::TABLES_V0,
            &crate::ecl::image::EclImage::empty(),
            &InputFrame::empty(2),
            |b| b.pulse_signal(3),
        );
        assert_eq!(w.body.bullets.angle[i], Angle::QUARTER, "当帧脉冲当帧放行");
        // 帧 3：无新脉冲——已放行的弹不受影响，且新停驻弹听不到旧脉冲
        let j = xf_bullet(
            &mut w,
            &[slot(0, OP_WAIT_SIGNAL, 3, 0), slot(0, OP_SET_SPRITE, 9, 0)],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(3));
        assert_eq!(
            w.body.bullets.sprite[j], 0,
            "旧脉冲是边沿不是电平：次帧不得放行"
        );
    }

    /// 坏通道号：create 期放行（op 合法），运行期 P4-b——计数 + 序列终止。
    #[test]
    fn wait_signal_bad_channel_terminates() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_WAIT_SIGNAL, 8, 0)]);
        let cv0 = w.body.diag.contract_viol;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
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

    /// STEP_SPEED 判别式：Linear 缓动 4 帧从 1.0 到 3.0——逐帧值与手算参考逐位相等，
    /// 第 4 帧后恰为精确终值且 active 清零。
    #[test]
    fn step_speed_linear_hits_exact_waypoints() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_STEP_SPEED, Fx::from_int(3).raw(), 4), // frames=4, easing=Linear(0)
                slot(0, OP_SET_SPRITE, 0, 0), // scratch 扩展槽由步进跳过——这里是槽 3
            ],
        );
        // 帧 0：SET_SPEED 发射 + STEP 发射（scratch 初始化，elapsed=0，本帧不 tick）
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(1), "发射帧不 tick");
        // 帧 1..4：每帧 +0.5（linear：1 + t×2，t = k/4）
        for k in 1..=4u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
            let expect = Fx::from_raw(65536 + (k as i32 * 2 * 65536) / 4);
            assert_eq!(w.body.bullets.speed[i], expect, "第 {k} tick");
        }
        // 完成：active 清零，速度停在精确终值
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(
            w.body.xforms.seg_slots(seg)[2].args[1] & (1 << 31),
            0,
            "active 应清"
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(5));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3), "完成后值冻结");
    }

    /// A1-(1) ARITY 步进判别：STEP 双槽——游标必须跳过扩展槽，直接发射其后的 op。
    /// 若步进错成 1，游标会把 scratch 当 op 读（垃圾/END）→ SET_SPRITE 永不发射。
    #[test]
    fn arity_stepping_skips_extension_slot() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_STEP_SPEED, Fx::from_int(2).raw(), 8),
                slot(0, OP_END, 0, 0), // 槽 1 = 扩展槽（发射时被 scratch 覆写）
                slot(0, OP_SET_SPRITE, 7, 0), // 槽 2：STEP 之后的下一个真 op
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.sprite[i], 7, "游标须按 1+ARITY 跳过扩展槽");
    }

    /// STEP_ANGLE 最短弧：从 350°(BAM 63715) 缓动到 10°(BAM 1820)——走 +20° 短弧而非 −340°。
    #[test]
    fn step_angle_shortest_arc_wraps() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_ANGLE, 63715, 0),
                slot(0, OP_STEP_ANGLE, 1820, 2), // 2 帧, Linear
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // t=0.5：中点应在回绕缝上
        let mid = w.body.bullets.angle[i].raw();
        assert!(
            !(1820..=63715).contains(&mid),
            "中点须在短弧上（跨 0），实际 {mid}"
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
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
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(5));
        assert_eq!(w.body.diag.contract_viol, cv0, "合法退化不计数");
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(
            w.body.xforms.seg_slots(seg)[1].args[1] & (1 << 31),
            0,
            "不激活"
        );
    }

    /// LOOP 重访 STEP = 自动重新武装（scratch 重初始化纪律）：第二轮从新起点插值。
    #[test]
    fn loop_rearms_step_from_new_start() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(2, OP_STEP_SPEED, Fx::from_int(2).raw(), 2), // 槽0；wait 2 让插值先跑完
                slot(0, OP_END, 0, 0), // 槽1：扩展槽占位（发射时被 scratch 覆写）
                slot(0, OP_ADD_SPEED, Fx::from_int(3).raw(), 0), // 槽2：完成后猛加 3（起点被改变）
                slot(1, OP_LOOP, 0, 2), // 槽3：跳回槽0，共 2 轮
            ],
        );
        for f in 0..12u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        // 第一轮：0→2（2帧）→ +3 = 5；第二轮重武装：5→2（2帧）→ +3 = 5；终态 5
        assert_eq!(
            w.body.bullets.speed[i],
            Fx::from_int(5),
            "第二轮须从 5 重新插到 2 再 +3"
        );
    }

    /// P4-b 兜底：STEP 在第 15 槽（无扩展槽空间）——发射期计数 + 终止，不 panic。
    /// create 期的空间校验已落地（A1-2 还债，见 `create_bullet_with_xform`）——这条路径
    /// 经写 API 已到不了这里；本测试改走"运行期段被涂改"绕过前门，钉住 fire 侧兜底仍在
    /// （与 `unknown_op_terminates_and_counts`/`loop_bad_target_terminates` 同款套路）。
    #[test]
    fn step_at_last_slot_terminates_without_panic() {
        let mut w = crate::step::World::new(1);
        let seq = [slot(0, OP_SET_SPRITE, 0, 0); 16];
        let i = xf_bullet(&mut w, &seq); // create 期合法：16 槽全瞬时 op
        let seg = w.body.bullets.transform_head[i];
        // 运行期涂改末槽为 STEP——前门已挡不了这条路，只能靠段直写模拟。
        w.body.xforms.seg_slots_mut(seg)[15] = slot(0, OP_STEP_SPEED, Fx::from_int(2).raw(), 4);
        let cv0 = w.body.diag.contract_viol;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 槽 0..15 全 wait=0 同帧连发
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "末槽 STEP 恰计一次");
        assert_eq!(w.body.bullets.xform_next[i], 16, "序列终止");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // 不再计数、不 panic
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }

    /// 招牌语义：`SET_COLOR` 保形、`SET_SHAPE` 保色。
    /// 判别力：若任一 op 退化成"整个 sprite = args[0]"（即 SET_SPRITE 的行为），
    /// 期望值 41/57 会变成 9/48，本测试立刻红。
    ///
    /// **直调 `run_transforms()`（相位 4）而非全量 `step_t`**：与 `integrate.rs`/
    /// `collide.rs`/`settle.rs` 里"设 phase_guard 后直调单相位函数"的先例同构——省去
    /// 无关相位（integrate 会因 speed=0 之外的原因扰动其它字段）的噪声。**slot 0 的 wait
    /// 故意设为 1（非 0）**：wait 语义是"发射本 op 后等 wait 帧再执行下一槽"，wait=0 会让
    /// 两个 op 在同一次 `run_transforms()` 调用内连锁触发（`wait_zero_chains_same_frame`
    /// 已钉死这个连锁行为）——本测试要拆成两次独立调用分别观测，必须让 slot 0 gate 住。
    #[test]
    fn set_color_preserves_shape_and_set_shape_preserves_color() {
        const STRIDE: i32 = 16;
        let mut w = crate::step::World::new(1);
        // 起点：第 2 形第 5 色 = 2*16+5 = 37
        let i = xf_bullet(
            &mut w,
            &[
                slot(1, OP_SET_COLOR, 9, STRIDE), // 只换色 → 2*16+9 = 41；wait=1 防同帧连锁
                slot(1, OP_SET_SHAPE, 3 * STRIDE, STRIDE), // 只换形 → 3*16+9 = 57
            ],
        );
        w.body.bullets.sprite[i] = (2 * STRIDE + 5) as u16;

        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_XFORM;
        }
        w.body.run_transforms();
        assert_eq!(w.body.bullets.sprite[i], 41, "SET_COLOR 必须保住形状位");

        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_XFORM;
        }
        w.body.run_transforms();
        assert_eq!(w.body.bullets.sprite[i], 57, "SET_SHAPE 必须保住颜色位");
    }

    /// P4-b：坏 stride（手工构造的槽，编译器不会产出）→ 计 contract_viol 且 sprite 不变，
    /// 不 panic、不除零。
    #[test]
    fn partial_sprite_ops_reject_bad_stride() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_SET_COLOR, 9, 0)]);
        w.body.bullets.sprite[i] = 37;
        let before = w.body.diag.contract_viol;

        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_XFORM;
        }
        w.body.run_transforms();

        assert_eq!(w.body.bullets.sprite[i], 37, "坏 stride 必须 no-op");
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "坏 stride 必须计一次契约违规"
        );
    }

    /// easing id 经 STEP 通路的判别：QuadIn(id=1) 2 帧从 1.0 到 3.0——
    /// t=0.5 时 QuadIn=0.25（烘焙表精确采样点），v = 1 + 2×0.25 = 1.5 逐位相等。
    #[test]
    fn step_speed_quadin_waypoint_exact() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_STEP_SPEED, Fx::from_int(3).raw(), 2 | (1 << 16)), // frames=2, QuadIn
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 发射帧不 tick
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // t=0.5 → 0.25
        assert_eq!(
            w.body.bullets.speed[i].raw(),
            98304,
            "1 + 2×QuadIn(0.5) = 1.5"
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3), "终值精确");
    }
}
