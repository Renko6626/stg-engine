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
use crate::player::{LIFE_ABSENT, LIFE_GAMEOVER};
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

    /// 改速率并回填 v（索引核；D4 op SET_SPEED/ADD_SPEED 与公开 setter 共用）。
    pub(crate) fn set_speed_at(&mut self, i: usize, speed: Fx) {
        self.bullets.speed[i] = speed;
        self.refresh_vel_from_polar(i);
    }

    /// 改朝向并回填 v（索引核；D4 op 与公开 setter 共用）。
    pub(crate) fn set_angle_at(&mut self, i: usize, angle: Angle) {
        self.bullets.angle[i] = angle;
        self.refresh_vel_from_polar(i);
    }

    /// 相对转向（回绕加）并回填 v（索引核；D4 op 与公开 setter 共用）。
    pub(crate) fn turn_at(&mut self, i: usize, delta: Angle) {
        self.bullets.angle[i] = self.bullets.angle[i].add(delta);
        self.refresh_vel_from_polar(i);
    }

    /// 改速率并回填 v（D3 极坐标 setter）。悬垂句柄 no-op。
    pub fn set_bullet_speed(&mut self, h: BulletHandle, speed: Fx) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.set_speed_at(i, speed);
        }
    }

    /// 改朝向并回填 v。
    pub fn set_bullet_angle(&mut self, h: BulletHandle, angle: Angle) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.set_angle_at(i, angle);
        }
    }

    /// 相对转向（回绕加）并回填 v。
    pub fn turn_bullet(&mut self, h: BulletHandle, delta: Angle) {
        if let Some(i) = self.bullet_index_checked(h) {
            self.turn_at(i, delta);
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

    /// 最近可瞄自机：平方距离最小、并列取低索引（I4：升序遍历 + 严格小于才替换）。
    /// 可瞄 = 非 ABSENT 且非 GAMEOVER（决死窗口/重生无敌期仍在场上，照瞄——ZUN 语义）。
    pub(crate) fn nearest_aimable_player(&self, x: Fx, y: Fx) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for p in 0..crate::MAX_PLAYERS {
            let st = self.players[p].life_state;
            if st == LIFE_ABSENT || st == LIFE_GAMEOVER {
                continue;
            }
            let d2 = len_sq(self.players[p].x - x, self.players[p].y - y);
            if best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((p, d2));
            }
        }
        best.map(|(p, _)| p)
    }

    /// 瞄最近可瞄自机 + delta 偏移，回填 v（索引核；D4 op 与公开 setter 共用）。
    /// 无可瞄自机 → 纯 no-op（不计数）。
    pub(crate) fn aim_at_player_at(&mut self, i: usize, delta: Angle) {
        let Some(p) = self.nearest_aimable_player(self.bullets.x[i], self.bullets.y[i]) else {
            return;
        };
        let dx = self.players[p].x - self.bullets.x[i];
        let dy = self.players[p].y - self.bullets.y[i];
        self.bullets.angle[i] = crate::math::cordic::atan2(dy, dx).add(delta);
        self.refresh_vel_from_polar(i);
    }

    /// 瞄最近可瞄自机 + delta 偏移，回填 v。无可瞄自机 → 纯 no-op（不计数）。
    pub fn aim_bullet_at_player(&mut self, h: BulletHandle, delta: Angle) {
        let Some(i) = self.bullet_index_checked(h) else {
            return;
        };
        self.aim_at_player_at(i, delta);
    }

    /// 四条速度动词共用的前置校验（P4-b）：悬垂 → None + 计数；`easing >= 8` → None + 计数。
    /// **两条都必须在任何字段落地之前**——坏参数是 no-op，不能留半个武装好的插值器。
    fn enemy_vel_precheck(&mut self, h: crate::enemy::EnemyHandle, easing: u8) -> Option<usize> {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return None;
        };
        if easing >= 8 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = crate::world::STATUS_BAD_ARGS;
            return None;
        }
        Some(i)
    }

    /// 武装速度插值器（`dur > 0` 专用；`dur == 0` 的瞬时路径由各 setter 自己走完）。
    /// `from_*` 一律取**当前**值 —— 重新武装 = 从此刻重新起算（同 `STEP_*` 的
    /// "scratch 无条件重初始化"）。
    #[allow(clippy::too_many_arguments)] // 插值器武装的天然参数面（同 create_bullets_batch 先例）
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
        h: crate::enemy::EnemyHandle,
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
    pub fn set_enemy_vel_cart(
        &mut self,
        h: crate::enemy::EnemyHandle,
        vx: Fx,
        vy: Fx,
        dur: u16,
        easing: u8,
    ) {
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
    /// **不委托** `set_enemy_vel_polar`——自己 precheck 一次，避免坏参数被计两次
    /// `contract_viol`（该计数参与校验和，多计一次即行为 bug，见 brief 裁定）。
    pub fn set_enemy_angle(
        &mut self,
        h: crate::enemy::EnemyHandle,
        angle: Angle,
        dur: u16,
        easing: u8,
    ) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        self.enemies.vel_touched[i] = 1;
        let speed = self.enemies.speed[i];
        if dur == 0 {
            self.enemies.angle[i] = angle;
            self.refresh_enemy_vel_from_polar(i);
            self.enemies.vel_active[i] = 0;
            return;
        }
        self.arm_enemy_vel(
            i,
            crate::enemy::VEL_SPACE_POLAR,
            speed.raw(),
            self.enemies.angle[i].raw() as i32,
            speed.raw(),
            angle.raw() as i32,
            dur,
            easing,
        );
    }

    /// 只调速、保持方向（ZUN `444 moveSpeed` / `445`）。走极坐标空间，角度分量填当前值。
    /// **不委托** `set_enemy_vel_polar`——理由同 [`WorldBody::set_enemy_angle`]。
    pub fn set_enemy_speed(
        &mut self,
        h: crate::enemy::EnemyHandle,
        speed: Fx,
        dur: u16,
        easing: u8,
    ) {
        let Some(i) = self.enemy_vel_precheck(h, easing) else {
            return;
        };
        self.enemies.vel_touched[i] = 1;
        let angle = self.enemies.angle[i];
        if dur == 0 {
            self.enemies.speed[i] = speed;
            self.refresh_enemy_vel_from_polar(i);
            self.enemies.vel_active[i] = 0;
            return;
        }
        self.arm_enemy_vel(
            i,
            crate::enemy::VEL_SPACE_POLAR,
            self.enemies.speed[i].raw(),
            angle.raw() as i32,
            speed.raw(),
            angle.raw() as i32,
            dur,
            easing,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::bullets::{BULLET_CART_FX, BULLET_CLEARED, BULLET_POLAR_FX};
    use crate::math::geom::polar_to_vec;
    use crate::math::{Angle, Fx};
    use crate::world::STATUS_STALE_HANDLE;
    use crate::world::test_support::bullet_at;

    /// 互斥律：开 POLAR 清 CART、开 CART 清 POLAR、stop 清两位；不碰其他位。
    #[test]
    fn mode_bits_mutually_exclusive() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        // 预置非模式位（BULLET_CLEARED + 位 3 D4 反弹计数预留）：全程必须原样存活。
        w.body.bullets.flags[0] |= BULLET_CLEARED | (1 << 3);
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
        // 判别腿：CART 已置位时再调 set_ang_vel_at，必须清 CART（此前从未在此状态下调用过它）。
        w.body.set_gravity_at(0, Fx::ZERO, Fx::from_raw(6554));
        assert_ne!(w.body.bullets.flags[0] & BULLET_CART_FX, 0);
        w.body.set_ang_vel_at(0, 256);
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
        // 其他位保真：模式切换（含 stop）全程不得误动 CLEARED / 位 3。
        assert_eq!(
            w.body.bullets.flags[0] & (BULLET_CLEARED | (1 << 3)),
            BULLET_CLEARED | (1 << 3),
            "模式切换不得误动其他位"
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

    /// 转发层覆盖：4 个纯转发 setter（ang_vel/accel/gravity/stop）在合法句柄上
    /// 字段落位 + 模式位切换均正确（此前只有互斥律测试间接覆盖，这里直打公开 API）。
    #[test]
    fn handle_setters_forwarding() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);

        w.body.set_bullet_ang_vel(h, 300);
        assert_eq!(w.body.bullets.ang_vel[0], 300);
        assert_ne!(
            w.body.bullets.flags[0] & BULLET_POLAR_FX,
            0,
            "set_bullet_ang_vel 应开 POLAR"
        );

        w.body.set_bullet_accel(h, Fx::from_raw(777));
        assert_eq!(w.body.bullets.accel[0].raw(), 777);
        assert_ne!(
            w.body.bullets.flags[0] & BULLET_POLAR_FX,
            0,
            "set_bullet_accel 应开 POLAR"
        );

        w.body
            .set_bullet_gravity(h, Fx::from_raw(11), Fx::from_raw(22));
        assert_eq!(w.body.bullets.ax[0].raw(), 11);
        assert_eq!(w.body.bullets.ay[0].raw(), 22);
        assert_ne!(
            w.body.bullets.flags[0] & BULLET_CART_FX,
            0,
            "set_bullet_gravity 应开 CART"
        );
        assert_eq!(
            w.body.bullets.flags[0] & BULLET_POLAR_FX,
            0,
            "set_bullet_gravity 应清 POLAR"
        );

        w.body.stop_bullet_fx(h);
        assert_eq!(
            w.body.bullets.flags[0] & (BULLET_POLAR_FX | BULLET_CART_FX),
            0,
            "stop_bullet_fx 应清两模式位"
        );
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

    /// 阈值边界值判别式：speed 恰好 == BACKFILL_MIN_SPEED 时也应回填（`>=` 语义，非 `>`）。
    #[test]
    fn backfill_boundary_speed_equals_threshold_backfills() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        w.body.bullets.angle[0] = Angle::QUARTER; // 旧朝向，若冻结会残留
        w.body.bullets.vx[0] = super::BACKFILL_MIN_SPEED; // 恰好等于阈值
        w.body.bullets.vy[0] = Fx::ZERO;
        w.body.backfill_polar(0);
        assert_eq!(
            w.body.bullets.speed[0].raw(),
            super::BACKFILL_MIN_SPEED.raw()
        );
        assert_eq!(
            w.body.bullets.angle[0],
            crate::math::cordic::atan2(Fx::ZERO, super::BACKFILL_MIN_SPEED),
            "阈值恰好等于 BACKFILL_MIN_SPEED 时角度也应回填（>= 语义）"
        );
    }

    /// 瞄准判别式：angle == atan2(dy,dx)+delta 参考；可瞄状态 = 非 ABSENT 非 GAMEOVER。
    #[test]
    fn aim_targets_nearest_alive_player() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 100, 100); // 自机在 (0,384)
        w.body.aim_bullet_at_player(h, Angle::ZERO);
        let expect = crate::math::cordic::atan2(Fx::from_int(384 - 100), Fx::from_int(0 - 100));
        assert_eq!(w.body.bullets.angle[0], expect);
        // delta 偏移生效
        w.body.aim_bullet_at_player(h, Angle::QUARTER);
        assert_eq!(w.body.bullets.angle[0], expect.add(Angle::QUARTER));
    }

    /// 无可瞄自机 → 纯 no-op（不计数，非违约——世界状态使然）。
    #[test]
    fn aim_with_no_alive_player_is_silent_noop() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 100, 100);
        w.body.bullets.angle[0] = Angle::QUARTER;
        w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
        let cv0 = w.body.diag.contract_viol;
        w.body.aim_bullet_at_player(h, Angle::ZERO);
        assert_eq!(w.body.bullets.angle[0], Angle::QUARTER, "角度不得变");
        assert_eq!(w.body.diag.contract_viol, cv0, "不得计违约");
    }

    /// 悬垂句柄照常计数（与其余 setter 同律）。
    #[test]
    fn aim_stale_handle_counts() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        w.body.bullets.free(h);
        let cv0 = w.body.diag.contract_viol;
        w.body.aim_bullet_at_player(h, Angle::ZERO);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }

    // ── 敌人运动动词族刀 2026-07-31（T1）：双表示同步核 ─────────────────────

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
        assert_eq!(
            w.body.enemies.vy[i],
            Fx::from_int(5),
            "90° 的 sin 分量应为满速"
        );
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
        assert_eq!(
            w.body.enemies.angle[i],
            Angle::QUARTER,
            "零向量不得改写朝向"
        );
    }

    // ── 敌人运动动词族刀 2026-07-31（T2）：速度插值器的武装 ─────────────────────

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
        assert_eq!(
            w.body.enemies.vy[i],
            Fx::from_int(5),
            "瞬时版也要刷积分真相"
        );
        assert_eq!(w.body.enemies.vel_active[i], 0, "dur=0 不武装插值器");
        assert_eq!(
            w.body.enemies.vel_touched[i], 1,
            "瞬时版同样算表达过速度意图"
        );
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
    /// 覆盖全部四条 setter——每条各只计一次（不是委托两次的 +2）。
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
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "set_enemy_vel_polar 只计一次"
        );

        let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let before = w.body.diag.contract_viol;
        w.body
            .set_enemy_vel_cart(h, Fx::from_int(3), Fx::from_int(4), 10, 8);
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "set_enemy_vel_cart 只计一次"
        );

        let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let before = w.body.diag.contract_viol;
        w.body.set_enemy_angle(h, Angle::QUARTER, 10, 8);
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "set_enemy_angle 只计一次（不得委托二次 precheck）"
        );

        let h = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let before = w.body.diag.contract_viol;
        w.body.set_enemy_speed(h, Fx::from_int(2), 10, 8);
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "set_enemy_speed 只计一次（不得委托二次 precheck）"
        );
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
}
