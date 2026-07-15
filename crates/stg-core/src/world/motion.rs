//! D3 运动写 API：setter 双层（公开 handle 层 + pub(crate) 索引核）+ 极坐标回填核。
//! 模型见 stg-world-design.md D3；本刀拍板见 specs/2026-07-15-d3-dual-representation-design.md。

use super::WorldBody;
use crate::bullets::BulletHandle;
use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
use crate::math::Angle;
use crate::math::Fx;
use crate::math::cordic::atan2;
use crate::math::geom::{len_sq, polar_to_vec};
use crate::math::isqrt::isqrt;
use crate::world::STATUS_STALE_HANDLE;

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

#[cfg(test)]
mod tests {
    use crate::bullets::{BULLET_CART_FX, BULLET_POLAR_FX};
    use crate::math::geom::polar_to_vec;
    use crate::math::{Angle, Fx};
    use crate::world::STATUS_STALE_HANDLE;
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
        assert_eq!(
            w.body.bullets.angle[0],
            crate::math::cordic::atan2(Fx::from_int(2), Fx::ZERO)
        );
        w.body.turn_bullet(h, Angle::QUARTER);
        let (rvx, rvy) = polar_to_vec(w.body.bullets.speed[0], w.body.bullets.angle[0]);
        assert_eq!((w.body.bullets.vx[0], w.body.bullets.vy[0]), (rvx, rvy));
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
