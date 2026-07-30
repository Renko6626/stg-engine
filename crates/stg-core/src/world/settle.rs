//! 相位 7 · 结算三趟（D9）——**唯一改状态者**。
//!
//! 趟一 清除/防护：行6 消弹（**标记不回收**，回收在 cleanup 相位9）+ 逐弹原位转星星
//!   （M0-15 一律转化：出生即磁吸存活自机，30 分经济回流）+ 按 field 索引升序发聚合
//!   `FieldCleared`。**先于趟二** —— 故同帧作用区能救下本会命中自机的弹（bomb 救命）。
//! 趟二 伤害：行4/7 敌人扣血（overkill/无敌帧门禁）；行1/3 自机中弹 → 决死窗口（行1 跳过已清除的弹）。
//! 趟三 计分/拾取：graze（`grazed_by` 逐弹一次；**不查已清除位** —— 擦在相位6 已发生、清弹是相位7
//!   的事）+ 行5 道具拾取（首见即标 `MAGNET_PICKED`，同帧多 hit 只入账一次，账本落 `credit_item`）。

use super::WorldBody;
use crate::enemy::ENEMY_DYING;
use crate::events::Event;
use crate::field::FieldPool;
use crate::tables::WorldTables;

impl WorldBody {
    /// 中弹触发（行 1/3 共用）：只 Alive 者转入决死窗口 —— 一次中弹只触发一次。
    fn trigger_player_hit(&mut self, p: usize) {
        if self.players[p].life_state != crate::player::LIFE_ALIVE {
            return; // 已在窗口/无敌/重生
        }
        self.players[p].life_state = crate::player::LIFE_DEATHWINDOW;
        self.players[p].state_timer = crate::player::DEATHBOMB_WINDOW;
    }

    /// 把敌身上的掉落计数原位撒出去——**只撒**：不清零、不加分、不发事件。
    ///
    /// 顺序是**类型升序**（I4）：这也是"掉落从表迁成计数后金向量不漂"的依据——内建掉落表 1
    /// 是 `[(ITEM_POWER,2),(ITEM_POINT,1)]` 而 `ITEM_POWER=0 < ITEM_POINT=1`，表序恰好
    /// 就是类型升序，故 `spawn_drop` 的 RNG 消耗顺序与迁移前逐字相同
    /// （由 `enemy_death_drop_sequence_is_pinned` 特征化测试押运）。
    ///
    /// 两个调用方：`kill_enemy`（死亡效果的一部分）/ `SYS_DROP_ITEMS`（脚本显式撒）。
    /// **不清零**是人类裁定（spec D-3）：故 `drop_items(); die();` 会掉两份，作者自负。
    pub(crate) fn spill_drops(&mut self, e: usize, tables: &WorldTables) {
        let (ex, ey) = (self.enemies.x[e], self.enemies.y[e]);
        for ty in 0..crate::items::ITEM_TYPE_COUNT {
            for _ in 0..self.enemies.drop_count[e][ty] {
                self.spawn_drop(ex, ey, ty as u8, tables);
            }
        }
    }

    /// 敌人扣血 + 致死则标记 dying 并产出 `EnemyDied`（行 4/行 7 共用；只发一次）。
    /// **只标记不回收**——槽要活到相位 8 供死亡脚本/表现层读；相位 9 cleanup 收尸。
    fn damage_enemy(&mut self, e: usize, dmg: u16, tables: &WorldTables) {
        self.enemies.hp[e] -= dmg as i32;
        // 伤害下钳（spec 2026-07-24 §3.1）：绑着某 active 符卡槽且 threshold>0 的敌，一发大
        // 伤害只把血打到血线为止，不许打穿（非最终卡的 boss 不该死在中途）。**必须**在这里
        // （扣血后、判 hp<=0 标 ENEMY_DYING 前）钳——天然不误标 dying，不需要另开一趟事后
        // 撤销 pass（threshold==0 的最终卡不钳，与 boss 死重合，dying 检测照常兜住）。
        if let Some(slot) = self.spell_slot_bound_to(e as u16, self.enemies.generation[e]) {
            let threshold = self.spells[slot].hp_threshold;
            if threshold > 0 {
                self.enemies.hp[e] = self.enemies.hp[e].max(threshold);
            }
        }
        self.enemies.hit_flash[e] = 4;
        if self.enemies.hp[e] <= 0 {
            self.enemies.flags[e] |= ENEMY_DYING;
            // 掉落直接分配（A6/A7）：撒敌身上的逐类型计数（表号已在生成时展开）。
            self.spill_drops(e, tables);
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
            // 死亡特效请求（蓝图 §207 机械产出者；args 约定见 `crate::reqs` 模块文档）。
            self.emit_req(
                crate::consts::REQ_ENEMY_DEATH,
                [
                    self.enemies.x[e].raw(),
                    self.enemies.y[e].raw(),
                    self.enemies.sprite[e] as i32,
                    self.enemies.score[e] as i32,
                    0,
                    0,
                ],
            );
        }
    }

    pub(crate) fn settle(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_SETTLE);
        // ── 趟一 · 清除/防护：行 6 消弹 ──────────────────────────────────
        // **只标记不回收**（趟二/趟三随后按索引读这颗弹；回收在相位 9 cleanup）。
        // **先于趟二**——故同帧作用区能救下本会命中自机的弹（bomb 救命）。
        let mut cleared_counts = [0i32; FieldPool::CAP];
        // 消弹转星星（M0-15，一律转化）：磁吸目标趟外算一次——升序首个 ALIVE 自机（I4），
        // 无则 MAGNET_NONE 正常下落。目标自机趟内不变，逐弹查与一次查确定性等价。
        let star_target = (0..crate::MAX_PLAYERS)
            .find(|&p| self.players[p].life_state == crate::player::LIFE_ALIVE)
            .map(|p| p as u8)
            .unwrap_or(crate::items::MAGNET_NONE);
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
            // 每颗被消的弹在原位转一颗星星（30 分经济回流；池满 P4-a 逐颗降级计数）。
            self.spawn_star_at(self.bullets.x[b], self.bullets.y[b], star_target);
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
                    let dmg = self.shots.damage[s];
                    // 命中事实（表现层消费：火花/音效/伤害数字）。坐标取**自机弹**当帧位置
                    // ——命中点在弹上、不在敌心；弹此刻尚存活（上一行刚判过），相位 9 才回收。
                    // 逐命中发不聚合的预算论证见 `events::EVT_SHOT_HIT_ENEMY` 文档。
                    let ev = Event {
                        kind: crate::events::EVT_SHOT_HIT_ENEMY,
                        a_index: e as u16,
                        a_gen: self.enemies.generation[e],
                        x: self.shots.x[s],
                        y: self.shots.y[s],
                        data: [dmg as i32, 0],
                    };
                    self.push_event(ev);
                    self.damage_enemy(e, dmg, tables);
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
                    self.damage_enemy(e, self.fields.dmg_per_frame[f], tables);
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
            match h.row {
                crate::events::ROW_BULLET_PLAYER_GRAZE => {
                    let b = h.active as usize;
                    let p = h.passive as usize;
                    let bit = 1u8 << p; // MAX_PLAYERS=2 → bit 0/1
                    if self.bullets.grazed_by[b] & bit == 0 {
                        self.bullets.grazed_by[b] |= bit;
                        self.players[p].graze = self.players[p].graze.wrapping_add(1);
                    }
                }
                crate::events::ROW_ITEM_PLAYER => {
                    let it = h.active as usize;
                    if !self.items.is_alive(it)
                        || self.items.magnet_to[it] == crate::items::MAGNET_PICKED
                    {
                        continue; // 首见即标：同帧多 hit 只入账一次
                    }
                    self.items.magnet_to[it] = crate::items::MAGNET_PICKED;
                    let ty = self.items.item_type[it];
                    self.credit_item(h.passive as usize, ty, tables);
                    let ev = Event {
                        kind: crate::events::EVT_ITEM_PICKED,
                        a_index: it as u16,
                        a_gen: self.items.generation[it],
                        x: self.items.x[it],
                        y: self.items.y[it],
                        data: [ty as i32, h.passive as i32],
                    };
                    self.push_event(ev);
                }
                _ => {}
            }
        }
        // ── 符卡趟（spec 2026-07-24 §3）：三趟之后、按槽升序推进 ─────────────
        self.settle_spells(tables);
    }

    /// 拾取入账（D9 趟三）——**唯一** per-type 逻辑居所（扩展四步第 ③ 步：新增类型在此加臂）。
    /// 未知类型：P4-b 计数忽略（两机同弃，无副作用）。
    fn credit_item(&mut self, p: usize, item_type: u8, tables: &WorldTables) {
        use crate::items::*;
        match item_type {
            ITEM_POWER => {
                let pl = &mut self.players[p];
                if pl.power < POWER_MAX {
                    pl.power += 1;
                } else {
                    pl.score += tables.item_cfg[ITEM_POINT as usize].score as u64; // 满 power 转化
                }
            }
            ITEM_POINT => {
                self.players[p].score += tables.item_cfg[ITEM_POINT as usize].score as u64;
            }
            ITEM_LIFE_PIECE => {
                let pl = &mut self.players[p];
                // `saturating_add`（非裸 `+= 1`）：与下面 `>=` 判据的"抗直写越界"论证自洽——
                // 若真有人直写 `life_pieces` 到 u8 上限附近，裸 `+= 1` 会先在 debug 下溢出
                // panic，根本走不到 `>=` 判据；饱和后不 panic，才轮到下面的进位判据兜底。
                pl.life_pieces = pl.life_pieces.saturating_add(1);
                // `>=`（非 `==`）单步追赶（B12）：抗直写越界——若碎片被直接写成远超阈值的
                // 值，一次进位只清零并 +1 命，超出阈值的部分被丢弃（不会连续进位多命）。
                // 正常引擎路径下无人直写该字段（只有本函数每次 +1），这是纯防御性降级；
                // 该降级本身可接受——比 `==` 版本"永远追不上、永不进位"的死锁强。
                if pl.life_pieces >= PIECES_PER_LIFE {
                    pl.life_pieces = 0;
                    pl.lives = pl.lives.saturating_add(1);
                }
            }
            ITEM_BOMB_PIECE => {
                let pl = &mut self.players[p];
                // 同上：饱和加 + `>=` 单步追赶，抗直写越界（B12）。
                pl.bomb_pieces = pl.bomb_pieces.saturating_add(1);
                if pl.bomb_pieces >= PIECES_PER_BOMB {
                    pl.bomb_pieces = 0;
                    pl.bombs = pl.bombs.saturating_add(1);
                }
            }
            ITEM_STAR => {
                self.players[p].score += tables.item_cfg[ITEM_STAR as usize].score as u64;
            }
            _ => {
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::math::Fx;
    use crate::world::PH_COLLIDE;
    use crate::world::test_support::*;

    /// 全 step 回路的端到端腿：自机按住射击 → 相位 1 发弹 → 弹上行 → 相位 6 撞上同列的敌
    /// → 相位 7 产出命中事件。上面那条只驱动 collide+settle 两相位，这条把"自机真会开火、
    /// 弹真会飞到、事件真会出现在整局流程里"一并钉住（桥级冒烟的等价物，但不依赖 Godot）。
    #[test]
    fn player_holding_shot_hits_enemy_in_its_column() {
        use crate::input::{BTN_SHOT, InputFrame};
        let mut w = crate::step::World::new(1);
        // 自机出生 (0,384)；敌摆在同列上方 y=200（场内，弹的越界回收线是 y ∈ [-64,512]）
        let _e = spawn_enemy(&mut w, 0, 200, 9999);
        let img = crate::ecl::image::EclImage::empty();
        let mut hits = 0usize;
        for f in 0..120u32 {
            let mut inp = InputFrame::empty(f);
            inp.actions[0].buttons = BTN_SHOT;
            crate::step::step_with_director(&mut w, &crate::tables::TABLES_V0, &img, &inp, |_| {});
            hits += w
                .frame_events()
                .iter()
                .filter(|ev| ev.kind == crate::events::EVT_SHOT_HIT_ENEMY)
                .count();
        }
        assert!(hits > 0, "按住射击 120 帧应至少命中一次（实际 {hits}）");
    }

    /// 命中事实：坐标必须取**自机弹**当帧位置，不是敌心。
    /// 判别力所在——照抄 `EVT_ENEMY_DIED` 的惰性实现会填敌坐标，那样本测试立刻红。
    #[test]
    fn settle_shot_hit_emits_event_at_the_shot_not_the_enemy() {
        use crate::events::EVT_SHOT_HIT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 99); // hp 99：不死，只出命中事件
        let ei = w.body.enemies.get(e).unwrap();
        let (sx, sy) = (Fx::from_int(8), Fx::from_int(86)); // 刻意偏离敌心 (0,80)
        w.body.create_player_shot(crate::shots::ShotInit {
            x: sx,
            y: sy,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 3,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);

        assert_eq!(w.body.events_len, 1, "只该有命中事件（敌未死）");
        let ev = w.body.events[0];
        assert_eq!(ev.kind, EVT_SHOT_HIT_ENEMY);
        assert_eq!((ev.x, ev.y), (sx, sy), "坐标须取自机弹位置，不是敌心");
        assert_ne!(
            (ev.x, ev.y),
            (w.body.enemies.x[ei], w.body.enemies.y[ei]),
            "前提：弹与敌心不同点，否则本测试无判别力"
        );
        assert_eq!(ev.data[0], 3, "data[0] = damage");
        assert_eq!(
            (ev.a_index, ev.a_gen),
            (ei as u16, w.body.enemies.generation[ei])
        );
    }

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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert!(w.body.enemies.hp[ei] <= 0);
        assert_ne!(w.body.enemies.flags[ei] & ENEMY_DYING, 0);
        // 两条事件、且**命中在死亡之前**（同一趟里先记命中事实、再结算伤害）
        assert_eq!(w.body.events_len, 2);
        assert_eq!(w.body.events[0].kind, crate::events::EVT_SHOT_HIT_ENEMY);
        assert_eq!(w.body.events[1].kind, EVT_ENEMY_DIED);
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        // 只死一次；而且**第二发连命中事件都不发**——它被 ENEMY_DYING 门禁在记事实之前
        // 就挡掉了（overkill 不该冒第二次火花）。比原来只数死亡事件的断言更严。
        let kinds: Vec<u8> = w.body.events[..w.body.events_len as usize]
            .iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                crate::events::EVT_SHOT_HIT_ENEMY,
                crate::events::EVT_ENEMY_DIED
            ],
            "overkill：恰一次命中 + 恰一次死亡"
        );
        assert_eq!(
            w.body.enemies.hp[ei], 0,
            "第二发必须被 ENEMY_DYING 门禁挡住——hp 不得二次扣减（B4）"
        );
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(w.body.players[0].state_timer, DEATHBOMB_WINDOW);
    }

    #[test]
    fn settle_graze_counts_once_per_bullet() {
        use crate::input::InputFrame;
        use crate::world::test_support::step_t;
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
            step_t(&mut w, &InputFrame::empty(0));
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_ne!(w.body.bullets.flags[0] & BULLET_CLEARED, 0);
        assert_ne!(w.body.bullets.flags[1] & BULLET_CLEARED, 0);
        // 聚合：一条事件、count=2
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.events[0].data[0], 2);
    }

    /// 消弹转星星（M0-15）：3 弹异位被消 → 恰 3 星、槽序=消弹序、各在原弹位、
    /// 零初速、出生即磁吸 P0（ALIVE）。
    #[test]
    fn clear_converts_bullets_to_stars_at_positions() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::items::ITEM_STAR;
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 40, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, -10, 100);
        bullet_at(&mut w, 0, 100);
        bullet_at(&mut w, 10, 90);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let expect = [(-10, 100), (0, 100), (10, 90)];
        assert_eq!(w.body.items.iter_alive().count(), 3, "恰 3 星");
        for (i, &(x, y)) in expect.iter().enumerate() {
            assert_eq!(w.body.items.item_type[i], ITEM_STAR, "槽 {i} 类型");
            assert_eq!(w.body.items.x[i], Fx::from_int(x), "槽 {i} 原弹位 x");
            assert_eq!(w.body.items.y[i], Fx::from_int(y), "槽 {i} 原弹位 y");
            assert_eq!(w.body.items.magnet_to[i], 0, "槽 {i} 出生即磁吸 P0");
            assert_eq!(
                w.body.items.vx[i],
                Fx::ZERO,
                "槽 {i} 零初速（无散布无 RNG）"
            );
        }
    }

    /// 无 ALIVE 自机（决死窗口）→ 星星 MAGNET_NONE 正常下落。
    #[test]
    fn star_without_alive_player_falls_unmagnetized() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::items::MAGNET_NONE;
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.items.iter_alive().count(), 1);
        assert_eq!(w.body.items.magnet_to[0], MAGNET_NONE, "无存活自机不磁吸");
    }

    /// 星星拾取入账 +30（全 step 管线：消弹→星生于自机位→次帧行 5 拾取）。
    #[test]
    fn star_pickup_credits_30_score() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::input::InputFrame;
        use crate::world::test_support::step_t;
        let mut w = crate::step::World::new(1);
        // 弹贴自机位；同帧 field 消掉（趟一先于趟二 → 不中弹）→ 星生于自机位 → 次帧拾取
        spawn_field(&mut w, 0, 384, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 0, 384);
        let s0 = w.body.players[0].score;
        for _ in 0..3 {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].score, s0 + 30, "星星入账恰 +30");
    }

    /// 道具池满 → 少生成 + pool_full[ITEM] 逐颗计数（P4-a）。
    #[test]
    fn star_pool_full_degrades_counted() {
        use crate::field::FIELD_CLEAR_BULLETS;
        let mut w = crate::step::World::new(1);
        for _ in 0..(crate::items::ItemPool::CAP - 1) {
            w.body.drop_item(
                Fx::ZERO,
                Fx::from_int(200),
                crate::items::ITEM_POWER,
                &crate::tables::TABLES_V0,
            );
        }
        let pf0 = w.body.diag.pool_full[crate::world::POOL_ITEM];
        spawn_field(&mut w, 0, 100, 40, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, -10, 100);
        bullet_at(&mut w, 0, 100);
        bullet_at(&mut w, 10, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_ITEM],
            pf0 + 2,
            "3 消弹只剩 1 位 → 1 星 + 2 计数"
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_POOL_FULL);
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
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
            drop_count: crate::tables::drop_counts(&crate::tables::TABLES_V0, 1).0,
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.enemies.hp[ei], 3); // 5 - 2
    }

    /// 行 5 判别几何：拾取半径 16 + graze 16 = 32——道具距自机 30 拾、34 不拾（非圆心重合）。
    #[test]
    fn item_pickup_discriminates_radius_sum() {
        use crate::events::EVT_ITEM_PICKED;
        use crate::items::{ITEM_POWER, MAGNET_NONE, MAGNET_PICKED};
        let mut w = crate::step::World::new(1);
        // 自机在 (0,384)：一颗放在距 30（拾取和 32 内）、一颗放在距 34（拾取和外）。
        let near = w.body.drop_item(
            Fx::ZERO,
            Fx::from_int(384 - 30),
            ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let far = w.body.drop_item(
            Fx::ZERO,
            Fx::from_int(384 + 34),
            ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let ni = w.body.items.get(near).unwrap();
        let fi = w.body.items.get(far).unwrap();
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_ITEM_PICKED);
        assert_eq!(w.body.events[0].data[0], ITEM_POWER as i32);
        assert_eq!(w.body.items.magnet_to[ni], MAGNET_PICKED, "30px 应拾中");
        assert_eq!(w.body.items.magnet_to[fi], MAGNET_NONE, "34px 应仍未锁定");
    }

    /// 四类入账 + 满 power 转化：power=127 吃 POWER → 128；再吃 POWER → power 不动、score += 100。
    #[test]
    fn credit_power_caps_then_converts_to_point_score() {
        use crate::items::{ITEM_POINT, ITEM_POWER, POWER_MAX};
        let mut w = crate::step::World::new(1);
        w.body.players[0].power = POWER_MAX - 1;
        w.body.credit_item(0, ITEM_POWER, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].power, POWER_MAX);
        assert_eq!(w.body.players[0].score, 0);
        w.body.credit_item(0, ITEM_POWER, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].power, POWER_MAX, "满 power 不再涨");
        assert_eq!(
            w.body.players[0].score,
            crate::tables::TABLES_V0.item_cfg[ITEM_POINT as usize].score as u64,
            "满 power 转化为 POINT 分值"
        );
    }

    /// 碎片进位跨界：life_pieces=4 再吃 1 → lives+1、pieces==0；bombs 同构。
    #[test]
    fn piece_carry_crosses_boundary_exactly() {
        use crate::items::{ITEM_BOMB_PIECE, ITEM_LIFE_PIECE, PIECES_PER_BOMB, PIECES_PER_LIFE};
        let mut w = crate::step::World::new(1);

        // 未满阈值不得进位（钉死 `>=` 没有把常量本身抹掉——I-1）：差 2 时吃一次仍差 1，
        // 命数不动。旧 `==` 语义下这半靠语义本身兜底，换 `>=` 后必须有测试显式钉住。
        w.body.players[0].life_pieces = PIECES_PER_LIFE - 2;
        let lives_before = w.body.players[0].lives;
        w.body
            .credit_item(0, ITEM_LIFE_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].lives, lives_before, "未满阈值不得进位");
        assert_eq!(w.body.players[0].life_pieces, PIECES_PER_LIFE - 1);

        // 再吃一次，正好到阈值 → 进位。
        let lives_before = w.body.players[0].lives;
        w.body
            .credit_item(0, ITEM_LIFE_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].lives, lives_before + 1);
        assert_eq!(w.body.players[0].life_pieces, 0);

        // bombs 同构：未满阈值不进位 → 到阈值再进位。
        w.body.players[0].bomb_pieces = PIECES_PER_BOMB - 2;
        let bombs_before = w.body.players[0].bombs;
        w.body
            .credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, bombs_before, "未满阈值不得进位");
        assert_eq!(w.body.players[0].bomb_pieces, PIECES_PER_BOMB - 1);

        let bombs_before = w.body.players[0].bombs;
        w.body
            .credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, bombs_before + 1);
        assert_eq!(w.body.players[0].bomb_pieces, 0);
    }

    /// P4-a 相邻：命数/炸弹到 u8 上限后继续吃碎片 → **饱和**，不 panic 不回绕（B12）。
    /// 确定性本来就没破（跨平台一致地回绕），但入账值荒谬；且 debug 会 panic。
    #[test]
    fn credit_item_saturates_lives_and_bombs_at_u8_max() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].lives = u8::MAX;
        w.body.players[0].life_pieces = crate::items::PIECES_PER_LIFE - 1;
        w.body
            .credit_item(0, crate::items::ITEM_LIFE_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].lives, u8::MAX, "命数须饱和在 u8::MAX");
        assert_eq!(w.body.players[0].life_pieces, 0, "碎片照常清零进位");

        w.body.players[0].bombs = u8::MAX;
        w.body.players[0].bomb_pieces = crate::items::PIECES_PER_BOMB - 1;
        w.body
            .credit_item(0, crate::items::ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, u8::MAX, "炸弹数须饱和");
        assert_eq!(w.body.players[0].bomb_pieces, 0);
    }

    /// 同帧双拾取幂等：趟三首见即标 `MAGNET_PICKED`，二次 hit 遇标即跳过入账（手工双推 hits，
    /// 复刻 `hits_push_clear_and_overflow` 一类手工构造 hits 的先例）。
    #[test]
    fn item_picked_only_once() {
        use crate::events::{EVT_ITEM_PICKED, ROW_ITEM_PLAYER};
        use crate::items::{ITEM_POINT, MAGNET_PICKED};
        let mut w = crate::step::World::new(1);
        let h = w.body.drop_item(
            Fx::ZERO,
            Fx::from_int(384),
            ITEM_POINT,
            &crate::tables::TABLES_V0,
        );
        let i = w.body.items.get(h).unwrap();
        w.body.push_hit(ROW_ITEM_PLAYER, i as u16, 0);
        w.body.push_hit(ROW_ITEM_PLAYER, i as u16, 0); // 同帧双 hit：收集层不会真产出，手工构造
        // 直调 settle（不经 collide）：phase_guard 须直接押到 PH_SETTLE。
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_SETTLE;
        }
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.items.magnet_to[i], MAGNET_PICKED);
        let picks = (0..w.body.events_len as usize)
            .filter(|&k| w.body.events[k].kind == EVT_ITEM_PICKED)
            .count();
        assert_eq!(picks, 1, "同帧多 hit 只入账一次");
        assert_eq!(
            w.body.players[0].score,
            crate::tables::TABLES_V0.item_cfg[ITEM_POINT as usize].score as u64,
            "账本也只入一次"
        );
    }

    /// 拾取入账事件字段：a_index/a_gen = 道具句柄位，data = [类型, 玩家号]。
    #[test]
    fn item_picked_event_shape() {
        use crate::events::EVT_ITEM_PICKED;
        use crate::items::ITEM_BOMB_PIECE;
        let mut w = crate::step::World::new(1);
        let h = w.body.drop_item(
            Fx::ZERO,
            Fx::from_int(384),
            ITEM_BOMB_PIECE,
            &crate::tables::TABLES_V0,
        );
        let i = w.body.items.get(h).unwrap();
        let item_gen = w.body.items.generation[i];
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.events_len, 1);
        let ev = w.body.events[0];
        assert_eq!(ev.kind, EVT_ITEM_PICKED);
        assert_eq!(ev.a_index, i as u16);
        assert_eq!(ev.a_gen, item_gen);
        assert_eq!(ev.data, [ITEM_BOMB_PIECE as i32, 0]);
    }

    #[test]
    fn settle_enemy_death_emits_render_req_with_pos_sprite_score() {
        use crate::consts::REQ_ENEMY_DEATH;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 1); // hp 1
        let ei = w.body.enemies.get(e).unwrap();
        w.body.enemies.sprite[ei] = 7;
        w.body.enemies.score[ei] = 450;
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
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1, "一敌一死一请求");
        assert_eq!(reqs[0].id, REQ_ENEMY_DEATH);
        assert_eq!(
            reqs[0].args,
            [
                w.body.enemies.x[ei].raw(),
                w.body.enemies.y[ei].raw(),
                7,
                450,
                0,
                0
            ],
            "位序 x/y/sprite/score——判别值防对调假绿"
        );
    }

    /// 全字段 EnemyInit（exhaustive）：位置固定 (0,80)、hp=1、其余惰性。
    /// 表号走 `tables::drop_counts` 展开——与 `sys_spawn_enemy` 的生成路径同一条口子。
    fn enemy_with_drop_table(table: u16) -> crate::enemy::EnemyInit {
        crate::enemy::EnemyInit {
            x: Fx::ZERO,
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
            drop_count: crate::tables::drop_counts(&crate::tables::TABLES_V0, table).0,
            score: 100,
        }
    }

    /// **特征化测试**（T1 重构的安全网）：敌死掉落的逐颗 `(type, vx, vy)` 全序列。
    /// `vx`/`vy` 来自 `spawn_drop` 的世界 RNG 散布，故本测试同时钉住"掉了什么"
    /// **与** "RNG 被消耗了几次、按什么顺序"——掉落迁成按类型计数后这三者都必须不变。
    /// 期望值是**重构前实测**填入的（见计划 T1 Step 2）。
    #[test]
    fn enemy_death_drop_sequence_is_pinned() {
        let mut w = crate::step::World::new(1);
        let e = w.body.create_enemy(enemy_with_drop_table(1));
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei],
            y: w.body.enemies.y[ei],
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 99,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);

        let got: Vec<(u8, i32, i32)> = w
            .body
            .items
            .iter_alive()
            .map(|i| {
                (
                    w.body.items.item_type[i],
                    w.body.items.vx[i].raw(),
                    w.body.items.vy[i].raw(),
                )
            })
            .collect();
        // 掉落表 1 = [(ITEM_POWER,2),(ITEM_POINT,1)]；POWER=0 < POINT=1，
        // 表序恰好就是类型升序 —— 这正是"改成按类型计数后顺序不漂"的原因。
        assert_eq!(
            got,
            vec![
                (0, 25626, -180916),
                (0, -35167, -188191),
                (1, 9857, -167669),
            ]
        );
    }
}
