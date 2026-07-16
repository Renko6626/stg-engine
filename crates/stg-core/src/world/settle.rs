//! 相位 7 · 结算三趟（D9）——**唯一改状态者**。
//!
//! 趟一 清除/防护：行6 消弹（**标记不回收**，回收在 cleanup 相位9）+ 按 field 索引升序发聚合
//!   `FieldCleared`。**先于趟二** —— 故同帧作用区能救下本会命中自机的弹（bomb 救命）。
//! 趟二 伤害：行4/7 敌人扣血（overkill/无敌帧门禁）；行1/3 自机中弹 → 决死窗口（行1 跳过已清除的弹）。
//! 趟三 计分：graze（`grazed_by` 逐弹一次；**不查已清除位** —— 擦在相位6 已发生、清弹是相位7 的事）。

use super::WorldBody;
use crate::enemy::ENEMY_DYING;
use crate::events::Event;
use crate::field::FieldPool;

impl WorldBody {
    /// 中弹触发（行 1/3 共用）：只 Alive 者转入决死窗口 —— 一次中弹只触发一次。
    fn trigger_player_hit(&mut self, p: usize) {
        if self.players[p].life_state != crate::player::LIFE_ALIVE {
            return; // 已在窗口/无敌/重生
        }
        self.players[p].life_state = crate::player::LIFE_DEATHWINDOW;
        self.players[p].state_timer = crate::player::DEATHBOMB_WINDOW;
    }

    /// 敌人扣血 + 致死则标记 dying 并产出 `EnemyDied`（行 4/行 7 共用；只发一次）。
    /// **只标记不回收**——槽要活到相位 8 供死亡脚本/表现层读；相位 9 cleanup 收尸。
    fn damage_enemy(&mut self, e: usize, dmg: u16) {
        self.enemies.hp[e] -= dmg as i32;
        self.enemies.hit_flash[e] = 4;
        if self.enemies.hp[e] <= 0 {
            self.enemies.flags[e] |= ENEMY_DYING;
            // 掉落直接分配（A6/A7）：按 drop_table 查表展开；越界表 → P4-b 计数 + 视同空表。
            let table = self.enemies.drop_table[e] as usize;
            if table >= crate::items::DROP_TABLES.len() {
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            } else {
                let (ex, ey) = (self.enemies.x[e], self.enemies.y[e]);
                for &(ty, n) in crate::items::DROP_TABLES[table] {
                    for _ in 0..n {
                        self.spawn_drop(ex, ey, ty);
                    }
                }
            }
            let ev = Event {
                kind: crate::events::EVT_ENEMY_DIED,
                a_index: e as u16,
                a_gen: self.enemies.generation[e],
                x: self.enemies.x[e],
                y: self.enemies.y[e],
                data: [
                    self.enemies.score[e] as i32,
                    self.enemies.death_script[e] as i32,
                ],
            };
            self.push_event(ev);
        }
    }

    pub(crate) fn settle(&mut self) {
        self.phase_enter(super::PH_SETTLE);
        // ── 趟一 · 清除/防护：行 6 消弹 ──────────────────────────────────
        // **只标记不回收**（趟二/趟三随后按索引读这颗弹；回收在相位 9 cleanup）。
        // **先于趟二**——故同帧作用区能救下本会命中自机的弹（bomb 救命）。
        let mut cleared_counts = [0i32; FieldPool::CAP];
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row != crate::events::ROW_FIELD_BULLET {
                continue;
            }
            let b = h.passive as usize;
            if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                continue; // 幂等：已被别的 field 消掉，不重复计
            }
            self.bullets.flags[b] |= crate::bullets::BULLET_CLEARED;
            cleared_counts[h.active as usize] += 1;
        }
        // 聚合事件：按 field 索引升序产出（不依赖 hits 的分组连续性 → 与 collide 循环结构解耦）
        for (f, &count) in cleared_counts.iter().enumerate() {
            if count > 0 {
                let ev = Event {
                    kind: crate::events::EVT_FIELD_CLEARED,
                    a_index: f as u16,
                    a_gen: self.fields.generation[f],
                    x: self.fields.x[f],
                    y: self.fields.y[f],
                    data: [count, 0],
                };
                self.push_event(ev);
            }
        }
        // ── 趟二 · 伤害 ──────────────────────────────────────────────────
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            match h.row {
                crate::events::ROW_SHOT_ENEMY => {
                    let s = h.active as usize;
                    let e = h.passive as usize;
                    if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 {
                        continue; // 悬垂 / overkill
                    }
                    if self.enemies.invuln[e] != 0 || !self.shots.is_alive(s) {
                        continue; // 无敌帧跳伤害；悬垂弹跳过
                    }
                    self.damage_enemy(e, self.shots.damage[s]);
                }
                crate::events::ROW_FIELD_ENEMY => {
                    let f = h.active as usize;
                    let e = h.passive as usize;
                    if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 {
                        continue; // 悬垂 / overkill
                    }
                    if self.enemies.invuln[e] != 0 || !self.fields.is_alive(f) {
                        continue; // 无敌帧跳伤害（收集时不查、结算时判）
                    }
                    self.damage_enemy(e, self.fields.dmg_per_frame[f]);
                }
                crate::events::ROW_BULLET_PLAYER_HIT => {
                    let b = h.active as usize;
                    if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                        continue; // 趟一清掉的弹不中弹 —— bomb 救命
                    }
                    self.trigger_player_hit(h.passive as usize);
                }
                crate::events::ROW_BODY_PLAYER_HIT => {
                    // 敌体无"被清除"概念（敌人经 hp≤0 → dying），故不查已清除位
                    self.trigger_player_hit(h.passive as usize);
                }
                _ => {}
            }
        }
        // ── 趟三 · 计分/拾取 ─────────────────────────────────────────────
        // graze **不**查已清除位：碰撞检测在相位 6 发生（那时弹活着、确实进了擦圈），
        // 清弹是相位 7 的事 —— 擦在先、清在后；且设计明写 graze 独立于中弹。
        // grazed_by 是 u8 位掩码，每自机占 1 位；MAX_PLAYERS 超过 8 会静默溢出（release 下 wrap，
        // 而非 panic），从而在跨自机间腐蚀 graze 位——编译期钉死上限，宁可编不过也不留隐患。
        const _: () = assert!(crate::MAX_PLAYERS <= 8, "grazed_by 位掩码只容 8 自机");
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row == crate::events::ROW_BULLET_PLAYER_GRAZE {
                let b = h.active as usize;
                let p = h.passive as usize;
                let bit = 1u8 << p; // MAX_PLAYERS=2 → bit 0/1
                if self.bullets.grazed_by[b] & bit == 0 {
                    self.bullets.grazed_by[b] |= bit;
                    self.players[p].graze = self.players[p].graze.wrapping_add(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::math::Fx;
    use crate::world::PH_COLLIDE;
    use crate::world::test_support::*;

    #[test]
    fn settle_shot_kills_enemy_marks_dying_and_event() {
        use crate::enemy::ENEMY_DYING;
        use crate::events::EVT_ENEMY_DIED;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei],
            y: w.body.enemies.y[ei],
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 1,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert!(w.body.enemies.hp[ei] <= 0);
        assert_ne!(w.body.enemies.flags[ei] & ENEMY_DYING, 0);
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_ENEMY_DIED);
    }

    #[test]
    fn settle_overkill_two_shots_one_death_event() {
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1，两发都打中
        let ei = w.body.enemies.get(e).unwrap();
        for _ in 0..2 {
            w.body.create_player_shot(crate::shots::ShotInit {
                x: w.body.enemies.x[ei],
                y: w.body.enemies.y[ei],
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                damage: 1,
                radius: Fx::from_int(4),
                sprite: 0,
                owner: 0,
                flags: 0,
            });
        }
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.events_len, 1); // 只死一次
    }

    #[test]
    fn settle_bullet_hit_triggers_deathwindow() {
        use crate::player::{DEATHBOMB_WINDOW, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(w.body.players[0].state_timer, DEATHBOMB_WINDOW);
    }

    #[test]
    fn settle_graze_counts_once_per_bullet() {
        use crate::input::InputFrame;
        use crate::step::step;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        // 一颗停在 graze 圈内、hit 圈外的弹（距 10px）
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::from_int(10),
            y: Fx::from_int(384),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: crate::math::Angle::ZERO,
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
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        });
        // 弹静止、贴着自机 → 连跑 3 帧，graze 只 +1（grazed_by 逐弹一次）
        for _ in 0..3 {
            step(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].graze, 1);
    }

    #[test]
    fn settle_field_clears_bullet_and_emits_aggregate() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::EVT_FIELD_CLEARED;
        use crate::field::FIELD_CLEAR_BULLETS;
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 0, 100);
        bullet_at(&mut w, 10, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_ne!(w.body.bullets.flags[0] & BULLET_CLEARED, 0);
        assert_ne!(w.body.bullets.flags[1] & BULLET_CLEARED, 0);
        // 聚合：一条事件、count=2
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.events[0].data[0], 2);
    }

    #[test]
    fn settle_two_fields_clear_same_bullet_counts_once() {
        use crate::field::FIELD_CLEAR_BULLETS;
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1); // field 0
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1); // field 1，同位置
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        // 幂等：弹只被计一次 → 只有 field 0 计到 1，field 1 计 0（无事件）
        let total: i32 = (0..w.body.events_len as usize)
            .map(|k| w.body.events[k].data[0])
            .sum();
        assert_eq!(total, 1);
    }

    #[test]
    fn settle_field_clear_saves_player_from_death() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::player::{LIFE_ALIVE, LIFE_DEATHWINDOW};
        // 招牌语义：弹压在自机身上 + 同帧 field 消它 → 自机不进决死窗口（趟一先于趟二）
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        spawn_field(&mut w, 0, 384, 20, FIELD_CLEAR_BULLETS, 1);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE); // 被救
        assert_ne!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(w.body.players[0].graze, 1); // 但 graze 照算（擦在先、清在后）
    }

    /// 敌死按 drop_table 掉落：表 1 = 2 POWER + 1 POINT，落点 = 敌死位置（散布只改速度）。
    /// 两敌同帧死 → 掉落顺序 = 结算序（低索引敌先掉，RNG 消耗序钉死）。
    #[test]
    fn settle_death_drops_by_table_in_settlement_order() {
        use crate::items::{ITEM_POINT, ITEM_POWER};
        let mut w = crate::step::World::new(7);
        let enemy_at = |x: i32| crate::enemy::EnemyInit {
            x: Fx::from_int(x),
            y: Fx::from_int(80),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 1,
            hp_max: 1,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 1,
            score: 100,
        };
        let ea = w.body.create_enemy(enemy_at(-100));
        let eb = w.body.create_enemy(enemy_at(100));
        let eai = w.body.enemies.get(ea).unwrap();
        let ebi = w.body.enemies.get(eb).unwrap();
        for &(ex, ey) in &[
            (w.body.enemies.x[eai], w.body.enemies.y[eai]),
            (w.body.enemies.x[ebi], w.body.enemies.y[ebi]),
        ] {
            w.body.create_player_shot(crate::shots::ShotInit {
                x: ex,
                y: ey,
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                damage: 1,
                radius: Fx::from_int(4),
                sprite: 0,
                owner: 0,
                flags: 0,
            });
        }
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.items.iter_alive().count(), 6, "两敌各掉 3 颗");
        let ax = w.body.enemies.x[eai];
        let bx = w.body.enemies.x[ebi];
        for i in 0..3 {
            assert_eq!(w.body.items.x[i], ax, "前 3 颗落在敌 A（低索引先掉）");
        }
        for i in 3..6 {
            assert_eq!(w.body.items.x[i], bx, "后 3 颗落在敌 B");
        }
        for base in [0usize, 3] {
            assert_eq!(
                [
                    w.body.items.item_type[base],
                    w.body.items.item_type[base + 1],
                    w.body.items.item_type[base + 2],
                ],
                [ITEM_POWER, ITEM_POWER, ITEM_POINT],
                "表内序：2 POWER + 1 POINT"
            );
        }
    }

    #[test]
    fn settle_field_damages_enemy() {
        use crate::field::FIELD_DAMAGE;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_field(crate::field::FieldInit {
            x: Fx::ZERO,
            y: Fx::from_int(100),
            radius: Fx::from_int(20),
            dmg_per_frame: 2,
            life: 1,
            owner: 0,
            flags: FIELD_DAMAGE,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        w.body.settle();
        assert_eq!(w.body.enemies.hp[ei], 3); // 5 - 2
    }
}
