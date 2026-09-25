//! 相位 7 · 结算三趟（D9）——**唯一改状态者**。
//!
//! 趟一 清除/防护：行6 消弹（**标记不回收**，回收在 cleanup 相位9）+ 逐弹原位转星星
//!   （M0-15 一律转化：出生即磁吸存活自机，30 分经济回流）+ 按 field 索引升序发聚合
//!   `FieldCleared`；行10 清弹 field × 激光 → 切收缩（幂等、不发事件）。**先于趟二** ——
//!   故同帧作用区能救下本会命中自机的弹/激光（bomb 救命）。
//! 趟二 伤害：行4/7 敌人扣血（overkill/无敌帧门禁）；行1/3 自机中弹 → 决死窗口（行1 跳过已清除的弹）；
//!   行9 激光中弹（跳过趟一被取消/已回收的激光）。
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
        self.players[p].hit_frame = self.frame; // 遡行落点的原点（时间机制内核刀）
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
            self.kill_enemy(e, tables);
        }
    }

    /// 敌人的**完整死亡效果**——死亡路径的唯一实现处。
    ///
    /// **幂等**：已 `ENEMY_DYING` 即直接返回（与 settle 趟二的 overkill 门禁同构）。
    /// 顺序：hp 下钳 → 标 dying → 撒掉落 → 记分 → 事件 → 死亡特效请求。
    ///
    /// 两个调用方：`damage_enemy` 的 `hp<=0` 分支（被自机打死）/ `SYS_DIE`（脚本显式）。
    /// **注意 D9 自燃不走这里**——主协程返回是"静默退场"，不掉道具不加分不发事件
    /// （`ecl::vm::run_tasks` 的 `Exec::End` 分支只置 `ENEMY_DYING`）。三条路径的差异是
    /// 脚本作者最容易搞混的一处，见 `docs/ecl-lang.md`。
    ///
    /// **只标记不回收**——槽要活到相位 8 供表现层读；相位 9 cleanup 收尸。
    pub(crate) fn kill_enemy(&mut self, e: usize, tables: &WorldTables) {
        if self.enemies.flags[e] & ENEMY_DYING != 0 {
            return; // 幂等
        }
        // `min(0)` 而非置 0 —— **对 `damage_enemy` 这个调用方是恒等的**（它只在 `hp<=0`
        // 分支里调，`min(0)` 取的必是 `hp` 自己），判别力全部来自另一个调用方 `SYS_DIE`：
        // 那条路径打在满血 boss 上，压到 0 才不会让 HUD 当帧显示"满血的死人"；而对已被
        // overkill 打成负血的敌，`min` 保住负值不被抹平（负血是可观测的诊断信息）。
        // **两条判别腿都在 `ecl::syscall.rs::tests::die_on_full_hp_enemy_zeroes_hp`**
        // （满血 die → hp 落 0；再 overkill 到 hp=-5 后 `kill_enemy` → 仍是 -5）。
        // 本文件的 `settle_overkill_two_shots_one_death_event` **区分不了**这两种写法
        // （它 hp=1/dmg=1 → hp 恰好是 0，第二发又被 dying 门禁挡在 damage_enemy 之外）。
        self.enemies.hp[e] = self.enemies.hp[e].min(0);
        self.enemies.flags[e] |= ENEMY_DYING;
        // 掉落直接分配（A6/A7）：撒敌身上的逐类型计数（表号已在生成时展开）。
        self.spill_drops(e, tables);
        // **强制加分**（人类裁定 D-5，2026-07-30）：此前 `enemies.score` 是纯装饰字段——
        // 只被塞进 `EVT_ENEMY_DIED.data[0]` 与 `REQ_ENEMY_DEATH` 供表现层显示，打死敌人的
        // 全部收益来自掉落被 `credit_item` 入账。记自机 0，与 `SYS_ADD_SCORE` 同口径。
        let bonus = self.enemies.score[e] as u64;
        self.players[0].score = self.players[0].score.saturating_add(bonus);
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

    pub(crate) fn settle(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_SETTLE);
        // 同 collide，spec §4：门禁挂 C 组、不是"是否冻结"。顺手堵上一个漏洞——符卡
        // 计时住本相位尾（`settle_spells`）⇒ 冻 C 时符卡不倒计时，没法用时停白嫖 survival 卡。
        if self.scene_frozen() {
            self.settle_stop_touch(); // 行 8 结算（玩法刀）；符卡趟照旧不跑 ⇒ 停止不烧符卡时间
            return;
        }
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
            // FIELD_NO_STAR 的区只清不转星（boss 换段刀 spec §6）；幂等门已保证一颗弹只处理一次，
            // 故多区重叠时由 hits 序中第一条命中的区决定，确定性。
            if self.fields.flags[h.active as usize] & crate::field::FIELD_NO_STAR == 0 {
                self.spawn_star_at(self.bullets.x[b], self.bullets.y[b], star_target);
            }
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
        // 行 10：清弹 field × 激光 → 切收缩。`cancel_laser_index` 本身幂等（多 field 同帧压
        // 同一条只切一次），不发事件；`fade == 0` 的回收留给下一帧相位 5。
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row != crate::events::ROW_FIELD_LASER {
                continue;
            }
            self.cancel_laser_index(h.passive as usize);
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
                crate::events::ROW_LASER_PLAYER_HIT => {
                    // 趟一被 field 取消（或已回收）的激光不杀人 —— bomb/清弹救命。
                    let i = h.active as usize;
                    if !self.lasers.is_alive(i)
                        || self.lasers.state[i] != crate::lasers::LASER_ACTIVE
                    {
                        continue;
                    }
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

    /// 行 8 结算（停止冻结中）：未清除的弹置 `BULLET_CLEARED` + 该自机 `STOP_TOUCH_SCORE`。
    /// 不转星星、不发事件；同一颗弹多个自机碰到按 hits 序首个入账（I4）。回收在相位 9 冻结分支。
    fn settle_stop_touch(&mut self) {
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row != crate::events::ROW_STOP_TOUCH {
                continue;
            }
            let b = h.active as usize;
            if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                continue;
            }
            self.bullets.flags[b] |= crate::bullets::BULLET_CLEARED;
            let p = h.passive as usize;
            self.players[p].score = self.players[p]
                .score
                .saturating_add(crate::player::STOP_TOUCH_SCORE);
        }
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
                    // 停止库存上限（玩法刀）：满了碎片照清、不加。
                    if pl.bombs < crate::player::STOP_STOCK_MAX {
                        pl.bombs += 1;
                    }
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
    use crate::math::{Angle, Fx};
    #[cfg(debug_assertions)]
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

        assert_eq!(w.body.frame_events_len, 1, "只该有命中事件（敌未死）");
        let ev = w.body.frame_events[0];
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
        assert_eq!(w.body.frame_events_len, 2);
        assert_eq!(
            w.body.frame_events[0].kind,
            crate::events::EVT_SHOT_HIT_ENEMY
        );
        assert_eq!(w.body.frame_events[1].kind, EVT_ENEMY_DIED);
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
        let kinds: Vec<u8> = w.body.frame_events[..w.body.frame_events_len as usize]
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
            born_frame: 0,
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
        assert_eq!(w.body.frame_events_len, 1);
        assert_eq!(w.body.frame_events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.frame_events[0].data[0], 2);
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

    /// 道具池满时的消弹降级（P4-a）：**逐颗计数、循环不短路**——`spawn_star_at` 的文档
    /// 明写这一条，而它此前无测试；F12 正是在真内容里踩到的这条路（收卡一帧 626 颗弹全转
    /// 星星，四个难度档都溢出过道具池）。
    ///
    /// 判别力来源：先把池灌满，再消 **3** 颗弹 ⇒ `pool_full[POOL_ITEM]` 必须恰好 +3。
    /// 若实现改成"第一颗分配失败就 break"（最自然的错法）得到 +1；若整批只记一次也得 +1。
    /// 两种错法都被这条捉住，而"消 1 颗"的写法对它们全瞎。
    #[test]
    fn star_pool_full_counts_every_missing_star() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::world::POOL_ITEM;
        let mut w = crate::step::World::new(1);
        // 灌满道具池（哑星星，位置无关紧要）。
        while w
            .body
            .items
            .alloc(crate::items::ItemInit {
                x: Fx::ZERO,
                y: Fx::ZERO,
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: crate::items::ITEM_STAR,
                magnet_to: crate::items::MAGNET_NONE,
                timer: 0,
            })
            .is_some()
        {}
        let alive_before = w.body.items.iter_alive().count();
        let full_before = w.body.diag.pool_full[POOL_ITEM];

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

        assert_eq!(
            w.body.diag.pool_full[POOL_ITEM],
            full_before + 3,
            "三颗弹被消而池满 ⇒ 逐颗计三次（短路或整批只计一次都会给 +1）"
        );
        assert_eq!(
            w.body.items.iter_alive().count(),
            alive_before,
            "池满 ⇒ 一颗星星都不该多出来"
        );
        // 弹照消不误——星星生不出来不影响消弹本身。（真正的回收在相位 9 `cleanup`，
        // 本相位只置 `BULLET_CLEARED`，故这里查标志位而非存活数。）
        for i in w.body.bullets.iter_alive() {
            assert!(
                w.body.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0,
                "弹 {i} 应已被标记消除"
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
        let total: i32 = (0..w.body.frame_events_len as usize)
            .map(|k| w.body.frame_events[k].data[0])
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

    /// 敌死撒 `drop_count` 掉落：表 1 展开后 = 2 POWER + 1 POINT，落点 = 敌死位置
    /// （散布只改速度）。两敌同帧死 → 掉落顺序 = 结算序（低索引敌先掉，RNG 消耗序钉死）。
    #[test]
    fn settle_death_drops_by_table_in_settlement_order() {
        use crate::items::{ITEM_POINT, ITEM_POWER};
        let mut w = crate::step::World::new(7);
        let enemy_at = |x: i32| crate::enemy::EnemyInit {
            x: Fx::from_int(x),
            y: Fx::from_int(80),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            vel_from_0: 0,
            vel_from_1: 0,
            vel_to_0: 0,
            vel_to_1: 0,
            vel_t: 0,
            vel_dur: 0,
            vel_easing: 0,
            vel_active: 0,
            vel_space: 0,
            vel_touched: 0,
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
            anm_state_frame: 0,
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
        assert_eq!(w.body.frame_events_len, 1);
        assert_eq!(w.body.frame_events[0].kind, EVT_ITEM_PICKED);
        assert_eq!(w.body.frame_events[0].data[0], ITEM_POWER as i32);
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
        assert_eq!(
            w.body.players[0].bombs,
            u8::MAX,
            "库存超上限的直写值不被进位推高"
        );
        assert_eq!(w.body.players[0].bomb_pieces, 0);
    }

    /// 停止碎片（玩法刀）：4 枚进 1 发；库存已满时碎片照清、不加（判别腿：满库存 5 与 4 各一次）。
    #[test]
    fn stop_piece_carry_respects_stock_max() {
        use crate::items::{ITEM_BOMB_PIECE, PIECES_PER_BOMB};
        use crate::player::STOP_STOCK_MAX;
        assert_eq!(PIECES_PER_BOMB, 4, "gameplay-design §1：4 碎片 = 1 发");
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = STOP_STOCK_MAX - 1;
        w.body.players[0].bomb_pieces = PIECES_PER_BOMB - 1;
        w.body
            .credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "未满：进位加一");
        assert_eq!(w.body.players[0].bomb_pieces, 0);
        w.body.players[0].bomb_pieces = PIECES_PER_BOMB - 1;
        w.body
            .credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "已满：不加");
        assert_eq!(w.body.players[0].bomb_pieces, 0, "已满：碎片照清");
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
        let picks = (0..w.body.frame_events_len as usize)
            .filter(|&k| w.body.frame_events[k].kind == EVT_ITEM_PICKED)
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
        assert_eq!(w.body.frame_events_len, 1);
        let ev = w.body.frame_events[0];
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

    /// D-5：敌死**强制加分**——`enemies.score[e]` 记进自机 0 的分数。
    /// **判别腿：测试内一颗道具都不拾取**。此前敌人的 score 是纯装饰字段（只塞进
    /// EVT_ENEMY_DIED 供表现层显示），打死敌人的全部收益来自掉落被 credit_item 入账。
    /// 若测试里让自机拾到了道具，就分不清这分是敌人加的还是道具加的——D9 那刀正是
    /// 在这里栽过（"score 未变"断言因分数走 credit_item 而失效，见 vm.rs 的订正注释）。
    #[test]
    fn enemy_death_credits_its_score_bonus() {
        use crate::enemy::ENEMY_DYING;
        let mut w = crate::step::World::new(1);
        // `spawn_enemy` 的敌 `drop_count` 全零、`score = 100`；摆在 (0,80)，
        // 自机在默认出生点 (0,384) —— 远离敌，也远离任何道具（本测试压根不生道具）。
        let e = spawn_enemy(&mut w, 0, 80, 1);
        let ei = w.body.enemies.get(e).unwrap();
        assert_eq!(
            w.body.enemies.drop_count[ei],
            [0u8; crate::items::ITEM_TYPE_COUNT],
            "判别腿前提：这敌不掉任何道具，否则分不清分数来自敌人还是 credit_item"
        );
        let before = w.body.players[0].score;
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
        assert_ne!(w.body.enemies.flags[ei] & ENEMY_DYING, 0, "前提：敌真死了");
        assert_eq!(
            w.body.items.iter_alive().count(),
            0,
            "判别腿：一颗道具都没生成 → 分数只可能是敌人自己加的"
        );
        assert_eq!(
            w.body.players[0].score,
            before + 100,
            "敌死把 enemies.score 记进自机 0"
        );
    }

    // ── 敌人死亡的三条路径，测试分居两处（导航线索，别让这组测试散丢）─────────────
    //
    // 1. **被自机打死** → `damage_enemy` 的 `hp<=0` 分支 → `kill_enemy`：
    //    本文件，`enemy_death_credits_its_score_bonus`（加分）
    //    + `settle_death_drops_by_table_in_settlement_order`（掉落）
    //    + `settle_enemy_death_emits_render_req_with_pos_sprite_score`（请求）。
    // 2. **脚本显式 `die()`** → `SYS_DIE` → `kill_enemy_by_handle` → `kill_enemy`：测试住
    //    `ecl::syscall.rs` 的"敌人死亡效果四 syscall"节（`die_runs_the_full_death_effect`
    //    等七条），表层"两指令降低"那半住 `stg-ecl-compiler` 的 codegen 测试。
    // 3. **D9 自燃**（主协程自然返回）→ `ecl::vm::run_tasks` 的 `Exec::End` 分支，
    //    **不走 `kill_enemy`**：静默退场，不掉道具、不加分、不发 `EVT_ENEMY_DIED`。
    //    守它的是 `ecl::vm::tests::enemy_main_task_returning_self_destructs_quietly`
    //    —— 在 vm.rs 而不在这里，因为它要 `async_image`/`spawn_sub_internal` 那套脚手架
    //    才能造出**真带 main_task**的敌；在本文件里手工置 `ENEMY_DYING` 造不出那条路径
    //    （`main_task=0` 的敌根本不会被 D9 分支求值，那样的测试无论实现对错都恒绿）。
    //    自本刀起，它那条 `score` 断言是**真判别腿**：谁把 `kill_enemy` 挂进 `Exec::End`
    //    它立刻红（本刀用变异实测确认过，见 task-2-report.md 修复轮）。

    /// 幂等：`kill_enemy` 对已 dying 的敌是 no-op（掉落 / 加分 / 事件各只发生一次）。
    #[test]
    fn kill_enemy_is_idempotent() {
        let mut w = crate::step::World::new(1);
        let e = w.body.create_enemy(enemy_with_drop_table(1)); // 3 颗掉落 + score=100
        let ei = w.body.enemies.get(e).unwrap();
        let before = w.body.players[0].score;
        w.body.kill_enemy(ei, &crate::tables::TABLES_V0);
        w.body.kill_enemy(ei, &crate::tables::TABLES_V0);
        assert_eq!(w.body.items.iter_alive().count(), 3, "掉落只撒一次");
        assert_eq!(w.body.players[0].score, before + 100, "加分只记一次");
        assert_eq!(w.body.frame_events_len, 1, "EVT_ENEMY_DIED 只发一次");
    }

    /// 全字段 EnemyInit（exhaustive）：位置固定 (0,80)、hp=1、其余惰性。
    /// 表号走 `tables::drop_counts` 展开——与 `sys_spawn_enemy` 的生成路径同一条口子。
    fn enemy_with_drop_table(table: u16) -> crate::enemy::EnemyInit {
        crate::enemy::EnemyInit {
            x: Fx::ZERO,
            y: Fx::from_int(80),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            vel_from_0: 0,
            vel_from_1: 0,
            vel_to_0: 0,
            vel_to_1: 0,
            vel_t: 0,
            vel_dur: 0,
            vel_easing: 0,
            vel_active: 0,
            vel_space: 0,
            vel_touched: 0,
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
            anm_state_frame: 0,
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

    /// FIELD_NO_STAR（boss 换段刀 spec §6）：弹照清、清弹事件照计，但不转星星。
    #[test]
    fn no_star_field_clears_without_spawning_stars() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 40, FIELD_CLEAR_BULLETS | FIELD_NO_STAR, 1);
        let b = bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let bi = w.body.bullets.get(b).unwrap();
        assert_ne!(w.body.bullets.flags[bi] & crate::bullets::BULLET_CLEARED, 0);
        assert_eq!(w.body.items.iter_alive().count(), 0, "不转星星");
        assert_eq!(
            w.body.frame_events[0].kind,
            crate::events::EVT_FIELD_CLEARED
        );
    }

    /// `set_enemy_invuln`（boss 换段刀 spec §5.1）：无敌期间自机弹重叠既不掉血也不发命中事件；
    /// 解除后同一发照打（对照腿，证明是 invuln 挡的而不是几何没碰上）。
    #[test]
    fn invulnerable_enemy_takes_no_damage_and_no_hit_event() {
        use crate::events::EVT_SHOT_HIT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 100, 50);
        w.body.set_enemy_invuln(e, 5);
        w.body.create_player_shot(crate::shots::ShotInit {
            x: Fx::ZERO,
            y: Fx::from_int(100),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 7,
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
        let i = w.body.enemies.get(e).unwrap();
        assert_eq!(w.body.enemies.hp[i], 50);
        assert!(
            (0..w.body.frame_events_len as usize)
                .all(|k| w.body.frame_events[k].kind != EVT_SHOT_HIT_ENEMY)
        );

        w.body.set_enemy_invuln(e, 0);
        w.body.hits_len = 0;
        w.body.frame_events_len = 0;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.enemies.hp[i], 43, "解除后同一发打得进");
    }

    /// `clear_field_at` 几何判别（boss 换段刀 spec §6）：圆内弹被清、圆外弹保留（证明半径真的生效、
    /// 不是全屏）；stars=true 给星。
    #[test]
    fn clear_field_at_clears_inside_and_keeps_outside() {
        let mut w = crate::step::World::new(1);
        w.body.create_field(crate::field::clear_field_at(
            Fx::ZERO,
            Fx::from_int(100),
            Fx::from_int(40),
            true,
        ));
        let inside = bullet_at(&mut w, 10, 100);
        let outside = bullet_at(&mut w, 150, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let cleared = |w: &crate::step::World, h| {
            let i = w.body.bullets.get(h).unwrap();
            w.body.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0
        };
        assert!(cleared(&w, inside));
        assert!(!cleared(&w, outside));
        assert_eq!(w.body.items.iter_alive().count(), 1, "stars=true 给星");
    }

    // ── Task 3：碰撞行 9/10（激光 × 自机 / 清弹 field × 激光）────────────────────

    /// 一条纵向、朝下（BAM 16384）、原点 (0,100) 的激光：start 0、end = start_len = len。
    /// `warn` 决定出生状态（0 → 出生即 state 1）。
    fn laser_init(
        warn: u16,
        active: u16,
        fade: u16,
        len: i32,
        width: i32,
    ) -> crate::lasers::LaserInit {
        crate::lasers::LaserInit {
            ox: Fx::ZERO,
            oy: Fx::from_int(100),
            angle: Angle(16384),
            omega: 0,
            start: Fx::ZERO,
            end: Fx::from_int(len),
            start_len: Fx::from_int(len),
            speed: Fx::ZERO,
            width: Fx::from_int(width),
            sprite: 0,
            warn,
            active,
            fade,
            timer: 0,
            state: 0,
            anchor_idx: crate::lasers::ANCHOR_NONE,
            anchor_gen: 0,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            dang: 0,
            px: Fx::ZERO,
            py: Fx::ZERO,
            pang: Angle::ZERO,
            flags: 0,
            born_frame: 0,
        }
    }

    /// 时序（Global Constraints）：warn 30 的激光步 0..29 后是 state 0，步 30 后是 state 1；
    /// 相位 5 切态先于相位 6 判定 ⇒ 生效当帧即杀人。
    #[test]
    fn laser_kills_only_when_active() {
        use crate::player::{LIFE_ALIVE, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.create_laser(laser_init(30, 9999, 0, 500, 16));
        // 预警 30 帧（步 0..29）不判定：自机始终存活。
        for f in 0..30u32 {
            step_t(&mut w, &crate::input::InputFrame::empty(f));
            assert_eq!(
                w.body.players[0].life_state, LIFE_ALIVE,
                "预警第 {f} 步不得判定"
            );
        }
        // 第 30 步相位 5 切到 state 1，同帧相位 6 收集、相位 7 结算。
        step_t(&mut w, &crate::input::InputFrame::empty(30));
        assert_eq!(
            w.body.players[0].life_state, LIFE_DEATHWINDOW,
            "生效当帧进入死亡窗"
        );
    }

    /// 门禁同行 1：`invuln != 0` 不判；清掉 invuln 后同一帧条件照样成立（对照腿，证明是
    /// invuln 挡的而不是几何没碰上）。
    #[test]
    fn laser_hit_respects_invuln() {
        use crate::player::{LIFE_ALIVE, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.players[0].invuln = 60;
        w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        // 激光出生即 ACTIVE（warn=0）；跑几帧穿过无敌窗。
        for f in 0..3u32 {
            step_t(&mut w, &crate::input::InputFrame::empty(f));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE, "无敌期间激光不判");
        // 对照腿：解开无敌后同一条激光立刻判死（否则本测试对实现全瞎）。
        w.body.players[0].invuln = 0;
        step_t(&mut w, &crate::input::InputFrame::empty(3));
        assert_eq!(
            w.body.players[0].life_state, LIFE_DEATHWINDOW,
            "解除无敌后同一激光照杀"
        );
    }

    /// 判定半高 = width/2（不是 width/4）：自机半径 r、激光 width 16。
    /// 离轴 8 + r − 1 命中、8 + r + 1 不中；误用 width/4 时第一条会漏判而红。
    #[test]
    fn laser_width_is_hit_width() {
        use crate::player::{LIFE_ALIVE, LIFE_DEATHWINDOW};
        let hit_r = crate::step::World::new(1).body.players[0].hit_radius;
        let half = Fx::from_int(8); // width 16 的半高
        let run = |offset: Fx| -> u8 {
            let mut w = crate::step::World::new(1);
            w.body.players[0].x = offset;
            w.body.players[0].y = Fx::from_int(200);
            w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = PH_COLLIDE;
            }
            w.body.collide(&crate::tables::TABLES_V0);
            w.body.settle(&crate::tables::TABLES_V0);
            w.body.players[0].life_state
        };
        // 夹具前提：r 非零且与 8 拉开，才可能区分 width/2 与 width/4。
        assert!(hit_r > Fx::ZERO, "前提：自机 hit_radius > 0");
        assert_eq!(
            run(half + hit_r - Fx::ONE),
            LIFE_DEATHWINDOW,
            "离轴 8 + r − 1 应命中（半高必须取 width/2）"
        );
        assert_eq!(
            run(half + hit_r + Fx::ONE),
            LIFE_ALIVE,
            "离轴 8 + r + 1 不应命中"
        );
    }

    /// 行 10：全屏清弹 field（`fullscreen_clear_field()` 的等价物）当帧把 state<2 的激光
    /// 切到 2；`fade == 0` 的激光下一帧相位 5 回收。
    #[test]
    fn fullscreen_field_cancels_all_lasers() {
        use crate::lasers::{LASER_ACTIVE, LASER_FADE, LASER_WARN};
        let mut w = crate::step::World::new(1);
        let h_warn = w.body.create_laser(laser_init(30, 9999, 0, 500, 16));
        let h_active = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        let (i_warn, i_active) = (
            w.body.lasers.get(h_warn).unwrap(),
            w.body.lasers.get(h_active).unwrap(),
        );
        assert_eq!(w.body.lasers.state[i_warn], LASER_WARN);
        assert_eq!(w.body.lasers.state[i_active], LASER_ACTIVE);

        // 场心 (0,224)、半径 400：两条激光的线段都落在圆内。
        w.body.create_field(crate::field::fullscreen_clear_field());
        step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.lasers.state[i_warn], LASER_FADE, "state 0 → 2");
        assert_eq!(w.body.lasers.state[i_active], LASER_FADE, "state 1 → 2");

        // fade == 0 ⇒ 下一帧相位 5 收起（相位 7 只切状态、回收在相位 5）。
        step_t(&mut w, &crate::input::InputFrame::empty(1));
        assert!(!w.body.laser_alive(h_warn), "fade==0 下一帧回收");
        assert!(!w.body.laser_alive(h_active), "fade==0 下一帧回收");
    }

    /// 行 10 几何：两条平行激光，小圆 field 只碰到其中一条。
    #[test]
    fn local_field_cancels_only_touched() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::lasers::{LASER_ACTIVE, LASER_FADE};
        let mut w = crate::step::World::new(1);
        let h_touched = w.body.create_laser(laser_init(0, 9999, 0, 500, 16)); // x = 0
        let mut far = laser_init(0, 9999, 0, 500, 16);
        far.ox = Fx::from_int(200);
        let h_far = w.body.create_laser(far);
        let (i_touched, i_far) = (
            w.body.lasers.get(h_touched).unwrap(),
            w.body.lasers.get(h_far).unwrap(),
        );
        // 半径 20、圆心 (0,200) 的局部清弹区：碰到 x=0 那条，够不着 x=200 那条。
        spawn_field(&mut w, 0, 200, 20, FIELD_CLEAR_BULLETS, 1);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.lasers.state[i_touched], LASER_FADE, "碰到的被取消");
        assert_eq!(w.body.lasers.state[i_far], LASER_ACTIVE, "没碰到的保持生效");
    }

    /// Review Focus 2：同一帧里 field 碰到激光、激光也罩住自机 —— 趟一先取消、趟二跳过，
    /// 自机不死。夹具同时断言两行都收了（否则"活着"是几何没碰上的假绿）。
    #[test]
    fn field_cancel_same_frame_saves_player() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::player::LIFE_ALIVE;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        spawn_field(&mut w, 0, 200, 20, FIELD_CLEAR_BULLETS, 1);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let rows: Vec<u8> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k].row)
            .collect();
        assert!(
            rows.contains(&crate::events::ROW_LASER_PLAYER_HIT),
            "前提：行 9 确实收了"
        );
        assert!(
            rows.contains(&crate::events::ROW_FIELD_LASER),
            "前提：行 10 确实收了"
        );
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.players[0].life_state, LIFE_ALIVE,
            "趟一先取消 → 趟二不杀人"
        );
    }

    /// Review Focus 5：时停期间激光不推进、不判定（`collide()` 在 `scene_frozen()` 提前
    /// return），自机站在生效激光上不死。
    #[test]
    fn frozen_scene_laser_does_not_kill() {
        use crate::player::LIFE_ALIVE;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        // 照 integrate.rs 敌人时停测试：freeze_left[0] > 0 = 冻 C（场景）。
        w.body.freeze_left = [5, 0];
        step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE, "时停期间激光不判");
    }

    // ── F1（终审）：判定原语对任意输入安全，挂靠原点饱和钳位 ──────────────────────

    /// field 在 (30000,30000)、激光原点 (0,0) 朝 45°：旧实现在 `dx·c + dy·s` 的 Fx 加法上
    /// 溢出（dev panic）。新原语把远点当无穷远，step 一帧不 panic 且激光不被取消。
    #[test]
    fn far_clear_field_does_not_overflow_or_cancel() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::lasers::LASER_ACTIVE;
        let mut w = crate::step::World::new(1);
        let mut it = laser_init(0, 9999, 0, 500, 16);
        it.ox = Fx::ZERO;
        it.oy = Fx::ZERO;
        it.angle = Angle(8192); // 45°
        let h = w.body.create_laser(it);
        let i = w.body.lasers.get(h).unwrap();
        spawn_field(&mut w, 30000, 30000, 20, FIELD_CLEAR_BULLETS, 9999);
        step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(
            w.body.lasers.state[i], LASER_ACTIVE,
            "远处的清弹区够不着，激光不被取消"
        );
    }

    /// 激光原点 (−4096,0) 朝 0°、field 在 (32000,0)：`px − ox` 在旧实现里溢出（dev panic）。
    /// 新原语视为无穷远，不 panic、不取消。
    #[test]
    fn far_clear_field_negative_origin_does_not_overflow() {
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::lasers::LASER_ACTIVE;
        let mut w = crate::step::World::new(1);
        let mut it = laser_init(0, 9999, 0, 500, 16);
        it.ox = Fx::from_int(-4096);
        it.oy = Fx::ZERO;
        it.angle = Angle(0);
        let h = w.body.create_laser(it);
        let i = w.body.lasers.get(h).unwrap();
        spawn_field(&mut w, 32000, 0, 20, FIELD_CLEAR_BULLETS, 9999);
        step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(
            w.body.lasers.state[i], LASER_ACTIVE,
            "远处的清弹区够不着，激光不被取消"
        );
    }
}
