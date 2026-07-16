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
            match self.fire_op(i, slot) {
                FireResult::Terminate => {
                    self.bullets.xform_next[i] = SLOTS_PER_SEG as u8;
                    return;
                }
                FireResult::Jumped => return, // LOOP 护栏：本帧到此为止
                FireResult::Continue => {}
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
            OP_SET_LIFE => self.bullets.life[i] = slot.args[0] as u16,
            OP_SET_ANG_VEL => self.set_ang_vel_at(i, slot.args[0] as i16),
            OP_SET_ACCEL => self.set_accel_at(i, Fx::from_raw(slot.args[0])),
            OP_SET_GRAVITY => {
                self.set_gravity_at(i, Fx::from_raw(slot.args[0]), Fx::from_raw(slot.args[1]))
            }
            OP_STOP_FX => self.stop_fx_at(i),
            OP_LOOP => return self.fire_loop(i, slot),
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
        crate::step::step(&mut w, &InputFrame::empty(0)); // 帧0：SET_SPEED 发射
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(1));
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "帧0 TURN 不得发射");
        crate::step::step(&mut w, &InputFrame::empty(1)); // 帧1：wait 中
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "帧1 TURN 不得发射");
        crate::step::step(&mut w, &InputFrame::empty(2)); // 帧2：TURN 发射
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
        crate::step::step(&mut w, &InputFrame::empty(0));
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
            crate::step::step(&mut w, &InputFrame::empty(f));
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
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "伪造越界段号恰计一次");
        assert_eq!(w.body.bullets.xform_next[i], 16, "序列终止（哨兵）");
        crate::step::step(&mut w, &InputFrame::empty(1));
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
            crate::step::step(&mut w, &InputFrame::empty(f));
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
        crate::step::step(&mut w, &InputFrame::empty(0));
        crate::step::step(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.speed[i], Fx::ZERO, "delay 期不发射");
        crate::step::step(&mut w, &InputFrame::empty(2));
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
        crate::step::step(&mut w, &InputFrame::empty(0));
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
            crate::step::step(&mut w, &InputFrame::empty(f));
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
            crate::step::step(&mut w, &InputFrame::empty(f));
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
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(
            w.body.bullets.angle[i],
            Angle(1024),
            "第一帧恰转一步——护栏生效"
        );
        crate::step::step(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.angle[i], Angle(2048), "次帧恢复再转一步");
    }

    /// LOOP target 越界：P4-b——contract_viol + 序列终止。
    #[test]
    fn loop_bad_target_terminates() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_LOOP, 16, 0)]);
        let cv0 = w.body.diag.contract_viol;
        crate::step::step(&mut w, &InputFrame::empty(0));
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
        crate::step::step(&mut w, &InputFrame::empty(0));
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
}
