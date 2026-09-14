//! 符卡计器机构（spec 2026-07-24）：记账归引擎、控制归脚本。逐帧推进挂 settle 符卡趟。
//! `SpellSlot` POD 入 WorldBody（checksum/copy_into/SaveBytes 三件套）。
//!
//! **A2 拍板范围修订**（评审 2026-07-24）：符卡记账（计时/bonus 衰减/miss·bomb 作废资格/
//! 超时判定/结算入分）收归本机构；换卡/阶段切换/弹幕行为仍归脚本（`boss_ui` 仍是表现
//! 公告板，世界逻辑不读它）。四个世界侧 API（`spell_begin_internal`/`spell_end_by_owner`/
//! `spell_frames_left_of`/`settle_spells`）供 syscall 绑定层（M1，另刀）直通。

use crate::enemy::{ENEMY_DYING, EnemyHandle};
use crate::math::Fx;
use crate::player::LIFE_ALIVE;
use crate::world::WorldBody;

/// 符卡计器槽（每 boss 一个；spec 2026-07-24 §2）。全字段 POD；checksum/copy_into/SaveBytes
/// 三件套照新字段四件套清单落（尺寸哨兵会逼）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct SpellSlot {
    /// 0 = 空闲。
    pub active: u8,
    /// bit0 [`SPELL_SURVIVAL`]（耐久卡）/ bit1 [`SPELL_NO_CLEAR`]（退订结束清弹）。
    pub flags: u8,
    /// 资格：1 = 仍可收卡；miss/bomb 即时清 0。
    pub capture_ok: u8,
    /// 显式占位（P6 全量校验，零初始化合法）。
    pub _pad: u8,
    pub spell_id: u16,
    /// 绑定 boss 敌句柄（宣言者 owner）。
    pub boss_index: u16,
    pub boss_gen: u16,
    /// 本槽当前占用的代际戳（ABA 修复，复审 Task 2）：`spell_begin_internal` 成功时取
    /// `WorldBody::spell_seq[slot]` 的新值写入（只增，槽结算清零无妨——inactive 槽的 gate
    /// 先被 `active` 挡）。`Task.spell_epoch` 捕获绑定当刻的这个值；相位 2 调度门禁除了看
    /// `active` 还须比较 epoch——同槽换卡（卡 A 结束→槽清→同趟卡 B begin）会让槽复用出新
    /// epoch，旧卡残留任务的 epoch 因而与新槽不匹配而被杀，不会与新卡并发（镜像 `boss_gen`
    /// 的代际身份先例）。
    pub epoch: u16,
    /// 时限余帧；0 即超时判定点。
    pub frames_left: u16,
    /// 破卡血线：绑定敌 hp≤此值 → 自动收卡；伤害对本敌下钳至此（非死）。
    pub hp_threshold: i32,
    /// begin 当刻绑定敌 hp（逐卡血条分母：`(hp − thr) / (hp_start − thr)`）。
    pub hp_start: i32,
    /// 当前 bonus（分）。
    pub bonus_now: u32,
    /// 衰减地板 = bonus0 / 10（整数除，begin 时定格）。
    pub bonus_floor: u32,
    /// = (bonus0 - floor) / time_limit（整数除，begin 时定格）。
    pub dec_per_frame: u32,
}

pub const SPELL_SURVIVAL: u8 = 1 << 0;
pub const SPELL_NO_CLEAR: u8 = 1 << 1;
/// 非符段（boss 换段刀 2026-09-14 spec §3.1）：计时/血线/模式随段/血条照旧；不宣言、bonus 恒 0、
/// SURVIVAL 位忽略；结算不付分、不发 CAPTURED/FAILED/REQ_SPELL_RESULT，改发 `EVT_PHASE_ENDED`。
pub const SPELL_NONSPELL: u8 = 1 << 2;

/// 结束方式（`WorldBody::spell_last_result` 取值 / `EVT_PHASE_ENDED.data[1]`；0 = 该槽还没结束过）。
pub const SPELL_END_HP: u8 = 1;
pub const SPELL_END_TIMEOUT: u8 = 2;
pub const SPELL_END_MANUAL: u8 = 3;

/// 结束原因（`EVT_SPELL_FAILED.data[1]`）。
pub(crate) const SPELL_FAIL_CAPTURE_LOST: i32 = 1;
pub(crate) const SPELL_FAIL_TIMEOUT: i32 = 2;

/// bonus 逐帧衰减（纯函数，I1 整数衰减）：饱和减法 + 地板钳制。`settle_spells` 与单测共用，
/// 独立于 World 便于判别式直验（衰减系数是整除定格值，逐帧只剩减法与比较）。
fn decay_bonus(now: u32, floor: u32, dec: u32) -> u32 {
    now.saturating_sub(dec).max(floor)
}

/// 逐卡血条比例（真定点除，钳 `[0,1]`）：分母 = `hp_start − hp_threshold`（**非** `hp_max`——
/// 多卡序里每张卡的满条起点是"这张卡开始时的 hp"，见 spec §9-6 判别）。分母 ≤0（退化：
/// threshold==hp_start）视为满条，避免除零/负比例。
fn hp_ratio(hp: i32, hp_start: i32, hp_threshold: i32) -> Fx {
    let denom = hp_start - hp_threshold;
    if denom <= 0 {
        return Fx::ONE;
    }
    let r = Fx::from_int(hp - hp_threshold) / Fx::from_int(denom);
    if r.raw() < 0 {
        Fx::ZERO
    } else if r.raw() > Fx::ONE.raw() {
        Fx::ONE
    } else {
        r
    }
}

impl WorldBody {
    /// settle 符卡趟（相位 7 settle 尾调用，spec §3 五步，按槽升序；`_tables` 现未消费，
    /// 签名与其余相位函数对齐，供将来符卡配置搬进表时零成本扩展）。
    /// 全部 active 符卡槽当场失格（玩法刀）。两个调用点：`try_stop`（冻结期间轮询不跑）、
    /// `rewind_landed`（快照带回了被弹前的资格）。
    pub(crate) fn void_spell_captures(&mut self) {
        for s in self.spells.iter_mut() {
            if s.active != 0 {
                s.capture_ok = 0;
            }
        }
    }

    pub(crate) fn settle_spells(&mut self, _tables: &crate::tables::WorldTables) {
        for slot in 0..crate::boss::MAX_BOSSES {
            if self.spells[slot].active == 0 {
                continue;
            }
            // 1. 资格轮询作废（先于一切）：中弹入决死窗 → capture_ok 清 0。停止不在这里——
            //    冻结期间 settle 不跑，由 `try_stop` 触发点调 `void_spell_captures`（玩法刀）。
            if self.players[0].life_state != LIFE_ALIVE {
                self.spells[slot].capture_ok = 0;
            }
            // 2. bonus 衰减（饱和减法 + 地板钳制）。
            self.spells[slot].bonus_now = decay_bonus(
                self.spells[slot].bonus_now,
                self.spells[slot].bonus_floor,
                self.spells[slot].dec_per_frame,
            );
            let s = self.spells[slot];
            let boss = EnemyHandle {
                index: s.boss_index,
                generation: s.boss_gen,
            };
            // 3. 破卡自动检测（HP 路径）：句柄失效 / ENEMY_DYING / hp≤threshold → 收卡结算。
            let hp_break = match self.enemies.get(boss) {
                None => true,
                Some(i) => {
                    self.enemies.flags[i] & ENEMY_DYING != 0 || self.enemies.hp[i] <= s.hp_threshold
                }
            };
            if hp_break {
                self.settle_one_spell(slot, SPELL_END_HP);
                continue;
            }
            // 4. 超时判定（耐久卡活到超时即收卡点、普通卡超时恒 FAILED——captured 口径在
            //    `settle_one_spell` 内按 flags 推导，boss 换段刀）。
            if s.frames_left == 0 {
                self.settle_one_spell(slot, SPELL_END_TIMEOUT);
                continue;
            }
            self.spells[slot].frames_left -= 1;
            // 5. boss_ui 自动喂（active 期间机械覆写；`phase_left` 不动——阶段规划归脚本）。
            if let Some(i) = self.enemies.get(boss) {
                let ratio = hp_ratio(self.enemies.hp[i], s.hp_start, s.hp_threshold);
                self.boss_ui[slot].enemy = boss;
                self.boss_ui[slot].spell_id = s.spell_id;
                self.boss_ui[slot].timer_frames = self.spells[slot].frames_left;
                self.boss_ui[slot].active = 1;
                self.boss_ui[slot].hp_ratio = ratio;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EVT_SPELL_CAPTURED, EVT_SPELL_DECLARED, EVT_SPELL_FAILED, Event};
    use crate::world::test_support::spawn_enemy;

    /// 造一个只含一个 boss 敌人的世界（世界 API 直驱，不经 syscall——那是 Task 2）。
    fn world_with_boss(hp: i32) -> (Box<crate::step::World>, EnemyHandle) {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 80, hp);
        (w, h)
    }

    fn last_event(w: &crate::step::World) -> Event {
        w.body.frame_events[(w.body.frame_events_len - 1) as usize]
    }

    #[test]
    fn bonus_decays_linearly_to_floor() {
        // begin(限 100 帧, bonus 1000) → floor=100, dec=(1000-100)/100=9
        let floor = 1000u32 / 10;
        let dec = (1000u32 - floor) / 100;
        assert_eq!((floor, dec), (100, 9));
        let mut bonus = 1000u32;
        for _ in 0..10 {
            bonus = decay_bonus(bonus, floor, dec);
        }
        assert_eq!(bonus, 1000 - 90, "10 帧后精确值 910");
        // 推进 200 帧（远超到达地板所需的 ~100 帧）→ 钳地板不再降
        let mut bonus2 = 1000u32;
        for _ in 0..200 {
            bonus2 = decay_bonus(bonus2, floor, dec);
        }
        assert_eq!(bonus2, floor, "超限后地板钳制");
    }

    /// 招牌不变量（判别式）：hp=1000、卡 threshold=300，一发 5000 伤害 → hp==300（非死非负）；
    /// 对照同一发在无符卡世界 → hp≤0 dying——证下钳只在符卡条件下生效，非无条件 max。
    #[test]
    fn damage_clamps_at_threshold_only_under_spell() {
        use crate::shots::ShotInit;
        let shot_at = |x: Fx, y: Fx, dmg: u16| ShotInit {
            x,
            y,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: dmg,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        };

        // 有卡：hp=1000, threshold=300
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 100, 1000, 0, 300));
        let bi = w.body.enemies.get(boss).unwrap();
        let (bx, by) = (w.body.enemies.x[bi], w.body.enemies.y[bi]);
        w.body.create_player_shot(shot_at(bx, by, 5000));
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.enemies.hp[bi], 300, "5000 伤害打不穿血线，钳在 300");
        assert_eq!(
            w.body.enemies.flags[bi] & ENEMY_DYING,
            0,
            "钳制后非死不误标 dying"
        );

        // 对照：无符卡，同一发 → 照常打穿致死
        let (mut w2, boss2) = world_with_boss(1000);
        let bi2 = w2.body.enemies.get(boss2).unwrap();
        let (bx2, by2) = (w2.body.enemies.x[bi2], w2.body.enemies.y[bi2]);
        w2.body.create_player_shot(shot_at(bx2, by2, 5000));
        #[cfg(debug_assertions)]
        {
            w2.body.phase_guard = crate::world::PH_COLLIDE;
        }
        w2.body.collide(&crate::tables::TABLES_V0);
        w2.body.settle(&crate::tables::TABLES_V0);
        assert!(
            w2.body.enemies.hp[bi2] <= 0,
            "无符卡时同发照常打穿致死（对照）"
        );
    }

    /// 破卡血线自动收卡：打到 hp≤threshold → CAPTURED，score += bonus_now（含本帧已衰减一次
    /// 的精确值），事件/req 正确，槽清空。
    #[test]
    fn hp_breakpoint_auto_captures_and_pays_bonus() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 6, 100, 1000, 0, 300));
        let bi = w.body.enemies.get(boss).unwrap();
        let (bx, by) = (w.body.enemies.x[bi], w.body.enemies.y[bi]);
        w.body.create_player_shot(crate::shots::ShotInit {
            x: bx,
            y: by,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 700, // 1000 - 700 == 300 == threshold，恰命中破卡点，无需下钳
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        let score0 = w.body.players[0].score;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0); // 内含伤害 + 符卡趟同帧
        assert_eq!(w.body.enemies.hp[bi], 300);
        assert_eq!(w.body.spells[0].active, 0, "已收卡清槽");
        // bonus 在同一 settle 调用内先衰减一次：1000 - (1000-100)/100 == 991
        assert_eq!(
            w.body.players[0].score,
            score0 + 991,
            "score 增量恰为已衰减一次的 bonus_now"
        );
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_CAPTURED);
        assert_eq!(ev.data, [6, 991]);
    }

    /// 资格清 0 后到线 → EVT_SPELL_FAILED reason=资格失（1），score 不增。
    #[test]
    fn miss_voids_capture_then_hp_break_fails() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 7, 100, 1000, 0, 300));
        let bi = w.body.enemies.get(boss).unwrap();
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW; // 中弹入决死窗
        w.body.enemies.hp[bi] = 300; // 直接推到血线（隔离测：跳过碰撞管线）
        let score0 = w.body.players[0].score;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].score, score0, "资格失不付分");
        assert_eq!(w.body.spells[0].active, 0, "槽已清空");
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_FAILED);
        assert_eq!(ev.data, [7, SPELL_FAIL_CAPTURE_LOST]);
    }

    /// 超时归零：普通卡 FAILED(超时)；`SPELL_SURVIVAL` 卡 CAPTURED（活到超时即收卡点）。
    #[test]
    fn timeout_normal_fails_survival_captures() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 3, 1, 1000, 0, 0));
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left: 1 → 0（未到判定点）
        assert_eq!(w.body.spells[0].active, 1, "首次调用只递减，未超时");
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left==0 → 超时判定
        assert_eq!(w.body.spells[0].active, 0);
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_FAILED);
        assert_eq!(ev.data, [3, SPELL_FAIL_TIMEOUT]);

        let (mut w2, boss2) = world_with_boss(1000);
        assert!(
            w2.body
                .spell_begin_internal(0, boss2, 4, 1, 1000, SPELL_SURVIVAL, 0)
        );
        let score0 = w2.body.players[0].score;
        w2.body.settle_spells(&crate::tables::TABLES_V0);
        w2.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w2.body.spells[0].active, 0);
        assert!(
            w2.body.players[0].score > score0,
            "耐久卡活到超时即收卡付分"
        );
        let ev2 = last_event(&w2);
        assert_eq!(ev2.kind, EVT_SPELL_CAPTURED);
        assert_eq!(ev2.data[0], 4);
    }

    /// 超时·普通卡对资格无关（Task 1 复审修 Fix 1 之一）：即使资格已先失（决死窗口清
    /// `capture_ok`），普通卡到线仍恒 `FAILED(reason=SPELL_FAIL_TIMEOUT=2)`——不是资格失
    /// （`=1`）。这一格钉的是 `settle_one_spell` 内「普通卡超时先于资格判定」的分支序
    /// （boss 换段刀起由 `cause` 推导 captured/reason）：分支序写反，普通卡超时就会误报资格失。
    #[test]
    fn timeout_normal_fails_with_timeout_reason_even_when_capture_lost() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 13, 1, 1000, 0, 0));
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW; // 资格先失
        let score0 = w.body.players[0].score;
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left: 1 → 0（未到判定点）
        assert_eq!(w.body.spells[0].active, 1, "首次调用只递减，未超时");
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left==0 → 超时判定
        assert_eq!(w.body.spells[0].active, 0);
        assert_eq!(w.body.players[0].score, score0, "普通卡超时不付分");
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_FAILED);
        assert_eq!(
            ev.data,
            [13, SPELL_FAIL_TIMEOUT],
            "reason 恒为超时（=2），不是资格失（=1）——资格无关"
        );
    }

    /// 超时·耐久卡资格失（Task 1 复审修 Fix 1 之二）：资格先失（决死窗口）再到线 →
    /// `FAILED(reason=SPELL_FAIL_CAPTURE_LOST=1)`（与资格在时"活到超时即收卡"CAPTURED
    /// 分叉，见 `timeout_normal_fails_survival_captures` 的资格在分支）。
    #[test]
    fn timeout_survival_fails_with_capture_lost_reason_when_ineligible() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(
            w.body
                .spell_begin_internal(0, boss, 14, 1, 1000, SPELL_SURVIVAL, 0)
        );
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW; // 资格先失
        let score0 = w.body.players[0].score;
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left: 1 → 0
        assert_eq!(w.body.spells[0].active, 1, "首次调用只递减，未超时");
        w.body.settle_spells(&crate::tables::TABLES_V0); // frames_left==0 → 超时判定
        assert_eq!(w.body.spells[0].active, 0);
        assert_eq!(w.body.players[0].score, score0, "资格失不付分");
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_FAILED);
        assert_eq!(
            ev.data,
            [14, SPELL_FAIL_CAPTURE_LOST],
            "耐久卡资格失时到线是资格失（=1），不是超时（=2）"
        );
    }

    /// 逐卡血条：hp_start=1000 threshold=600 时 hp=1000→ratio=1.0、hp=800→ratio=0.5
    /// （真定点除，非圆心重合式判别）。
    #[test]
    fn per_card_hp_ratio_full_at_start() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 9, 1000, 1000, 0, 600));
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.boss_ui[0].hp_ratio, Fx::ONE, "满血起 ratio=1.0");
        assert_eq!(w.body.boss_ui[0].spell_id, 9);
        let bi = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[bi] = 800;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.boss_ui[0].hp_ratio,
            Fx::from_raw(1 << 15),
            "(800-600)/(1000-600)=0.5"
        );
    }

    /// 多卡序判别（分母是 hp_start−thr 非 hp_max）：卡一 1000→600 收卡后，卡二从 600 血起
    /// hp_start=600（当刻值）→ 立刻满条 ratio=1.0；若误用 hp_max(1000) 会得 0.6，本测试判别之。
    #[test]
    fn sequential_cards_hp_ratio_denominator_is_hp_start_not_hp_max() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 1000, 1000, 0, 600));
        let bi = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[bi] = 600; // 打到线（隔离测：直改）
        w.body.settle_spells(&crate::tables::TABLES_V0); // HP 路径收卡
        assert_eq!(w.body.spells[0].active, 0);
        assert!(w.body.spell_begin_internal(0, boss, 2, 1000, 1000, 0, 0)); // 第二卡，hp_start=600
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.boss_ui[0].hp_ratio,
            Fx::ONE,
            "第二卡起步满条——分母是当刻 hp_start(600) 不是 hp_max(1000)"
        );
    }

    /// `boss_ui` 自动喂：active 期间 enemy/spell_id/timer/active 逐帧命中；`phase_left`
    /// 经 `boss_set` 写后不被机构覆写（阶段规划归脚本）。
    #[test]
    fn boss_ui_auto_feed_leaves_phase_left_untouched() {
        let (mut w, boss) = world_with_boss(1000);
        w.body.boss_set(
            0,
            crate::boss::BossUiSlot {
                phase_left: 5,
                ..Default::default()
            },
        );
        assert!(w.body.spell_begin_internal(0, boss, 11, 100, 1000, 0, 0));
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.boss_ui[0].enemy, boss);
        assert_eq!(w.body.boss_ui[0].spell_id, 11);
        assert_eq!(w.body.boss_ui[0].timer_frames, 99, "首次调用已递减一次");
        assert_eq!(w.body.boss_ui[0].active, 1);
        assert_eq!(
            w.body.boss_ui[0].phase_left, 5,
            "阶段规划归脚本，机构不覆写"
        );
    }

    /// 六字段逐一对比 `BossUiSlot::default()`（判别式纪律：不许只看 `active`）。
    fn assert_boss_ui_default(slot: crate::boss::BossUiSlot) {
        let d = crate::boss::BossUiSlot::default();
        assert_eq!(slot.enemy, d.enemy, "enemy 未清默认");
        assert_eq!(slot.hp_ratio, d.hp_ratio, "hp_ratio 未清默认");
        assert_eq!(slot.spell_id, d.spell_id, "spell_id 未清默认");
        assert_eq!(slot.timer_frames, d.timer_frames, "timer_frames 未清默认");
        assert_eq!(slot.phase_left, d.phase_left, "phase_left 未清默认");
        assert_eq!(slot.active, d.active, "active 未清默认");
    }

    /// B16② 回归：`settle_one_spell` 结算时必须同步清 `boss_ui[slot]`，否则无后续卡的场景里
    /// 公告板无界陈旧——次帧起 `settle_spells` 首行因 `spells[slot].active==0` 整槽跳过，
    /// 不会自然覆写旧值（不止"≤1 帧"陈旧，是永久冻结）。判别式纪律：结算前一帧先证真喂过
    /// 非零值（非圆心重合）；结算后六字段逐一比对 default；再空转 3 帧（无新卡）仍 default
    /// （冻结回归：修复前此处会永久保留结算前的旧值）。
    #[test]
    fn spell_settle_clears_boss_ui_slot() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 15, 100, 1000, 0, 300));
        w.body.settle_spells(&crate::tables::TABLES_V0); // 喂一次真值
        assert_eq!(
            w.body.boss_ui[0].active, 1,
            "结算前一帧已喂非零态（反向对照：证明测试真走到非零态，非圆心重合）"
        );

        // 打到破卡线（隔离测：直改 hp，路数同 miss_voids_capture_then_hp_break_fails）。
        let bi = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[bi] = 300;
        w.body.settle_spells(&crate::tables::TABLES_V0); // hp_break → settle_one_spell 结算
        assert_eq!(w.body.spells[0].active, 0, "已收卡清槽");
        assert_boss_ui_default(w.body.boss_ui[0]);

        // 冻结回归：无新卡再空转 3 帧，公告板须继续保持 default（修复前会永久保留旧值）。
        for _ in 0..3 {
            w.body.settle_spells(&crate::tables::TABLES_V0);
            assert_boss_ui_default(w.body.boss_ui[0]);
        }
    }

    /// `hp_break` 三路 OR 的 `ENEMY_DYING` 分支判别**①·被打死**（Task 1 复审修 Fix 2）：经真
    /// `damage_enemy`→`ENEMY_DYING` 路径打死 boss（非手写 `hp[i]=0`），断言死亡触发收卡结算。
    ///
    /// **判别设计的关键取舍**：字面"最终卡 `threshold=0`"打不出判别力——`damage_enemy` 里
    /// `hp<=0` 才标 `ENEMY_DYING`，而 `threshold=0` 时 hp_break 第三路 `hp<=s.hp_threshold`
    /// 恰好是同一个条件（`hp<=0` ⇔ `hp<=0`），删 `ENEMY_DYING` 那路 OR 测试仍绿（已实测，见
    /// task-1-report.md 复审修节的红绿证据）；`threshold>0` 时下钳会先兜底把 hp 摁在
    /// threshold（>0）之上，`ENEMY_DYING` 反而永远不会置位——两种取值下第三路都独立盖过它。
    /// 故**在被打死这条路径上**，唯一能让 `hp<=0`（`ENEMY_DYING` 置位判据）与
    /// `hp<=s.hp_threshold`（第三路判据）分道的取值是 **threshold 严格 < 0**（校验只拒
    /// `threshold>hp`，不拒负值，故为合法输入）：hp 精确落 0 时 `0<=0` 假（threshold=-1）
    /// 但 `hp<=0` 真——`ENEMY_DYING` 成为唯一触发源。
    ///
    /// **订正（敌人死亡效果刀 T4，2026-07-30）**：上一段的"唯一"只对**被打死**这条路径成立，
    /// 它的隐含前提"置 `ENEMY_DYING` 的路径只有 `hp<=0`"如今已不成立。置旗的路径现有三条：
    /// - **被打死**（`damage_enemy` 的 `hp<=0` 分支）—— 本测试，需 threshold<0 才判别；
    /// - **D9 自燃**（`ecl::vm::run_tasks` 的 `Exec::End` 分支）—— **只置旗、完全不碰 hp**，
    ///   故满血绑卡 boss 的主任务一返回，第一路（句柄失效）与第三路（`hp<=threshold`）双假，
    ///   `ENEMY_DYING` 是唯一触发源。判别②就走这条，`threshold>0` 即可，测试住
    ///   `ecl::vm::tests::spell_bound_boss_self_destruct_settles_spell_with_hp_above_threshold`
    ///   （脚手架要 `async_image`/`spawn_sub_internal`，故与其余 D9 测试同居 vm.rs）；
    /// - **脚本 `die()`**（`SYS_DIE`→`kill_enemy`）—— **不**属于上一类：`kill_enemy` 会
    ///   `hp.min(0)`，`threshold>=0` 时第三路照样同真，判别力与本测试同源（都要负 threshold），
    ///   所以它不值得再补一条同构测试。
    #[test]
    fn boss_death_via_enemy_dying_flag_triggers_hp_break() {
        use crate::shots::ShotInit;
        let (mut w, boss) = world_with_boss(5);
        assert!(w.body.spell_begin_internal(0, boss, 21, 100, 1000, 0, -1));
        let bi = w.body.enemies.get(boss).unwrap();
        let (bx, by) = (w.body.enemies.x[bi], w.body.enemies.y[bi]);
        w.body.create_player_shot(ShotInit {
            x: bx,
            y: by,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 5, // == hp：真 damage_enemy 精确击杀，threshold<=0 不下钳，hp 落 0（非负）
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        let score0 = w.body.players[0].score;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0); // 内含伤害（真标 ENEMY_DYING）+ 符卡趟同帧
        assert_eq!(
            w.body.enemies.hp[bi], 0,
            "threshold<0 不触发下钳，hp 精确落 0（非负）"
        );
        assert_ne!(
            w.body.enemies.flags[bi] & ENEMY_DYING,
            0,
            "damage_enemy 真标 dying（非手写）"
        );
        assert_eq!(
            w.body.spells[0].active, 0,
            "ENEMY_DYING 触发收卡结算（hp<=threshold 此处为假，唯一触发源是 dying 旗）"
        );
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_SPELL_CAPTURED, "资格在 → CAPTURED");
        assert_eq!(ev.data[0], 21);
        assert!(w.body.players[0].score > score0, "收卡付分");
    }

    /// begin 成功：DECLARED 事件 + `REQ_SPELL_DECLARE` req 的字段/参数形状。
    #[test]
    fn begin_success_emits_declared_event_and_req() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(
            w.body
                .spell_begin_internal(0, boss, 42, 100, 1000, SPELL_SURVIVAL, 300)
        );
        assert_eq!(w.body.frame_events_len, 1);
        let ev = w.body.frame_events[0];
        assert_eq!(ev.kind, EVT_SPELL_DECLARED);
        assert_eq!(ev.a_index, boss.index);
        assert_eq!(ev.a_gen, boss.generation);
        assert_eq!(ev.data, [42, 1000]);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, crate::consts::REQ_SPELL_DECLARE);
        assert_eq!(reqs[0].args, [42, 1000, 100, 1, 0, 0], "survival_bit=1");
    }

    /// begin 坏参四路（slot 越界/time_limit=0/threshold>hp/悬垂 boss）→ P4-b 计数 + 零副作用；
    /// 重复 begin 同槽 → 拒。
    #[test]
    fn begin_bad_args_are_guarded_no_op() {
        let (mut w, boss) = world_with_boss(100);
        let cv0 = w.body.diag.contract_viol;
        assert!(
            !w.body
                .spell_begin_internal(crate::boss::MAX_BOSSES, boss, 1, 10, 100, 0, 0)
        );
        assert!(!w.body.spell_begin_internal(0, boss, 1, 0, 100, 0, 0));
        assert!(!w.body.spell_begin_internal(0, boss, 1, 10, 100, 0, 999));
        assert!(
            !w.body
                .spell_begin_internal(0, EnemyHandle::NULL, 1, 10, 100, 0, 0)
        );
        assert_eq!(w.body.diag.contract_viol, cv0 + 4);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.spells[0].active, 0, "全部失败零副作用");

        assert!(w.body.spell_begin_internal(0, boss, 1, 10, 100, 0, 0));
        assert!(
            !w.body.spell_begin_internal(0, boss, 2, 10, 100, 0, 0),
            "该槽已 active 拒"
        );
    }

    /// 逃生舱口 `spell_end_by_owner`：HP 路径结算 + 清槽；`spell_frames_left_of` 读族
    /// active 时命中、清槽后 -1；重复调用 no-op 不计数（同 `spell_end` 语义）。
    #[test]
    fn spell_end_by_owner_settles_and_frames_left_of_tracks_binding() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 5, 200, 1000, 0, 0));
        assert_eq!(w.body.spell_frames_left_of(boss), 200);
        let score0 = w.body.players[0].score;
        w.body.spell_end_by_owner(boss); // 资格在 → CAPTURED
        assert_eq!(w.body.spells[0].active, 0);
        assert!(w.body.players[0].score > score0);
        assert_eq!(w.body.spell_frames_left_of(boss), -1, "无绑定 -1");

        let cv0 = w.body.diag.contract_viol;
        w.body.spell_end_by_owner(boss); // 重复调用：no-op 不计数
        assert_eq!(w.body.diag.contract_viol, cv0, "逃生舱口重复调用安全");
    }

    /// 超时钉血（boss 换段刀 spec §3.2 ①）：普通卡 / 耐久卡 / 非符段三种超时都把 hp 钉到血线，
    /// 下一段 `hp_start` 从血线起（剩血不漏段）。
    #[test]
    fn timeout_pins_hp_to_threshold_for_all_three_kinds() {
        for flags in [0u8, SPELL_SURVIVAL, SPELL_NONSPELL] {
            let (mut w, boss) = world_with_boss(1000);
            assert!(w.body.spell_begin_internal(0, boss, 3, 1, 1000, flags, 300));
            w.body.settle_spells(&crate::tables::TABLES_V0);
            w.body.settle_spells(&crate::tables::TABLES_V0);
            let i = w.body.enemies.get(boss).unwrap();
            assert_eq!(w.body.enemies.hp[i], 300, "flags={flags}：超时钉到血线");
            assert!(w.body.spell_begin_internal(0, boss, 4, 60, 1000, 0, 0));
            assert_eq!(
                w.body.spells[0].hp_start, 300,
                "flags={flags}：下一段从血线起"
            );
        }
    }

    /// 钉血只在超时路径、且是 `min` 不是赋值：HP 路径结算时 hp 已低于血线也不被抬回血线。
    #[test]
    fn hp_break_does_not_raise_hp_to_threshold() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 3, 60, 1000, 0, 300));
        let i = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[i] = 200;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spells[0].active, 0);
        assert_eq!(w.body.enemies.hp[i], 200);
    }

    /// 超时结算铺的全屏清弹区带 FIELD_NO_STAR；HP 路径不带（spec §3.2 ③）。
    #[test]
    fn timeout_clear_field_has_no_star_bit_hp_break_does_not() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 3, 1, 0, 0, 0));
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        let f = w
            .body
            .fields
            .iter_alive()
            .next()
            .expect("超时结算应铺清弹区");
        assert_eq!(w.body.fields.flags[f], FIELD_CLEAR_BULLETS | FIELD_NO_STAR);

        let (mut w2, boss2) = world_with_boss(1000);
        assert!(w2.body.spell_begin_internal(0, boss2, 3, 60, 0, 0, 300));
        let i = w2.body.enemies.get(boss2).unwrap();
        w2.body.enemies.hp[i] = 300;
        w2.body.settle_spells(&crate::tables::TABLES_V0);
        let f2 = w2
            .body
            .fields
            .iter_alive()
            .next()
            .expect("HP 结算应铺清弹区");
        assert_eq!(w2.body.fields.flags[f2], FIELD_CLEAR_BULLETS);
    }

    /// 非符段（spec §3.1/§3.2 ②）：不宣言、bonus 恒 0、SURVIVAL 位被忽略、不付分、
    /// 不发 CAPTURED/FAILED/REQ_SPELL_RESULT，结束发 EVT_PHASE_ENDED[spell_id, cause]。
    #[test]
    fn nonspell_skips_declare_bonus_and_result_and_emits_phase_ended() {
        use crate::events::{EVT_PHASE_ENDED, EVT_SPELL_DECLARED};
        let (mut w, boss) = world_with_boss(1000);
        let score0 = w.body.players[0].score;
        assert!(w.body.spell_begin_internal(
            0,
            boss,
            9,
            1,
            5000,
            SPELL_NONSPELL | SPELL_SURVIVAL,
            300
        ));
        assert_eq!(w.body.spells[0].bonus_now, 0);
        assert!(
            (0..w.body.frame_events_len as usize)
                .all(|k| w.body.frame_events[k].kind != EVT_SPELL_DECLARED)
        );
        assert!(
            w.body
                .take_requests()
                .iter()
                .all(|r| r.id != crate::consts::REQ_SPELL_DECLARE)
        );
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_PHASE_ENDED);
        assert_eq!(ev.data, [9, SPELL_END_TIMEOUT as i32]);
        assert_eq!(w.body.players[0].score, score0);
        assert!(
            w.body
                .take_requests()
                .iter()
                .all(|r| r.id != crate::consts::REQ_SPELL_RESULT)
        );
    }

    /// `spell_last_result`（spec §3.3）：初值 0；三种结束方式各记 1/2/3；
    /// 新 begin 不清；只动本槽。
    #[test]
    fn spell_last_result_records_cause_and_survives_next_begin() {
        let (mut w, boss) = world_with_boss(1000);
        assert_eq!(w.body.spell_last_result, [0; crate::boss::MAX_BOSSES]);
        assert!(w.body.spell_begin_internal(0, boss, 1, 60, 0, 0, 900));
        let i = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[i] = 900;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_HP);

        assert!(w.body.spell_begin_internal(0, boss, 2, 1, 0, 0, 800));
        assert_eq!(w.body.spell_last_result[0], SPELL_END_HP, "begin 不清读口");
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_TIMEOUT);

        assert!(w.body.spell_begin_internal(0, boss, 3, 60, 0, 0, 0));
        w.body.spell_end_by_owner(boss);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_MANUAL);
        assert_eq!(w.body.spell_last_result[1], 0, "别的槽不动");
    }
}
