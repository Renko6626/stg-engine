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
                    if self.bullets.transform_head[i] != crate::xform::XFORM_NONE {
                        self.xforms.free(self.bullets.transform_head[i]);
                    }
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
    use crate::input::InputFrame;
    use crate::math::Fx;
    use crate::world::test_support::spawn_enemy;

    /// 敌人越界回收 —— 金向量压不到这条路径（它的敌人静止在 y=80，全靠 `ENEMY_DYING` 死）。
    ///
    /// 敌人放 x=150（远离 x=0 的自机），故行 3 体碰不会介入；无输入 → 无自机弹 → 不会被打死。
    /// 于是唯一能让它消失的就是本相位的越界判据。
    #[test]
    fn cleanup_frees_out_of_bounds_enemy() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 150, 100, 5);
        let i = w.body.enemies.get(h).unwrap();
        w.body.enemies.vy[i] = Fx::from_int(120); // 下行：y 100 →220→340→460→580

        // 回收线 = FIELD_HEIGHT(448) + OOB_MARGIN(64) = 512。第 3 帧 y=460 仍在场内。
        for f in 0..3 {
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        assert!(
            w.body.enemies.get(h).is_some(),
            "y=460 未越界，不该被回收（若此处已亡说明是别的机制杀的，测试就失去意义）"
        );

        // 第 4 帧 y=580 > 512 → 越界回收
        crate::step::step(&mut w, &InputFrame::empty(3));
        assert_eq!(w.body.enemies.get(h), None, "越界敌人应被 cleanup 回收");
    }

    /// 弹越界回收 → 还段：直测 Task 4 的"还段接线"（弹死后段号可被复得）。
    /// op 选 SET_SPRITE（运动无关）——Task 5 起 run_transforms 真跑，若选 SET_SPEED 会在帧 0
    /// 就用极小 speed 回填 vx，吃掉本测试赖以越界的 vx=1000（与本测试意图无关的耦合）。
    #[test]
    fn oob_bullet_recycle_returns_segment() {
        use crate::math::Angle;
        let seq = [crate::xform::XformSlot {
            wait: 0,
            op: crate::xform::OP_SET_SPRITE,
            _pad: 0,
            args: [1, 0],
        }];
        let mut w = crate::step::World::new(1);
        let init = crate::bullets::BulletInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::from_int(1000), // 快速飞出场外
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
            transform_head: 0xFFFF, // 被覆写，值无关
            xform_wait: 0,
            xform_next: 0,
        };
        let h = w.body.create_bullet_with_xform(init, &seq);
        let i = w.body.bullets.get(h).unwrap();
        let seg = w.body.bullets.transform_head[i];
        assert_ne!(seg, crate::xform::XFORM_NONE);

        crate::step::step(&mut w, &InputFrame::empty(0));

        assert_eq!(w.body.bullets.get(h), None, "越界弹应被回收");
        assert_eq!(w.body.xforms.alloc().unwrap(), seg, "段应已还，复得原段号");
    }

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
