//! 相位 5 · 积分（各池 `pos += vel` + 计时器倒数）。
//!
//! 弹/自机弹/敌人：`pos += vel`（弹另有 `delay` 门与 `life` 倒数；敌人另 tick `invuln`/`hit_flash`；
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
    use crate::math::Fx;
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
}
