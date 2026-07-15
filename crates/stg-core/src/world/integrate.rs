//! 相位 5 · 积分（各池 `pos += vel` + 计时器倒数）。
//!
//! 弹：delay 门 → 模式效果（POLAR/CART 互斥）→ `pos += vel` → life 倒数。
//! 自机弹/敌人：`pos += vel`（敌人另 tick `invuln`/`hit_flash`；
//! 敌人的 `move_to` 插值器待后续切片，`mv_*` 字段现为惰性）。
//! 作用区：`life` 倒数 —— `life=1` 本帧减到 0、相位 6 仍参与判定、相位 9 才回收（"每帧重铺=跟随"的时序基础）。

use super::WorldBody;

impl WorldBody {
    pub(crate) fn integrate(&mut self) {
        self.phase_enter(super::PH_INTEGRATE);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.bullets.delay[i] > 0 {
                    self.bullets.delay[i] -= 1; // delay 期不动
                    continue;
                }
                let fl = self.bullets.flags[i];
                debug_assert_ne!(
                    fl & (crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX),
                    crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX,
                    "模式位互斥被破坏（P4-c 帧内断言）"
                );
                if fl & crate::bullets::BULLET_POLAR_FX != 0 {
                    self.bullets.angle[i] =
                        self.bullets.angle[i].add_delta(self.bullets.ang_vel[i]);
                    self.bullets.speed[i] = self.bullets.speed[i] + self.bullets.accel[i];
                    self.refresh_vel_from_polar(i);
                }
                self.bullets.x[i] = self.bullets.x[i] + self.bullets.vx[i];
                self.bullets.y[i] = self.bullets.y[i] + self.bullets.vy[i];
                if self.bullets.life[i] != 0xFFFF && self.bullets.life[i] > 0 {
                    self.bullets.life[i] -= 1;
                }
            }
        }
        // 自机弹：pos += vel（无 delay/life）
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.shots.x[i] = self.shots.x[i] + self.shots.vx[i];
                self.shots.y[i] = self.shots.y[i] + self.shots.vy[i];
            }
        }
        // 敌人：pos += vel（move_to 插值器延后，mv_* 惰性）+ 计时器 tick
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.enemies.x[i] = self.enemies.x[i] + self.enemies.vx[i];
                self.enemies.y[i] = self.enemies.y[i] + self.enemies.vy[i];
                if self.enemies.invuln[i] > 0 {
                    self.enemies.invuln[i] -= 1;
                }
                if self.enemies.hit_flash[i] > 0 {
                    self.enemies.hit_flash[i] -= 1;
                }
            }
        }
        // 作用区：寿命倒数（照抄弹的模式；life=1 → 本帧减到 0，相位6 仍参与判定，相位9 回收）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] > 0 {
                    self.fields.life[i] -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::input::InputFrame;
    use crate::math::Angle;
    use crate::math::Fx;
    use crate::math::geom::polar_to_vec;
    use crate::world::test_support::bullet_at;

    /// `delay` 门：delay 期弹只倒数、不移动；delay 尽后才开始积分。
    ///
    /// 走真实 `step`（而非直接调 `collide`）—— 这是唯一能触达 integrate 里那个 delay 门的路径：
    /// M0-9 复审实测，既有的 delay 测试直接调 `collide()`，根本到不了相位 5。
    #[test]
    fn integrate_delay_gate_holds_bullet_then_releases() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        w.body.bullets.vx[0] = Fx::from_int(3);
        w.body.bullets.delay[0] = 2;

        // delay 期：不动，只倒数
        crate::step::step(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.x[0], Fx::ZERO, "delay 期弹不该移动");
        assert_eq!(w.body.bullets.delay[0], 1);

        crate::step::step(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.x[0], Fx::ZERO, "delay 期弹不该移动");
        assert_eq!(w.body.bullets.delay[0], 0);

        // delay 尽 → 开始积分
        crate::step::step(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.bullets.x[0], Fx::from_int(3), "delay 尽后应开始移动");
    }

    /// 螺旋判别式：ω=1024 BAM/帧 × 16 帧 = 1/4 圈，vx/vy 与查表参考逐位相等。
    #[test]
    fn polar_fx_spiral_matches_table_after_quarter_turn() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(2);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_ang_vel_at(i, 1024);
        for f in 0..16u32 {
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle::QUARTER);
        let (rvx, rvy) = polar_to_vec(Fx::from_int(2), Angle::QUARTER);
        assert_eq!(w.body.bullets.vx[i], rvx);
        assert_eq!(w.body.bullets.vy[i], rvy);
    }

    /// 沿向加速判别式：speed 线性累加，v 与查表参考一致。
    #[test]
    fn polar_fx_accel_grows_speed() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(1);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_accel_at(i, Fx::from_raw(3277)); // ~0.05 px/帧²
        for f in 0..10u32 {
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.speed[i].raw(), 65536 + 10 * 3277);
        let (rvx, _) = polar_to_vec(Fx::from_raw(65536 + 10 * 3277), Angle::ZERO);
        assert_eq!(w.body.bullets.vx[i], rvx);
    }

    /// delay 门冻结 POLAR：delay 期 angle/speed/位置全不动（D3：变换不走）。
    #[test]
    fn delay_gate_freezes_polar_fx() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(2);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_ang_vel_at(i, 1024);
        w.body.bullets.delay[i] = 2;
        for f in 0..2u32 {
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "delay 期角度不得推进");
        assert_eq!(w.body.bullets.x[i], Fx::ZERO, "delay 期不得移动");
        crate::step::step(&mut w, &InputFrame::empty(2));
        assert_eq!(
            w.body.bullets.angle[i],
            Angle(1024),
            "delay 尽后首帧推进一步"
        );
    }
}
