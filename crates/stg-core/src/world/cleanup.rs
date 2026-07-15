//! 相位 9 · 回收（越界 / 寿命尽 / 已清除弹 / dying 敌人 / 寿命尽作用区）。
//!
//! **敌人与已清除弹在此收尸**：settle（相位 7）只打标记，槽要活过相位 8（ECL 挂钩）供死亡脚本
//! 与表现层读取死亡坐标，故回收统一延到本相位。

use super::WorldBody;
use crate::enemy::ENEMY_DYING;
use crate::math::Fx;

impl WorldBody {
    pub(crate) fn cleanup(&mut self) {
        self.phase_enter(super::PH_CLEANUP);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0)
                    || self.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0
                    || Self::out_of_bounds(self.bullets.x[i], self.bullets.y[i]);
                if dead {
                    self.bullets.free_index(i);
                }
            }
        }
        // 自机弹越界回收
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if Self::out_of_bounds(self.shots.x[i], self.shots.y[i]) {
                    self.shots.free_index(i);
                }
            }
        }
        // 敌人：dying 标记或越界 → 回收
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.enemies.flags[i] & ENEMY_DYING != 0)
                    || Self::out_of_bounds(self.enemies.x[i], self.enemies.y[i]);
                if dead {
                    self.enemies.free_index(i);
                }
            }
        }
        // 作用区：寿命尽回收（不做越界——field 是有意放置的静止圆，非飞行物）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] == 0 {
                    self.fields.free_index(i);
                }
            }
        }
    }

    /// 越界判定（含边距）。
    fn out_of_bounds(x: Fx, y: Fx) -> bool {
        let xi = x.to_int_floor();
        let yi = y.to_int_floor();
        !(-super::FIELD_HALF_W - super::OOB_MARGIN..=super::FIELD_HALF_W + super::OOB_MARGIN)
            .contains(&xi)
            || !(-super::OOB_MARGIN..=super::FIELD_HEIGHT + super::OOB_MARGIN).contains(&yi)
    }
}

#[cfg(test)]
mod tests {
    use super::WorldBody;
    use crate::math::Fx;

    #[test]
    fn oob_detects_margin() {
        assert!(!WorldBody::out_of_bounds(
            Fx::from_int(0),
            Fx::from_int(200)
        ));
        assert!(WorldBody::out_of_bounds(
            Fx::from_int(1000),
            Fx::from_int(0)
        ));
        assert!(WorldBody::out_of_bounds(
            Fx::from_int(0),
            Fx::from_int(-100)
        ));
        assert!(WorldBody::out_of_bounds(Fx::from_int(0), Fx::from_int(600)));
    }
}
