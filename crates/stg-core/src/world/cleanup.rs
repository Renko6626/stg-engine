//! 相位 9 · 回收（越界 / 寿命尽 / 已清除弹 / dying 敌人 / 寿命尽作用区 / 已拾取道具）。
//!
//! **敌人与已清除弹在此收尸**：settle（相位 7）只打标记，槽要活过相位 8（ECL 挂钩）供死亡脚本
//! 与表现层读取死亡坐标，故回收统一延到本相位。道具同款：settle 趟三只标 `MAGNET_PICKED`，
//! 回收延到本相位（同弹回收骨架，另加越界判据——磁吸失败/未拾取的道具飞出场外也要收）。
//! 敌人越界判据用 `ENEMY_OOB_MARGIN`（大边界兜底）：主导回收靠纪律——敌人主协程返回即自燃
//! （ZUN ECL 语义，D9 落地：`ecl::vm::run_tasks` 的 `Exec::End` 分支置 `ENEMY_DYING`），
//! 本判据只防脚本失手导致的泄漏。

use super::WorldBody;
use crate::enemy::ENEMY_DYING;
use crate::math::Fx;

impl WorldBody {
    pub(crate) fn cleanup(&mut self) {
        self.phase_enter(super::PH_CLEANUP);
        // C 组：冻 C 时没有新的越界/消弹/死亡标记产生（相位 4~7 全停），无需回收。
        // **推论**：冻结开始前那一帧刚被标记的东西（`ENEMY_DYING` 的敌、`BULLET_CLEARED`
        // 的弹）会一直挂在池里、槽位占着不还，直到 C 解冻那一帧才真正被收走——是确定性的
        // "残留"而非泄漏（帧号/回放/校验和都不受影响，只是槽位暂时不空）。
        //
        // **同一类推论的手足**：`signals[]`（信号黑板）只在**相位 4**（`run_transforms`,
        // `transform.rs` 的 `WAIT_SIGNAL` 分支）按边沿消费——`signals[ch] == frame + 1`
        // 才算命中。点火那一帧（`director` 在相位 2 之后才把 `freeze_left` 写上）相位 2
        // 仍按冻结前的旧状态跑，脚本这一帧发出的 `pulse_signal` 照常写入 `signals[ch] =
        // frame + 1`；但同一帧的相位 4 已经看得到刚写好的 `freeze_left`，被冻结跳过——
        // 这一戳永远不会被看到。下一帧 `frame` 已经 +1，`signals[ch]` 却还停在
        // "旧 frame + 1"，条件再也凑不齐：这一发脉冲**永久性地**丢了。确定性（同样的输入
        // 序列永远丢在同一处），但脚本作者若恰好在开启时停的那一帧脉冲信号，会发现等在
        // `WAIT_SIGNAL` 上的弹再也等不到那个边沿。
        if self.scene_frozen() {
            return;
        }
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let life_out = self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0;
                let cleared = self.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0;
                let oob = Self::out_of_bounds(self.bullets.x[i], self.bullets.y[i]);
                let dead = life_out || cleared || oob;
                if dead {
                    // `vanished`（表现契约 v2 §3.4）：只记场内的两种死因，越界不记（屏外没有
                    // 淡出可画）。同时越界又寿尽/被清的弹按越界处置——它已经在屏外。
                    // 清除优先于寿尽：被 bomb 消掉的那一帧恰好寿尽，表现层要的是"被消"。
                    if !oob {
                        let reason = if cleared {
                            crate::events::VANISH_CLEARED
                        } else {
                            crate::events::VANISH_LIFE
                        };
                        self.push_vanished(
                            self.bullets.x[i],
                            self.bullets.y[i],
                            self.bullets.sprite[i],
                            reason,
                        );
                    }
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
                    || Self::out_of_bounds_enemy(self.enemies.x[i], self.enemies.y[i]);
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
        // 道具：已拾取（settle 趟三标记 MAGNET_PICKED）或越界 → 回收（同弹回收骨架）
        let nw = self.items.alive.len();
        for w in 0..nw {
            let mut bits = self.items.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = self.items.magnet_to[i] == crate::items::MAGNET_PICKED
                    || Self::out_of_bounds(self.items.x[i], self.items.y[i]);
                if dead {
                    self.items.free_index(i);
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

    /// 敌人专用越界判定（大边界兜底；主导回收靠纪律，见 `ENEMY_OOB_MARGIN` 文档）。
    fn out_of_bounds_enemy(x: Fx, y: Fx) -> bool {
        let xi = x.to_int_floor();
        let yi = y.to_int_floor();
        !(-super::FIELD_HALF_W - super::ENEMY_OOB_MARGIN
            ..=super::FIELD_HALF_W + super::ENEMY_OOB_MARGIN)
            .contains(&xi)
            || !(-super::ENEMY_OOB_MARGIN..=super::FIELD_HEIGHT + super::ENEMY_OOB_MARGIN)
                .contains(&yi)
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
        w.body.enemies.vy[i] = Fx::from_int(120); // 下行：y 100→220→340→460→580→700→820

        // 回收线 = FIELD_HEIGHT(448) + ENEMY_OOB_MARGIN(256) = 704。第 5 帧 y=700 仍在场内。
        for f in 0..5 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert!(
            w.body.enemies.get(h).is_some(),
            "y=700 未越界，不该被回收（若此处已亡说明是别的机制杀的，测试就失去意义）"
        );

        // 第 6 帧 y=820 > 704 → 越界回收
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(5));
        assert_eq!(w.body.enemies.get(h), None, "越界敌人应被 cleanup 回收");
    }

    /// 大边界判别：y=600 在旧共用界（448+64=512）外、新敌人界（448+256=704）内 → 必须存活。
    /// （y=500 两界皆内无判别力——spec 值勘误，见计划 Global Constraints。）
    #[test]
    fn enemy_survives_beyond_old_margin_within_new() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 600, 5);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert!(
            w.body.enemies.get(h).is_some(),
            "600 < 704：大边界内必须存活"
        );
    }

    /// 越新界必收：y=720 > 704。
    #[test]
    fn enemy_recycled_beyond_enemy_margin() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 720, 5);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert!(w.body.enemies.get(h).is_none(), "720 > 704：越大边界必收");
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
            born_frame: 0,
        };
        let h = w.body.create_bullet_with_xform(init, &seq);
        let i = w.body.bullets.get(h).unwrap();
        let seg = w.body.bullets.transform_head[i];
        assert_ne!(seg, crate::xform::XFORM_NONE);

        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));

        assert_eq!(w.body.bullets.get(h), None, "越界弹应被回收");
        assert_eq!(w.body.xforms.alloc().unwrap(), seg, "段应已还，复得原段号");
    }

    /// 道具回收：MAGNET_PICKED（settle 趟三留下的拾取标记）当帧回收；越界（同弹 OOB 判据）回收。
    #[test]
    fn item_recycled_after_pick_and_when_oob() {
        use crate::items::{ITEM_POWER, MAGNET_PICKED};
        #[cfg(debug_assertions)]
        use crate::world::PH_CLEANUP;
        let mut w = crate::step::World::new(1);
        let picked = w.body.drop_item(
            Fx::ZERO,
            Fx::from_int(384),
            ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let pi = w.body.items.get(picked).unwrap();
        w.body.items.magnet_to[pi] = MAGNET_PICKED; // 模拟 settle 趟三已标记

        let oob = w.body.drop_item(
            Fx::from_int(2000),
            Fx::ZERO,
            ITEM_POWER,
            &crate::tables::TABLES_V0,
        ); // 远出场外

        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_CLEANUP;
        }
        w.body.cleanup();

        assert_eq!(w.body.items.get(picked), None, "已拾取道具当帧回收");
        assert_eq!(w.body.items.get(oob), None, "越界道具回收");
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

    // ── `vanished`（表现契约 v2 spec §3.4）────────────────────────────────────

    /// 场内寿尽 / 被清各记一行（reason 可辨）、越界不记；同时越界又被清按越界处置。
    #[test]
    fn vanished_records_life_and_cleared_but_not_oob() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::{VANISH_CLEARED, VANISH_LIFE};
        #[cfg(debug_assertions)]
        use crate::world::PH_CLEANUP;
        let mut w = crate::step::World::new(1);
        let mk = crate::world::test_support::bullet_at;
        let life = mk(&mut w, 10, 20);
        let cleared = mk(&mut w, 30, 40);
        let oob = mk(&mut w, 2000, 0);
        let oob_cleared = mk(&mut w, -2000, 0);
        let survivor = mk(&mut w, 50, 60);
        {
            let b = &mut w.body.bullets;
            let i = b.get(life).unwrap();
            b.life[i] = 0;
            b.sprite[i] = 7;
            let i = b.get(cleared).unwrap();
            b.flags[i] |= BULLET_CLEARED;
            b.sprite[i] = 9;
            let i = b.get(oob_cleared).unwrap();
            b.flags[i] |= BULLET_CLEARED;
        }
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_CLEANUP;
        }
        w.body.cleanup();
        assert!(w.body.bullets.get(life).is_none());
        assert!(w.body.bullets.get(cleared).is_none());
        assert!(w.body.bullets.get(oob).is_none());
        assert!(w.body.bullets.get(oob_cleared).is_none());
        assert!(w.body.bullets.get(survivor).is_some());
        let v = w.body.vanished();
        assert_eq!(v.len(), 2, "只有场内两颗入账（越界两颗不记）");
        // 池索引升序 = 创建序：life 在前、cleared 在后
        assert_eq!((v[0].x, v[0].y), (Fx::from_int(10), Fx::from_int(20)));
        assert_eq!((v[0].sprite, v[0].reason), (7, VANISH_LIFE));
        assert_eq!((v[1].x, v[1].y), (Fx::from_int(30), Fx::from_int(40)));
        assert_eq!((v[1].sprite, v[1].reason), (9, VANISH_CLEARED));
        assert_eq!(w.body.diag.vanished_overflow, 0);
    }

    /// 被清优先于寿尽：同一颗弹两个标记都在，reason = CLEARED。
    #[test]
    fn vanished_prefers_cleared_over_life() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::VANISH_CLEARED;
        #[cfg(debug_assertions)]
        use crate::world::PH_CLEANUP;
        let mut w = crate::step::World::new(1);
        let h = crate::world::test_support::bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.life[i] = 0;
        w.body.bullets.flags[i] |= BULLET_CLEARED;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_CLEANUP;
        }
        w.body.cleanup();
        assert_eq!(w.body.vanished().len(), 1);
        assert_eq!(w.body.vanished()[0].reason, VANISH_CLEARED);
    }

    /// 冻 C（玩家时停）时 cleanup 早退：寿尽弹留池、`vanished` 无行——与"残留而非泄漏"推论一致。
    #[test]
    fn vanished_is_empty_while_scene_frozen() {
        #[cfg(debug_assertions)]
        use crate::world::PH_CLEANUP;
        let mut w = crate::step::World::new(1);
        let h = crate::world::test_support::bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.life[i] = 0;
        w.body.freeze_left[0] = 5;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_CLEANUP;
        }
        w.body.cleanup();
        assert!(w.body.bullets.get(h).is_some(), "冻结帧不回收");
        assert!(w.body.vanished().is_empty());
    }
}
