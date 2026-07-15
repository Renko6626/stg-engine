//! D3 运动写 API：setter 双层（公开 handle 层 + pub(crate) 索引核）+ 极坐标回填核。
//! 模型见 stg-world-design.md D3；本刀拍板见 specs/2026-07-15-d3-dual-representation-design.md。

use super::WorldBody;
use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
use crate::math::Fx;
use crate::math::cordic::atan2;
use crate::math::geom::{len_sq, polar_to_vec};
use crate::math::isqrt::isqrt;

/// 低速回填阈值 = 1/16 px/帧。契约常量：`speed` 恒回填、`angle` 仅 `speed >= 此值` 时回填
/// （近停冻结朝向：防 CORDIC 低幅垃圾角污染作者视图与 sprite 朝向）。
/// 改值 = 确定性契约变更，须过评审（spec 2026-07-15）。
pub const BACKFILL_MIN_SPEED: Fx = Fx::from_raw(4096);

impl WorldBody {
    /// 极坐标 → 积分真相：`(vx,vy) = polar_to_vec(speed, angle)`。
    /// 一切改动 speed/angle 的路径改完必须调它（"忘了回填"火药桶的唯一出口）。
    #[inline]
    pub(crate) fn refresh_vel_from_polar(&mut self, i: usize) {
        let (vx, vy) = polar_to_vec(self.bullets.speed[i], self.bullets.angle[i]);
        self.bullets.vx[i] = vx;
        self.bullets.vy[i] = vy;
    }

    // D3 切片 Task 2/3：`refresh_vel_from_polar`/`backfill_polar` 已分别被 integrate.rs 的
    // POLAR_FX/CART_FX 分支接入生产路径，摘掉了 allow。以下三个 setter 仍只有测试调用点——
    // `set_gravity_at` 只设字段+模式位、integrate 直接读 ax/ay 不经它，故仍无生产调用点；
    // `set_ang_vel_at`/`set_accel_at`/`stop_fx_at` 等后续 ECL syscall 任务接入，届时逐个摘。

    /// 开 POLAR_FX（清 CART_FX，互斥律）；只动两模式位。
    #[allow(dead_code)]
    pub(crate) fn set_ang_vel_at(&mut self, i: usize, w: i16) {
        self.bullets.ang_vel[i] = w;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_POLAR_FX) & !BULLET_CART_FX;
    }

    /// 沿向加速，开 POLAR_FX（清 CART_FX）。
    #[allow(dead_code)]
    pub(crate) fn set_accel_at(&mut self, i: usize, a: Fx) {
        self.bullets.accel[i] = a;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_POLAR_FX) & !BULLET_CART_FX;
    }

    /// 笛卡尔加速（重力/漂移），开 CART_FX（清 POLAR_FX）。
    #[allow(dead_code)]
    pub(crate) fn set_gravity_at(&mut self, i: usize, ax: Fx, ay: Fx) {
        self.bullets.ax[i] = ax;
        self.bullets.ay[i] = ay;
        self.bullets.flags[i] = (self.bullets.flags[i] | BULLET_CART_FX) & !BULLET_POLAR_FX;
    }

    /// 清两模式位（字段留陈值，确定性无损——ZUN 语义只关开关）。
    #[allow(dead_code)]
    pub(crate) fn stop_fx_at(&mut self, i: usize) {
        self.bullets.flags[i] &= !(BULLET_POLAR_FX | BULLET_CART_FX);
    }

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
        assert_eq!(
            w.body.bullets.flags[0] & BULLET_POLAR_FX,
            0,
            "开 CART 应清 POLAR"
        );
        assert_ne!(w.body.bullets.flags[0] & BULLET_CART_FX, 0);
        w.body.set_accel_at(0, Fx::from_raw(100));
        assert_ne!(
            w.body.bullets.flags[0] & BULLET_POLAR_FX,
            0,
            "开 POLAR 应置位"
        );
        assert_eq!(
            w.body.bullets.flags[0] & BULLET_CART_FX,
            0,
            "开 POLAR 应清 CART"
        );
        w.body.stop_fx_at(0);
        assert_eq!(
            w.body.bullets.flags[0] & (BULLET_POLAR_FX | BULLET_CART_FX),
            0
        );
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
        assert_eq!(
            w.body.bullets.angle[0],
            crate::math::cordic::atan2(Fx::ZERO, Fx::from_raw(8192))
        );
    }
}
