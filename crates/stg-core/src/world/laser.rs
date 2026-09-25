//! 激光写 API（P1：调用方只走这里，不直接碰池内存）。
use super::{POOL_LASER, STATUS_POOL_FULL, WorldBody};
use crate::lasers::*;
use crate::math::Fx;

impl WorldBody {
    /// 建一条激光。P4-a：池满 → NULL + 计数。P4-b：width/start/end/start_len/speed 为负 → 钳到 0 并计一次 contract_viol。
    /// 调用方给几何、外观、时长和 omega，其余字段由本函数定：state 按 warn 是否为 0、timer 0、不挂靠、
    /// 观测字段清零、px/py/pang = 初值、born_frame = 当前帧。
    pub fn create_laser(&mut self, mut init: LaserInit) -> LaserHandle {
        let mut bad = false;
        for v in [
            &mut init.width,
            &mut init.start,
            &mut init.end,
            &mut init.start_len,
            &mut init.speed,
        ] {
            if *v < Fx::ZERO {
                *v = Fx::ZERO;
                bad = true;
            }
        }
        if bad {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        init.state = if init.warn == 0 {
            LASER_ACTIVE
        } else {
            LASER_WARN
        };
        init.timer = 0;
        init.anchor_idx = ANCHOR_NONE;
        init.anchor_gen = 0;
        init.dx = Fx::ZERO;
        init.dy = Fx::ZERO;
        init.dang = 0;
        init.px = init.ox;
        init.py = init.oy;
        init.pang = init.angle;
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

#[cfg(test)]
mod tests {
    use crate::lasers::{ANCHOR_NONE, LASER_ACTIVE, LASER_WARN, LaserHandle, LaserInit, LaserPool};
    use crate::math::{Angle, Fx};
    use crate::step::World;
    use crate::world::{POOL_LASER, STATUS_POOL_FULL};

    /// 一条可用的激光初始值（`warn` 决定出生状态）。
    fn init(warn: u16) -> LaserInit {
        LaserInit {
            ox: Fx::from_int(10),
            oy: Fx::from_int(20),
            angle: Angle::QUARTER,
            omega: 0,
            start: Fx::ZERO,
            end: Fx::from_int(100),
            start_len: Fx::from_int(100),
            speed: Fx::ZERO,
            width: Fx::from_int(16),
            sprite: 0,
            warn,
            active: 60,
            fade: 0,
            timer: 0,
            state: 0,
            anchor_idx: 0,
            anchor_gen: 0,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            dang: 0,
            px: Fx::ZERO,
            py: Fx::ZERO,
            pang: Angle::ZERO,
            flags: 0,
            born_frame: 0,
        }
    }

    /// `warn == 0` 出生即 state 1；`warn != 0` 出生 state 0。顺带钉死 `create_laser` 的
    /// 派生字段：timer 0、不挂靠、观测清零、px/py/pang = 初值、born_frame = 当前帧。
    #[test]
    fn warn_zero_spawns_active_else_warn() {
        let mut w = World::new(1);
        w.body.frame = 7;
        let ha = w.body.create_laser(init(0));
        let p = w.body.view().lasers();
        let ia = p.get(ha).unwrap();
        assert_eq!(p.state[ia], LASER_ACTIVE, "warn==0 出生即生效");
        assert_eq!(p.timer[ia], 0);
        assert_eq!(p.anchor_idx[ia], ANCHOR_NONE, "出生不挂靠");
        assert_eq!(p.anchor_gen[ia], 0);
        assert_eq!(p.px[ia], Fx::from_int(10), "px = ox");
        assert_eq!(p.py[ia], Fx::from_int(20), "py = oy");
        assert_eq!(p.pang[ia], Angle::QUARTER, "pang = angle");
        assert_eq!(p.born_frame[ia], 7, "born_frame = 当前帧");

        let hw = w.body.create_laser(init(30));
        let iw = w.body.lasers.get(hw).unwrap();
        assert_eq!(w.body.lasers.state[iw], LASER_WARN);
    }

    /// P4-a：池满第 257 条返回 NULL，`pool_full[POOL_LASER]` 加 1，last_status = POOL_FULL。
    #[test]
    fn pool_full_returns_null_and_counts() {
        let mut w = World::new(1);
        for k in 0..LaserPool::CAP {
            assert_ne!(
                w.body.create_laser(init(0)),
                LaserHandle::NULL,
                "第 {k} 条应成功"
            );
        }
        let h = w.body.create_laser(init(0));
        assert_eq!(h, LaserHandle::NULL, "池满 → NULL");
        assert_eq!(w.body.view().diag().pool_full[POOL_LASER], 1);
        assert_eq!(w.body.view().last_status(), STATUS_POOL_FULL);
    }

    /// P4-b：width/start/end/start_len/speed 为负 → 逐个钳到 0，`contract_viol` 只加 1
    /// （一次调用多个坏字段不重复计数）。
    #[test]
    fn negative_params_clamped_and_counted_once() {
        let mut w = World::new(1);
        let mut it = init(0);
        it.width = Fx::from_int(-5);
        it.start = Fx::from_int(-1);
        it.end = Fx::from_int(-2);
        it.start_len = Fx::from_int(-3);
        it.speed = Fx::from_int(-4);
        let h = w.body.create_laser(it);
        let i = w.body.lasers.get(h).unwrap();
        let p = &w.body.lasers;
        assert_eq!(p.width[i], Fx::ZERO);
        assert_eq!(p.start[i], Fx::ZERO);
        assert_eq!(p.end[i], Fx::ZERO);
        assert_eq!(p.start_len[i], Fx::ZERO);
        assert_eq!(p.speed[i], Fx::ZERO);
        assert_eq!(w.body.view().diag().contract_viol, 1, "多坏字段只计一次");
    }

    /// 快照往返 + 校验和敏感（照 `step.rs::snapshot_covers_item_pool` 写法）：
    /// `copy_into` 漏拷 `lasers` 即红，字段不入校验和也红。
    #[test]
    fn snapshot_covers_laser_pool() {
        let mut w = World::new(3);
        w.body.create_laser(init(0));
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "快照必须带 lasers 全部字节");
        snap.body.lasers.width[0] = snap.body.lasers.width[0] - Fx::ONE;
        assert_ne!(snap.checksum(), ck, "lasers 必须真的参与校验和");
    }
}
