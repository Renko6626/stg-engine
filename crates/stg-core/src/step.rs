//! 组装层（stg_core::step）—— §3.5/A4 宪法顺序的唯一持有者（P2）。World 定义 + 构造 + step。

use std::alloc::{Layout, alloc_zeroed, handle_alloc_error};

use crate::rng::Pcg32;
use crate::world::{PH_DIRECTOR, PH_ECL_HOOK, RNG_SEQ, WorldBody};

/// 权威可变状态。M1 起加 `pub tasks: TaskPool`。
#[repr(C)]
#[derive(crate::checksum::Checksum)]
pub struct World {
    pub body: WorldBody,
}

impl World {
    /// 堆零初始化的新世界，再播种 rng。
    ///
    /// **为何堆零构造 + 一处 unsafe**：`World` 是 POD（全整数/数组、无 Drop、无引用、无枚举无效
    /// 判别式），**全零是合法值**（空池 alive 全 0、gen/字段 0、frame 0、diag 0）。按值构造 ~450KB
    /// 的 World 会在栈上放巨型临时量（debug 未优化时栈溢出）；`alloc_zeroed` 直接在堆上零构造，
    /// 避开栈。这是 stg-core 唯一一处 unsafe，也是设计既定的 World 堆分配落点。
    pub fn new(seed: u64) -> Box<World> {
        let layout = Layout::new::<World>();
        // SAFETY: World 全零合法（见上）；layout 由类型给出；分配失败走 handle_alloc_error；
        // Box::from_raw 接管同一全局分配器的这块内存，Drop 时正确释放。
        let mut w: Box<World> = unsafe {
            let ptr = alloc_zeroed(layout) as *mut World;
            if ptr.is_null() {
                handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        };
        w.body.rng = Pcg32::new(seed, RNG_SEQ);
        w.body.players[0] = crate::player::PlayerState::spawn(0); // 自机 1 出场；自机 2 保持全零=不在场
        w
    }

    /// 整块快照（安全逐字段，I7/D11）。
    pub fn copy_into(&self, dst: &mut World) {
        let s = &self.body;
        let d = &mut dst.body;
        d.frame = s.frame;
        d.rng = s.rng;
        s.bullets.copy_into(&mut d.bullets);
        d.players = s.players; // [PlayerState; N] 是 Copy
        s.shots.copy_into(&mut d.shots);
        s.enemies.copy_into(&mut d.enemies);
        s.fields.copy_into(&mut d.fields);
        s.xforms.copy_into(&mut d.xforms);
        d.signals = s.signals;
        d.diag = s.diag;
        d.last_status = s.last_status;
        // 帧内私有输出缓冲（hits/events）checksum-skip、不随快照复制数组本体——安全性今天靠
        // "begin 在任何生产者跑之前清 len" 这条相位顺序撑着。但 events 是 pub，规格给了两个未来
        // 消费者（phase-8 ECL 钩子、M2 表现层）；若表现层在 rollback 恢复后读到清旧数组前的
        // events，会重放刚回滚掉的帧里的"幽灵死亡"。显式清 len，把这条从相位顺序的巧合变成
        // 明写的契约：恢复出的 World 必须无陈旧输出。
        d.hits_len = 0;
        d.events_len = 0;
        #[cfg(debug_assertions)]
        {
            d.phase_guard = s.phase_guard;
        }
    }

    #[inline]
    pub fn checksum(&self) -> u64 {
        crate::checksum::Checksum::checksum(self)
    }
}

/// 空导演 = 纯世界模拟（P2：空租户零次循环）。
pub fn step(world: &mut World, input: &crate::input::InputFrame) {
    step_with_director(world, input, |_| {});
}

/// §3.5 宪法顺序（导演槽在 step-3 跑一次）。相位由 world 出，顺序由此焊死，PhaseGuard 押运。
pub fn step_with_director<F: FnMut(&mut WorldBody)>(
    world: &mut World,
    input: &crate::input::InputFrame,
    mut director: F,
) {
    let b = &mut world.body;
    b.begin(); // 0
    b.decode_input(input); // 1
    b.phase_enter(PH_DIRECTOR); // 2：导演槽（护栏在组装层押）
    director(b);
    b.update_players(); // 3
    b.run_transforms(); // 4
    b.integrate(); // 5
    b.collide(); // 6
    b.settle(); // 7
    b.phase_enter(PH_ECL_HOOK); // 8：ECL 事件挂钩槽（M0-4 空）
    b.cleanup(); // 9
    b.advance(); // 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bullets::{BulletHandle, BulletInit, BulletPool};
    use crate::input::InputFrame;
    use crate::math::{Angle, Fx};
    use crate::world::{POOL_BULLET, STATUS_POOL_FULL};

    fn straight(x: i32, y: i32, vx: i32, vy: i32, life: u16) -> BulletInit {
        BulletInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            vx: Fx::from_int(vx),
            vy: Fx::from_int(vy),
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            ang_vel: 0,
            accel: Fx::ZERO,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            sprite: 0,
            radius: Fx::from_int(2),
            delay: 0,
            life,
            flags: 0,
            grazed_by: 0,
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        }
    }

    #[test]
    fn create_bullet_ok_and_pool_full() {
        let mut w = World::new(1);
        assert_ne!(
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF)),
            BulletHandle::NULL
        );
        for _ in 0..(BulletPool::CAP - 1) {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        assert_eq!(
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF)),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], 1);
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    fn slot(wait: u16, op: u8, a0: i32, a1: i32) -> crate::xform::XformSlot {
        crate::xform::XformSlot {
            wait,
            op,
            _pad: 0,
            args: [a0, a1],
        }
    }

    /// 成功路径：序列拷贝进段、尾部清零（复用段的陈值不可泄漏）、transform_head 被覆写。
    #[test]
    fn create_with_xform_copies_and_zero_fills_tail() {
        let mut w = World::new(1);
        // 先污染 0 号段（占用→写脏→还段），验证复用时尾零
        let s0 = w.body.xforms.alloc().unwrap();
        for sl in w.body.xforms.seg_slots_mut(s0) {
            sl.args[0] = -1;
        }
        w.body.xforms.free(s0);
        let seq = [slot(3, crate::xform::OP_SET_SPEED, 65536, 0)];
        let h = w
            .body
            .create_bullet_with_xform(straight(0, 0, 0, 0, 0xFFFF), &seq);
        assert_ne!(h, BulletHandle::NULL);
        let i = w.body.bullets.get(h).unwrap();
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(seg, 0, "最低空段");
        let slots = w.body.xforms.seg_slots(seg);
        assert_eq!(slots[0], seq[0]);
        assert!(
            slots[1..].iter().all(|s| *s == Default::default()),
            "尾部必须清零 = 天然 END"
        );
    }

    /// 坏参整体失败：>16 槽 / 含未知 op → NULL + BAD_ARGS 计数 + 零副作用（弹与段都不产生）。
    #[test]
    fn create_with_xform_bad_args_total_failure() {
        let mut w = World::new(1);
        let long = [slot(0, crate::xform::OP_SET_SPEED, 1, 0); 17];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &long),
            BulletHandle::NULL
        );
        let unknown = [slot(0, 99, 0, 0)]; // 99 = 族外垃圾值，未实现
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &unknown),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.contract_viol, 2);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "无段泄漏");
    }

    /// 段满 → NULL + POOL_FULL(XFORM)，零副作用。
    #[test]
    fn create_with_xform_segpool_full() {
        let mut w = World::new(1);
        for _ in 0..crate::xform::SEG_CAP {
            w.body.xforms.alloc().unwrap();
        }
        let seq = [slot(0, crate::xform::OP_SET_SPEED, 1, 0)];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[crate::world::POOL_XFORM], 1);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "宁缺勿哑：弹也不产生"
        );
    }

    /// 弹池满 → 还段回滚（先段后弹的另一半）。
    #[test]
    fn create_with_xform_bulletpool_full_rolls_back_segment() {
        let mut w = World::new(1);
        for _ in 0..BulletPool::CAP {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        let seq = [slot(0, crate::xform::OP_SET_SPEED, 1, 0)];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], 1);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "段已回滚归还");
    }

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
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
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
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
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
            slot(0, 0, 0, 0),                               // 扩展槽
            slot(0, crate::xform::OP_LOOP, 1, 0),           // target=1 = 扩展槽中间 → 拒
        ];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &bad),
            BulletHandle::NULL
        );
        let ok = [
            slot(0, crate::xform::OP_SET_SPRITE, 1, 0),
            slot(0, crate::xform::OP_LOOP, 10, 3), // target=10 在 zero-tail：落地即 END，合法
        ];
        assert_ne!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &ok),
            BulletHandle::NULL
        );
    }

    /// easing id ≥ 8 → BAD_ARGS（作者错误 create 期就拒）。
    #[test]
    fn create_rejects_bad_easing_id() {
        let mut w = World::new(1);
        let seq = [slot(0, crate::xform::OP_STEP_SPEED, 65536, 4 | (8 << 16))];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
    }

    /// create_bullet（哑弹路径）覆写 transform_head——调用方伪造段号无效。
    #[test]
    fn create_bullet_overrides_forged_transform_head() {
        let mut w = World::new(1);
        let mut init = straight(0, 0, 0, 0, 0xFFFF);
        init.transform_head = 7; // 伪造
        let h = w.body.create_bullet(init);
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.transform_head[i], crate::xform::XFORM_NONE);
    }

    #[test]
    fn new_is_deterministic_and_seed_matters() {
        assert_eq!(World::new(42).checksum(), World::new(42).checksum());
        assert_ne!(World::new(1).checksum(), World::new(2).checksum()); // 种子入 rng 入校验和
    }

    #[test]
    fn new_spawns_player0_alive() {
        use crate::player::{LIFE_ABSENT, LIFE_ALIVE};
        let w = World::new(1);
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, 3);
        assert_eq!(w.body.players[0].y, Fx::from_int(384));
        assert_eq!(w.body.players[1].life_state, LIFE_ABSENT); // 第 2 人不在场
    }

    #[test]
    fn player_moves_right_with_input() {
        use crate::input::BTN_RIGHT;
        let mut w = World::new(1);
        let x0 = w.body.players[0].x.raw();
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_RIGHT;
        step(&mut w, &f);
        assert!(w.body.players[0].x.raw() > x0); // 右移
    }

    #[test]
    fn player_shot_fires_on_button() {
        use crate::input::BTN_SHOT;
        let mut w = World::new(1);
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_SHOT;
        step(&mut w, &f);
        assert_eq!(w.body.shots.iter_alive().count(), 1); // shot_cd 从 0 → 发 1 发
    }

    #[test]
    fn player_clamped_to_left_edge() {
        use crate::input::BTN_LEFT;
        let mut w = World::new(1);
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_LEFT;
        for _ in 0..200 {
            step(&mut w, &f); // 一直左移
        }
        assert_eq!(w.body.players[0].x, Fx::from_int(-192)); // 钳到左边界
    }

    #[test]
    fn player_deterministic_with_scripted_input() {
        use crate::input::{BTN_RIGHT, BTN_SHOT};
        let run = || {
            let mut w = World::new(9);
            let mut cks = Vec::new();
            for frame in 0..60u32 {
                let mut f = InputFrame::empty(frame);
                f.actions[0].buttons = BTN_RIGHT | BTN_SHOT;
                step(&mut w, &f);
                cks.push(w.checksum());
            }
            cks
        };
        assert_eq!(run(), run()); // 玩家 + 自机弹演化确定
    }

    #[test]
    fn integrate_moves_and_advances_frame() {
        let mut w = World::new(1);
        let h = w.body.create_bullet(straight(0, 0, 1, 2, 0xFFFF));
        step(&mut w, &InputFrame::empty(0));
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.x[i], Fx::from_int(1));
        assert_eq!(w.body.bullets.y[i], Fx::from_int(2));
        assert_eq!(w.body.frame, 1);
    }

    #[test]
    fn cleanup_frees_out_of_bounds_and_expired() {
        let mut w = World::new(1);
        let h_far = w.body.create_bullet(straight(1000, 0, 0, 0, 0xFFFF)); // 越界
        let h_life = w.body.create_bullet(straight(0, 0, 0, 0, 1)); // 寿命 1
        step(&mut w, &InputFrame::empty(0)); // life:1→0(integrate)，cleanup 释放两者
        assert_eq!(w.body.bullets.get(h_far), None);
        assert_eq!(w.body.bullets.get(h_life), None);
    }

    #[test]
    fn deterministic_replay() {
        let run = || {
            let mut w = World::new(7);
            let mut cks = Vec::new();
            for _ in 0..50u32 {
                step_with_director(&mut w, &InputFrame::empty(0), |b| {
                    let vx = b.rng.rand_range(5) as i32 - 2;
                    b.create_bullet(straight(0, 0, vx, 3, 100));
                });
                cks.push(w.checksum());
            }
            cks
        };
        assert_eq!(run(), run()); // 同种子+同导演 → 逐帧 checksum 全等
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let mut w = World::new(3);
        for _ in 0..10 {
            step_with_director(&mut w, &InputFrame::empty(0), |b| {
                b.create_bullet(straight(0, 0, 1, 1, 200));
            });
        }
        let snap_ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), snap_ck);
        step(&mut w, &InputFrame::empty(0));
        assert_ne!(w.checksum(), snap_ck);
        snap.copy_into(&mut w); // 恢复
        assert_eq!(w.checksum(), snap_ck);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "相位乱序")]
    fn phase_guard_catches_out_of_order() {
        let mut w = World::new(1);
        w.body.advance(); // 不经 begin 直接 advance → 护栏 panic
    }

    #[test]
    fn snapshot_roundtrip_covers_xform_pool() {
        let mut w = World::new(3);
        let seg = w.body.xforms.alloc().unwrap();
        w.body.xforms.seg_slots_mut(seg)[0].args[0] = 42;
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck); // 段池随快照
        snap.body.xforms.seg_slots_mut(seg)[0].args[0] = 43;
        assert_ne!(snap.checksum(), ck); // 且真的在参与指纹
    }

    #[test]
    fn snapshot_covers_signals() {
        let mut w = World::new(3);
        w.body.pulse_signal(5);
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "signals 随快照且入校验和");
        snap.body.signals[5] ^= 1;
        assert_ne!(
            snap.checksum(),
            ck,
            "signals 必须真的参与校验和（防未来误加 skip）"
        );
    }
}
