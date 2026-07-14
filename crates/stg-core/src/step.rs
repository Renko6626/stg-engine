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
        w
    }

    /// 整块快照（安全逐字段，I7/D11）。
    pub fn copy_into(&self, dst: &mut World) {
        let s = &self.body;
        let d = &mut dst.body;
        d.frame = s.frame;
        d.rng = s.rng;
        s.bullets.copy_into(&mut d.bullets);
        d.diag = s.diag;
        d.last_status = s.last_status;
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
pub fn step(world: &mut World) {
    step_with_director(world, |_| {});
}

/// §3.5 宪法顺序（导演槽在 step-3 跑一次）。相位由 world 出，顺序由此焊死，PhaseGuard 押运。
pub fn step_with_director<F: FnMut(&mut WorldBody)>(world: &mut World, mut director: F) {
    let b = &mut world.body;
    b.begin(); // 0
    b.decode_input(); // 1
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

    #[test]
    fn new_is_deterministic_and_seed_matters() {
        assert_eq!(World::new(42).checksum(), World::new(42).checksum());
        assert_ne!(World::new(1).checksum(), World::new(2).checksum()); // 种子入 rng 入校验和
    }

    #[test]
    fn integrate_moves_and_advances_frame() {
        let mut w = World::new(1);
        let h = w.body.create_bullet(straight(0, 0, 1, 2, 0xFFFF));
        step(&mut w);
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
        step(&mut w); // life:1→0(integrate)，cleanup 释放两者
        assert_eq!(w.body.bullets.get(h_far), None);
        assert_eq!(w.body.bullets.get(h_life), None);
    }

    #[test]
    fn deterministic_replay() {
        let run = || {
            let mut w = World::new(7);
            let mut cks = Vec::new();
            for _ in 0..50u32 {
                step_with_director(&mut w, |b| {
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
            step_with_director(&mut w, |b| {
                b.create_bullet(straight(0, 0, 1, 1, 200));
            });
        }
        let snap_ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), snap_ck);
        step(&mut w);
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
}
